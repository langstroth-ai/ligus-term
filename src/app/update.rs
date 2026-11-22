//! The reducer: `update(state, msg)` changes the state and returns the
//! backend calls to make. It never calls the backend itself and never
//! draws, so it is tested with plain values.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::input::TextBuf;
use crate::app::state::*;
use crate::backend::model::*;
use crate::backend::{Event, Result};
use crate::keymap::Action;

/// Something that happened: a key, a paste, a tick, a backend event, or the
/// answer to an [`Effect`].
#[derive(Debug, Clone)]
pub enum Msg {
    Key(KeyEvent),
    Paste(String),
    /// Wall clock, for ages and toasts.
    Tick(Millis),
    /// The terminal's new size (columns, rows).
    Resize(u16, u16),
    Event(Event),
    Startup(Result<StartupState>),
    Workspaces(Result<Vec<WorkspaceSummary>>),
    Chats(WorkspaceId, Result<Vec<ChatSummary>>),
    Transcript(WorkspaceId, ChatId, Result<Vec<Message>>),
    Mentionables(WorkspaceId, Result<Vec<Mentionable>>),
    Proposals(WorkspaceId, Result<Vec<Proposal>>),
    Diff(WorkspaceId, ProposalId, Result<Diff>),
    /// The selection rested on a proposal long enough to preview its diff.
    PreviewDue(WorkspaceId, ProposalId),
    Agents(WorkspaceId, Result<Vec<Agent>>),
    Runtimes(WorkspaceId, Result<Vec<Runtime>>),
    LocalRuntimes(WorkspaceId, Result<Vec<LocalRuntime>>),
    /// An agent added on a runtime of this device: its handle.
    AgentAdded(WorkspaceId, Result<String>),
    Hosts(WorkspaceId, Result<Vec<Host>>),
    Members(WorkspaceId, Result<Vec<Member>>),
    Notifications(Result<Vec<Notification>>),
    Settings(Result<Settings>),
    Created(Result<Created>),
    Joined(Result<WorkspaceSummary>),
    ChatCreated(WorkspaceId, Result<ChatSummary>),
    Sent(Result<Message>),
    ProposalUpdated(WorkspaceId, Result<Proposal>),
    Invite(Result<String>),
    /// The invite (as typed) and what it says about its sender.
    InviteChecked(String, Result<InviteCheck>),
    /// A join request read (no passkey yet): the request and its fingerprint.
    JoinRequestChecked(String, Result<JoinRequestCheck>),
    Admitted(Result<BoxAdmission>),
    Workflows(WorkspaceId, Result<Vec<Workflow>>),
    Runs(WorkspaceId, Result<Vec<Run>>),
    RunUpdated(WorkspaceId, Result<Run>),
    Memory(WorkspaceId, Result<Memory>),
    Tasks(WorkspaceId, Result<Vec<TaskState>>),
    Handoffs(WorkspaceId, Result<Vec<Handoff>>),
    Checkout(WorkspaceId, Result<Option<String>>),
    /// A fire-and-report action finished; `ok` is the toast on success.
    Done {
        ok: String,
        result: Result<()>,
    },
}

/// A backend call for the runtime to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Startup,
    LoadWorkspaces,
    LoadChats(WorkspaceId),
    LoadTranscript(WorkspaceId, ChatId),
    LoadMentionables(WorkspaceId),
    LoadProposals(WorkspaceId),
    LoadDiff(WorkspaceId, ProposalId),
    /// Wait [`DIFF_PREVIEW_DELAY`], then answer [`Msg::PreviewDue`]: moving
    /// through the list loads only the diff the selection rests on.
    PreviewDiff(WorkspaceId, ProposalId),
    LoadAgents(WorkspaceId),
    LoadRuntimes(WorkspaceId),
    /// Look for runtimes on this device (CLIs, Ollama, API keys).
    LoadLocalRuntimes(WorkspaceId),
    /// Add an agent with this name on a runtime of this device.
    AddLocalAgent(WorkspaceId, RuntimeId, String),
    LoadHosts(WorkspaceId),
    LoadMembers(WorkspaceId),
    LoadNotifications,
    LoadSettings,
    CreateWorkspace {
        your_name: String,
        name: String,
    },
    JoinWorkspace {
        your_name: String,
        invite: String,
    },
    ConfirmRecovery(WorkspaceId),
    CreateChat(WorkspaceId, String),
    Send {
        workspace: WorkspaceId,
        chat: ChatId,
        text: String,
    },
    MarkRead(WorkspaceId, ChatId),
    LockChat(WorkspaceId, ChatId),
    ClearLockdown(WorkspaceId, ChatId),
    Vote(WorkspaceId, ProposalId, VoteChoice),
    RequestApply(WorkspaceId, ProposalId, HostId),
    WithdrawApply(WorkspaceId, ProposalId),
    SetAgentRuntime(WorkspaceId, AgentId, RuntimeId),
    /// Who may use an agent: everyone (`true`) or only its owner.
    SetAgentAccess(WorkspaceId, AgentId, bool),
    SetRole(WorkspaceId, MemberId, Role),
    RemoveMember(WorkspaceId, MemberId),
    CreateInvite(WorkspaceId, Role),
    MarkNotificationsRead,
    SetSetting(SettingKey, bool),
    AnswerPresence(PresenceId, PresenceAnswer),
    InspectInvite(String),
    ConnectDeployment {
        your_name: String,
        spec: DeploymentSpec,
    },
    InspectJoinRequest(String),
    AdmitBox(WorkspaceId, Option<String>),
    LoadWorkflows(WorkspaceId),
    LoadRuns(WorkspaceId),
    StartRun(WorkspaceId, WorkflowId, String),
    CancelRun(WorkspaceId, RunId),
    DecideGate(WorkspaceId, RunId, String, bool),
    LoadMemory(WorkspaceId),
    LoadTasks(WorkspaceId),
    LoadHandoffs(WorkspaceId),
    LoadCheckout(WorkspaceId),
    SetCheckout(WorkspaceId, Option<String>),
}

/// How long the proposal selection rests before its diff loads.
pub const DIFF_PREVIEW_DELAY: std::time::Duration = std::time::Duration::from_millis(120);

/// Loads for a workspace becoming active.
fn load_workspace(ws: &WorkspaceId) -> Vec<Effect> {
    vec![
        Effect::LoadChats(ws.clone()),
        Effect::LoadMentionables(ws.clone()),
        Effect::LoadProposals(ws.clone()),
        Effect::LoadAgents(ws.clone()),
        Effect::LoadRuntimes(ws.clone()),
        Effect::LoadHosts(ws.clone()),
        Effect::LoadMembers(ws.clone()),
        Effect::LoadWorkflows(ws.clone()),
        Effect::LoadRuns(ws.clone()),
        Effect::LoadCheckout(ws.clone()),
        // Agents' open questions make a chat "needs you".
        Effect::LoadTasks(ws.clone()),
    ]
}

fn is_active(s: &State, ws: &WorkspaceId) -> bool {
    s.active.as_ref() == Some(ws)
}

pub fn update(s: &mut State, msg: Msg) -> Vec<Effect> {
    match msg {
        Msg::Key(k) => key(s, k),
        Msg::Paste(text) => paste(s, &text),
        Msg::Tick(now) => {
            s.now = now;
            s.toasts.iter_mut().for_each(|t| t.ttl = t.ttl.saturating_sub(1));
            s.toasts.retain(|t| t.ttl > 0);
            vec![]
        }
        Msg::Resize(w, h) => {
            s.term = (w, h);
            let max = crate::ui::help_max_scroll(s, h);
            if let Some(Overlay::Help { scroll, .. }) = &mut s.overlay {
                *scroll = (*scroll).min(max);
            }
            vec![]
        }
        Msg::Event(e) => event(s, e),
        Msg::Startup(Ok(st)) => {
            s.your_name = st.your_name.clone();
            s.workspaces = st.workspaces;
            if s.workspaces.is_empty() {
                s.phase = Phase::Onboarding(WorkspaceForm::new(st.your_name.is_none()));
                vec![Effect::LoadSettings]
            } else {
                s.phase = Phase::Main;
                let ws = s.workspaces[0].id.clone();
                s.active = Some(ws.clone());
                let mut fx = load_workspace(&ws);
                fx.extend([Effect::LoadNotifications, Effect::LoadSettings]);
                fx
            }
        }
        Msg::Startup(Err(e)) => {
            s.toast(ToastKind::Error, format!("couldn't start: {e}"));
            vec![]
        }
        Msg::Workspaces(Ok(w)) => {
            s.workspaces = w;
            vec![]
        }
        Msg::Chats(ws, Ok(chats)) if is_active(s, &ws) => {
            let selected = s.chats.get(s.chat_sel.index).map(|c| c.id.clone());
            s.chats = chats;
            if let Some(id) = selected
                && let Some(i) = s.chats.iter().position(|c| c.id == id)
            {
                s.chat_sel.index = i;
            }
            s.chat_sel.clamp(s.chats.len());
            if s.open_chat.is_none()
                && let Some(c) = s.chats.get(s.chat_sel.index)
            {
                let id = c.id.clone();
                let fx = open_chat(s, id);
                if std::mem::take(&mut s.compose_on_open) && s.view == View::Chats && s.overlay.is_none() {
                    s.focus = Focus::Composer;
                }
                return fx;
            }
            vec![]
        }
        Msg::Transcript(ws, chat, Ok(t)) if is_active(s, &ws) && s.open_chat.as_ref() == Some(&chat) => {
            s.transcript = t;
            vec![]
        }
        Msg::Mentionables(ws, Ok(m)) if is_active(s, &ws) => {
            s.mentionables = m;
            vec![]
        }
        Msg::Proposals(ws, Ok(p)) if is_active(s, &ws) => {
            s.proposals = p;
            s.proposal_sel.clamp(s.proposals.len());
            // The selected proposal's diff shows without asking.
            if s.view == View::Proposals { load_selected_diff(s) } else { vec![] }
        }
        // A diff that arrives after the selection moved on is dropped.
        Msg::Diff(ws, p, Ok(d)) if is_active(s, &ws) && s.selected_proposal().is_some_and(|sel| sel.id == p) => {
            s.diff = Some(DiffState { proposal: p, diff: d, scroll: 0 });
            vec![]
        }
        Msg::PreviewDue(ws, p) => {
            let resting = is_active(s, &ws) && s.view == View::Proposals && s.selected_proposal().is_some_and(|sel| sel.id == p);
            if resting { load_selected_diff(s) } else { vec![] }
        }
        Msg::Agents(ws, Ok(a)) if is_active(s, &ws) => {
            s.agents = a;
            s.agent_sel.clamp(s.agent_rows());
            vec![]
        }
        Msg::Runtimes(ws, Ok(r)) if is_active(s, &ws) => {
            s.runtimes = r;
            vec![]
        }
        Msg::LocalRuntimes(ws, Ok(l)) if is_active(s, &ws) => {
            s.local = Some(l);
            s.agent_sel.clamp(s.agent_rows());
            vec![]
        }
        Msg::AgentAdded(ws, Ok(handle)) => {
            s.toast(ToastKind::Ok, format!("Added @{handle}: it runs on this device, and only you can use it (E to share)"));
            if is_active(s, &ws) {
                vec![Effect::LoadAgents(ws.clone()), Effect::LoadRuntimes(ws.clone()), Effect::LoadLocalRuntimes(ws)]
            } else {
                vec![]
            }
        }
        Msg::Hosts(ws, Ok(h)) if is_active(s, &ws) => {
            s.hosts = h;
            s.host_sel.clamp(s.hosts.len());
            vec![]
        }
        Msg::Members(ws, Ok(m)) if is_active(s, &ws) => {
            s.members = m;
            s.member_sel.clamp(s.members.len());
            vec![]
        }
        Msg::Notifications(Ok(n)) => {
            s.notifications = n;
            s.notification_sel.clamp(s.notifications.len());
            vec![]
        }
        Msg::Settings(Ok(st)) => {
            s.settings = Some(st);
            vec![]
        }
        Msg::Created(Ok(c)) => {
            s.overlay = None;
            s.your_name = s.your_name.clone().or_else(|| Some(String::new()));
            let ws = c.workspace.clone();
            if !s.workspaces.iter().any(|w| w.id == ws.id) {
                s.workspaces.push(ws);
            }
            s.phase = Phase::Recovery(c);
            vec![Effect::LoadSettings]
        }
        Msg::Created(Err(e)) | Msg::Joined(Err(e)) => {
            form_error(s, e.0);
            vec![]
        }
        Msg::Joined(Ok(ws)) => {
            s.overlay = None;
            if !s.workspaces.iter().any(|w| w.id == ws.id) {
                s.workspaces.push(ws.clone());
            }
            s.toast(ToastKind::Ok, format!("Joined “{}”", ws.name));
            s.phase = Phase::Main;
            let mut fx = switch_workspace(s, ws.id);
            fx.push(Effect::LoadSettings);
            fx
        }
        Msg::ChatCreated(ws, Ok(c)) if is_active(s, &ws) => {
            s.chats.insert(0, c.clone());
            s.chat_sel.index = 0;
            let mut fx = open_chat(s, c.id);
            s.focus = Focus::Composer;
            fx.push(Effect::LoadChats(ws));
            fx
        }
        Msg::Sent(Err(e)) => {
            s.toast(ToastKind::Error, e.0);
            vec![]
        }
        Msg::Sent(Ok(m)) => {
            if s.open_chat.as_ref() == Some(&m.chat) && !s.transcript.iter().any(|x| x.id == m.id) {
                s.transcript.push(m);
            }
            vec![]
        }
        Msg::ProposalUpdated(ws, Ok(p)) => {
            upsert_proposal(s, &ws, p);
            vec![]
        }
        Msg::Invite(Ok(code)) => {
            s.overlay = Some(Overlay::Invite { code: Some(code) });
            vec![]
        }
        Msg::InviteChecked(invite, r) => {
            let form = match (&mut s.overlay, &mut s.phase) {
                (Some(Overlay::AddWorkspace(f)), _) => f,
                (_, Phase::Onboarding(f)) => f,
                _ => return vec![],
            };
            form.busy = false;
            match r {
                Ok(c) => {
                    form.error = (c.trust == InviteTrust::Mismatch)
                        .then(|| "Not joining: the founder's key doesn't match the one your workspaces know".to_string());
                    form.checked = Some((invite, c));
                }
                Err(e) => form.error = Some(e.0),
            }
            vec![]
        }
        Msg::JoinRequestChecked(r, res) => {
            if let Some(Overlay::AdmitBox { request, busy, checked, result: None }) = &mut s.overlay
                && request.text.trim() == r
            {
                *busy = false;
                match res {
                    Ok(c) => *checked = Some(c),
                    Err(e) => s.toast(ToastKind::Error, format!("Couldn't read the join request: {e}")),
                }
            }
            vec![]
        }
        Msg::Admitted(r) => {
            match (&mut s.overlay, r) {
                (Some(Overlay::AdmitBox { busy, result, .. }), Ok(a)) => {
                    *busy = false;
                    *result = Some(a);
                }
                (Some(Overlay::AdmitBox { busy, .. }), Err(e)) => {
                    *busy = false;
                    if e.0 != "cancelled" {
                        s.toast(ToastKind::Error, format!("Couldn't admit the box: {e}"));
                    }
                }
                (_, Err(e)) => s.toast(ToastKind::Error, e.0),
                _ => {}
            }
            vec![]
        }
        Msg::Workflows(ws, Ok(w)) if is_active(s, &ws) => {
            s.workflows = w;
            s.workflow_sel.clamp(s.workflow_rows());
            vec![]
        }
        Msg::Runs(ws, Ok(r)) if is_active(s, &ws) => {
            s.runs = r;
            s.workflow_sel.clamp(s.workflow_rows());
            vec![]
        }
        Msg::RunUpdated(ws, Ok(r)) => {
            let id = r.id.clone();
            upsert_run(s, &ws, r);
            // Follow the run just started or decided.
            if s.view == View::Workflows
                && let Some(i) = s.runs.iter().position(|x| x.id == id)
            {
                s.workflow_sel.index = s.workflows.len() + i;
            }
            vec![]
        }
        Msg::Memory(ws, Ok(m)) if is_active(s, &ws) => {
            s.memory = m;
            s.memory_sel.clamp(s.memory_rows());
            vec![]
        }
        Msg::Tasks(ws, Ok(t)) if is_active(s, &ws) => {
            s.tasks = t;
            s.memory_sel.clamp(s.memory_rows());
            vec![]
        }
        Msg::Handoffs(ws, Ok(h)) if is_active(s, &ws) => {
            s.handoffs = h;
            vec![]
        }
        Msg::Checkout(ws, Ok(c)) if is_active(s, &ws) => {
            s.checkout = c;
            vec![]
        }
        Msg::Done { ok, result: Ok(()) } => {
            if !ok.is_empty() {
                s.toast(ToastKind::Ok, ok);
            }
            vec![]
        }
        Msg::Done { result: Err(e), .. }
        | Msg::ProposalUpdated(_, Err(e))
        | Msg::Invite(Err(e))
        | Msg::ChatCreated(_, Err(e))
        | Msg::Workspaces(Err(e))
        | Msg::Chats(_, Err(e))
        | Msg::Transcript(_, _, Err(e))
        | Msg::Mentionables(_, Err(e))
        | Msg::Proposals(_, Err(e))
        | Msg::Diff(_, _, Err(e))
        | Msg::Agents(_, Err(e))
        | Msg::Runtimes(_, Err(e))
        | Msg::LocalRuntimes(_, Err(e))
        | Msg::AgentAdded(_, Err(e))
        | Msg::Hosts(_, Err(e))
        | Msg::Members(_, Err(e))
        | Msg::Notifications(Err(e))
        | Msg::Workflows(_, Err(e))
        | Msg::Runs(_, Err(e))
        | Msg::RunUpdated(_, Err(e))
        | Msg::Memory(_, Err(e))
        | Msg::Tasks(_, Err(e))
        | Msg::Handoffs(_, Err(e))
        | Msg::Checkout(_, Err(e))
        | Msg::Settings(Err(e)) => {
            if e.0 != "cancelled" {
                s.toast(ToastKind::Error, e.0);
            } else {
                s.toast(ToastKind::Info, "Cancelled");
            }
            vec![]
        }
        // Results for a workspace that is no longer active.
        Msg::Chats(..)
        | Msg::Transcript(..)
        | Msg::Mentionables(..)
        | Msg::Proposals(..)
        | Msg::Diff(..)
        | Msg::Agents(..)
        | Msg::Runtimes(..)
        | Msg::LocalRuntimes(..)
        | Msg::Hosts(..)
        | Msg::Members(..)
        | Msg::Workflows(..)
        | Msg::Runs(..)
        | Msg::Memory(..)
        | Msg::Tasks(..)
        | Msg::Handoffs(..)
        | Msg::Checkout(..)
        | Msg::ChatCreated(..) => vec![],
    }
}

fn upsert_run(s: &mut State, ws: &WorkspaceId, r: Run) {
    if !is_active(s, ws) {
        return;
    }
    match s.runs.iter_mut().find(|x| x.id == r.id) {
        Some(x) => *x = r,
        None => {
            s.runs.insert(0, r);
            // Keep the selection on the row it was on.
            if s.workflow_sel.index >= s.workflows.len() && s.view == View::Workflows {
                s.workflow_sel.index += 1;
            }
        }
    }
    s.workflow_sel.clamp(s.workflow_rows());
}

fn form_error(s: &mut State, e: String) {
    let form = match (&mut s.overlay, &mut s.phase) {
        (Some(Overlay::AddWorkspace(f)), _) => Some(f),
        (_, Phase::Onboarding(f)) => Some(f),
        _ => None,
    };
    match form {
        Some(f) => {
            f.busy = false;
            f.error = Some(if e == "cancelled" { "Cancelled: the passkey wasn't created".into() } else { e });
        }
        None => s.toast(ToastKind::Error, e),
    }
}

fn upsert_proposal(s: &mut State, ws: &WorkspaceId, p: Proposal) {
    if !is_active(s, ws) {
        return;
    }
    match s.proposals.iter_mut().find(|x| x.id == p.id) {
        Some(x) => *x = p,
        None => s.proposals.insert(0, p),
    }
}

fn event(s: &mut State, e: Event) -> Vec<Effect> {
    match e {
        Event::MessageAdded { workspace, message } => {
            let mut fx = vec![];
            if is_active(s, &workspace) {
                if s.open_chat.as_ref() == Some(&message.chat) {
                    if !s.transcript.iter().any(|m| m.id == message.id) {
                        s.transcript.push(message.clone());
                    }
                } else if let Some(c) = s.chats.iter_mut().find(|c| c.id == message.chat) {
                    c.unread = true;
                }
                if !s.chats.iter().any(|c| c.id == message.chat) {
                    fx.push(Effect::LoadChats(workspace));
                }
            }
            fx
        }
        Event::MessageDelta { workspace, chat, message, delta } => {
            if is_active(s, &workspace)
                && s.open_chat.as_ref() == Some(&chat)
                && let Some(m) = s.transcript.iter_mut().find(|m| m.id == message)
            {
                m.text.push_str(&delta);
            }
            vec![]
        }
        Event::MessageDone { workspace, chat, message, error } => {
            if is_active(s, &workspace)
                && s.open_chat.as_ref() == Some(&chat)
                && let Some(m) = s.transcript.iter_mut().find(|m| m.id == message)
            {
                m.streaming = false;
            }
            if is_active(s, &workspace) {
                if error.is_some() {
                    s.turn_outcomes.insert(chat.clone(), SessionState::Failed);
                } else if s.open_chat.as_ref() != Some(&chat) {
                    s.turn_outcomes.insert(chat.clone(), SessionState::Done);
                }
            }
            if let Some(e) = error {
                s.toast(ToastKind::Error, e);
            }
            vec![]
        }
        Event::ToolActivity { workspace, chat, message, call } => {
            if is_active(s, &workspace)
                && s.open_chat.as_ref() == Some(&chat)
                && let Some(m) = s.transcript.iter_mut().find(|m| m.id == message)
            {
                m.tools.push(call);
            }
            vec![]
        }
        Event::Thinking { workspace, chat, agent, on } => {
            if is_active(s, &workspace) {
                let set = s.thinking.entry(chat).or_default();
                if on {
                    set.insert(agent);
                } else {
                    set.remove(&agent);
                }
            }
            vec![]
        }
        Event::ChatsChanged { workspace } if is_active(s, &workspace) => vec![Effect::LoadChats(workspace)],
        Event::ProposalChanged { workspace, proposal } => {
            upsert_proposal(s, &workspace, proposal);
            vec![]
        }
        Event::MembersChanged { workspace } if is_active(s, &workspace) => {
            vec![Effect::LoadMembers(workspace.clone()), Effect::LoadMentionables(workspace)]
        }
        Event::HostsChanged { workspace } if is_active(s, &workspace) => {
            vec![Effect::LoadHosts(workspace.clone()), Effect::LoadCheckout(workspace)]
        }
        Event::AgentsChanged { workspace } if is_active(s, &workspace) => {
            let mut fx = vec![Effect::LoadAgents(workspace.clone()), Effect::LoadMentionables(workspace.clone())];
            if s.view == View::Agents {
                fx.extend([Effect::LoadRuntimes(workspace.clone()), Effect::LoadLocalRuntimes(workspace)]);
            }
            fx
        }
        Event::WorkspacesChanged => vec![Effect::LoadWorkspaces],
        Event::WorkflowsChanged { workspace } if is_active(s, &workspace) => vec![Effect::LoadWorkflows(workspace)],
        Event::RunChanged { workspace, run } => {
            upsert_run(s, &workspace, run);
            vec![]
        }
        Event::MemoryChanged { workspace } if is_active(s, &workspace) => {
            if matches!(s.view, View::Memory | View::Agents) {
                vec![Effect::LoadMemory(workspace.clone()), Effect::LoadTasks(workspace.clone()), Effect::LoadHandoffs(workspace)]
            } else {
                // Task state feeds the chat list's "needs you".
                vec![Effect::LoadTasks(workspace)]
            }
        }
        Event::Notification(n) => {
            s.bell = s.settings.as_ref().is_none_or(|st| st.notify_mentions);
            s.toast(ToastKind::Info, n.text.clone());
            s.notifications.insert(0, n);
            vec![]
        }
        Event::PresencePrompt(p) => {
            s.presence.push_back(PresenceSheet { prompt: p, stage: PresenceStage::Asking, error: None, remote: None });
            vec![]
        }
        Event::PresenceProblem { id, error } => {
            if let Some(p) = s.presence.iter_mut().find(|p| p.prompt.id == id) {
                p.error = Some(error);
                p.remote = None;
                if p.stage == PresenceStage::WaitingBrowser {
                    p.stage = PresenceStage::Asking;
                }
            }
            vec![]
        }
        Event::PresenceUrl { id, url, port } => {
            if let Some(p) = s.presence.iter_mut().find(|p| p.prompt.id == id) {
                p.remote = Some((url, port));
            }
            vec![]
        }
        Event::PresenceDone { id, ok, error } => {
            s.presence.retain(|p| p.prompt.id != id);
            if ok {
                s.toast(ToastKind::Ok, "Confirmed with your passkey");
            } else if let Some(e) = error.filter(|e| e != "cancelled") {
                s.toast(ToastKind::Error, e);
            }
            vec![]
        }
        // "synced", "sync failed: …", "sync off: …" are the status bar's;
        // anything else on this channel is a background failure (a turn,
        // a workflow, an apply) worth a toast.
        Event::Sync { workspace, status } => {
            if status.starts_with("sync") {
                s.sync_status.insert(workspace, status);
            } else {
                s.toast(ToastKind::Error, status);
            }
            vec![]
        }
        Event::ChatsChanged { .. }
        | Event::MembersChanged { .. }
        | Event::HostsChanged { .. }
        | Event::AgentsChanged { .. }
        | Event::WorkflowsChanged { .. }
        | Event::MemoryChanged { .. } => vec![],
    }
}

fn paste(s: &mut State, text: &str) -> Vec<Effect> {
    if let Some(p) = s.presence.front_mut() {
        if let PresenceStage::EnteringPath(b) = &mut p.stage {
            b.insert_str(text.trim());
        }
        return vec![];
    }
    match (&mut s.overlay, &mut s.phase) {
        (Some(Overlay::Help { query, searching: true, scroll }), _) => {
            query.insert_str(text.trim());
            *scroll = 0;
        }
        (Some(Overlay::AddWorkspace(f)), _) | (None, Phase::Onboarding(f)) => {
            match f.field {
                FormField::Name => f.name.insert_str(text),
                FormField::Value | FormField::Relay | FormField::Room | FormField::Key | FormField::Token => {
                    let field = f.field;
                    if let Some(b) = f.buf_mut(field) {
                        b.insert_str(text.trim());
                    }
                }
                FormField::Mode => {
                    // Pasting on the mode row is almost always an invite.
                    f.mode = FormMode::Join;
                    f.field = FormField::Value;
                    f.invite.insert_str(text.trim());
                }
            }
        }
        (Some(Overlay::NewChat(b)), _) | (Some(Overlay::BindCheckout(b)), _) | (Some(Overlay::AddAgent { name: b, .. }), _) => {
            b.insert_str(text.trim())
        }
        (Some(Overlay::StartRun { input, .. }), _) => input.insert_str(text),
        (Some(Overlay::AdmitBox { request, checked: None, result: None, .. }), _) => request.insert_str(text.trim()),
        (None, Phase::Main) if s.view == View::Chats && s.open_chat.is_some() => {
            s.focus = Focus::Composer;
            s.composer.insert_str(text);
        }
        _ => {}
    }
    vec![]
}

fn key(s: &mut State, k: KeyEvent) -> Vec<Effect> {
    // Ctrl-C quits whatever has focus: at once, unless this device is
    // driving work, which asks first; a second ctrl-c then quits.
    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
        let asking = matches!(s.overlay, Some(Overlay::Confirm { action: ConfirmAction::Quit, .. }));
        if asking || !s.presence.is_empty() || !matches!(s.phase, Phase::Main) {
            s.quit = true;
        } else {
            ask_quit(s);
        }
        return vec![];
    }
    if !s.presence.is_empty() {
        return presence_key(s, k);
    }
    if s.overlay.is_some() {
        return overlay_key(s, k);
    }
    match s.phase.clone() {
        Phase::Loading => vec![],
        Phase::Onboarding(_) => {
            let Phase::Onboarding(f) = &mut s.phase else { unreachable!() };
            let your = f.name.text.trim().to_string();
            match form_key(f, &k) {
                FormOutcome::Submit => submit_form(s, Some(your)),
                FormOutcome::Cancel => {
                    s.quit = true;
                    vec![]
                }
                FormOutcome::None => vec![],
            }
        }
        Phase::Recovery(c) => match k.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                s.phase = Phase::Main;
                let mut fx = vec![Effect::ConfirmRecovery(c.workspace.id.clone())];
                fx.extend(switch_workspace(s, c.workspace.id));
                fx.push(Effect::LoadNotifications);
                fx
            }
            _ => vec![],
        },
        Phase::Main => main_key(s, k),
    }
}

enum FormOutcome {
    None,
    Submit,
    Cancel,
}

fn form_key(f: &mut WorkspaceForm, k: &KeyEvent) -> FormOutcome {
    if f.busy {
        return FormOutcome::None;
    }
    let fields = f.fields();
    let i = fields.iter().position(|x| *x == f.field).unwrap_or(0);
    match k.code {
        KeyCode::Esc => return FormOutcome::Cancel,
        KeyCode::Tab | KeyCode::Down => f.field = fields[(i + 1) % fields.len()],
        KeyCode::BackTab | KeyCode::Up => f.field = fields[(i + fields.len() - 1) % fields.len()],
        KeyCode::Enter => {
            if f.field == FormField::Value {
                return FormOutcome::Submit;
            }
            f.field = fields[(i + 1) % fields.len()];
        }
        _ => match f.field {
            FormField::Mode => {
                let mode = match k.code {
                    KeyCode::Right | KeyCode::Char(' ') | KeyCode::Char('l') => Some(f.mode.next()),
                    KeyCode::Left | KeyCode::Char('h') => Some(f.mode.prev()),
                    KeyCode::Char('c') => Some(FormMode::Create),
                    KeyCode::Char('j') => Some(FormMode::Join),
                    KeyCode::Char('d') => Some(FormMode::Connect),
                    _ => None,
                };
                if let Some(m) = mode {
                    f.mode = m;
                    f.error = None;
                    f.checked = None;
                }
            }
            field => {
                if let Some(b) = f.buf_mut(field)
                    && b.handle(k)
                    && field == FormField::Value
                {
                    f.checked = None;
                }
            }
        },
    }
    FormOutcome::None
}

fn submit_form(s: &mut State, your_name: Option<String>) -> Vec<Effect> {
    let your_name = your_name.filter(|n| !n.is_empty()).or_else(|| s.your_name.clone()).unwrap_or_default();
    let form = match (&mut s.overlay, &mut s.phase) {
        (Some(Overlay::AddWorkspace(f)), _) => f,
        (_, Phase::Onboarding(f)) => f,
        _ => return vec![],
    };
    if form.ask_name && your_name.is_empty() {
        form.error = Some("Tell us your name first".into());
        form.field = FormField::Name;
        return vec![];
    }
    let value = form.value().text.trim().to_string();
    if form.mode == FormMode::Connect {
        let spec = DeploymentSpec {
            name: value,
            relay_url: form.relay.text.trim().to_string(),
            room: form.room.text.trim().to_string(),
            key: form.key.text.trim().to_string(),
            token: Some(form.token.text.trim().to_string()).filter(|t| !t.is_empty()),
        };
        let missing = [
            (spec.relay_url.is_empty(), FormField::Relay, "Give the deployment's relay URL"),
            (spec.room.is_empty(), FormField::Room, "Give the workspace room"),
            (spec.key.is_empty(), FormField::Key, "Give the workspace key"),
        ]
        .into_iter()
        .find(|(m, _, _)| *m);
        if let Some((_, field, msg)) = missing {
            form.error = Some(msg.into());
            form.field = field;
            return vec![];
        }
        form.error = None;
        form.busy = true;
        return vec![Effect::ConnectDeployment { your_name, spec }];
    }
    if value.is_empty() {
        form.error = Some(match form.mode {
            FormMode::Create | FormMode::Connect => "Give the workspace a name".into(),
            FormMode::Join => "Paste an invite code".into(),
        });
        return vec![];
    }
    form.error = None;
    form.busy = true;
    match form.mode {
        FormMode::Create | FormMode::Connect => vec![Effect::CreateWorkspace { your_name, name: value }],
        FormMode::Join => match &form.checked {
            // First check who the invite is from; join on the second Enter.
            Some((checked, c)) if *checked == value => {
                if c.trust == InviteTrust::Mismatch {
                    form.busy = false;
                    form.error = Some("Not joining: the founder's key doesn't match the one your workspaces know".into());
                    return vec![];
                }
                vec![Effect::JoinWorkspace { your_name, invite: value }]
            }
            _ => vec![Effect::InspectInvite(value)],
        },
    }
}

fn presence_key(s: &mut State, k: KeyEvent) -> Vec<Effect> {
    let Some(p) = s.presence.front_mut() else { return vec![] };
    let id = p.prompt.id;
    let confirm = p.prompt.op == PresenceOp::Get;
    match &mut p.stage {
        PresenceStage::EnteringPath(buf) => match k.code {
            KeyCode::Esc => {
                p.stage = PresenceStage::Asking;
                vec![]
            }
            KeyCode::Enter => {
                let path = expand_tilde(buf.text.trim());
                if path.is_empty() {
                    p.error = Some("Type the path to your recovery-words file".into());
                    return vec![];
                }
                p.error = None;
                p.stage = PresenceStage::WaitingBrowser;
                vec![Effect::AnswerPresence(id, PresenceAnswer::RecoveryWordsFile(path))]
            }
            _ => {
                buf.handle(&k);
                vec![]
            }
        },
        PresenceStage::Asking | PresenceStage::WaitingBrowser => match k.code {
            KeyCode::Esc | KeyCode::Char('c') => vec![Effect::AnswerPresence(id, PresenceAnswer::Cancel)],
            KeyCode::Enter | KeyCode::Char('o') if p.stage == PresenceStage::Asking => {
                p.stage = PresenceStage::WaitingBrowser;
                p.error = None;
                vec![Effect::AnswerPresence(id, PresenceAnswer::Browser)]
            }
            KeyCode::Char('f') if confirm => {
                p.stage = PresenceStage::EnteringPath(TextBuf::single());
                vec![]
            }
            _ => vec![],
        },
    }
}

fn expand_tilde(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => p.to_string(),
    }
}

fn list_nav(sel: &mut Sel, len: usize, a: crate::keymap::Action) -> bool {
    use crate::keymap::Action::*;
    match a {
        Up => sel.up(1),
        Down => sel.down(1, len),
        PageUp => sel.up(10),
        PageDown => sel.down(10, len),
        Top => sel.index = 0,
        Bottom => sel.index = len.saturating_sub(1),
        _ => return false,
    }
    true
}

fn overlay_key(s: &mut State, k: KeyEvent) -> Vec<Effect> {
    let nav = s.keymap.resolve(&[crate::keymap::Context::Nav], &k, false);
    let ws = s.active.clone();
    let help_max = if matches!(s.overlay, Some(Overlay::Help { .. })) { crate::ui::help_max_scroll(s, s.term.1) } else { 0 };
    let Some(overlay) = s.overlay.as_mut() else { return vec![] };
    match overlay {
        Overlay::Help { scroll, query, searching } => {
            let page = (s.term.1 as usize).saturating_sub(6).max(1);
            // Typing the search: arrows still scroll, enter keeps the
            // filter and gives the keys back, esc drops it.
            if *searching {
                match k.code {
                    KeyCode::Esc => {
                        query.take();
                        *searching = false;
                        *scroll = 0;
                    }
                    KeyCode::Enter => *searching = false,
                    KeyCode::Down => *scroll = (*scroll + 1).min(help_max),
                    KeyCode::Up => *scroll = scroll.saturating_sub(1),
                    KeyCode::PageDown => *scroll = (*scroll + page).min(help_max),
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(page),
                    _ => {
                        if query.handle(&k) {
                            *scroll = 0;
                        }
                    }
                }
                return vec![];
            }
            match k.code {
                KeyCode::Char('/') => {
                    *searching = true;
                    return vec![];
                }
                // With a filter on, esc clears it first.
                KeyCode::Esc if !query.text.is_empty() => {
                    query.take();
                    *scroll = 0;
                    return vec![];
                }
                _ => {}
            }
            match nav {
                Some(Action::Down) => *scroll = (*scroll + 1).min(help_max),
                Some(Action::Up) => *scroll = scroll.saturating_sub(1),
                Some(Action::PageDown) => *scroll = (*scroll + page).min(help_max),
                Some(Action::PageUp) => *scroll = scroll.saturating_sub(page),
                Some(Action::Top) => *scroll = 0,
                Some(Action::Bottom) => *scroll = help_max,
                _ => s.overlay = None,
            }
            vec![]
        }
        Overlay::Messages { scroll } => {
            let max = s.log.len().saturating_sub(1);
            match nav {
                Some(Action::Down) => *scroll = (*scroll + 1).min(max),
                Some(Action::Up) => *scroll = scroll.saturating_sub(1),
                Some(Action::PageDown) => *scroll = (*scroll + 10).min(max),
                Some(Action::PageUp) => *scroll = scroll.saturating_sub(10),
                Some(Action::Top) => *scroll = 0,
                Some(Action::Bottom) => *scroll = max,
                _ => s.overlay = None,
            }
            vec![]
        }
        Overlay::Switcher { sel } => {
            let len = s.workspaces.len();
            let mut i = Sel { index: *sel };
            if let Some(a) = nav
                && a != Action::Select
                && list_nav(&mut i, len, a)
            {
                *sel = i.index;
                return vec![];
            }
            match k.code {
                KeyCode::Esc | KeyCode::Char('w') | KeyCode::Char('q') => {
                    s.overlay = None;
                    vec![]
                }
                KeyCode::Char('+') => {
                    s.overlay = Some(Overlay::AddWorkspace(WorkspaceForm::new(false)));
                    vec![]
                }
                KeyCode::Enter | KeyCode::Char('l') => {
                    let id = s.workspaces.get(*sel).map(|w| w.id.clone());
                    s.overlay = None;
                    id.map(|id| switch_workspace(s, id)).unwrap_or_default()
                }
                KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                    let i = c as usize - '1' as usize;
                    let id = s.workspaces.get(i).map(|w| w.id.clone());
                    s.overlay = None;
                    id.map(|id| switch_workspace(s, id)).unwrap_or_default()
                }
                _ => vec![],
            }
        }
        Overlay::AddWorkspace(f) => match form_key(f, &k) {
            FormOutcome::Submit => submit_form(s, None),
            FormOutcome::Cancel => {
                s.overlay = None;
                vec![]
            }
            FormOutcome::None => vec![],
        },
        Overlay::NewChat(buf) => match k.code {
            KeyCode::Esc => {
                s.overlay = None;
                vec![]
            }
            KeyCode::Enter => {
                let title = buf.text.trim().to_string();
                s.overlay = None;
                ws.map(|w| vec![Effect::CreateChat(w, title)]).unwrap_or_default()
            }
            _ => {
                buf.handle(&k);
                vec![]
            }
        },
        Overlay::AddAgent { runtime, name } => match k.code {
            KeyCode::Esc => {
                s.overlay = None;
                vec![]
            }
            KeyCode::Enter => {
                let (rt, name) = (runtime.id.clone(), name.text.trim().to_string());
                if name.is_empty() {
                    s.toast(ToastKind::Error, "Give the agent a name");
                    return vec![];
                }
                s.overlay = None;
                ws.map(|w| vec![Effect::AddLocalAgent(w, rt, name)]).unwrap_or_default()
            }
            _ => {
                name.handle(&k);
                vec![]
            }
        },
        Overlay::HostPicker { proposal, sel } => {
            let len = s.hosts.len();
            let mut i = Sel { index: *sel };
            if let Some(a) = nav
                && a != Action::Select
                && list_nav(&mut i, len, a)
            {
                *sel = i.index;
                return vec![];
            }
            match k.code {
                KeyCode::Esc => {
                    s.overlay = None;
                    vec![]
                }
                KeyCode::Enter => {
                    let p = proposal.clone();
                    let host = s.hosts.get(*sel).cloned();
                    s.overlay = None;
                    match (ws, host) {
                        (Some(w), Some(h)) => vec![Effect::RequestApply(w, p, h.id)],
                        _ => vec![],
                    }
                }
                _ => vec![],
            }
        }
        Overlay::RolePicker { member, sel } => {
            let len = Role::ALL.len();
            let mut i = Sel { index: *sel };
            if let Some(a) = nav
                && a != Action::Select
                && list_nav(&mut i, len, a)
            {
                *sel = i.index;
                return vec![];
            }
            match k.code {
                KeyCode::Esc => {
                    s.overlay = None;
                    vec![]
                }
                KeyCode::Enter => {
                    let member = member.clone();
                    let role = Role::ALL[*sel];
                    let allowed = s
                        .members
                        .iter()
                        .find(|m| m.id == member)
                        .and_then(|m| m.can_set_role.iter().find(|(r, _)| *r == role).map(|(_, a)| a.clone()))
                        .unwrap_or(Allowed::No("not a member".into()));
                    match allowed {
                        Allowed::Yes => {
                            s.overlay = None;
                            ws.map(|w| vec![Effect::SetRole(w, member, role)]).unwrap_or_default()
                        }
                        Allowed::No(r) => {
                            s.toast(ToastKind::Error, format!("Can't make them {}: {r}", role.label()));
                            vec![]
                        }
                    }
                }
                _ => vec![],
            }
        }
        Overlay::RuntimePicker { agent, sel } => {
            let configured: Vec<RuntimeId> = s.runtimes.iter().filter(|r| r.configured).map(|r| r.id.clone()).collect();
            let mut i = Sel { index: *sel };
            if let Some(a) = nav
                && a != Action::Select
                && list_nav(&mut i, configured.len(), a)
            {
                *sel = i.index;
                return vec![];
            }
            match k.code {
                KeyCode::Esc => {
                    s.overlay = None;
                    vec![]
                }
                KeyCode::Enter => {
                    let agent = agent.clone();
                    let rt = configured.get(*sel).cloned();
                    s.overlay = None;
                    match (ws, rt) {
                        (Some(w), Some(r)) => vec![Effect::SetAgentRuntime(w, agent, r)],
                        _ => vec![],
                    }
                }
                _ => vec![],
            }
        }
        Overlay::ConfirmRemove { member, .. } => match k.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let m = member.clone();
                s.overlay = None;
                ws.map(|w| vec![Effect::RemoveMember(w, m)]).unwrap_or_default()
            }
            _ => {
                s.overlay = None;
                vec![]
            }
        },
        Overlay::Invite { .. } => {
            s.overlay = None;
            vec![]
        }
        Overlay::Confirm { action, .. } => {
            let action = action.clone();
            s.overlay = None;
            if !matches!(k.code, KeyCode::Char('y') | KeyCode::Enter) {
                return vec![];
            }
            match (action, ws) {
                (ConfirmAction::Quit, _) => {
                    s.quit = true;
                    vec![]
                }
                (ConfirmAction::CancelRun(r), Some(w)) => vec![Effect::CancelRun(w, r)],
                (ConfirmAction::Reject(p), Some(w)) => vec![Effect::Vote(w, p, VoteChoice::Reject)],
                (ConfirmAction::RejectGate { run, stage }, Some(w)) => vec![Effect::DecideGate(w, run, stage, false)],
                (_, None) => vec![],
            }
        }
        Overlay::AdmitBox { request, busy, checked, result } => {
            if *busy {
                return vec![];
            }
            if result.is_some() {
                s.overlay = None;
                return vec![];
            }
            // The fingerprint is on screen: y admits (the passkey prompt
            // comes only now), esc goes back to the request.
            if checked.is_some() {
                return match k.code {
                    KeyCode::Char('y') => {
                        *busy = true;
                        let r = request.text.trim().to_string();
                        ws.map(|w| vec![Effect::AdmitBox(w, Some(r))]).unwrap_or_default()
                    }
                    KeyCode::Esc => {
                        *checked = None;
                        vec![]
                    }
                    _ => vec![],
                };
            }
            match k.code {
                KeyCode::Esc => {
                    s.overlay = None;
                    vec![]
                }
                KeyCode::Enter => {
                    let r = request.text.trim().to_string();
                    *busy = true;
                    if r.is_empty() {
                        // A bearer code: no box to compare with.
                        ws.map(|w| vec![Effect::AdmitBox(w, None)]).unwrap_or_default()
                    } else {
                        vec![Effect::InspectJoinRequest(r)]
                    }
                }
                _ => {
                    request.handle(&k);
                    vec![]
                }
            }
        }
        Overlay::StartRun { workflow, input, .. } => match k.code {
            KeyCode::Esc => {
                s.overlay = None;
                vec![]
            }
            KeyCode::Enter => {
                let (wf, text) = (workflow.clone(), input.text.trim().to_string());
                s.overlay = None;
                ws.map(|w| vec![Effect::StartRun(w, wf, text)]).unwrap_or_default()
            }
            _ => {
                input.handle(&k);
                vec![]
            }
        },
        Overlay::BindCheckout(buf) => match k.code {
            KeyCode::Esc => {
                s.overlay = None;
                vec![]
            }
            KeyCode::Enter => {
                let path = Some(expand_tilde(buf.text.trim())).filter(|p| !p.is_empty());
                s.overlay = None;
                ws.map(|w| vec![Effect::SetCheckout(w, path)]).unwrap_or_default()
            }
            _ => {
                buf.handle(&k);
                vec![]
            }
        },
    }
}

pub fn switch_workspace(s: &mut State, id: WorkspaceId) -> Vec<Effect> {
    if s.active.as_ref() == Some(&id) && !s.chats.is_empty() {
        return vec![];
    }
    save_draft(s);
    s.active = Some(id.clone());
    s.chats.clear();
    s.chat_sel = Sel::default();
    s.open_chat = None;
    s.transcript.clear();
    s.transcript_scroll = 0;
    s.proposals.clear();
    s.proposal_sel = Sel::default();
    s.diff = None;
    s.agents.clear();
    s.runtimes.clear();
    s.local = None;
    s.agent_sel = Sel::default();
    s.hosts.clear();
    s.members.clear();
    s.mentionables.clear();
    s.workflows.clear();
    s.runs.clear();
    s.workflow_sel = Sel::default();
    s.memory = Memory::default();
    s.tasks.clear();
    s.memory_sel = Sel::default();
    s.handoffs.clear();
    s.checkout = None;
    s.turn_outcomes.clear();
    s.focus = Focus::List;
    load_workspace(&id)
}

fn save_draft(s: &mut State) {
    if let Some(c) = s.open_chat.clone() {
        let text = s.composer.take();
        if text.is_empty() {
            s.drafts.remove(&c);
        } else {
            s.drafts.insert(c, text);
        }
    }
}

fn open_chat(s: &mut State, id: ChatId) -> Vec<Effect> {
    let Some(ws) = s.active.clone() else { return vec![] };
    if s.open_chat.as_ref() != Some(&id) {
        save_draft(s);
        s.transcript.clear();
        s.transcript_scroll = 0;
        if let Some(d) = s.drafts.remove(&id) {
            s.composer.set(&d);
        }
    }
    s.open_chat = Some(id.clone());
    s.turn_outcomes.remove(&id);
    if let Some(c) = s.chats.iter_mut().find(|c| c.id == id) {
        c.unread = false;
        c.mentioned = false;
    }
    vec![Effect::LoadTranscript(ws.clone(), id.clone()), Effect::MarkRead(ws, id)]
}

fn go(s: &mut State, v: View) -> Vec<Effect> {
    s.view = v;
    s.focus = Focus::List;
    match v {
        View::Notifications => vec![Effect::LoadNotifications],
        View::Settings => vec![Effect::LoadSettings],
        _ => s
            .active
            .clone()
            .map(|w| match v {
                View::Proposals => {
                    let mut fx = vec![Effect::LoadProposals(w)];
                    fx.extend(load_selected_diff(s));
                    fx
                }
                View::People => vec![Effect::LoadMembers(w)],
                View::Hosts => vec![Effect::LoadHosts(w.clone()), Effect::LoadCheckout(w)],
                View::Agents => vec![
                    Effect::LoadAgents(w.clone()),
                    Effect::LoadRuntimes(w.clone()),
                    Effect::LoadLocalRuntimes(w.clone()),
                    Effect::LoadHandoffs(w),
                ],
                View::Workflows => vec![Effect::LoadWorkflows(w.clone()), Effect::LoadRuns(w)],
                View::Memory => vec![Effect::LoadTasks(w.clone()), Effect::LoadMemory(w)],
                _ => vec![],
            })
            .unwrap_or_default(),
    }
}

fn main_key(s: &mut State, k: KeyEvent) -> Vec<Effect> {
    let typing = s.typing_text();
    // The mention popup takes ↑/↓/Tab/Enter/Esc while it is open.
    if s.focus == Focus::Composer && s.view == View::Chats {
        let n = s.mention_candidates().len();
        if n > 0 {
            match k.code {
                KeyCode::Up => {
                    s.mention_sel = (s.mention_sel + n - 1) % n;
                    return vec![];
                }
                KeyCode::Down => {
                    s.mention_sel = (s.mention_sel + 1) % n;
                    return vec![];
                }
                KeyCode::Tab | KeyCode::Enter if k.modifiers.is_empty() => {
                    let c = s.mention_candidates()[s.mention_sel.min(n - 1)];
                    // An agent you may not use isn't completed (agent
                    // access controls): its reason instead.
                    if let Some(why) = c.unavailable.clone() {
                        let h = c.handle.clone();
                        s.toast(ToastKind::Error, format!("@{h}: {why}"));
                        return vec![];
                    }
                    let h = c.handle.clone();
                    s.composer.complete_mention(&h);
                    s.mention_sel = 0;
                    return vec![];
                }
                KeyCode::Esc => {
                    s.mention_dismissed = Some(s.composer.text.clone());
                    return vec![];
                }
                _ => {}
            }
        }
    }
    let contexts = s.contexts();
    let action = s.keymap.resolve(&contexts, &k, typing);
    let Some(action) = action else {
        if s.focus == Focus::Composer && s.view == View::Chats && s.composer.handle(&k) {
            s.mention_sel = 0;
        }
        return vec![];
    };
    main_action(s, action)
}

fn main_action(s: &mut State, action: Action) -> Vec<Effect> {
    let ws = s.active.clone();
    use Action::*;
    match action {
        Quit => {
            ask_quit(s);
            vec![]
        }
        Messages => {
            s.overlay = Some(Overlay::Messages { scroll: 0 });
            vec![]
        }
        Help => {
            s.overlay = Some(Overlay::Help { scroll: 0, query: TextBuf::single(), searching: false });
            vec![]
        }
        Back => {
            match (s.view, s.focus) {
                (View::Proposals, Focus::Detail) => s.focus = Focus::List,
                (View::Chats, Focus::Detail) => s.focus = Focus::List,
                (View::Chats, Focus::List) => {}
                _ => s.view = View::Chats,
            }
            vec![]
        }
        SwitchWorkspace => {
            let sel = s.active.as_ref().and_then(|a| s.workspaces.iter().position(|w| &w.id == a)).unwrap_or(0);
            s.overlay = Some(Overlay::Switcher { sel });
            vec![]
        }
        AddWorkspace => {
            s.overlay = Some(Overlay::AddWorkspace(WorkspaceForm::new(false)));
            vec![]
        }
        GoChats => go(s, View::Chats),
        GoProposals => go(s, View::Proposals),
        GoAgents => go(s, View::Agents),
        GoHosts => go(s, View::Hosts),
        GoPeople => go(s, View::People),
        GoNotifications => go(s, View::Notifications),
        GoSettings => go(s, View::Settings),
        GoWorkflows => go(s, View::Workflows),
        GoMemory => go(s, View::Memory),
        FocusNext | FocusPrev => {
            s.focus = match (s.view, s.focus, action) {
                (View::Chats, Focus::List, FocusNext) => Focus::Detail,
                (View::Chats, Focus::Detail, FocusNext) => Focus::Composer,
                (View::Chats, Focus::Composer, FocusNext) => Focus::List,
                (View::Chats, Focus::List, _) => Focus::Composer,
                (View::Chats, Focus::Detail, _) => Focus::List,
                (View::Chats, Focus::Composer, _) => Focus::Detail,
                (View::Proposals, Focus::List, _) if s.diff.is_some() => Focus::Detail,
                _ => Focus::List,
            };
            vec![]
        }
        Up | Down | PageUp | PageDown | Top | Bottom => nav_action(s, action),
        Select => select(s),
        Compose => {
            if s.open_chat.is_some() {
                s.focus = Focus::Composer;
            }
            vec![]
        }
        NewChat => {
            s.overlay = Some(Overlay::NewChat(TextBuf::single()));
            vec![]
        }
        LockChat => match (ws, s.open_chat.clone()) {
            (Some(w), Some(c)) => vec![Effect::LockChat(w, c)],
            _ => vec![],
        },
        ClearLockdown => {
            let Some(c) = s.open_chat_summary() else { return vec![] };
            match &c.lockdown {
                None => {
                    s.toast(ToastKind::Info, "This chat isn't locked down");
                    vec![]
                }
                Some(Lockdown { clear: Allowed::No(r), .. }) => {
                    let r = r.clone();
                    s.toast(ToastKind::Error, format!("Can't clear the lockdown: {r}"));
                    vec![]
                }
                Some(_) => {
                    let c = c.id.clone();
                    ws.map(|w| vec![Effect::ClearLockdown(w, c)]).unwrap_or_default()
                }
            }
        }
        Send => {
            let (Some(w), Some(c)) = (ws, s.open_chat.clone()) else { return vec![] };
            if s.composer.is_empty() {
                return vec![];
            }
            let text = s.composer.take();
            s.turn_outcomes.remove(&c);
            s.mention_dismissed = None;
            s.transcript_scroll = 0;
            vec![Effect::Send { workspace: w, chat: c, text }]
        }
        Newline => {
            s.composer.insert('\n');
            vec![]
        }
        CancelCompose => {
            s.focus = Focus::List;
            vec![]
        }
        Complete => vec![],
        Approve | Reject => {
            let Some(p) = s.selected_proposal() else { return vec![] };
            if let Allowed::No(r) = &p.can_vote {
                let r = r.clone();
                s.toast(ToastKind::Error, format!("Can't vote: {r}"));
                return vec![];
            }
            let id = p.id.clone();
            if action == Reject {
                let title = p.title.clone();
                confirm(s, "Reject", format!("Vote to reject \u{201c}{title}\u{201d}?"), "reject", ConfirmAction::Reject(id));
                return vec![];
            }
            ws.map(|w| vec![Effect::Vote(w, id, VoteChoice::Approve)]).unwrap_or_default()
        }
        ApplyOnHost => {
            let Some(p) = s.selected_proposal() else { return vec![] };
            if p.status != ProposalStatus::Approved {
                let st = p.status.label();
                s.toast(ToastKind::Error, format!("Only an approved proposal can be applied (this one is {st})"));
                return vec![];
            }
            if matches!(p.apply, ApplyState::Waiting { .. }) {
                s.toast(ToastKind::Error, "One applier per proposal: withdraw that request first");
                return vec![];
            }
            let id = p.id.clone();
            if s.hosts.len() == 1 {
                let h = s.hosts[0].id.clone();
                return ws.map(|w| vec![Effect::RequestApply(w, id, h)]).unwrap_or_default();
            }
            let sel = s.default_apply_host().and_then(|d| s.hosts.iter().position(|h| h.id == d.id)).unwrap_or(0);
            s.overlay = Some(Overlay::HostPicker { proposal: id, sel });
            vec![]
        }
        WithdrawApply => {
            let Some(p) = s.selected_proposal() else { return vec![] };
            let id = p.id.clone();
            ws.map(|w| vec![Effect::WithdrawApply(w, id)]).unwrap_or_default()
        }
        NextHunk | PrevHunk | NextFile | PrevFile => {
            if let Some(d) = s.diff.as_mut() {
                let rows = d.rows();
                let is_target = |r: &DiffRow| match action {
                    NextHunk | PrevHunk => matches!(r, DiffRow::Hunk(_)),
                    _ => matches!(r, DiffRow::File { .. }),
                };
                let next = if matches!(action, NextHunk | NextFile) {
                    rows.iter().enumerate().skip(d.scroll + 1).find(|(_, r)| is_target(r)).map(|(i, _)| i)
                } else {
                    rows.iter().enumerate().take(d.scroll).rev().find(|(_, r)| is_target(r)).map(|(i, _)| i)
                };
                if let Some(i) = next {
                    d.scroll = i;
                }
            }
            vec![]
        }
        ChangeRole => {
            let Some(m) = s.selected_member() else { return vec![] };
            if m.can_set_role.iter().all(|(_, a)| !a.is_yes()) {
                let reason =
                    m.can_set_role.iter().find(|(r, _)| *r != m.role).and_then(|(_, a)| a.reason()).unwrap_or("not allowed").to_string();
                let name = m.name.clone();
                s.toast(ToastKind::Error, format!("You can't change {name}'s role: {reason}"));
                return vec![];
            }
            let sel = Role::ALL.iter().position(|r| *r == m.role).unwrap_or(0);
            s.overlay = Some(Overlay::RolePicker { member: m.id.clone(), sel });
            vec![]
        }
        RemoveMember => {
            let Some(m) = s.selected_member() else { return vec![] };
            match &m.can_remove {
                Allowed::No(r) => {
                    let msg = format!("Can't remove {}: {r}", m.name);
                    s.toast(ToastKind::Error, msg);
                }
                Allowed::Yes => s.overlay = Some(Overlay::ConfirmRemove { member: m.id.clone(), name: m.name.clone() }),
            }
            vec![]
        }
        Invite => {
            if let Some(Allowed::No(r)) = s.active_workspace().map(|w| w.can_invite.clone()) {
                s.toast(ToastKind::Error, format!("Can't invite: {r}"));
                return vec![];
            }
            s.overlay = Some(Overlay::Invite { code: None });
            ws.map(|w| vec![Effect::CreateInvite(w, Role::Contributor)]).unwrap_or_default()
        }
        ToggleAccess => {
            let Some(a) = s.agents.get(s.agent_sel.index) else { return vec![] };
            if !a.can_manage {
                let who = if a.shared { "an Admin" } else { "its owner" };
                s.toast(ToastKind::Error, format!("Only {who} changes who can use @{}", a.handle));
                return vec![];
            }
            let everyone = a.access != "everyone";
            ws.map(|w| vec![Effect::SetAgentAccess(w, a.id.clone(), everyone)]).unwrap_or_default()
        }
        AddAgent => {
            let Some(l) = s.selected_local().cloned() else {
                let hint = if s.available_local().is_empty() {
                    "Nothing on this device to add: see \u{201c}Available on this device\u{201d}"
                } else {
                    "Select a runtime under \u{201c}Available on this device\u{201d} first"
                };
                s.toast(ToastKind::Info, hint);
                return vec![];
            };
            if let Some(why) = &l.unsupported {
                s.toast(ToastKind::Error, why.clone());
                return vec![];
            }
            let mut name = TextBuf::single();
            name.insert_str(&l.suggested_name);
            s.overlay = Some(Overlay::AddAgent { runtime: l, name });
            vec![]
        }
        ChangeRuntime => {
            let Some(a) = s.agents.get(s.agent_sel.index) else { return vec![] };
            let configured: Vec<&Runtime> = s.runtimes.iter().filter(|r| r.configured).collect();
            let sel = configured.iter().position(|r| r.id == a.runtime).unwrap_or(0);
            s.overlay = Some(Overlay::RuntimePicker { agent: a.id.clone(), sel });
            vec![]
        }
        Toggle => {
            let Some(st) = &s.settings else { return vec![] };
            match s.settings_sel.index {
                0 => vec![Effect::SetSetting(SettingKey::NotifyMentions, !st.notify_mentions)],
                1 => vec![Effect::SetSetting(SettingKey::AgentsMayEdit, !st.agents_may_edit)],
                _ => vec![],
            }
        }
        MarkAllRead => {
            s.notifications.iter_mut().for_each(|n| n.read = true);
            vec![Effect::MarkNotificationsRead]
        }
        StartRun => {
            let def = match s.selected_workflow_item() {
                Some(WorkflowItem::Def(d)) => d.clone(),
                Some(WorkflowItem::Run(r)) => match s.workflows.iter().find(|d| d.id == r.workflow) {
                    Some(d) => d.clone(),
                    None => return vec![],
                },
                None => return vec![],
            };
            if let Allowed::No(r) = &def.can_start {
                s.toast(ToastKind::Error, format!("Can't start “{}”: {r}", def.name));
                return vec![];
            }
            s.overlay = Some(Overlay::StartRun { workflow: def.id, name: def.name, label: def.input_label, input: TextBuf::single() });
            vec![]
        }
        ApproveGate | RejectGate => {
            let Some(WorkflowItem::Run(r)) = s.selected_workflow_item() else {
                s.toast(ToastKind::Info, "Select a run to decide its gate");
                return vec![];
            };
            let Some(gate) = r.stages.iter().find(|st| st.status == StageStatus::AwaitingApproval) else {
                s.toast(ToastKind::Info, "This run isn't waiting on a gate");
                return vec![];
            };
            let (run, stage) = (r.id.clone(), gate.id.clone());
            if action == RejectGate {
                let q =
                    format!("Reject the gate \u{201c}{}\u{201d} of the run \u{201c}{}\u{201d}? The run stops there.", gate.name, r.name);
                confirm(s, "Reject the gate", q, "reject", ConfirmAction::RejectGate { run, stage });
                return vec![];
            }
            ws.map(|w| vec![Effect::DecideGate(w, run, stage, true)]).unwrap_or_default()
        }
        CancelRun => {
            let Some(WorkflowItem::Run(r)) = s.selected_workflow_item() else { return vec![] };
            if r.status.is_terminal() {
                let st = r.status.label();
                s.toast(ToastKind::Info, format!("This run already {st}"));
                return vec![];
            }
            let q = format!("Cancel the run \u{201c}{}\u{201d}? Its stages stop and it can't be resumed.", r.name);
            let run = r.id.clone();
            confirm(s, "Cancel the run", q, "cancel the run", ConfirmAction::CancelRun(run));
            vec![]
        }
        BindCheckout => {
            let mut buf = TextBuf::single();
            if let Some(c) = &s.checkout {
                buf.set(c);
            }
            s.overlay = Some(Overlay::BindCheckout(buf));
            vec![]
        }
        AdmitBox => {
            if s.active_workspace().is_some_and(|w| w.your_role < Role::Admin) {
                s.toast(ToastKind::Error, "Can't admit a box: only an Admin or Owner admits one");
                return vec![];
            }
            s.overlay = Some(Overlay::AdmitBox { request: TextBuf::single(), busy: false, checked: None, result: None });
            vec![]
        }
    }
}

fn confirm(s: &mut State, title: &str, question: String, yes: &str, action: ConfirmAction) {
    s.overlay = Some(Overlay::Confirm { title: title.into(), question, yes: yes.into(), action });
}

/// Quit, or ask first when quitting would stop work on this device.
fn ask_quit(s: &mut State) {
    match s.quit_guard() {
        Some(why) => confirm(s, "Quit?", why, "quit", ConfirmAction::Quit),
        None => s.quit = true,
    }
}

/// Settings rows that can be toggled.
pub const SETTINGS_TOGGLES: usize = 2;

fn nav_action(s: &mut State, a: Action) -> Vec<Effect> {
    match s.view {
        View::Chats => match s.focus {
            Focus::Detail => {
                use Action::*;
                match a {
                    Up => s.transcript_scroll += 1,
                    Down => s.transcript_scroll = s.transcript_scroll.saturating_sub(1),
                    PageUp => s.transcript_scroll += 10,
                    PageDown => s.transcript_scroll = s.transcript_scroll.saturating_sub(10),
                    Top => s.transcript_scroll = usize::MAX / 2,
                    Bottom => s.transcript_scroll = 0,
                    _ => {}
                }
                vec![]
            }
            _ => {
                let before = s.chat_sel.index;
                list_nav(&mut s.chat_sel, s.chats.len(), a);
                if before != s.chat_sel.index
                    && let Some(c) = s.chats.get(s.chat_sel.index)
                {
                    let id = c.id.clone();
                    return open_chat(s, id);
                }
                vec![]
            }
        },
        View::Proposals => {
            if s.focus == Focus::Detail {
                if let Some(d) = s.diff.as_mut() {
                    let max = d.rows().len().saturating_sub(1);
                    use Action::*;
                    d.scroll = match a {
                        Up => d.scroll.saturating_sub(1),
                        Down => (d.scroll + 1).min(max),
                        PageUp => d.scroll.saturating_sub(10),
                        PageDown => (d.scroll + 10).min(max),
                        Top => 0,
                        Bottom => max,
                        _ => d.scroll,
                    };
                }
                return vec![];
            }
            let before = s.proposal_sel.index;
            list_nav(&mut s.proposal_sel, s.proposals.len(), a);
            if before != s.proposal_sel.index {
                return preview_selected_diff(s);
            }
            vec![]
        }
        View::Agents => {
            let rows = s.agent_rows();
            list_nav(&mut s.agent_sel, rows, a);
            vec![]
        }
        View::Hosts => {
            list_nav(&mut s.host_sel, s.hosts.len(), a);
            vec![]
        }
        View::People => {
            list_nav(&mut s.member_sel, s.members.len(), a);
            vec![]
        }
        View::Notifications => {
            list_nav(&mut s.notification_sel, s.notifications.len(), a);
            vec![]
        }
        View::Settings => {
            list_nav(&mut s.settings_sel, SETTINGS_TOGGLES, a);
            vec![]
        }
        View::Workflows => {
            let n = s.workflow_rows();
            list_nav(&mut s.workflow_sel, n, a);
            vec![]
        }
        View::Memory => {
            let n = s.memory_rows();
            list_nav(&mut s.memory_sel, n, a);
            vec![]
        }
    }
}

/// The selection moved: drop the old diff and preview the new one once the
/// selection rests, so holding ↓ through a long list loads one diff.
fn preview_selected_diff(s: &mut State) -> Vec<Effect> {
    let (Some(w), Some(p)) = (s.active.clone(), s.selected_proposal().map(|p| p.id.clone())) else { return vec![] };
    if s.diff.as_ref().is_some_and(|d| d.proposal == p) {
        return vec![];
    }
    s.diff = None;
    vec![Effect::PreviewDiff(w, p)]
}

fn load_selected_diff(s: &mut State) -> Vec<Effect> {
    let (Some(w), Some(p)) = (s.active.clone(), s.selected_proposal().map(|p| p.id.clone())) else { return vec![] };
    if s.diff.as_ref().is_some_and(|d| d.proposal == p) {
        return vec![];
    }
    s.diff = None;
    vec![Effect::LoadDiff(w, p)]
}

fn select(s: &mut State) -> Vec<Effect> {
    match s.view {
        View::Chats => {
            match s.focus {
                Focus::List => {
                    if let Some(c) = s.chats.get(s.chat_sel.index) {
                        let id = c.id.clone();
                        let fx = open_chat(s, id);
                        s.focus = Focus::Composer;
                        return fx;
                    }
                }
                Focus::Detail => s.focus = Focus::Composer,
                Focus::Composer => {}
            }
            vec![]
        }
        View::Proposals => {
            let fx = load_selected_diff(s);
            if s.selected_proposal().is_some() {
                s.focus = Focus::Detail;
            }
            fx
        }
        View::Notifications => {
            let Some(n) = s.notifications.get(s.notification_sel.index).cloned() else { return vec![] };
            let mut fx = vec![];
            if s.active.as_ref() != Some(&n.workspace) {
                fx.extend(switch_workspace(s, n.workspace.clone()));
            }
            s.view = View::Chats;
            if let Some(c) = n.chat {
                // Open it once the chat list arrives, or now if it's here.
                if let Some(i) = s.chats.iter().position(|x| x.id == c) {
                    s.chat_sel.index = i;
                }
                fx.extend(open_chat(s, c));
            }
            if let Some(x) = s.notifications.get_mut(s.notification_sel.index) {
                x.read = true;
            }
            fx
        }
        View::Settings => {
            let Some(st) = &s.settings else { return vec![] };
            match s.settings_sel.index {
                0 => vec![Effect::SetSetting(SettingKey::NotifyMentions, !st.notify_mentions)],
                1 => vec![Effect::SetSetting(SettingKey::AgentsMayEdit, !st.agents_may_edit)],
                _ => vec![],
            }
        }
        View::Workflows => {
            // Enter on a run opens its chat; on a definition, starts a run.
            match s.selected_workflow_item() {
                Some(WorkflowItem::Run(r)) => {
                    let chat = r.chat.clone();
                    s.view = View::Chats;
                    if let Some(i) = s.chats.iter().position(|c| c.id == chat) {
                        s.chat_sel.index = i;
                    }
                    open_chat(s, chat)
                }
                Some(WorkflowItem::Def(_)) => main_action(s, Action::StartRun),
                None => vec![],
            }
        }
        // Enter on a runtime available on this device adds it as an agent.
        View::Agents if s.selected_local().is_some() => main_action(s, Action::AddAgent),
        View::People | View::Agents | View::Hosts | View::Memory => vec![],
    }
}
