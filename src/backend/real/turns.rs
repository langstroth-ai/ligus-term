//! Turns this device answers.
//!
//! Both gates go through `ligus_runtime::turn_dispatch` with the same inputs
//! as the desktop: the **local-send** gate when the person posts here
//! ([`post_and_plan`]) and the **relay-sync** gate when a message arrives
//! through sync ([`plan_synced`]). A workspace with no relay, or with one
//! person in it, always answers here. A turn that runs here is claimed first
//! (`claim_turn`), so only one of the owner's devices answers.
//!
//! Before a turn runs, [`guard_turn`] applies what every Ligus host applies
//! (`turn_engine::decide_gate_with`): the author-role gate (H1: below the
//! floor a CLI runs read-only, or not at all), the shell rule (the workspace
//! Owner's, or granted), the chat's sandbox envelope (a lockdown or dearth
//! mode narrows egress, or refuses a runtime it can't confine), and this
//! host's OS sandbox (no sandbox: no CLI agent). The repo gate names the
//! checkout (`turn_engine::repo_checkout`).
//!
//! The turn then runs in the shared engine (`ligus_runtime::turn_engine`),
//! with this device as its host ([`TermHost`]): tools, edits in a per-turn
//! worktree filed as a proposal, hand-offs through this device's Ligus MCP
//! server, every child under the turn's policy. The reply streams as
//! `MessageDelta` events, tool calls as `ToolActivity`; the change feed
//! turns each append into `MessageAdded` / `MessageDelta` / `MessageDone`.

use std::future::Future;
use std::sync::{Arc, Mutex};

use ligus_core::{ChatSession, ModelProviderKind, WorkspaceAgent, WorkspaceRole, WorkspaceRuntime};
use ligus_runtime::ChatService;
use ligus_runtime::ResolvedRuntime;
use ligus_runtime::executor::HostKind;
use ligus_runtime::ligus_mcp::{LigusHost, LigusMcp};
use ligus_runtime::provider::dispatch::StreamActivity;
use ligus_runtime::turn_dispatch::{DispatchTarget, HostBinding, dispatch_target, host_binding, runs_here_linked, synced_user_triggers};
use ligus_runtime::turn_engine::{self, Checkout, EngineOptions, Gated, Responder, ToolOptions, TurnHost, TurnInput};
use uuid::Uuid;

use super::{Inner, one_line, views};
use crate::backend::model::{ChatId, Message, MessageId, ToolCall, WorkspaceId};
use crate::backend::{BackendError, Event, Result};

/// A turn this device will run.
pub struct Plan {
    pub ws: Uuid,
    pub sid: Uuid,
    /// The message it answers.
    pub trigger: Uuid,
    pub agent: Option<WorkspaceAgent>,
    pub runtime_id: String,
}

/// Who answers `target` in `s`: its agent, or the chat's primary runtime.
/// Returns (agent, runtime id, owner, host).
fn responder(s: &ChatSession, target: DispatchTarget) -> Option<(Option<WorkspaceAgent>, String, String, String)> {
    match target {
        DispatchTarget::Agent(id) => {
            let a = s.workspace_agents.iter().find(|a| a.id == id)?.clone();
            let (rt, owner, host) = (a.runtime_id.clone(), a.owner_actor_id.clone(), a.host_id.clone());
            Some((Some(a), rt, owner, host))
        }
        DispatchTarget::Primary => Some((None, s.runtime_id.clone(), s.creator_actor_id.clone(), String::new())),
    }
}

/// What the gate decided about a message.
pub enum Gate {
    /// Nobody answers it (no agent addressed, or someone else's to answer
    /// on a host that is online).
    None,
    /// This device answers it (the turn is claimed).
    Run(Plan),
    /// Its owner's device should answer; if none has after
    /// `DEFERRED_CLAIM_TIMEOUT`, this device may (the deferred-claim
    /// fallback). Not claimed.
    Defer(Plan),
}

/// The gate both paths share: what `message` triggers, and whether it runs
/// here. Claims the turn when it does.
fn gate(i: &Inner, ws: Uuid, sid: Uuid, trigger: Uuid, body: &str) -> Result<Gate> {
    let core = i.core()?;
    let local = core.identity.actor().id;
    let device = core.identity.device.id.to_string();
    let local_only = i.local_only(ws);
    let (session, links) = i.read(ws, |svc| Ok((svc.load(sid)?, svc.device_links(ws).unwrap_or_default())))?;
    let Some(session) = session else { return Ok(Gate::None) };
    let Some(target) = dispatch_target(body, &session, local_only, &links) else { return Ok(Gate::None) };
    let Some((agent, runtime_id, owner, host)) = responder(&session, target) else { return Ok(Gate::None) };
    let binding = host_binding(&host, &device);
    if !runs_here_linked(local_only, &links, &session, &local, &owner, binding) {
        // The desktop arms the fallback only for a top-level turn whose
        // host isn't bound elsewhere and online (that host will answer).
        let host_online = binding == HostBinding::Elsewhere && i.host_online(ws, &host);
        if session.cascade_depth() == 0 && !host_online {
            return Ok(Gate::Defer(Plan { ws, sid, trigger, agent, runtime_id }));
        }
        return Ok(Gate::None);
    }
    if !i.write(ws, |svc| svc.claim_turn(sid, ws, trigger))? {
        return Ok(Gate::None);
    }
    Ok(Gate::Run(Plan { ws, sid, trigger, agent, runtime_id }))
}

/// The local-send gate: post the person's message, then decide whether this
/// device answers it.
pub fn post_and_plan(i: &Inner, ws: Uuid, sid: Uuid, text: &str) -> Result<(Message, Gate)> {
    let posted = i.write(ws, |svc| svc.post_user_message(sid, ws, text))?;
    let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
    let message = views::message(&s, &posted);
    let plan = gate(i, ws, sid, posted.id, text)?;
    Ok((message, plan))
}

/// The deferred-claim fallback (the desktop's `arm_deferred_claim_fallback`):
/// wait `deferred_timeout`; if the turn is still unanswered and unclaimed
/// (`turn_dispatch::deferred_turn_unanswered`) and its host still isn't
/// online elsewhere, claim it and answer here.
pub fn arm_fallback(inner: Arc<Inner>, plan: Plan) {
    tokio::spawn(async move {
        tokio::time::sleep(inner.cfg.deferred_timeout).await;
        let claimed = blocking(&inner, move |i| {
            let Some(s) = i.read(plan.ws, |svc| svc.load(plan.sid))? else { return Ok(None) };
            if !ligus_runtime::turn_dispatch::deferred_turn_unanswered(&s, plan.trigger) {
                return Ok(None);
            }
            let device = i.core()?.identity.device.id.to_string();
            let host = plan.agent.as_ref().map(|a| a.host_id.clone()).unwrap_or_default();
            if host_binding(&host, &device) == HostBinding::Elsewhere && i.host_online(plan.ws, &host) {
                return Ok(None);
            }
            if !i.write(plan.ws, |svc| svc.claim_turn(plan.sid, plan.ws, plan.trigger))? {
                return Ok(None);
            }
            Ok(Some(plan))
        })
        .await;
        if let Ok(Some(plan)) = claimed
            && let Err(e) = run_turn(&inner, &plan).await
        {
            tracing_like(&inner, &plan, &e.0);
        }
    });
}

/// The relay-sync gate: a user message that arrived through sync, answered
/// here when it is unanswered, unclaimed by another device, and ours.
pub fn plan_synced(i: &Inner, ws: Uuid, sid: Uuid, trigger: Uuid) -> Result<Option<Plan>> {
    let device = i.core()?.identity.device.id;
    let Some(s) = i.read(ws, |svc| svc.load(sid))? else { return Ok(None) };
    if synced_user_triggers(&s, &[trigger], device).is_empty() {
        return Ok(None);
    }
    let Some(m) = s.messages.iter().find(|m| m.id == trigger) else { return Ok(None) };
    Ok(match gate(i, ws, sid, trigger, &m.body.clone())? {
        Gate::Run(plan) => Some(plan),
        // The fallback is armed by the device the message was sent from.
        Gate::Defer(_) | Gate::None => None,
    })
}

/// Whether a workspace runtime can be dispatched here (its key resolves, or
/// it needs none).
pub fn configured(r: &WorkspaceRuntime) -> bool {
    !needs_key(r.provider, &r.endpoint) || ligus_runtime::resolve_workspace_credential(&r.credential).is_some()
}

fn needs_key(p: ModelProviderKind, endpoint: &str) -> bool {
    matches!(
        p,
        ModelProviderKind::Anthropic
            | ModelProviderKind::OpenAI
            | ModelProviderKind::OpenRouter
            | ModelProviderKind::Azure
            | ModelProviderKind::Custom
    ) && !ligus_core::endpoint::endpoint_is_local_server(endpoint)
}

fn resolved(wr: &WorkspaceRuntime) -> Result<ResolvedRuntime> {
    let api_key = ligus_runtime::resolve_workspace_credential(&wr.credential);
    if api_key.is_none() && needs_key(wr.provider, &wr.endpoint) {
        return Err(BackendError(format!("the runtime '{}' has no key on this device", wr.name)));
    }
    let mut rt = base_runtime(wr.provider, wr.model.clone(), wr.endpoint.clone(), api_key, wr.driver);
    // Workspace runtimes carry no "Supports tools" switch (it is
    // device-local on the desktop): like `ligus worker`, this device opts its
    // local-capable runtimes into the tool loop unless `LT_AGENT_TOOLS=0`.
    // Hosted APIs never get it (`ResolvedRuntime::runs_agent_tools`), and
    // the file tools go only to a private endpoint.
    rt.tools = agent_tools_enabled();
    Ok(rt)
}

/// `LT_AGENT_TOOLS` (default on): whether agents' turns here may use tools.
fn agent_tools_enabled() -> bool {
    env_on("LT_AGENT_TOOLS", true)
}

/// A yes/no environment switch.
fn env_on(name: &str, default: bool) -> bool {
    match std::env::var(name).map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Ok("1" | "true" | "on" | "yes") => true,
        Ok("0" | "false" | "off" | "no") => false,
        _ => default,
    }
}

fn base_runtime(
    provider: ModelProviderKind,
    model: String,
    endpoint: String,
    api_key: Option<String>,
    driver: ligus_core::Driver,
) -> ResolvedRuntime {
    ResolvedRuntime {
        provider,
        model,
        endpoint,
        api_key,
        args: Vec::new(),
        model_provider_id: None,
        model_base_url: None,
        context_window_tokens: None,
        keep_alive: None,
        think: None,
        tools: false,
        max_tool_rounds: None,
        allow_remote_file_tools: false,
        driver,
        gateway: None,
    }
}

/// The runtime a turn runs on: the workspace runtime it names, else (when
/// allowed) the keyless Claude Code CLI, as the desktop falls back.
fn runtime_for(i: &Inner, ws: Uuid, runtime_id: &str) -> Result<(ResolvedRuntime, String)> {
    let rts = i.read(ws, |svc| svc.list_workspace_runtimes(ws))?;
    let found = rts.iter().find(|r| r.id == runtime_id).or_else(|| rts.iter().find(|r| r.id == "*")).or(if runtime_id.is_empty() {
        rts.first()
    } else {
        None
    });
    match found {
        Some(wr) => Ok((resolved(wr)?, wr.id.clone())),
        None if i.cfg.cli_fallback => Ok((
            base_runtime(ModelProviderKind::ClaudeCode, String::new(), String::new(), None, ligus_core::Driver::default()),
            "claude-code".into(),
        )),
        None => Err(BackendError("no runtime answers here: add a workspace runtime".into())),
    }
}

/// The runtime and policy a turn runs under, as the desktop and the worker
/// decide them (`turn_engine::decide_gate_with`): the author gate on `rt`'s
/// args for the trigger's author (`author_role`, the role they held when
/// they posted; `None` fails closed), the shell rule (the workspace Owner's:
/// `shell_role`, or granted to the agent), the chat's envelope (its extra
/// flags for `rt`, or a refusal), and this host's OS sandbox. `Err` is why
/// the turn doesn't run.
pub fn guard_turn(
    session: &ChatSession,
    rt: ResolvedRuntime,
    author_role: Option<WorkspaceRole>,
    shell_role: Option<WorkspaceRole>,
    shell_granted: bool,
    grants: ligus_core::sandbox::PathGrants,
) -> std::result::Result<Gated, String> {
    turn_engine::decide_gate_with(session, rt, author_role, shell_role, shell_granted, grants)
}

/// Run `plan` in the background.
pub fn spawn_turn(inner: Arc<Inner>, plan: Plan) {
    tokio::spawn(async move {
        if let Err(e) = run_turn(&inner, &plan).await {
            tracing_like(&inner, &plan, &e.0);
        }
    });
}

fn tracing_like(inner: &Inner, plan: &Plan, error: &str) {
    inner.emit(Event::Sync {
        workspace: crate::backend::model::WorkspaceId::new(plan.ws.to_string()),
        status: format!("turn failed: {error}"),
    });
}

async fn blocking<T: Send + 'static>(inner: &Arc<Inner>, f: impl FnOnce(&Inner) -> Result<T> + Send + 'static) -> Result<T> {
    let i = inner.clone();
    tokio::task::spawn_blocking(move || f(&i)).await.map_err(one_line)?
}

/// Run `plan` here; the reply's id and its final body.
///
/// The turn runs in the shared engine (`ligus_runtime::turn_engine`), as on
/// the desktop and the worker: tools (Ligus's built-in file tools for a
/// runtime that runs the tool loop, the folded Ligus tools), edits in a
/// per-turn worktree of this device's checkout filed as a proposal,
/// hand-offs, and every child confined by this host's sandbox.
pub async fn run_turn(inner: &Arc<Inner>, plan: &Plan) -> Result<(Uuid, String)> {
    let (ws, sid, trigger) = (plan.ws, plan.sid, plan.trigger);
    let agent = plan.agent.clone();
    let runtime_id = plan.runtime_id.clone();
    let who = agent.as_ref().map(|a| format!("@{}", a.name)).unwrap_or_else(|| "@primary".into());
    let wsid = WorkspaceId::new(ws.to_string());
    let chat = ChatId::new(sid.to_string());
    inner.emit(Event::Thinking { workspace: wsid.clone(), chat: chat.clone(), agent: who.clone(), on: true });
    let prepared = blocking(inner, move |i| prepare(i, ws, sid, trigger, agent.as_ref(), &runtime_id)).await;
    let (input, checkout) = match prepared {
        Ok(p) => p,
        Err(e) => {
            inner.emit(Event::Thinking { workspace: wsid, chat, agent: who, on: false });
            return Err(e);
        }
    };
    let host = TermHost {
        inner: inner.clone(),
        ws,
        svc: inner.core()?.svc.clone(),
        founder: inner.founder(ws),
        ligus: ligus_server(inner, ws),
        checkout,
        wsid: wsid.clone(),
        chat: chat.clone(),
        who,
        calls: Default::default(),
    };
    let result = turn_engine::run(&host, input).await;
    host.thinking(false);
    match result {
        Ok(o) => Ok((o.message_id, o.body)),
        // A stopped turn's reply is empty; a failed one says why.
        Err(e) if e.stopped => Ok((e.message_id.unwrap_or_default(), String::new())),
        Err(e) => match e.message_id {
            Some(mid) => Ok((mid, if e.message.starts_with("[error]") { e.message } else { format!("[error] {}", e.message) })),
            None => Err(BackendError(e.message)),
        },
    }
}

/// Everything the engine needs, decided here: the runtime, the gate and
/// the shell rule, the context, the checkout.
fn prepare(
    i: &Inner,
    ws: Uuid,
    sid: Uuid,
    trigger: Uuid,
    agent: Option<&WorkspaceAgent>,
    runtime_id: &str,
) -> Result<(TurnInput, Checkout)> {
    let (rt, rt_id) = runtime_for(i, ws, runtime_id)?;
    let author = agent.map(|a| a.name.clone()).unwrap_or_else(|| "Hive".into());
    let agent_id = agent.map(|a| a.id);
    let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
    let provider = rt.provider;
    // The author's role is recognized from the signed log, never from the
    // message's self-asserted identity; so is the shell rule's.
    let (role, shell_role, grants) = i.read(ws, |svc| {
        Ok((
            svc.trigger_author_role(sid, trigger)?,
            svc.trigger_shell_role(sid, trigger)?,
            svc.sandbox_grants_here(ws, ligus_runtime::sandbox::grants_setting_from_env()).0,
        ))
    })?;
    let granted = agent_id.is_some_and(|id| grants.shell_granted(id));
    let gated = match guard_turn(&s, rt, role, shell_role, granted, grants.paths_for(agent_id)) {
        Ok(g) => g,
        Err(reason) => {
            let _ = i.write(ws, |svc| svc.post_system_note(sid, ws, format!("A {provider:?} turn was not run: {reason}")));
            return Err(BackendError(reason));
        }
    };
    // The repo gate: a chat with a repo runs only in this device's checkout
    // of it; one without, in the checkout bound for the workspace (if any).
    let legacy = super::lock(&i.local).checkouts.get(&ws.to_string()).map(std::path::PathBuf::from);
    let checkout = {
        let core = i.core()?;
        core.svc.lock().unwrap_or_else(|e| e.into_inner()).set_trusted_founder(i.founder(ws));
        match turn_engine::repo_checkout(&core.svc, &s, &i.cfg.device_name, legacy) {
            Ok(c) => c,
            Err(reason) => {
                let _ = i.write(ws, |svc| svc.post_system_note(sid, ws, format!("{author} did not run: {reason}")));
                return Err(BackendError(reason));
            }
        }
    };
    let through = s.messages.iter().position(|m| m.id == trigger).map(|n| n + 1).unwrap_or(s.messages.len());
    let mut upto = s.clone();
    upto.messages.truncate(through);
    let turns = ligus_runtime::turns_for(&upto, agent_id, &author);
    let system = match agent {
        Some(a) if !a.role.is_empty() => format!("You are @{}, an agent in a Ligus workspace chat. Your role: {}.", a.name, a.role),
        Some(a) => format!("You are @{}, an agent in a Ligus workspace chat.", a.name),
        None => "You are the assistant in a Ligus workspace chat.".to_string(),
    };
    let system = match ligus_runtime::prompt::requester_guidance(&upto, Some(trigger)) {
        g if g.is_empty() => system,
        g => format!("{system}\n\n{g}"),
    };
    // A CLI agent writes only at a raised tier, and only in a worktree.
    let write_capable = gated.gate.allows_writes() && ligus_runtime::author_gate::turn_writes(gated.runtime.provider, &gated.runtime.args);
    let input = TurnInput {
        workspace_id: ws,
        chat_id: sid,
        session: upto,
        reply_to: Some(trigger),
        responder: Responder { author: author.clone(), effect_author: author, runtime_id: rt_id, agent_id, runtime: gated.runtime.clone() },
        system,
        turns,
        gate: gated.gate.clone(),
        envelope: gated.envelope.clone(),
        shell_allowed: gated.shell_allowed,
        policy: gated.policy(HostKind::Desktop),
        executor: ligus_runtime::executor::default_executor(),
        // `run_command` is a shell: opt-in (`LT_RUN_COMMAND=1`), and only
        // where the envelope and the shell rule allow one.
        tools: ToolOptions {
            builtin: true,
            write_in_place: false,
            run_command: env_on("LT_RUN_COMMAND", false) && gated.allow_run_command && gated.shell_allowed,
            ..Default::default()
        },
        write_capable,
        require_isolation: true,
        stop: None,
        timeout: turn_engine::turn_timeout(),
        options: EngineOptions { empty_reply_notice: true, scan_injections: true, detailed_report: true, ..Default::default() },
    };
    Ok((input, checkout))
}

/// This device's Ligus MCP server for `ws` (one per workspace, so its
/// effects are judged under that workspace's founder pin).
fn ligus_server(inner: &Arc<Inner>, ws: Uuid) -> Option<Arc<LigusMcp>> {
    let mut map = super::lock(&inner.ligus);
    let s = map.entry(ws).or_insert_with(|| LigusMcp::new(Arc::new(TermLigusHost { inner: Arc::downgrade(inner), ws })));
    Some(s.clone())
}

/// `lt` as the Ligus MCP server's host: agent effects go through the write
/// service pinned to the workspace's founder; a hand-off runs the receiver
/// here when it is this device's to answer.
struct TermLigusHost {
    inner: std::sync::Weak<Inner>,
    ws: Uuid,
}

impl LigusHost for TermLigusHost {
    fn with_service(&self, f: &mut dyn FnMut(&mut ChatService)) {
        let Some(inner) = self.inner.upgrade() else { return };
        let Ok(core) = inner.core() else { return };
        let mut svc = core.svc.lock().unwrap_or_else(|e| e.into_inner());
        svc.set_trusted_founder(inner.founder(self.ws));
        f(&mut svc);
    }

    fn start_turn(&self, chat: Uuid, agent: Uuid, trigger: Uuid) {
        let Some(inner) = self.inner.upgrade() else { return };
        let ws = self.ws;
        let i = inner.clone();
        tokio::spawn(async move {
            match blocking(&i, move |i| plan_handoff(i, ws, chat, agent, trigger)).await {
                Ok(Some(plan)) => spawn_turn(inner, plan),
                Ok(None) => {}
                Err(e) => inner.emit(Event::Sync { workspace: WorkspaceId::new(ws.to_string()), status: format!("hand-off failed: {e}") }),
            }
        });
    }

    fn ask_posted(&self, chat: Uuid, _ask: &ligus_core::flow::HumanAsk) {
        if let Some(inner) = self.inner.upgrade() {
            let _ = chat;
            inner.emit(Event::ChatsChanged { workspace: WorkspaceId::new(self.ws.to_string()) });
        }
    }
}

/// A hand-off's receiver runs here when it is this device's to answer (the
/// same rule as a mention: its host binding, or its owner here), claimed
/// first so no other device answers it too.
fn plan_handoff(i: &Inner, ws: Uuid, sid: Uuid, agent_id: Uuid, trigger: Uuid) -> Result<Option<Plan>> {
    let core = i.core()?;
    let local = core.identity.actor().id;
    let device = core.identity.device.id.to_string();
    let (session, links) = i.read(ws, |svc| Ok((svc.load(sid)?, svc.device_links(ws).unwrap_or_default())))?;
    let Some(session) = session else { return Ok(None) };
    let Some(agent) = session.workspace_agents.iter().find(|a| a.id == agent_id).cloned() else { return Ok(None) };
    if !runs_here_linked(i.local_only(ws), &links, &session, &local, &agent.owner_actor_id, host_binding(&agent.host_id, &device)) {
        return Ok(None);
    }
    if !i.write(ws, |svc| svc.claim_turn(sid, ws, trigger))? {
        return Ok(None);
    }
    let runtime_id = agent.runtime_id.clone();
    Ok(Some(Plan { ws, sid, trigger, agent: Some(agent), runtime_id }))
}

/// `lt` as the turn engine's host: the write service pinned to the
/// workspace's founder, the repo gate's checkout, and the live events the
/// UI renders (the streamed reply and the agent's tool calls).
struct TermHost {
    inner: Arc<Inner>,
    ws: Uuid,
    svc: Arc<Mutex<ChatService>>,
    founder: Option<String>,
    ligus: Option<Arc<LigusMcp>>,
    checkout: Checkout,
    wsid: WorkspaceId,
    chat: ChatId,
    who: String,
    /// Tool calls in flight: call id → (name, input preview).
    calls: Mutex<std::collections::HashMap<String, (String, String)>>,
}

impl TermHost {
    fn thinking(&self, on: bool) {
        self.inner.emit(Event::Thinking { workspace: self.wsid.clone(), chat: self.chat.clone(), agent: self.who.clone(), on });
    }
}

/// A tool call's input, one line, short.
fn preview(input_json: &str) -> String {
    let one: String = input_json.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 80 { format!("{}…", one.chars().take(79).collect::<String>()) } else { one }
}

impl TurnHost for TermHost {
    fn service(&self) -> &Mutex<ChatService> {
        &self.svc
    }
    fn pin(&self, svc: &mut ChatService) {
        svc.set_trusted_founder(self.founder.clone());
    }
    fn ligus(&self) -> Option<Arc<LigusMcp>> {
        self.ligus.clone()
    }
    fn checkout(&self, _session: &ChatSession, _message_id: Uuid) -> impl Future<Output = std::result::Result<Checkout, String>> + Send {
        // The repo gate ran before the turn started.
        let c = self.checkout.clone();
        async move { Ok(c) }
    }
    fn begun(&self, mid: Uuid) {
        self.thinking(false);
        super::lock(&self.inner.emitted).entry(mid).or_insert(0);
        let s = self.inner.read(self.ws, |svc| svc.load(self.chat_uuid()));
        if let Some(m) = s.ok().flatten().and_then(|s| s.messages.iter().find(|m| m.id == mid).map(|m| views::message(&s, m))) {
            self.inner.emit(Event::MessageAdded { workspace: self.wsid.clone(), message: m });
        }
    }
    fn delta(&self, mid: Uuid, text: &str) {
        *super::lock(&self.inner.emitted).entry(mid).or_insert(0) += text.len();
        self.inner.emit(Event::MessageDelta {
            workspace: self.wsid.clone(),
            chat: self.chat.clone(),
            message: MessageId::new(mid.to_string()),
            delta: text.to_string(),
        });
    }
    fn activity(&self, mid: Uuid, act: &StreamActivity) {
        let call = match act {
            StreamActivity::Tool { id, name, input_json } => {
                super::lock(&self.calls).insert(id.clone(), (name.clone(), preview(input_json)));
                return;
            }
            StreamActivity::ToolResult { call_id, is_error, .. } => {
                let (name, preview) = super::lock(&self.calls).remove(call_id).unwrap_or_else(|| ("tool".into(), String::new()));
                ToolCall { name, preview, ok: !is_error }
            }
            _ => return,
        };
        self.inner.emit(Event::ToolActivity {
            workspace: self.wsid.clone(),
            chat: self.chat.clone(),
            message: MessageId::new(mid.to_string()),
            call,
        });
    }
    fn proposal_filed(&self, _svc: &mut ChatService, _proposal: Uuid) {
        self.inner.emit(Event::ChatsChanged { workspace: self.wsid.clone() });
    }
}

impl TermHost {
    fn chat_uuid(&self) -> Uuid {
        Uuid::parse_str(self.chat.as_str()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ligus_core::{ActorIdentity, ActorKind, SigningKeypair};
    use ligus_runtime::author_gate::GatePlan;
    use ligus_runtime::{ChatService, EventStore};

    fn claude() -> ResolvedRuntime {
        base_runtime(ModelProviderKind::ClaudeCode, String::new(), String::new(), None, ligus_core::Driver::default())
    }

    /// A chat, and the same chat locked down.
    fn chats() -> (ChatSession, ChatSession) {
        let author = ActorIdentity::new("ana".to_string(), "Ana".to_string(), ActorKind::Human);
        let kp = SigningKeypair::generate().unwrap();
        let account = SigningKeypair::generate().unwrap();
        let mut svc = ChatService::new(EventStore::open_in_memory().unwrap(), Uuid::new_v4(), kp, account, author);
        let ws = Uuid::new_v4();
        svc.ensure_workspace_config(ws).unwrap();
        let chat = svc.create_chat("c", ws, "").unwrap();
        let open = svc.load(chat.id).unwrap().unwrap();
        svc.self_medicate(ws, chat.id, None, &[(ligus_core::guard::LockdownSignal::Manual, "manual".into())]).unwrap();
        let locked = svc.load(chat.id).unwrap().unwrap();
        assert!(locked.guards.lockdown_active());
        (open, locked)
    }

    /// This host has no OS sandbox: no CLI agent runs here at all.
    fn no_sandbox() -> bool {
        ligus_runtime::executor::refuses_writes()
    }

    fn guard(s: &ChatSession, rt: ResolvedRuntime, role: Option<WorkspaceRole>) -> std::result::Result<Gated, String> {
        guard_turn(s, rt, role, role, false, Default::default())
    }

    #[test]
    fn an_owner_runs_claude_code_inside_the_sandbox_policy() {
        let (open, _) = chats();
        match guard(&open, claude(), Some(WorkspaceRole::Owner)) {
            Ok(g) => {
                assert!(!no_sandbox());
                assert_eq!(g.gate, GatePlan::Full);
                assert!(g.envelope.egress.is_open());
                assert!(g.shell_allowed, "the workspace Owner keeps the shell");
                assert!(g.policy(HostKind::Desktop).shell_allowed);
                assert!(g.runtime.args.iter().any(|a| a == "--strict-mcp-config"), "{:?}", g.runtime.args);
            }
            Err(e) => assert!(no_sandbox(), "refused on a host with a sandbox: {e}"),
        }
    }

    /// An author below the floor, or one the log can't vouch for, never gets
    /// the runtime's configured tier (the desktop's H1 gate); a writing tier
    /// needs an Admin.
    #[test]
    fn a_low_or_unknown_author_never_gets_the_configured_tier() {
        let (open, _) = chats();
        for role in [None, Some(WorkspaceRole::Viewer)] {
            if let Ok(g) = guard(&open, claude(), role) {
                assert_ne!(g.gate, GatePlan::Full, "{role:?}");
                assert!(!g.shell_allowed);
            }
        }
        let mut writing = claude();
        writing.args = vec!["--permission-mode".into(), "acceptEdits".into()];
        if let Ok(g) = guard(&open, writing, Some(WorkspaceRole::Contributor)) {
            assert_eq!(g.gate, GatePlan::ReadOnly);
            assert!(!g.runtime.args.iter().any(|a| a == "acceptEdits"), "{:?}", g.runtime.args);
        }
    }

    /// The shell is the workspace Owner's: an Admin's turn runs at its tier
    /// but without the shell (bypass capped at accepting edits), unless an
    /// Owner granted it to the agent.
    #[test]
    fn an_admins_turn_has_no_shell_unless_granted() {
        let (open, _) = chats();
        let mut bypass = claude();
        bypass.args = vec!["--permission-mode".into(), "bypassPermissions".into()];
        if let Ok(g) = guard(&open, bypass.clone(), Some(WorkspaceRole::Admin)) {
            assert!(!g.shell_allowed);
            assert!(!g.policy(HostKind::Desktop).shell_allowed);
            assert!(!g.runtime.args.iter().any(|a| a == "bypassPermissions"), "{:?}", g.runtime.args);
        }
        if let Ok(g) = guard_turn(&open, bypass, Some(WorkspaceRole::Admin), Some(WorkspaceRole::Admin), true, Default::default()) {
            assert!(g.shell_allowed, "an Owner's grant gives the agent the shell");
        }
    }

    /// A locked-down chat closes the turn's egress and takes Claude Code's
    /// network and shell tools away.
    #[test]
    fn a_lockdown_closes_egress_for_the_turn() {
        let (_, locked) = chats();
        match guard(&locked, claude(), Some(WorkspaceRole::Owner)) {
            Ok(g) => {
                assert!(!g.envelope.egress.is_open());
                assert!(g.runtime.args.windows(2).any(|w| w[0] == "--disallowedTools" && w[1] == "WebFetch"), "{:?}", g.runtime.args);
            }
            Err(e) => assert!(no_sandbox(), "{e}"),
        }
    }
}
