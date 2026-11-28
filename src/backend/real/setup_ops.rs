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
    AgentEdit, AgentFact, FolderFact, HarnessFact, HostFact, HostSourceFact, InviteFact, McpFact, OpsError, OpsResult, ProbeFact, RepoFact,
    RuntimeFact, RuntimeSpec, SetupOps, Surface, TestTurn, service,
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
    /// Workspace runtimes this run wrote (for its undo).
    shared_defined: std::collections::HashSet<String>,
    /// Tool tokens this run saved (by server id).
    tokens_put: std::collections::HashSet<String>,
    /// A run (not a form) waits for a sign-in check still under way.
    wait_for_checks: bool,
    /// A form's one read of the Antigravity sign-in: every look at the
    /// harnesses while one form is built sees the same answer, so a check
    /// that ends mid-form can't leave it saying "sign-in unknown".
    agy_seen: std::cell::OnceCell<Option<(LoginState, String)>>,
    /// Model lists read while building one form or run, by harness and
    /// endpoint: the engine asks more than once, a CLI lists once.
    models_seen: std::cell::RefCell<HashMap<(String, String), ModelList>>,
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
        RealOps {
            inner,
            ws,
            env,
            defined: HashMap::new(),
            shared_defined: Default::default(),
            tokens_put: Default::default(),
            wait_for_checks: false,
            agy_seen: Default::default(),
            models_seen: Default::default(),
        }
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
            let ask = || {
                let inner = self.inner.clone();
                let on_done = move || inner.emit(Event::SetupFactsChanged);
                self.inner.agy.result(&self.env, self.wait_for_checks, on_done)
            };
            // A run asks every time (it waits for the answer); a form asks once.
            let answer = if self.wait_for_checks { ask() } else { self.agy_seen.get_or_init(ask).clone() };
            match answer {
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
        // A form whose agy sign-in check is still under way doesn't start a
        // second `agy models` for the list: it asks again when the check
        // reports back.
        if harness == "antigravity" && !self.wait_for_checks && matches!(self.agy_seen.get(), Some(None)) {
            return ModelList::none(CHECKING);
        }
        let key = (harness.to_string(), endpoint.to_string());
        if let Some(l) = self.models_seen.borrow().get(&key) {
            return l.clone();
        }
        let l = service::models(harness, endpoint, &self.env);
        self.models_seen.borrow_mut().insert(key, l.clone());
        l
    }
    fn runtimes(&self) -> Vec<RuntimeFact> {
        let ollama = self.ollama();
        // The workspace's runtimes (other hosts can run them; a hosted API's
        // names a secret reference, never a key).
        let mut out: Vec<RuntimeFact> = self.read(service::workspace_runtimes);
        // The device-local runtimes this workspace's agents already name.
        for a in self.agents() {
            if out.iter().any(|r| r.id == a.runtime_id) {
                continue;
            }
            if let Some((p, model)) = detect::parse_local_id(&a.runtime_id)
                && let Some(h) = service::harness_of(p, false)
            {
                let endpoint = if h == "local" { ollama.clone() } else { String::new() };
                out.push(RuntimeFact {
                    id: a.runtime_id.clone(),
                    harness: h.into(),
                    model,
                    endpoint,
                    shared: false,
                    secret_ref: String::new(),
                    provider: ligus_runtime::host_offers::harness_name(p),
                });
            }
        }
        out
    }
    fn add_runtime(&mut self, spec: &RuntimeSpec) -> OpsResult<()> {
        if spec.shared {
            // Another host runs it: a workspace runtime (names and a secret
            // reference only), with the passkey on a shared workspace.
            self.write(|svc, ws| service::add_workspace_runtime(svc, ws, spec))?;
            self.shared_defined.insert(spec.id.clone());
            return Ok(());
        }
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
        if self.shared_defined.remove(id) {
            return self.write(|svc, ws| svc.remove_workspace_runtime(ws, id).map_err(OpsError::from));
        }
        self.defined.remove(id);
        Ok(())
    }
    fn host_sources(&self) -> Vec<HostSourceFact> {
        let Some(ws) = self.ws else { return Vec::new() };
        let beats = self.inner.beats(ws);
        let now = ligus_core::Timestamp::now();
        self.read(|svc, ws| service::host_sources(svc, ws, |id| beats.status_of(id), |h| beats.is_online(h, &now)))
    }
    fn this_device_label(&self) -> String {
        "this computer".into()
    }
    fn passkey_writes(&self) -> bool {
        // A shared workspace holds agents and workspace runtimes for the
        // author's passkey (lt asks the desktop or phone for it).
        self.relay_url().is_some() && self.my_role().is_some()
    }
    fn confirming(&mut self, step: usize, of: usize, what: &str) {
        self.inner.emit(Event::SetupConfirming { step, of, what: what.to_string() });
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
    fn add_agent_placed(&mut self, name: &str, runtime_id: &str, host_id: &str) -> OpsResult<Uuid> {
        if host_id.is_empty() {
            return self.add_agent(name, runtime_id);
        }
        // On a host: one record carrying its name, runtime and host.
        let rt = self.defined.get(runtime_id).cloned().unwrap_or_else(|| runtime_id.to_string());
        let id = self.write(|svc, ws| service::add_agent_placed(svc, ws, name, &rt, host_id))?;
        if let Some(ws) = self.ws {
            self.inner.emit(Event::AgentsChanged { workspace: WorkspaceId::new(ws.to_string()) });
        }
        Ok(id)
    }
    fn change_agent(&mut self, agent_id: Uuid, edit: &AgentEdit) -> OpsResult<()> {
        // A runtime "defined" during this run is lt's own `local:` id.
        let mut edit = edit.clone();
        if let Some(r) = edit.runtime_id.as_mut()
            && let Some(local) = self.defined.get(r.as_str())
        {
            *r = local.clone();
        }
        self.write(|svc, ws| service::change_agent(svc, ws, agent_id, &edit))?;
        if let Some(ws) = self.ws {
            self.inner.emit(Event::AgentsChanged { workspace: WorkspaceId::new(ws.to_string()) });
        }
        Ok(())
    }
    fn rules(&self) -> u32 {
        self.read(service::rules)
    }
    fn local_servers(&self, agent_id: Uuid) -> bool {
        self.read(|svc, ws| service::local_servers(svc, ws, agent_id))
    }
    fn set_local_servers(&mut self, agent_id: Uuid, on: bool) -> OpsResult<()> {
        self.write(|svc, ws| service::set_local_servers(svc, ws, agent_id, on))
    }
    fn test_turn(&mut self, runtime_id: &str) -> OpsResult<TestTurn> {
        self.test_turn_with(runtime_id, false)
    }
    fn test_turn_with(&mut self, runtime_id: &str, local_servers: bool) -> OpsResult<TestTurn> {
        let ws = self.ws()?;
        let rid = self.defined.get(runtime_id).cloned().unwrap_or_else(|| runtime_id.to_string());
        let (rt, _) = super::turns::runtime_for(&self.inner, ws, &rid).map_err(|e| failed(e.0))?;
        service::block_on(async move { service::ping_with(&rt, None, local_servers).await })
            .ok_or_else(|| failed("couldn't run the test turn"))
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
