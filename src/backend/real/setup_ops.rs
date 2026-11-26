//! The setup engine's side on a real lt device (`ligus_runtime::setup`).
//!
//! lt is a person's device (`Surface::Term`), like the CLI, so the flows
//! that need a person elsewhere hand off as the engine says: a box joins
//! from the box (`ligus setup box`), a phone pairs with `ligus pair` in the
//! person's own terminal, an invite is made where its code is shown (the
//! desktop, or lt's own Invite with `I`). Everything else is done here:
//!
//! * agents: on the runtimes lt runs (`local:<provider>[/<model>]`, or a
//!   keyless workspace runtime); a runtime "defined" for a new agent is
//!   only that id (nothing is written until the agent is);
//! * tools: this device's MCP servers ([`super::mcp`]);
//! * folders: the workspace's repositories and this device's bindings
//!   (`repos::bind`), and lt's checkout for the workspace when none is set;
//! * relay: this device's copy of the workspace, its token in the secret
//!   store, then the sync loop restarted on the new relay.
//!
//! Each method runs on the blocking pool (the backend calls the engine in
//! `spawn_blocking`) and returns facts, never a secret.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar};
use std::time::{Duration, Instant};

use ligus_core::WorkspaceRole;
use ligus_runtime::cli_detect::{self, DetectEnv, LoginState};
use ligus_runtime::mcp::McpServerSpec;
use ligus_runtime::model_catalog::ModelList;
use ligus_runtime::setup::validate::{origin, same_origin};
use ligus_runtime::setup::{
    AgentFact, FolderFact, HarnessFact, HostFact, InviteFact, McpFact, OpsError, OpsResult, ProbeFact, RepoFact, RuntimeFact, RuntimeSpec,
    SetupOps, Surface, TestTurn, service,
};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{Inner, detect, lock};
use crate::backend::Event;
use crate::backend::model::WorkspaceId;

pub(crate) struct RealOps {
    inner: Arc<Inner>,
    ws: Option<Uuid>,
    env: DetectEnv,
    /// Runtimes "defined" during this run: the engine's id → lt's `local:` id.
    defined: HashMap<String, String>,
    /// Tool tokens this run saved (by server id).
    tokens_put: std::collections::HashSet<String>,
    /// A run (not a form) waits for a sign-in check still under way.
    wait_for_checks: bool,
}

/// What the Antigravity harness shows while `agy models` runs.
const CHECKING: &str = crate::setup::CHECKING_SIGN_IN;
/// How long a sign-in answer is reused before `agy models` runs again.
const AGY_FRESH: Duration = Duration::from_secs(5 * 60);
/// The longest a run waits for a check under way (`agy models` itself is
/// capped at 20 s by `cli_detect`).
const AGY_WAIT: Duration = Duration::from_secs(25);

/// The Antigravity sign-in check, shared by every form and run on this
/// device: one `agy models` at a time, on its own thread, its answer kept
/// for a few minutes. `cli_detect` runs it with stdin from /dev/null and
/// stdout and stderr captured, so nothing reaches lt's terminal.
#[derive(Default)]
pub(crate) struct AgyCheck {
    state: std::sync::Mutex<AgyState>,
    done: Condvar,
}

#[derive(Default)]
enum AgyState {
    #[default]
    Idle,
    Running,
    Done {
        path: Option<PathBuf>,
        at: Instant,
        login: LoginState,
        detail: String,
    },
}

impl AgyCheck {
    /// The answer for the `agy` on `env`'s `PATH`, if one is known: else
    /// the check starts (or goes on) in the background, `on_done` running
    /// when it ends. With `wait`, waits for it instead (at most
    /// [`AGY_WAIT`]).
    pub(crate) fn result(
        self: &Arc<Self>,
        env: &DetectEnv,
        wait: bool,
        on_done: impl FnOnce() + Send + 'static,
    ) -> Option<(LoginState, String)> {
        let path = cli_detect::find_on_path("agy", &env.path);
        let mut st = lock(&self.state);
        let fresh = |st: &AgyState| match st {
            AgyState::Done { path: p, at, login, detail } if *p == path && at.elapsed() < AGY_FRESH => Some((*login, detail.clone())),
            _ => None,
        };
        if let Some(r) = fresh(&st) {
            return Some(r);
        }
        if !matches!(*st, AgyState::Running) {
            *st = AgyState::Running;
            let (me, env, path) = (self.clone(), env.clone(), path.clone());
            std::thread::spawn(move || {
                let (login, detail) = cli_detect::detect_cli_agents(&env, true)
                    .into_iter()
                    .find(|c| c.id == "antigravity")
                    .map(|c| (c.login, c.login_detail))
                    .unwrap_or((LoginState::Unknown, String::new()));
                *lock(&me.state) = AgyState::Done { path, at: Instant::now(), login, detail };
                me.done.notify_all();
                on_done();
            });
        }
        if !wait {
            return None;
        }
        let (st, _) = self.done.wait_timeout_while(st, AGY_WAIT, |s| matches!(s, AgyState::Running)).unwrap_or_else(|e| e.into_inner());
        match &*st {
            AgyState::Done { login, detail, .. } => Some((*login, detail.clone())),
            _ => None,
        }
    }
}

fn failed(e: impl std::fmt::Display) -> OpsError {
    OpsError::Failed(ligus_runtime::secret_redact::redacted(&e.to_string()))
}

impl RealOps {
    pub(crate) fn new(inner: Arc<Inner>, ws: Option<Uuid>) -> Self {
        let env = inner.cfg.detect_env.clone().unwrap_or_else(detect::process_env);
        RealOps { inner, ws, env, defined: HashMap::new(), tokens_put: Default::default(), wait_for_checks: false }
    }

    /// For a run: harness facts wait for a sign-in check under way, so its
    /// checks say what is known rather than "can't tell".
    pub(crate) fn for_run(inner: Arc<Inner>, ws: Option<Uuid>) -> Self {
        RealOps { wait_for_checks: true, ..Self::new(inner, ws) }
    }

    fn ws(&self) -> OpsResult<Uuid> {
        self.ws.ok_or_else(|| OpsError::Failed("create or join a workspace first".into()))
    }

    /// Read with the read service (pinned to the workspace's founder).
    fn read<T: Default>(&self, f: impl FnOnce(&ligus_runtime::ChatService, Uuid) -> T) -> T {
        let Some(ws) = self.ws else { return T::default() };
        self.inner.read(ws, |svc| Ok(f(svc, ws))).unwrap_or_default()
    }

    /// Write with the write service (pinned to the workspace's founder).
    fn write<T>(&self, f: impl FnOnce(&mut ligus_runtime::ChatService, Uuid) -> OpsResult<T>) -> OpsResult<T> {
        let ws = self.ws()?;
        let core = self.inner.core().map_err(|e| OpsError::Failed(e.0))?;
        self.inner.may_sign(&core, ws).map_err(|e| OpsError::Failed(e.0))?;
        let mut svc = lock(&core.svc);
        svc.set_trusted_founder(self.inner.founder(ws));
        f(&mut svc, ws)
    }

    /// The model server lt's local-model agents talk to.
    fn ollama(&self) -> String {
        self.inner.cfg.ollama_url.clone().unwrap_or_else(|| detect::OLLAMA_DEFAULT.into())
    }

    fn abs(&self, path: &str) -> String {
        let p = crate::setup::complete::expand(path.trim(), dirs::home_dir().as_deref());
        std::fs::canonicalize(&p).unwrap_or(p).display().to_string()
    }
}

/// `localhost` and `127.0.0.1` are one machine.
fn loopback_same(a: &str, b: &str) -> bool {
    let norm = |u: &str| origin(u).replace("://localhost", "://127.0.0.1");
    norm(a) == norm(b)
}

impl SetupOps for RealOps {
    fn surface(&self) -> Surface {
        Surface::Term
    }

    fn my_role(&self) -> Option<WorkspaceRole> {
        self.read(service::my_role)
    }
    fn lockdown(&self) -> bool {
        self.read(|svc, ws| service::lockdown(svc, ws).0)
    }
    fn lockdown_mcp_allowed(&self) -> Vec<String> {
        self.read(|svc, ws| service::lockdown(svc, ws).1)
    }
    fn relay_url(&self) -> Option<String> {
        self.inner.saved(self.ws?).ok()?.relay().map(str::to_string)
    }

    // agent
    fn harnesses(&self) -> Vec<HarnessFact> {
        let mut facts = service::harnesses(&self.env, false);
        // Antigravity keeps its sign-in in the keychain: `agy models` says
        // (read-only, no quota), checked off the UI thread ([`AgyCheck`]).
        if let Some(f) = facts.iter_mut().find(|f| f.id == "antigravity" && f.installed) {
            let inner = self.inner.clone();
            let on_done = move || inner.emit(Event::SetupFactsChanged);
            match self.inner.agy.result(&self.env, self.wait_for_checks, on_done) {
                Some((login, detail)) => {
                    f.login = login;
                    f.login_detail = detail;
                }
                None if !self.wait_for_checks => f.login_detail = CHECKING.into(),
                None => {}
            }
        }
        facts
    }
    fn models(&self, harness: &str, endpoint: &str) -> ModelList {
        // No request to an endpoint the engine would refuse.
        if harness == "local" && ligus_runtime::setup::validate::url_problem(endpoint).is_some() {
            return ModelList::none("type a model name");
        }
        service::models(harness, endpoint, &self.env)
    }
    fn runtimes(&self) -> Vec<RuntimeFact> {
        let ollama = self.ollama();
        let mut out: Vec<RuntimeFact> = self.read(|svc, ws| {
            let keyless = svc
                .list_workspace_runtimes(ws)
                .unwrap_or_default()
                .into_iter()
                .filter(|r| matches!(r.credential, ligus_core::WorkspaceCredential::None));
            keyless
                .filter(|r| r.gateway.trim().is_empty())
                .filter_map(|r| {
                    let h = service::harness_of(r.provider, false)?;
                    Some(RuntimeFact {
                        id: r.id.clone(),
                        harness: h.into(),
                        model: r.model.clone(),
                        endpoint: if h == "local" { r.endpoint.clone() } else { String::new() },
                    })
                })
                .collect()
        });
        // The device-local runtimes this workspace's agents already name.
        for a in self.agents() {
            if out.iter().any(|r| r.id == a.runtime_id) {
                continue;
            }
            if let Some((p, model)) = detect::parse_local_id(&a.runtime_id)
                && let Some(h) = service::harness_of(p, false)
            {
                let endpoint = if h == "local" { ollama.clone() } else { String::new() };
                out.push(RuntimeFact { id: a.runtime_id.clone(), harness: h.into(), model, endpoint });
            }
        }
        out
    }
    fn add_runtime(&mut self, spec: &RuntimeSpec) -> OpsResult<()> {
        if !crate::setup::LT_HARNESSES.contains(&spec.harness.as_str()) {
            return Err(OpsError::Failed(format!("lt doesn't run {} agents yet", spec.harness)));
        }
        if spec.harness == "local" && !loopback_same(&spec.endpoint, &self.ollama()) {
            return Err(OpsError::Failed(format!(
                "lt's local-model agents talk to {}; {} isn't it",
                origin(&self.ollama()),
                origin(&spec.endpoint)
            )));
        }
        let p = service::provider_of(&spec.harness).ok_or_else(|| failed(format!("unknown harness {}", spec.harness)))?;
        // Nothing to write: the id names the runtime.
        self.defined.insert(spec.id.clone(), detect::local_id(p, Some(&spec.model)));
        Ok(())
    }
    fn remove_runtime(&mut self, id: &str) -> OpsResult<()> {
        self.defined.remove(id);
        Ok(())
    }
    fn agents(&self) -> Vec<AgentFact> {
        self.read(service::agents)
    }
    fn member_names(&self) -> Vec<String> {
        self.read(service::member_names)
    }
    fn add_agent(&mut self, name: &str, runtime_id: &str) -> OpsResult<Uuid> {
        let ws = self.ws()?;
        let rt = self.defined.get(runtime_id).cloned().unwrap_or_else(|| runtime_id.to_string());
        self.inner.add_agent_here(ws, name, rt).map_err(|e| {
            if e.0.contains("only") || e.0.contains("Viewer") || e.0.contains("not allowed") { OpsError::Refused(e.0) } else { failed(e.0) }
        })
    }
    fn test_turn(&mut self, runtime_id: &str) -> OpsResult<TestTurn> {
        let ws = self.ws()?;
        let rid = self.defined.get(runtime_id).cloned().unwrap_or_else(|| runtime_id.to_string());
        let (rt, _) = super::turns::runtime_for(&self.inner, ws, &rid).map_err(|e| failed(e.0))?;
        service::block_on(async move { service::ping(&rt, None).await }).ok_or_else(|| failed("couldn't run the test turn"))
    }

    // tool
    fn mcp_servers(&self) -> Vec<McpFact> {
        match self.ws {
            Some(ws) => super::mcp::facts(&self.inner.cfg.data_dir, self.inner.secrets.as_ref(), ws),
            None => Vec::new(),
        }
    }
    fn mcp_handshake(&mut self, spec: &McpServerSpec, token: Option<&Zeroizing<String>>) -> OpsResult<Vec<String>> {
        service::mcp_handshake(spec, token)
    }
    fn put_mcp_secret(&mut self, id: &str, url: &str, secret: &Zeroizing<String>) -> OpsResult<()> {
        super::mcp::put_token(self.inner.secrets.as_ref(), self.ws()?, id, url, secret).map_err(failed)?;
        self.tokens_put.insert(id.to_string());
        Ok(())
    }
    fn delete_mcp_secret(&mut self, id: &str, _url: &str) -> OpsResult<()> {
        super::mcp::delete_token(self.inner.secrets.as_ref(), self.ws()?, id).map_err(failed)
    }
    fn add_mcp_server(&mut self, spec: &McpServerSpec, reach: &[Uuid]) -> OpsResult<()> {
        let ws = self.ws()?;
        // A token this run didn't save (one left from an earlier server of
        // this id, or from a run stopped half way) never follows the new one.
        if !self.tokens_put.contains(&spec.id) {
            super::mcp::delete_token(self.inner.secrets.as_ref(), ws, &spec.id).map_err(failed)?;
        }
        super::mcp::add(&self.inner.cfg.data_dir, ws, spec, reach).map_err(failed)
    }
    fn remove_mcp_server(&mut self, id: &str) -> OpsResult<()> {
        super::mcp::remove(&self.inner.cfg.data_dir, self.inner.secrets.as_ref(), self.ws()?, id).map_err(failed)
    }

    // folder
    fn repos(&self) -> Vec<RepoFact> {
        self.read(service::repos)
    }
    fn inspect_folder(&self, path: &str) -> FolderFact {
        let abs = self.abs(path);
        match self.ws {
            Some(ws) => self
                .inner
                .read(ws, |svc| Ok(service::inspect_folder(svc, ws, &abs)))
                .unwrap_or_else(|_| ligus_runtime::setup::validate::inspect_folder(&abs)),
            None => ligus_runtime::setup::validate::inspect_folder(&abs),
        }
    }
    fn add_repo(&mut self, folder: &FolderFact, display_name: &str, default_branch: &str) -> OpsResult<String> {
        let id = self.write(|svc, ws| service::add_repo(svc, ws, folder, display_name, default_branch))?;
        if let Some(ws) = self.ws {
            self.inner.emit(Event::HostsChanged { workspace: WorkspaceId::new(ws.to_string()) });
        }
        Ok(id)
    }
    fn bind_folder(&mut self, repo_id: &str, path: &str) -> OpsResult<()> {
        let abs = self.abs(path);
        let label = self.inner.label();
        self.write(|svc, ws| service::bind_folder(svc, ws, &label, repo_id, &abs))?;
        // lt applies proposals in one checkout per workspace: this one,
        // when none is set yet.
        let ws = WorkspaceId::new(self.ws()?.to_string());
        let has = lock(&self.inner.local).checkouts.contains_key(ws.as_str());
        if !has {
            self.inner.bind(&ws, Some(std::path::Path::new(&abs))).map_err(|e| failed(e.0))?;
        }
        self.inner.emit(Event::HostsChanged { workspace: ws });
        Ok(())
    }
    fn create_scratch(&mut self, name: &str) -> OpsResult<(String, String)> {
        let label = self.inner.label();
        let root = self.inner.scratch_root();
        let made = self.write(|svc, ws| service::create_scratch(svc, ws, &label, &root, &super::scratch::forbidden(), name))?;
        if let Some(ws) = self.ws {
            self.inner.emit(Event::ChatsChanged { workspace: WorkspaceId::new(ws.to_string()) });
            self.inner.emit(Event::HostsChanged { workspace: WorkspaceId::new(ws.to_string()) });
        }
        Ok(made)
    }
    fn box_state(&self) -> OpsResult<ligus_runtime::setup::BoxFact> {
        // lt is a person's device: a box joins from the box itself.
        Err(OpsError::Failed("lt is a person's device; a box joins from the box".into()))
    }
    fn hosts(&self) -> Vec<HostFact> {
        self.read(service::hosts)
    }

    // phone, invite
    fn phones(&self) -> Vec<String> {
        self.read(service::phones)
    }
    fn invites(&self) -> Vec<InviteFact> {
        self.read(service::invites)
    }

    // relay
    fn probe_relay(&self, url: &str, token: Option<&Zeroizing<String>>) -> ProbeFact {
        let Some(saved) = self.ws.and_then(|ws| self.inner.saved(ws).ok()) else {
            return ProbeFact::Unreachable("create or join a workspace first".into());
        };
        // The saved access token goes only to the relay it was saved for.
        let same = saved.relay().is_some_and(|r| same_origin(r, url));
        let token = token.cloned().or_else(|| if same { saved.secrets.token.clone().map(Zeroizing::new) } else { None });
        service::probe_relay(url, token.as_ref(), &saved.room)
    }
    fn switch_relay(&mut self, url: &str, token: Option<&Zeroizing<String>>) -> OpsResult<()> {
        let ws = self.ws()?;
        let mut saved = self.inner.saved(ws).map_err(|e| failed(e.0))?;
        let moved = !saved.relay().is_some_and(|r| same_origin(r, url));
        saved.relay_url = url.to_string();
        match token {
            Some(t) => saved.secrets.token = Some(t.to_string()),
            // The old relay's token never follows the workspace to another.
            None if moved => saved.secrets.token = None,
            None => {}
        }
        lock(&self.inner.list).upsert(saved);
        self.inner.save_list().map_err(|e| failed(e.0))?;
        self.inner.start_loops(ws);
        self.inner.emit(Event::WorkspacesChanged);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localhost_is_this_machine() {
        assert!(loopback_same("http://localhost:11434", "http://127.0.0.1:11434"));
        assert!(!loopback_same("http://localhost:11434", "http://127.0.0.1:8080"));
    }
}
