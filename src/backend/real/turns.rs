//! Turns this device answers.
//!
//! Both gates go through `ligus_runtime::turn_dispatch` with the same inputs
//! as the desktop: the **local-send** gate when the person posts here
//! ([`post_and_plan`]) and the **relay-sync** gate when a message arrives
//! through sync ([`plan_synced`]). A workspace with no relay, or with one
//! person in it, always answers here. A turn that runs here is claimed first
//! (`claim_turn`), so only one of the owner's devices answers.
//!
//! Before a turn runs, [`guard_turn`] applies what every Ligus host applies:
//! the author-role gate (H1: below the floor a CLI runs read-only, or not at
//! all), the chat's sandbox envelope (a lockdown or dearth mode narrows
//! egress, or refuses a runtime it can't confine), and this host's OS
//! sandbox (no sandbox: no CLI agent). The reply then streams inside
//! `executor::with_turn`, so any agent child is spawned under that policy.
//!
//! The runner streams the reply from the responder's runtime
//! (`provider::dispatch::stream`) into `begin_assistant_message` /
//! `append_chunk` / `complete_assistant_message`; the change feed turns each
//! append into `MessageAdded` / `MessageDelta` / `MessageDone` events.

use std::sync::Arc;

use ligus_core::{ChatSession, ModelProviderKind, WorkspaceAgent, WorkspaceRole, WorkspaceRuntime};
use ligus_runtime::author_gate::GatePlan;
use ligus_runtime::executor::{HostKind, TurnPolicy};
use ligus_runtime::turn_dispatch::{DispatchTarget, dispatch_target, host_binding, runs_here_linked, synced_user_triggers};
use ligus_runtime::{ResolvedRuntime, dispatch};
use uuid::Uuid;

use super::{Inner, one_line, views};
use crate::backend::model::Message;
use crate::backend::{BackendError, Event, Result};

/// Most tokens one reply may take.
const MAX_TOKENS: u32 = 4096;

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

/// The gate both paths share: what `message` triggers, and whether it runs
/// here. Claims the turn when it does.
fn gate(i: &Inner, ws: Uuid, sid: Uuid, trigger: Uuid, body: &str) -> Result<Option<Plan>> {
    let core = i.core()?;
    let local = core.identity.actor().id;
    let device = core.identity.device.id.to_string();
    let local_only = i.local_only(ws);
    let (session, links) = i.read(ws, |svc| Ok((svc.load(sid)?, svc.device_links(ws).unwrap_or_default())))?;
    let Some(session) = session else { return Ok(None) };
    let Some(target) = dispatch_target(body, &session, local_only, &links) else { return Ok(None) };
    let Some((agent, runtime_id, owner, host)) = responder(&session, target) else { return Ok(None) };
    if !runs_here_linked(local_only, &links, &session, &local, &owner, host_binding(&host, &device)) {
        return Ok(None);
    }
    if !i.write(ws, |svc| svc.claim_turn(sid, ws, trigger))? {
        return Ok(None);
    }
    Ok(Some(Plan { ws, sid, trigger, agent, runtime_id }))
}

/// The local-send gate: post the person's message, then decide whether this
/// device answers it.
pub fn post_and_plan(i: &Inner, ws: Uuid, sid: Uuid, text: &str) -> Result<(Message, Option<Plan>)> {
    let posted = i.write(ws, |svc| svc.post_user_message(sid, ws, text))?;
    let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
    let message = views::message(&s, &posted);
    let plan = gate(i, ws, sid, posted.id, text)?;
    Ok((message, plan))
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
    gate(i, ws, sid, trigger, &m.body.clone())
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
    Ok(base_runtime(wr.provider, wr.model.clone(), wr.endpoint.clone(), api_key, wr.driver))
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

/// The runtime and sandbox policy a turn runs under, as the desktop and the
/// worker decide them: the author gate on `rt`'s args for the trigger's
/// author (`author_role`, the role they held when they posted; `None`
/// fails closed), the chat's envelope (its extra flags for `rt`, or a
/// refusal), and this host's OS sandbox. `Err` is why the turn doesn't run.
pub fn guard_turn(
    session: &ChatSession,
    mut rt: ResolvedRuntime,
    author_role: Option<WorkspaceRole>,
) -> std::result::Result<(ResolvedRuntime, TurnPolicy), String> {
    let gate = ligus_runtime::author_gate::gate_turn(rt.provider, &mut rt.args, author_role);
    let envelope = ligus_core::sandbox::envelope_for_session(session);
    let enforcement = ligus_runtime::sandbox::enforce(rt.provider, &envelope, &Vec::new());
    if let Some(reason) = enforcement.refuse {
        return Err(reason);
    }
    if let GatePlan::Refuse(reason) = &gate {
        return Err(reason.clone());
    }
    ligus_runtime::executor::turn_allowed(rt.provider)?;
    rt.args.extend(enforcement.extra_args);
    Ok((rt, TurnPolicy::new(HostKind::Desktop, envelope, gate)))
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

async fn run_turn(inner: &Arc<Inner>, plan: &Plan) -> Result<()> {
    let (ws, sid, trigger) = (plan.ws, plan.sid, plan.trigger);
    let agent = plan.agent.clone();
    let runtime_id = plan.runtime_id.clone();
    let who = agent.as_ref().map(|a| format!("@{}", a.name)).unwrap_or_else(|| "@primary".into());
    let wsid = crate::backend::model::WorkspaceId::new(ws.to_string());
    let chat = crate::backend::model::ChatId::new(sid.to_string());
    inner.emit(Event::Thinking { workspace: wsid.clone(), chat: chat.clone(), agent: who.clone(), on: true });
    let prepared = blocking(inner, move |i| {
        let (rt, rt_id) = runtime_for(i, ws, &runtime_id)?;
        let author = agent.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| "Hive".into());
        let agent_id = agent.as_ref().map(|a| a.id);
        let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
        // The author's role is recognized from the signed log, never from
        // the message's self-asserted identity.
        let role = i.read(ws, |svc| svc.trigger_author_role(sid, trigger))?;
        let provider = rt.provider;
        let (rt, policy) = match guard_turn(&s, rt, role) {
            Ok(g) => g,
            Err(reason) => {
                let _ = i.write(ws, |svc| svc.post_system_note(sid, ws, format!("A {provider:?} turn was not run: {reason}")));
                return Err(BackendError(reason));
            }
        };
        let through = s.messages.iter().position(|m| m.id == trigger).map(|n| n + 1).unwrap_or(s.messages.len());
        let mut upto = s.clone();
        upto.messages.truncate(through);
        let turns = ligus_runtime::turns_for(&upto, agent_id, &author);
        let system = match &agent {
            Some(a) if !a.role.is_empty() => format!("You are @{}, an agent in a Ligus workspace chat. Your role: {}.", a.name, a.role),
            Some(a) => format!("You are @{}, an agent in a Ligus workspace chat.", a.name),
            None => "You are the assistant in a Ligus workspace chat.".to_string(),
        };
        let mid = i.write(ws, |svc| svc.begin_assistant_message(sid, ws, author, rt_id, agent_id, Some(trigger)))?;
        let placeholder =
            i.read(ws, |svc| svc.load(sid))?.and_then(|s| s.messages.iter().find(|m| m.id == mid).map(|m| views::message(&s, m)));
        Ok((rt, policy, turns, system, mid, placeholder))
    })
    .await;
    inner.emit(Event::Thinking { workspace: wsid.clone(), chat: chat.clone(), agent: who, on: false });
    let (rt, policy, turns, system, mid, placeholder) = prepared?;
    // Stream to the UI as the model answers; the log gets the same text
    // (redacted, a little behind) through `append_chunk`.
    inner.emitted.lock().unwrap_or_else(|e| e.into_inner()).entry(mid).or_insert(0);
    if let Some(m) = placeholder {
        inner.emit(Event::MessageAdded { workspace: wsid.clone(), message: m });
    }

    // Chunks go to the log through one writer, in order.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let writer = {
        let inner = inner.clone();
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut chunk = first;
                while let Ok(more) = rx.try_recv() {
                    chunk.push_str(&more);
                }
                let _ = blocking(&inner, move |i| i.write(ws, |svc| svc.append_chunk(sid, ws, mid, chunk))).await;
            }
        })
    };
    let message = crate::backend::model::MessageId::new(mid.to_string());
    // Every agent child this turn spawns runs under its policy.
    let result = ligus_runtime::executor::with_turn(
        policy,
        dispatch::stream(
            &rt,
            Some(&system),
            &turns,
            None,
            &[],
            MAX_TOKENS,
            |d| {
                *inner.emitted.lock().unwrap_or_else(|e| e.into_inner()).entry(mid).or_insert(0) += d.len();
                inner.emit(Event::MessageDelta { workspace: wsid.clone(), chat: chat.clone(), message: message.clone(), delta: d.clone() });
                let _ = tx.send(d);
            },
            |_| {},
        ),
    )
    .await;
    drop(tx);
    let _ = writer.await;
    let body = match result {
        Ok(text) => text,
        Err(e) => format!("[error] {e}"),
    };
    blocking(inner, move |i| i.write(ws, |svc| svc.complete_assistant_message(sid, ws, mid, body))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ligus_core::{ActorIdentity, ActorKind, SigningKeypair};
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

    #[test]
    fn an_owner_runs_claude_code_inside_the_sandbox_policy() {
        let (open, _) = chats();
        match guard_turn(&open, claude(), Some(WorkspaceRole::Owner)) {
            Ok((rt, policy)) => {
                assert!(!no_sandbox());
                assert_eq!(policy.gate, GatePlan::Full);
                assert!(policy.envelope.egress.is_open());
                assert!(rt.args.iter().any(|a| a == "--strict-mcp-config"), "{:?}", rt.args);
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
            if let Ok((_, policy)) = guard_turn(&open, claude(), role) {
                assert_ne!(policy.gate, GatePlan::Full, "{role:?}");
            }
        }
        let mut writing = claude();
        writing.args = vec!["--permission-mode".into(), "acceptEdits".into()];
        if let Ok((rt, policy)) = guard_turn(&open, writing, Some(WorkspaceRole::Contributor)) {
            assert_eq!(policy.gate, GatePlan::ReadOnly);
            assert!(!rt.args.iter().any(|a| a == "acceptEdits"), "{:?}", rt.args);
        }
    }

    /// A locked-down chat closes the turn's egress and takes Claude Code's
    /// network and shell tools away.
    #[test]
    fn a_lockdown_closes_egress_for_the_turn() {
        let (_, locked) = chats();
        match guard_turn(&locked, claude(), Some(WorkspaceRole::Owner)) {
            Ok((rt, policy)) => {
                assert!(!policy.envelope.egress.is_open());
                assert!(rt.args.windows(2).any(|w| w[0] == "--disallowedTools" && w[1] == "WebFetch"), "{:?}", rt.args);
            }
            Err(e) => assert!(no_sandbox(), "{e}"),
        }
    }
}
