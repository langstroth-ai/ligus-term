//! The reducer: `update(state, msg)` changes the state and returns the
//! backend calls to make. It never calls the backend itself and never
//! draws, so it is tested with plain values.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::input::TextBuf;
use crate::app::state::*;
use crate::backend::model::*;
use crate::backend::{Event, Result};
use crate::keymap::Action;
use crate::setup::LtFlow;
use ligus_runtime::setup::FlowId;

/// Something that happened: a key, a paste, a tick, a backend event, or the
/// answer to an [`Effect`].
#[derive(Debug, Clone)]
pub enum Msg {
    /// A copy reached a clipboard (the toast), or how to get the code.
    Copied(std::result::Result<String, String>),
    Key(KeyEvent),
    Paste(String),
    /// The terminal gained (`true`) or lost focus.
    Focus(bool),
    /// A mouse event (when mouse support is on).
    Mouse(crossterm::event::MouseEvent),
    /// Wall clock, for ages and toasts.
    Tick(Millis),
    /// The terminal's new size (columns, rows).
    Resize(u16, u16),
    Event(Event),
    Startup(Result<StartupState>),
    Workspaces(Result<Vec<WorkspaceSummary>>),
    Chats(WorkspaceId, Result<Vec<ChatSummary>>),
    Transcript(WorkspaceId, ChatId, Result<Vec<Message>>),
    /// A chat's decision cards.
    Asks(WorkspaceId, ChatId, Result<Vec<Ask>>),
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
    /// A proposal archived, restored, hidden here or shown again.
    Shelved(WorkspaceId, Result<Proposal>),
    Invite(Result<String>),
    /// The invite (as typed) and what it says about its sender.
    InviteChecked(String, Result<InviteCheck>),
    /// This computer's link request (Add to my devices).
    LinkRequested(Result<LinkRequestView>),
    /// A grant (as typed) and who it says approved this computer.
    LinkChecked(String, Result<LinkGrantView>),
    /// The grant was redeemed: this computer is the person's device here.
    Linked(Result<WorkspaceSummary>),
    /// A live link request was posted (or the relay can't carry one).
    LiveStarted(Result<LiveLinkStart>),
    /// The answer to the code went to the request.
    LiveAnswered(Result<()>),
    /// The answer to the verified person: linked (`Some`) or not.
    LivePerson(Result<Option<WorkspaceSummary>>),
    /// The workspace's live approval requests (read only).
    Approvals(WorkspaceId, Result<Vec<ApprovalRequest>>),
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
    /// What the chat repo picker offers.
    RepoChoices(WorkspaceId, Result<RepoChoices>),
    /// A scratch repo made for a chat.
    ScratchMade(WorkspaceId, Result<ChatRepo>),
    /// A scratch repo kept: where it is now.
    ScratchKept(WorkspaceId, Result<String>),
    /// A stop was asked for: how many turns stopped here.
    Stopped(WorkspaceId, ChatId, Result<usize>),
    /// The text went to the pager (`true`) or the scrollback.
    Shown(Result<bool>),
    /// The runtime switched the theme (`state.theme_id` says to what).
    ThemeChanged,
    /// The runtime switched the appearance (`state.appearance`).
    AppearanceChanged,
    /// The draft came back from the external editor.
    Edited(Result<String>),
    /// The Set up list's rows.
    SetupRows(Result<Vec<crate::setup::FlowRow>>),
    /// A flow's form.
    SetupForm(crate::setup::LtFlow, Result<crate::setup::SetupForm>),
    /// A flow's run finished.
    SetupRan(crate::setup::LtFlow, Result<crate::setup::SetupOutcome>),
    /// A path field's completion: the field and what matched.
    PathCompleted(String, crate::setup::complete::Completion),
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
    LoadAsks(WorkspaceId, ChatId),
    /// Answer a decision card on its chat's log.
    AnswerAsk(WorkspaceId, ChatId, AskId, AskReply),
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
    /// Archive (`true`) or restore a proposal, for everyone.
    ArchiveProposal(WorkspaceId, ProposalId, bool),
    /// Hide (`true`) a proposal on this device, or show it again.
    HideProposal(WorkspaceId, ProposalId, bool),
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
    /// Make this computer's link request.
    LinkRequest,
    /// Read a desktop's grant (nothing saved).
    InspectLink(String),
    /// Redeem it, after the person confirmed.
    RedeemLink(String),
    /// Post this computer's link request to an approval address (live).
    LinkLive(String),
    /// The person's answer to the code of live request `rid`.
    LinkLiveCode(String, bool),
    /// The person's answer to "Link this computer as … (verified)?".
    LinkLivePerson(String, bool),
    /// End live request `rid` from this computer.
    LinkLiveCancel(String),
    /// The workspace's live approval requests.
    LoadApprovals(WorkspaceId),
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
    LoadRepoChoices(WorkspaceId),
    /// Set a chat's repository (`None`: no repository).
    SetChatRepo(WorkspaceId, ChatId, Option<String>),
    /// Make a scratch repo on this device for a chat.
    NewScratchRepo(WorkspaceId, ChatId),
    /// Keep a chat's scratch repo at this folder.
    KeepScratch(WorkspaceId, ChatId, String),
    /// Stop the turns running on this device in a chat.
    StopTurn(WorkspaceId, ChatId),
    /// Show this text in `$PAGER`, or leave it in the terminal's
    /// scrollback.
    ShowText(String),
    /// Suspend the screen and edit this draft in `$VISUAL`/`$EDITOR`; the
    /// runtime answers [`Msg::Edited`].
    EditDraft(String),
    /// The Set up list for the workspace (or before there is one).
    LoadSetup(Option<WorkspaceId>),
    /// A flow's form for the values so far.
    SetupForm(Option<WorkspaceId>, crate::setup::LtFlow, crate::setup::SetupInputs),
    /// Run a flow (as a person). The secrets go to the engine only.
    SetupRun {
        workspace: Option<WorkspaceId>,
        flow: crate::setup::LtFlow,
        inputs: crate::setup::SetupInputs,
        secrets: crate::setup::SetupSecrets,
        dry_run: bool,
        cancel: crate::setup::CancelFlag,
    },
    /// Stop a run between steps.
    CancelSetup(crate::setup::CancelFlag),
    /// Complete a path field's folder (lists one directory).
    CompletePath(String, String),
    /// Copy a code to the clipboard (only ever on a key press); the
    /// runtime answers [`Msg::Copied`].
    Copy(crate::clipboard::Clip),
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
    // A workspace's first lists set what already needs you, quietly.
    let quiet = match &msg {
        Msg::Startup(_) => true,
        Msg::Chats(ws, Ok(_)) if is_active(s, ws) => s.loaded.insert("chats"),
        Msg::Proposals(ws, Ok(_)) if is_active(s, ws) => s.loaded.insert("proposals"),
        Msg::Tasks(ws, Ok(_)) if is_active(s, ws) => s.loaded.insert("tasks"),
        _ => false,
    };
    let fx = update_inner(s, msg);
    watch_attention(s, quiet);
    fx
}

/// A chat newly needing you rings the bell and, while the terminal is
/// unfocused, sends one desktop notification (OSC 9) for everything that
/// arrived together: "2 chats need you".
fn watch_attention(s: &mut State, quiet: bool) {
    if !matches!(s.phase, Phase::Main) {
        return;
    }
    let now: std::collections::BTreeSet<ChatId> =
        s.chats.iter().filter(|c| s.session_state(&c.id) == SessionState::NeedsYou).map(|c| c.id.clone()).collect();
    let new: Vec<ChatId> = now.difference(&s.attention).cloned().collect();
    s.attention = now;
    // What was already waiting when the workspace opened isn't news.
    if quiet || new.is_empty() || !s.settings.as_ref().is_none_or(|st| st.notify_mentions) {
        return;
    }
    let text = match new.as_slice() {
        [one] => {
            let title = s.chats.iter().find(|c| &c.id == one).map(|c| c.title.clone()).unwrap_or_default();
            format!("\u{201c}{title}\u{201d} needs you")
        }
        many => format!("{} chats need you", many.len()),
    };
    s.bell = true;
    s.signal(text);
}

fn update_inner(s: &mut State, msg: Msg) -> Vec<Effect> {
    match msg {
        Msg::Focus(on) => {
            s.focused = on;
            vec![]
        }
        Msg::Key(k) => key(s, k),
        Msg::Paste(text) => paste(s, &text),
        Msg::Mouse(m) => mouse(s, m),
        Msg::Tick(now) => {
            s.now = now;
            s.toasts.iter_mut().for_each(|t| t.ttl = t.ttl.saturating_sub(1));
            s.toasts.retain(|t| t.ttl > 0);
            live_tick(s)
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
        Msg::Asks(ws, chat, Ok(a)) if is_active(s, &ws) && s.open_chat.as_ref() == Some(&chat) => {
            s.asks = a;
            vec![]
        }
        Msg::Mentionables(ws, Ok(m)) if is_active(s, &ws) => {
            s.mentionables = m;
            vec![]
        }
        Msg::Proposals(ws, Ok(p)) if is_active(s, &ws) => {
            let (inbox, off): (Vec<_>, Vec<_>) = p.into_iter().partition(|p| p.in_inbox());
            s.proposals = inbox;
            s.archived_proposals = off;
            s.proposal_sel.clamp(s.shown_proposals().len());
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
            // A workspace with no agents yet: look for what this device
            // could run, for the getting-started list.
            if s.agents.is_empty() && s.local.is_none() { vec![Effect::LoadLocalRuntimes(ws)] } else { vec![] }
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
            // A link started and not finished: say so once, on start.
            if !s.link_notice_shown
                && let Some(line) = s.link_pending_line()
            {
                s.link_notice_shown = true;
                s.toast(ToastKind::Info, line);
            }
            vec![]
        }
        Msg::Copied(Ok(m)) => {
            s.toast(ToastKind::Ok, m);
            vec![]
        }
        Msg::Copied(Err(m)) => {
            s.toast(ToastKind::Info, m);
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
        Msg::Shelved(ws, Ok(p)) => {
            let what = match p.shelf {
                Shelf::Inbox => "Back in Proposals",
                Shelf::Archived | Shelf::AutoArchived => "Archived for everyone",
                Shelf::Hidden => "Hidden on this device",
            };
            s.toast(ToastKind::Ok, format!("{what}: \u{201c}{}\u{201d}", p.title));
            upsert_proposal(s, &ws, p);
            // The selection moved onto another row: show its diff.
            if is_active(s, &ws) && s.view == View::Proposals { preview_selected_diff(s) } else { vec![] }
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
        Msg::LinkRequested(r) => {
            let form = match (&mut s.overlay, &mut s.phase) {
                (Some(Overlay::AddWorkspace(f)), _) => f,
                (_, Phase::Onboarding(f)) => f,
                _ => return vec![],
            };
            match r {
                Ok(v) => form.link = Some(v),
                Err(e) => {
                    form.error = Some(e.0);
                    // Asked again on the next key.
                    form.link_asked = false;
                }
            }
            vec![]
        }
        Msg::LinkChecked(grant, r) => {
            let form = match (&mut s.overlay, &mut s.phase) {
                (Some(Overlay::AddWorkspace(f)), _) => f,
                (_, Phase::Onboarding(f)) => f,
                _ => return vec![],
            };
            form.busy = false;
            match r {
                Ok(v) => form.link_checked = Some((grant, v)),
                Err(e) => form.error = Some(e.0),
            }
            vec![]
        }
        Msg::Linked(Err(e)) => {
            form_error(s, e.0);
            vec![]
        }
        Msg::Linked(Ok(ws)) => {
            let who = match (&s.overlay, &s.phase) {
                (Some(Overlay::AddWorkspace(f)), _) | (_, Phase::Onboarding(f)) => match &f.live {
                    Some(LiveLink { stage: LiveStage::Confirm { person, .. }, .. }) => Some(person.clone()),
                    _ => f.link_checked.as_ref().map(|(_, v)| v.person_name.clone()),
                },
                _ => None,
            };
            s.overlay = None;
            if !s.workspaces.iter().any(|w| w.id == ws.id) {
                s.workspaces.push(ws.clone());
            }
            s.your_name = who.clone().or_else(|| s.your_name.clone()).or_else(|| Some(String::new()));
            s.toast(ToastKind::Ok, format!("Linked: one of {}'s devices in “{}”", who.unwrap_or_else(|| "your".into()), ws.name));
            s.phase = Phase::Main;
            let mut fx = switch_workspace(s, ws.id);
            fx.push(Effect::LoadSettings);
            fx
        }
        Msg::LiveStarted(r) => {
            let Some(form) = link_form(s) else { return vec![] };
            form.busy = false;
            match r {
                Ok(LiveLinkStart::Posted { rid, request }) => {
                    form.link = Some(request);
                    form.live = Some(LiveLink { rid: Some(rid), stage: LiveStage::Waiting });
                }
                // No approval mailbox on that relay: paste the request on
                // the desktop instead (the fingerprint compare, as before).
                Ok(LiveLinkStart::NoMailbox { request }) => {
                    form.link = Some(request);
                    form.live = None;
                    form.live_unavailable = true;
                    form.grant.take();
                }
                Err(e) => {
                    form.live = None;
                    form.error = Some(e.0);
                }
            }
            vec![]
        }
        Msg::LiveAnswered(Err(e)) => {
            if let Some(f) = link_form(s)
                && let Some(l) = &mut f.live
                && !matches!(l.stage, LiveStage::Ended(_))
            {
                l.stage = LiveStage::Ended(LiveLinkEnd::Refused(e.0));
            }
            vec![]
        }
        Msg::LiveAnswered(Ok(())) => vec![],
        Msg::LivePerson(Ok(Some(ws))) => update(s, Msg::Linked(Ok(ws))),
        Msg::LivePerson(Ok(None)) => vec![],
        Msg::LivePerson(Err(e)) => {
            if let Some(f) = link_form(s)
                && let Some(l) = &mut f.live
            {
                l.stage = LiveStage::Ended(LiveLinkEnd::Refused(e.0));
            }
            vec![]
        }
        Msg::Approvals(w, r) => {
            if s.active.as_ref() == Some(&w)
                && let Some(Overlay::Approvals { list }) = &mut s.overlay
            {
                *list = Some(r.map_err(|e| e.0));
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
        Msg::RepoChoices(ws, Ok(c)) if is_active(s, &ws) => {
            let current = s.open_chat_summary().and_then(|c| c.repo.as_ref()).map(|r| r.id.clone());
            if let Some(Overlay::RepoPicker { sel, choices, .. }) = &mut s.overlay {
                // On the chat's repository, else on "New scratch repo".
                *sel = current.and_then(|id| c.repos.iter().position(|r| r.id == id)).map_or(0, |i| i + 1);
                *choices = Some(c);
            }
            vec![]
        }
        Msg::RepoChoices(_, Err(e)) => {
            if matches!(s.overlay, Some(Overlay::RepoPicker { .. })) {
                s.overlay = None;
            }
            s.toast(ToastKind::Error, format!("Couldn't list the repositories: {e}"));
            vec![]
        }
        Msg::ScratchMade(ws, Ok(r)) => {
            s.toast(ToastKind::Ok, format!("Scratch repo {} made on this device: this chat's agents work in it", r.name));
            if is_active(s, &ws) { vec![Effect::LoadChats(ws)] } else { vec![] }
        }
        Msg::ScratchMade(_, Err(e)) => {
            s.toast(ToastKind::Error, format!("Couldn't make a scratch repo: {e}"));
            vec![]
        }
        Msg::ScratchKept(ws, Ok(path)) => {
            s.toast(ToastKind::Ok, format!("Kept at {path}: an ordinary repository now"));
            if is_active(s, &ws) { vec![Effect::LoadChats(ws)] } else { vec![] }
        }
        Msg::ScratchKept(_, Err(e)) => {
            s.toast(ToastKind::Error, format!("Couldn't keep the scratch repo: {e}"));
            vec![]
        }
        Msg::Stopped(_, _, Ok(0)) => {
            s.toast(ToastKind::Info, "Nothing runs on this device in this chat: the turn's host stops it");
            vec![]
        }
        Msg::Stopped(ws, chat, Ok(n)) => {
            let who = s.thinking.get(&chat).map(|a| a.iter().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let what = if n == 1 { "the turn".to_string() } else { format!("{n} turns") };
            s.toast(ToastKind::Ok, if who.is_empty() { format!("Stopped {what}") } else { format!("Stopped {what} ({who})") });
            if is_active(s, &ws) { vec![Effect::LoadTranscript(ws, chat)] } else { vec![] }
        }
        Msg::Stopped(_, _, Err(e)) => {
            s.toast(ToastKind::Error, format!("Couldn't stop the turn: {e}"));
            vec![]
        }
        Msg::Shown(Ok(false)) => {
            s.toast(ToastKind::Ok, "The transcript is in your terminal's scrollback (set $PAGER to read it in a pager)");
            vec![]
        }
        Msg::Shown(Ok(true)) => vec![],
        Msg::Shown(Err(e)) => {
            s.toast(ToastKind::Error, format!("Couldn't show the transcript: {e}"));
            vec![]
        }
        Msg::ThemeChanged => {
            let label = s.themes.iter().find(|(id, _)| *id == s.theme_id).map(|(_, l)| l.clone()).unwrap_or_else(|| s.theme_id.clone());
            let id = s.theme_id.clone();
            s.toast(ToastKind::Ok, format!("Theme: {label}, for this session (theme = \"{id}\" in the config file keeps it)"));
            vec![]
        }
        Msg::AppearanceChanged => {
            let a = s.appearance.name();
            s.toast(ToastKind::Ok, format!("Appearance: {a}, for this session (appearance = \"{a}\" in the config file keeps it)"));
            vec![]
        }
        Msg::SetupRows(Ok(rows)) => {
            s.setup_rows = Some(rows);
            if let Some(Overlay::Setup { sel }) = &mut s.overlay {
                *sel = (*sel).min(s.setup_rows.as_ref().map_or(0, |r| r.len().saturating_sub(1)));
            }
            vec![]
        }
        Msg::SetupRows(Err(e)) => {
            s.toast(ToastKind::Error, format!("Couldn't list the setup flows: {e}"));
            vec![]
        }
        Msg::SetupForm(flow, r) => {
            crate::app::setup::formed(s, flow, r);
            vec![]
        }
        Msg::SetupRan(flow, r) => crate::app::setup::ran(s, flow, r),
        Msg::PathCompleted(field, c) => {
            crate::app::setup::completed(s, &field, c);
            vec![]
        }
        Msg::Edited(Ok(text)) => {
            if s.open_chat.is_some() {
                s.composer.set(text.trim_end_matches(['\n', '\r']));
                s.history_pos = None;
                if s.view == View::Chats {
                    s.focus = Focus::Composer;
                }
            }
            vec![]
        }
        Msg::Edited(Err(e)) => {
            s.toast(ToastKind::Error, format!("Couldn't open the editor: {e}"));
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
        | Msg::Shelved(_, Err(e))
        | Msg::Invite(Err(e))
        | Msg::ChatCreated(_, Err(e))
        | Msg::Workspaces(Err(e))
        | Msg::Chats(_, Err(e))
        | Msg::Transcript(_, _, Err(e))
        | Msg::Asks(_, _, Err(e))
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
        | Msg::Asks(..)
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
        | Msg::RepoChoices(..)
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
    // Each proposal is on one list: the inbox, or archived and hidden.
    let (list, other) =
        if p.in_inbox() { (&mut s.proposals, &mut s.archived_proposals) } else { (&mut s.archived_proposals, &mut s.proposals) };
    other.retain(|x| x.id != p.id);
    match list.iter_mut().find(|x| x.id == p.id) {
        Some(x) => *x = p,
        None => list.insert(0, p),
    }
    let n = s.shown_proposals().len();
    s.proposal_sel.clamp(n);
}

fn event(s: &mut State, e: Event) -> Vec<Effect> {
    match e {
        Event::LiveLink { rid, update } => live_event(s, &rid, update),
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
                // A turn you asked for ended while you were away.
                let mine = s.my_chats.contains(&chat);
                if mine && !s.focused && s.settings.as_ref().is_none_or(|st| st.notify_mentions) {
                    let title = s.chats.iter().find(|c| c.id == chat).map(|c| c.title.clone()).unwrap_or_default();
                    let what = if error.is_some() { "failed" } else { "finished" };
                    s.bell = true;
                    s.signal(format!("A turn {what} in \u{201c}{title}\u{201d}"));
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
        // A card asked or answered: the open chat's cards, and the list's
        // "needs you" counts.
        Event::AsksChanged { workspace, chat } if is_active(s, &workspace) => {
            let mut fx = vec![Effect::LoadChats(workspace.clone())];
            if s.open_chat.as_ref() == Some(&chat) {
                fx.push(Effect::LoadAsks(workspace, chat));
            }
            fx
        }
        Event::AsksChanged { .. } => vec![],
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
        Event::SetupFactsChanged => crate::app::setup::refresh(s),
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
        Event::Notification(mut n) => {
            // A mention in the chat you're looking at is already on screen:
            // no bell, no toast, and it counts as read.
            let here = s.focused
                && s.overlay.is_none()
                && s.view == View::Chats
                && is_active(s, &n.workspace)
                && n.chat.is_some()
                && n.chat == s.open_chat;
            if here {
                n.read = true;
                let (w, c) = (n.workspace.clone(), n.chat.clone());
                s.notifications.insert(0, n);
                // The backend's history agrees it was seen.
                return c.map(|c| vec![Effect::MarkRead(w, c)]).unwrap_or_default();
            }
            s.bell = s.settings.as_ref().is_none_or(|st| st.notify_mentions);
            if s.bell {
                s.signal(n.text.clone());
            }
            // A burst is one notice: "3 new notifications", not three.
            let burst = if !s.toasts.is_empty() && s.notice_burst > 0 { s.notice_burst + 1 } else { 1 };
            if burst > 1 {
                let key = s.keymap.hint(crate::keymap::Context::Global, Action::GoNotifications);
                s.toast_logging(ToastKind::Info, format!("{burst} new notifications ({key} reads them)"), n.text.clone());
            } else {
                s.toast(ToastKind::Info, n.text.clone());
            }
            s.notice_burst = burst;
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

/// A click or the wheel. Overlays take the wheel as ↑/↓; in the main
/// screen a click on a tab opens its view, on a row selects it, on a pane
/// focuses it, and the wheel scrolls what is under the pointer.
fn mouse(s: &mut State, m: crossterm::event::MouseEvent) -> Vec<Effect> {
    use crossterm::event::{MouseButton, MouseEventKind};
    let arrow = |code| Msg::Key(KeyEvent::new(code, KeyModifiers::NONE));
    let (up, down) = match m.kind {
        MouseEventKind::ScrollUp => (true, false),
        MouseEventKind::ScrollDown => (false, true),
        MouseEventKind::Down(MouseButton::Left) => (false, false),
        _ => return vec![],
    };
    if !s.presence.is_empty() || !matches!(s.phase, Phase::Main) {
        return vec![];
    }
    if s.overlay.is_some() {
        return match (up, down) {
            (true, _) => update(s, arrow(KeyCode::Up)),
            (_, true) => update(s, arrow(KeyCode::Down)),
            _ => vec![],
        };
    }
    if m.row == 0 && !s.focus_mode {
        if !(up || down)
            && let Some(v) = crate::ui::tab_at(s, s.term.0, m.column)
        {
            return go(s, v);
        }
        return vec![];
    }
    let body = crate::ui::body_area(s);
    if m.row >= body.bottom() {
        return vec![];
    }
    let wheel = |s: &mut State, a: Action| {
        let mut fx = vec![];
        for _ in 0..3 {
            fx.extend(nav_action(s, a));
        }
        fx
    };
    let step = if up { Action::Up } else { Action::Down };
    match s.view {
        View::Chats => match crate::ui::chat_hit(s, body, m.column, m.row) {
            Some(crate::ui::ChatHit::List(_)) if up || down => {
                let before = s.focus;
                s.focus = Focus::List;
                let fx = nav_action(s, step);
                s.focus = if before == Focus::Composer { Focus::List } else { before };
                fx
            }
            Some(crate::ui::ChatHit::List(row)) => {
                s.focus = Focus::List;
                match row.and_then(|i| s.chats.get(i).map(|c| (i, c.id.clone()))) {
                    Some((i, id)) => {
                        s.chat_sel.index = i;
                        open_chat(s, id)
                    }
                    None => vec![],
                }
            }
            Some(crate::ui::ChatHit::Transcript) if up || down => {
                let before = s.focus;
                s.focus = Focus::Detail;
                let fx = wheel(s, step);
                s.focus = before;
                fx
            }
            Some(crate::ui::ChatHit::Transcript) => {
                s.focus = Focus::Detail;
                vec![]
            }
            Some(crate::ui::ChatHit::Composer) if !(up || down) && s.open_chat.is_some() => {
                s.focus = Focus::Composer;
                vec![]
            }
            _ => vec![],
        },
        View::Proposals => match crate::ui::proposal_hit(s, body, m.column, m.row) {
            crate::ui::ProposalHit::List(_) if up || down => {
                s.focus = Focus::List;
                nav_action(s, step)
            }
            crate::ui::ProposalHit::List(Some(i)) => {
                s.focus = Focus::List;
                s.proposal_sel.index = i;
                preview_selected_diff(s)
            }
            crate::ui::ProposalHit::List(None) => vec![],
            crate::ui::ProposalHit::Detail if s.diff.is_some() => {
                s.focus = Focus::Detail;
                if up || down { wheel(s, step) } else { vec![] }
            }
            crate::ui::ProposalHit::Detail => vec![],
        },
        _ if up || down => nav_action(s, step),
        _ => vec![],
    }
}

/// A message as a sentence: its first letter upper-case.
fn sentence(m: &str) -> String {
    let mut c = m.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn paste(s: &mut State, text: &str) -> Vec<Effect> {
    if let Some(p) = s.presence.front_mut() {
        if let PresenceStage::EnteringPath(b) = &mut p.stage {
            b.insert_str(text.trim());
        }
        return vec![];
    }
    if s.find.is_some() && s.overlay.is_none() {
        if let Some(f) = s.find.as_mut() {
            f.query.insert_str(text.trim());
        }
        let q = s.find.as_ref().map(|f| f.query.text.clone()).unwrap_or_default();
        return match find_matches(s, &q).first().copied() {
            Some(i) => select_row(s, i),
            None => vec![],
        };
    }
    match (&mut s.overlay, &mut s.phase) {
        (Some(Overlay::Help { query, searching: true, scroll }), _) => {
            query.insert_str(text.trim());
            *scroll = 0;
        }
        (Some(Overlay::AddWorkspace(f)), _) | (None, Phase::Onboarding(f))
            if f.mode == FormMode::Link && text.trim().starts_with("ligusaddr1:") =>
        {
            // A pasted address or code replaces what the field held.
            f.grant.take();
            f.grant.insert_str(text.trim());
            f.field = FormField::Value;
            f.link_checked = None;
            f.error = None;
        }
        (Some(Overlay::AddWorkspace(f)), _) | (None, Phase::Onboarding(f)) => {
            // A desktop's code goes straight to who approved it, whatever
            // came with it (whitespace, line breaks, a copied command).
            let on_code = matches!(f.field, FormField::Value | FormField::Mode);
            if on_code && !f.busy && (f.mode == FormMode::Link || text.contains(crate::link_code::GRANT_PREFIX)) {
                match crate::link_code::grant_in(text) {
                    crate::link_code::Pasted::Grant(g) => {
                        f.mode = FormMode::Link;
                        f.field = FormField::Value;
                        f.grant.set(&g);
                        f.error = None;
                        f.checked = None;
                        f.link_checked = None;
                        f.busy = true;
                        // An older desktop's code: the paste path, and any
                        // live request ends.
                        let mut fx: Vec<Effect> = live_in_flight(f).map(Effect::LinkLiveCancel).into_iter().collect();
                        f.live = None;
                        fx.push(Effect::InspectLink(g));
                        return fx;
                    }
                    other if f.mode == FormMode::Link => {
                        f.error = Some(sentence(&crate::link_code::not_a_grant(&other, false)));
                        return vec![];
                    }
                    _ => {}
                }
            }
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
        (Some(Overlay::Palette { query: b, sel }), _) | (Some(Overlay::History { query: b, sel }), _) => {
            b.insert_str(text.trim());
            *sel = 0;
        }
        (Some(Overlay::NewChat(b)), _)
        | (Some(Overlay::BindCheckout(b)), _)
        | (Some(Overlay::AddAgent { name: b, .. }), _)
        | (Some(Overlay::KeepScratch { path: b, .. }), _) => b.insert_str(text.trim()),
        (Some(Overlay::AskReply { text: b, .. }), _) => b.insert_str(&text.replace(['\n', '\r'], " ")),
        (Some(Overlay::StartRun { input, .. }), _) => input.insert_str(text),
        (Some(Overlay::SetupFlow(w)), _) => crate::app::setup::paste(w, text),
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
                FormOutcome::Cancel if live_in_flight(f).is_some() => live_stop(f),
                FormOutcome::Cancel => {
                    s.quit = true;
                    vec![]
                }
                FormOutcome::None => link_effects(f),
                out => link_code_outcome(s, out),
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
    /// Copy this computer's link request (`c`, or ctrl-y).
    CopyLink,
    /// Show the link request alone, full screen (`o`).
    ShowLink,
    /// Answer a live request's question (the code, or the verified person).
    LiveAnswer(bool),
}

/// The fallback when no clipboard is reachable for the link request.
const LINK_FALLBACK: &str = "run lt --plain link-request";

/// Copy the link request, or open it full screen, for a form outcome.
fn link_code_outcome(s: &mut State, out: FormOutcome) -> Vec<Effect> {
    if let FormOutcome::LiveAnswer(yes) = out {
        return link_form(s).map(|f| live_answer(f, yes)).unwrap_or_default();
    }
    let form = match (&s.overlay, &s.phase) {
        (Some(Overlay::AddWorkspace(f)), _) | (None, Phase::Onboarding(f)) => f,
        _ => return vec![],
    };
    let Some(r) = form.link.clone() else { return vec![] };
    match out {
        FormOutcome::CopyLink => vec![Effect::Copy(crate::clipboard::Clip::new(r.code, LINK_FALLBACK))],
        FormOutcome::ShowLink => {
            show_code(s, "This computer's link request", r.code, Some(r.fingerprint), LINK_FALLBACK);
            vec![]
        }
        _ => vec![],
    }
}

/// Show a code alone on the screen, with no border in its lines, so a
/// mouse selection takes just the code; `esc` goes back to what was open.
pub(crate) fn show_code(s: &mut State, title: &str, code: String, fingerprint: Option<String>, fallback: &'static str) {
    let back = s.overlay.take().map(Box::new);
    s.overlay = Some(Overlay::Code { title: title.into(), code, fingerprint, fallback, back });
}

/// On Add to my devices, make this computer's link request once.
fn link_effects(f: &mut WorkspaceForm) -> Vec<Effect> {
    if f.mode == FormMode::Link && !f.link_asked {
        f.link_asked = true;
        return vec![Effect::LinkRequest];
    }
    vec![]
}

/// The Add to my devices form, wherever it is open.
fn link_form(s: &mut State) -> Option<&mut WorkspaceForm> {
    match (&mut s.overlay, &mut s.phase) {
        (Some(Overlay::AddWorkspace(f)), _) => Some(f),
        (_, Phase::Onboarding(f)) => Some(f),
        _ => None,
    }
    .filter(|f| f.mode == FormMode::Link)
}

/// A live request still in flight (not ended): its id.
fn live_in_flight(f: &WorkspaceForm) -> Option<String> {
    match &f.live {
        Some(LiveLink { rid: Some(rid), stage }) if !matches!(stage, LiveStage::Ended(_)) => Some(rid.clone()),
        _ => None,
    }
}

/// Esc on a live request ends it (nothing is granted); a second Esc closes.
fn live_stop(f: &mut WorkspaceForm) -> Vec<Effect> {
    let Some(rid) = live_in_flight(f) else { return vec![] };
    if let Some(l) = &mut f.live {
        l.stage = LiveStage::Ended(LiveLinkEnd::Cancelled);
    }
    vec![Effect::LinkLiveCancel(rid)]
}

/// A live link request moved on. The first code is the one shown: a later
/// `Opened` (a racing desktop) never replaces it.
fn live_event(s: &mut State, rid: &str, update: LiveLinkUpdate) -> Vec<Effect> {
    let Some(f) = link_form(s) else { return vec![] };
    let Some(l) = f.live.as_mut().filter(|l| l.rid.as_deref() == Some(rid)) else { return vec![] };
    if matches!(l.stage, LiveStage::Ended(_)) {
        return vec![];
    }
    match update {
        LiveLinkUpdate::Opened(code) => {
            if matches!(l.stage, LiveStage::Sending | LiveStage::Waiting) {
                l.stage = LiveStage::Code { code, yes: false };
            }
        }
        LiveLinkUpdate::Acknowledged => {
            if let LiveStage::Matched { acknowledged, .. } = &mut l.stage {
                *acknowledged = true;
            }
        }
        LiveLinkUpdate::Checking => l.stage = LiveStage::Checking,
        LiveLinkUpdate::Verified { person, workspace } => l.stage = LiveStage::Confirm { person, workspace, yes: false, busy: false },
        LiveLinkUpdate::Ended(e) => l.stage = LiveStage::Ended(e),
    }
    vec![]
}

/// At zero the code is dropped (the desktop drops it too).
fn live_tick(s: &mut State) -> Vec<Effect> {
    let now = s.now;
    let Some(f) = link_form(s) else { return vec![] };
    let Some(l) = f.live.as_mut() else { return vec![] };
    match &l.stage {
        LiveStage::Code { code, .. } if now > code.expires_at => {
            l.stage = LiveStage::Ended(LiveLinkEnd::Expired);
            l.rid.clone().map(Effect::LinkLiveCancel).into_iter().collect()
        }
        _ => vec![],
    }
}

/// The person answered the code (`Some(yes)`) or the verified person.
fn live_answer(f: &mut WorkspaceForm, yes: bool) -> Vec<Effect> {
    let Some(l) = f.live.as_mut() else { return vec![] };
    let Some(rid) = l.rid.clone() else { return vec![] };
    match l.stage.clone() {
        LiveStage::Code { code, .. } => {
            if yes {
                l.stage = LiveStage::Matched { code, acknowledged: false };
            } else {
                l.stage = LiveStage::Ended(LiveLinkEnd::Mismatch);
            }
            vec![Effect::LinkLiveCode(rid, yes)]
        }
        LiveStage::Confirm { person, workspace, busy: false, .. } => {
            if yes {
                l.stage = LiveStage::Confirm { person, workspace, yes: true, busy: true };
            } else {
                l.stage = LiveStage::Ended(LiveLinkEnd::NotLinked { person });
            }
            vec![Effect::LinkLivePerson(rid, yes)]
        }
        _ => vec![],
    }
}

/// Keys while a live request is on the form; `None` lets the form have
/// the key.
fn live_key(f: &mut WorkspaceForm, k: &KeyEvent) -> Option<FormOutcome> {
    let stage = &mut f.live.as_mut()?.stage;
    match stage {
        LiveStage::Code { yes, .. } | LiveStage::Confirm { yes, busy: false, .. } => Some(match k.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('h') | KeyCode::Char('l') => {
                *yes = !*yes;
                FormOutcome::None
            }
            KeyCode::Char('y') => FormOutcome::LiveAnswer(true),
            KeyCode::Char('n') => FormOutcome::LiveAnswer(false),
            KeyCode::Enter => FormOutcome::LiveAnswer(*yes),
            KeyCode::Esc => FormOutcome::Cancel,
            _ => FormOutcome::None,
        }),
        LiveStage::Confirm { .. } => Some(FormOutcome::None),
        // Waiting: the request can still be copied for an older desktop,
        // and that desktop's code pasted into the field.
        LiveStage::Sending | LiveStage::Waiting | LiveStage::Matched { .. } | LiveStage::Checking => Some(match k.code {
            KeyCode::Esc => FormOutcome::Cancel,
            KeyCode::Char('c') if k.modifiers.is_empty() && f.link.is_some() => FormOutcome::CopyLink,
            KeyCode::Char('o') if k.modifiers.is_empty() && f.link.is_some() => FormOutcome::ShowLink,
            KeyCode::Char('y') if k.modifiers.contains(KeyModifiers::CONTROL) && f.link.is_some() => FormOutcome::CopyLink,
            KeyCode::Enter if f.grant.text.trim().starts_with(ligus_runtime::computer_link::GRANT_PREFIX) => FormOutcome::Submit,
            _ => FormOutcome::None,
        }),
        LiveStage::Ended(_) => None,
    }
}

fn form_key(f: &mut WorkspaceForm, k: &KeyEvent) -> FormOutcome {
    if f.busy {
        return FormOutcome::None;
    }
    if f.mode == FormMode::Link
        && let Some(out) = live_key(f, k)
    {
        return out;
    }
    let fields = f.fields();
    let i = fields.iter().position(|x| *x == f.field).unwrap_or(0);
    match k.code {
        KeyCode::Esc => return FormOutcome::Cancel,
        // A link code never holds a `?` (base64url): it opens the longer
        // explanation instead.
        KeyCode::Char('?') if f.mode == FormMode::Link => f.link_about = !f.link_about,
        // The request is pasted on the desktop: ctrl-y copies it anywhere
        // on the form; c and o work while the desktop's code is empty (a
        // grant is pasted, never typed, and starts with "ligusgrant1:").
        KeyCode::Char('y') if f.mode == FormMode::Link && f.link.is_some() && k.modifiers.contains(KeyModifiers::CONTROL) => {
            return FormOutcome::CopyLink;
        }
        KeyCode::Char('c') if f.mode == FormMode::Link && f.link.is_some() && f.grant.text.is_empty() && k.modifiers.is_empty() => {
            return FormOutcome::CopyLink;
        }
        KeyCode::Char('o') if f.mode == FormMode::Link && f.link.is_some() && f.grant.text.is_empty() && k.modifiers.is_empty() => {
            return FormOutcome::ShowLink;
        }
        KeyCode::Tab | KeyCode::Down => f.field = fields[(i + 1) % fields.len()],
        KeyCode::BackTab | KeyCode::Up => f.field = fields[(i + fields.len() - 1) % fields.len()],
        KeyCode::Enter => {
            if f.field == FormField::Value || (f.mode == FormMode::Demo && f.field == FormField::Mode) {
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
                    KeyCode::Char('a') => Some(FormMode::Link),
                    KeyCode::Char('d') => Some(FormMode::Connect),
                    _ => None,
                };
                // The demo is offered on the first run only.
                let mode = match mode {
                    Some(FormMode::Demo) if !f.ask_name => {
                        Some(if k.code == KeyCode::Left || k.code == KeyCode::Char('h') { FormMode::Connect } else { FormMode::Create })
                    }
                    m => m,
                };
                if let Some(m) = mode {
                    f.mode = m;
                    f.error = None;
                    f.checked = None;
                    f.link_checked = None;
                }
            }
            field => {
                if let Some(b) = f.buf_mut(field)
                    && b.handle(k)
                    && field == FormField::Value
                {
                    f.checked = None;
                    f.link_checked = None;
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
    if form.mode == FormMode::Demo {
        s.exec_demo = true;
        s.quit = true;
        return vec![];
    }
    if form.mode == FormMode::Link {
        let typed = form.grant.text.trim().to_string();
        if typed.is_empty() {
            form.error = Some("Paste the approval address your desktop shows".into());
            return vec![];
        }
        let compact: String = typed.split_whitespace().collect();
        // The approval address: the live path (code match).
        if compact.starts_with("ligusaddr1:") {
            form.error = None;
            if live_in_flight(form).is_some() {
                return vec![];
            }
            form.live = Some(LiveLink { rid: None, stage: LiveStage::Sending });
            form.live_unavailable = false;
            form.link_checked = None;
            return vec![Effect::LinkLive(compact)];
        }
        let grant = match crate::link_code::grant_in(&typed) {
            crate::link_code::Pasted::Grant(g) => g,
            other => {
                form.error = Some(sentence(&crate::link_code::not_a_grant(&other, false)));
                return vec![];
            }
        };
        form.error = None;
        // An older desktop's code: the paste path, and any live request ends.
        let mut fx: Vec<Effect> = live_in_flight(form).map(Effect::LinkLiveCancel).into_iter().collect();
        form.live = None;
        form.busy = true;
        // First show who approved it; link on the second Enter.
        fx.push(match &form.link_checked {
            Some((checked, _)) if *checked == grant => Effect::RedeemLink(grant),
            _ => Effect::InspectLink(grant),
        });
        return fx;
    }
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
            FormMode::Create | FormMode::Connect | FormMode::Demo => "Give the workspace a name".into(),
            FormMode::Join => "Paste an invite code".into(),
            FormMode::Link => "Paste the code your desktop shows".into(),
        });
        return vec![];
    }
    form.error = None;
    form.busy = true;
    match form.mode {
        FormMode::Create | FormMode::Connect | FormMode::Demo | FormMode::Link => {
            vec![Effect::CreateWorkspace { your_name, name: value }]
        }
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
    let available: Vec<LocalRuntime> =
        if matches!(s.overlay, Some(Overlay::AddAgentPicker { .. })) { s.available_local().into_iter().cloned().collect() } else { vec![] };
    let Some(overlay) = s.overlay.as_mut() else { return vec![] };
    match overlay {
        Overlay::Setup { sel } => {
            let sel = *sel;
            crate::app::setup::list_key(s, &k, sel)
        }
        Overlay::SetupFlow(_) => crate::app::setup::flow_key(s, &k),
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
        Overlay::Palette { query, sel } => {
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            match k.code {
                KeyCode::Esc => {
                    s.overlay = None;
                    if std::mem::take(&mut s.theme_previewing) {
                        s.theme_request = Some(crate::theme::ThemeRequest::Revert);
                    }
                }
                KeyCode::Up => *sel = sel.saturating_sub(1),
                KeyCode::Char('p') if ctrl => *sel = sel.saturating_sub(1),
                KeyCode::Down => *sel += 1,
                KeyCode::Char('n') if ctrl => *sel += 1,
                KeyCode::PageUp => *sel = sel.saturating_sub(10),
                KeyCode::PageDown => *sel += 10,
                KeyCode::Enter => {
                    let q = query.text.clone();
                    let i = *sel;
                    s.overlay = None;
                    s.theme_previewing = false;
                    let items = crate::app::palette::filtered(s, &q);
                    return match items.get(i.min(items.len().saturating_sub(1))) {
                        Some(it) => run_palette(s, it.target.clone()),
                        None => vec![],
                    };
                }
                _ => {
                    if query.handle(&k) {
                        *sel = 0;
                    }
                }
            }
            // Keep the selection on an entry.
            let q = match &s.overlay {
                Some(Overlay::Palette { query, .. }) => query.text.clone(),
                _ => return vec![],
            };
            let items = crate::app::palette::filtered(s, &q);
            let n = items.len();
            if let Some(Overlay::Palette { sel, .. }) = &mut s.overlay {
                *sel = (*sel).min(n.saturating_sub(1));
            }
            // A theme under the selection shows at once; moving off it
            // goes back to the theme kept.
            let at = match &s.overlay {
                Some(Overlay::Palette { sel, .. }) => items.get(*sel).map(|it| it.target.clone()),
                _ => None,
            };
            use crate::app::palette::Target;
            use crate::theme::ThemeRequest;
            match at {
                Some(Target::Theme(id)) => {
                    s.theme_request = Some(ThemeRequest::Preview { theme: Some(id), appearance: None });
                    s.theme_previewing = true;
                }
                Some(Target::Appearance(a)) => {
                    s.theme_request = Some(ThemeRequest::Preview { theme: None, appearance: Some(a) });
                    s.theme_previewing = true;
                }
                _ if s.theme_previewing => {
                    s.theme_request = Some(ThemeRequest::Revert);
                    s.theme_previewing = false;
                }
                _ => {}
            }
            vec![]
        }
        Overlay::History { query, sel } => {
            let matches = |s: &State, q: &str| -> Vec<String> {
                let q = q.to_lowercase();
                s.my_history().into_iter().filter(|t| t.to_lowercase().contains(q.trim())).collect()
            };
            match k.code {
                KeyCode::Esc => s.overlay = None,
                KeyCode::Up => *sel = sel.saturating_sub(1),
                KeyCode::Down => *sel += 1,
                KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => *sel += 1,
                KeyCode::Enter => {
                    let (q, i) = (query.text.clone(), *sel);
                    s.overlay = None;
                    if let Some(t) = matches(s, &q).get(i) {
                        s.composer.set(t);
                        s.history_pos = None;
                        s.focus = Focus::Composer;
                    }
                    return vec![];
                }
                _ => {
                    if query.handle(&k) {
                        *sel = 0;
                    }
                }
            }
            let q = match &s.overlay {
                Some(Overlay::History { query, .. }) => query.text.clone(),
                _ => return vec![],
            };
            let n = matches(s, &q).len();
            if let Some(Overlay::History { sel, .. }) = &mut s.overlay {
                *sel = (*sel).min(n.saturating_sub(1));
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
            FormOutcome::Cancel if live_in_flight(f).is_some() => live_stop(f),
            FormOutcome::Cancel => {
                s.overlay = None;
                vec![]
            }
            FormOutcome::None => link_effects(f),
            out => link_code_outcome(s, out),
        },
        Overlay::Approvals { .. } => match k.code {
            KeyCode::Char('r') => {
                let Some(w) = ws else { return vec![] };
                s.overlay = Some(Overlay::Approvals { list: None });
                vec![Effect::LoadApprovals(w)]
            }
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                s.overlay = None;
                vec![]
            }
            _ => vec![],
        },
        Overlay::Code { code, fallback, back, .. } => match k.code {
            KeyCode::Char('c') => vec![Effect::Copy(crate::clipboard::Clip::new(code.clone(), fallback))],
            KeyCode::Char('?') => vec![],
            _ => {
                s.overlay = back.take().map(|b| *b);
                vec![]
            }
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
        Overlay::Invite { code } => match (k.code, code.clone()) {
            // An invite admits whoever holds it: copied only on this key,
            // and never logged.
            (KeyCode::Char('c'), Some(c)) => vec![Effect::Copy(crate::clipboard::Clip::new(c, "press o and select it"))],
            (KeyCode::Char('o'), Some(c)) => {
                show_code(s, "Invite code (it works once)", c, None, "select it with the mouse");
                vec![]
            }
            _ => {
                s.overlay = None;
                vec![]
            }
        },
        Overlay::RowMenu { items, sel, .. } => {
            let mut i = Sel { index: *sel };
            if let Some(a) = nav
                && a != Action::Select
                && list_nav(&mut i, items.len(), a)
            {
                *sel = i.index;
                return vec![];
            }
            let chosen = match (k.code, nav) {
                (KeyCode::Esc, _) | (_, Some(Action::RowMenu)) => {
                    s.overlay = None;
                    return vec![];
                }
                (_, Some(Action::Select)) => items.get(*sel).cloned(),
                // An entry's own key chooses it, as it would without the menu.
                _ => {
                    let contexts: Vec<crate::keymap::Context> = items.iter().map(|it| it.context).collect();
                    let a = s.keymap.resolve(&contexts, &k, false);
                    items.iter().find(|it| Some(it.action) == a).cloned()
                }
            };
            let Some(it) = chosen else { return vec![] };
            if let Allowed::No(r) = &it.allowed {
                s.toast(ToastKind::Error, format!("{}: {r}", it.label.trim_end_matches('…')));
                return vec![];
            }
            s.overlay = None;
            main_action(s, it.action)
        }
        Overlay::AddAgentPicker { sel } => {
            let mut i = Sel { index: *sel };
            if let Some(a) = nav
                && a != Action::Select
                && list_nav(&mut i, available.len(), a)
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
                    let Some(l) = available.get(*sel).cloned() else {
                        s.overlay = None;
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
                _ => vec![],
            }
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
                (ConfirmAction::AllowAsk { chat, ask }, Some(w)) => vec![Effect::AnswerAsk(w, chat, ask, AskReply::Option(0))],
                (_, None) => vec![],
            }
        }
        Overlay::AdmitBox { request, busy, checked, result } => {
            if *busy {
                return vec![];
            }
            if let Some(a) = result {
                return match k.code {
                    KeyCode::Char('c') => vec![Effect::Copy(crate::clipboard::Clip::new(a.code.clone(), "press o and select it"))],
                    KeyCode::Char('o') => {
                        let (code, fp) = (a.code.clone(), a.fingerprint.clone());
                        show_code(s, "The box's admission code", code, fp, "select it with the mouse");
                        vec![]
                    }
                    _ => {
                        s.overlay = None;
                        vec![]
                    }
                };
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
                    KeyCode::Char('c') => {
                        let fp = checked.as_ref().map(|c| c.fingerprint.clone()).unwrap_or_default();
                        vec![Effect::Copy(crate::clipboard::Clip::new(fp, "compare it on screen"))]
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
        Overlay::RepoPicker { chat, sel, choices } => {
            let rows = choices.as_ref().map_or(0, |c| c.repos.len()) + 2;
            let mut i = Sel { index: *sel };
            if let Some(a) = nav
                && a != Action::Select
                && list_nav(&mut i, rows, a)
            {
                *sel = i.index;
                return vec![];
            }
            if k.code == KeyCode::Esc {
                s.overlay = None;
                return vec![];
            }
            if k.code != KeyCode::Enter {
                return vec![];
            }
            // Still loading: only "New scratch repo" and "No repository".
            let (chat, sel, choices) = (chat.clone(), *sel, choices.clone());
            let Some(w) = ws else { return vec![] };
            let n = choices.as_ref().map_or(0, |c| c.repos.len());
            if sel == 0 {
                if let Some(Allowed::No(r)) = choices.as_ref().map(|c| &c.new_scratch) {
                    s.toast(ToastKind::Error, format!("Can't make a scratch repo: {r}"));
                    return vec![];
                }
                s.overlay = None;
                s.toast(ToastKind::Info, "Making a scratch repo on this device…");
                return vec![Effect::NewScratchRepo(w, chat)];
            }
            if sel == n + 1 {
                s.overlay = None;
                return vec![Effect::SetChatRepo(w, chat, None)];
            }
            let Some(r) = choices.and_then(|c| c.repos.get(sel - 1).cloned()) else { return vec![] };
            s.overlay = None;
            vec![Effect::SetChatRepo(w, chat, Some(r.id))]
        }
        Overlay::KeepScratch { chat, path, .. } => match k.code {
            KeyCode::Esc => {
                s.overlay = None;
                vec![]
            }
            KeyCode::Enter => {
                let dest = path.text.trim().to_string();
                if dest.is_empty() {
                    s.toast(ToastKind::Error, "Pick a folder to keep it in");
                    return vec![];
                }
                let chat = chat.clone();
                s.overlay = None;
                ws.map(|w| vec![Effect::KeepScratch(w, chat, dest)]).unwrap_or_default()
            }
            _ => {
                path.handle(&k);
                vec![]
            }
        },
        Overlay::AskReply { chat, ask, text, .. } => match k.code {
            KeyCode::Esc => {
                s.overlay = None;
                vec![]
            }
            KeyCode::Enter => {
                let reply = text.text.trim().to_string();
                if reply.is_empty() {
                    return vec![];
                }
                let (chat, ask) = (chat.clone(), ask.clone());
                s.overlay = None;
                ws.map(|w| vec![Effect::AnswerAsk(w, chat, ask, AskReply::Text(reply))]).unwrap_or_default()
            }
            _ => {
                text.handle(&k);
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
    s.asks.clear();
    s.transcript_scroll = 0;
    s.proposals.clear();
    s.archived_proposals.clear();
    s.show_archived = false;
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
    s.loaded.clear();
    s.attention.clear();
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
        s.asks.clear();
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
    vec![Effect::LoadTranscript(ws.clone(), id.clone()), Effect::LoadAsks(ws.clone(), id.clone()), Effect::MarkRead(ws, id)]
}

/// Go to a view, remembering where you were (`ctrl-o`, `esc` go back).
fn go(s: &mut State, v: View) -> Vec<Effect> {
    if v != s.view {
        s.back.push(s.view);
        if s.back.len() > 50 {
            s.back.remove(0);
        }
        s.forward.clear();
    }
    show(s, v)
}

/// Back (or forward) through the views you went to.
fn view_history(s: &mut State, back: bool) -> Vec<Effect> {
    let (from, to) = if back { (&mut s.back, &mut s.forward) } else { (&mut s.forward, &mut s.back) };
    let Some(v) = from.pop() else { return vec![] };
    to.push(s.view);
    show(s, v)
}

/// Select row `i` of the focused list, with what moving there does (a
/// chat opens, a proposal's diff previews).
fn select_row(s: &mut State, i: usize) -> Vec<Effect> {
    match s.view {
        View::Chats => {
            s.chat_sel.index = i;
            match s.chats.get(i).map(|c| c.id.clone()) {
                Some(id) => open_chat(s, id),
                None => vec![],
            }
        }
        View::Proposals => {
            s.proposal_sel.index = i;
            preview_selected_diff(s)
        }
        View::Agents => {
            s.agent_sel.index = i;
            vec![]
        }
        View::Hosts => {
            s.host_sel.index = i;
            vec![]
        }
        View::People => {
            s.member_sel.index = i;
            vec![]
        }
        View::Notifications => {
            s.notification_sel.index = i;
            vec![]
        }
        View::Workflows => {
            s.workflow_sel.index = i;
            vec![]
        }
        View::Memory => {
            s.memory_sel.index = i;
            vec![]
        }
        View::Settings => vec![],
    }
}

/// The rows matching a find query, in list order.
fn find_matches(s: &State, q: &str) -> Vec<usize> {
    let q = q.trim().to_lowercase();
    let Some((rows, _)) = s.list_rows() else { return vec![] };
    rows.iter().enumerate().filter(|(_, r)| !q.is_empty() && r.to_lowercase().contains(&q)).map(|(i, _)| i).collect()
}

/// A key while `/` is finding: typing jumps to the first match from where
/// the find started, ↑/↓ go to the previous or next match, enter keeps the
/// selection, esc goes back to where it was.
fn find_key(s: &mut State, k: &KeyEvent) -> Vec<Effect> {
    let Some(find) = s.find.as_mut() else { return vec![] };
    let origin = find.origin;
    match k.code {
        KeyCode::Esc => {
            s.find = None;
            return select_row(s, origin);
        }
        KeyCode::Enter => {
            s.find = None;
            return vec![];
        }
        KeyCode::Up | KeyCode::Down => {
            let q = find.query.text.clone();
            let cur = s.list_rows().map(|(_, sel)| sel).unwrap_or(0);
            let m = find_matches(s, &q);
            let next = if k.code == KeyCode::Down {
                m.iter().find(|i| **i > cur).or(m.first())
            } else {
                m.iter().rev().find(|i| **i < cur).or(m.last())
            };
            return match next.copied() {
                Some(i) => select_row(s, i),
                None => vec![],
            };
        }
        _ => {
            if !find.query.handle(k) {
                return vec![];
            }
        }
    }
    let q = s.find.as_ref().map(|f| f.query.text.clone()).unwrap_or_default();
    let m = find_matches(s, &q);
    match m.iter().find(|i| **i >= origin).or(m.first()).copied() {
        Some(i) => select_row(s, i),
        None => vec![],
    }
}

/// Show a view and load what it needs, without touching the history.
fn show(s: &mut State, v: View) -> Vec<Effect> {
    s.find = None;
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
    // After space, the next key acts as usual; esc or space again just
    // put the sheet away.
    if s.which_key.take().is_some() && matches!(k.code, KeyCode::Esc | KeyCode::Char(' ')) {
        return vec![];
    }
    if s.find.is_some() && !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return find_key(s, &k);
    }
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
    // ↑ on an empty draft (or while browsing) recalls what you sent, ↓
    // comes back; any other edit keeps the recalled text as the draft.
    if s.focus == Focus::Composer && s.view == View::Chats && k.modifiers.is_empty() {
        let first_line = !s.composer.text[..s.composer.cursor].contains('\n');
        let last_line = !s.composer.text[s.composer.cursor..].contains('\n');
        match k.code {
            KeyCode::Up if first_line && (s.composer.text.is_empty() || s.history_pos.is_some()) => {
                let h = s.my_history();
                let next = s.history_pos.map_or(0, |p| p + 1);
                if let Some(t) = h.get(next) {
                    s.composer.set(t);
                    s.history_pos = Some(next);
                }
                return vec![];
            }
            KeyCode::Down if last_line && s.history_pos.is_some() => {
                let h = s.my_history();
                match s.history_pos {
                    Some(0) | None => {
                        s.composer.take();
                        s.history_pos = None;
                    }
                    Some(p) => {
                        s.composer.set(h.get(p - 1).map(String::as_str).unwrap_or(""));
                        s.history_pos = Some(p - 1);
                    }
                }
                return vec![];
            }
            _ => {}
        }
    }
    let contexts = s.contexts();
    let action = s.keymap.resolve(&contexts, &k, typing);
    let Some(action) = action else {
        if s.focus == Focus::Composer && s.view == View::Chats && s.composer.handle(&k) {
            s.mention_sel = 0;
            s.history_pos = None;
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
        Palette => {
            s.overlay = Some(Overlay::Palette { query: TextBuf::single(), sel: 0 });
            vec![]
        }
        WhichKey => {
            s.which_key = Some(s.now);
            vec![]
        }
        SwitchTheme => {
            let i = s.themes.iter().position(|(id, _)| *id == s.theme_id).map(|i| i + 1).unwrap_or(0);
            let next = s.themes.get(i % s.themes.len().max(1)).map(|(id, _)| id.clone());
            s.theme_request = next.map(|t| crate::theme::ThemeRequest::Set { theme: Some(t), appearance: None });
            vec![]
        }
        SwitchAppearance => {
            s.theme_request = Some(crate::theme::ThemeRequest::Set { theme: None, appearance: Some(s.appearance.next()) });
            vec![]
        }
        Help => {
            s.overlay = Some(Overlay::Help { scroll: 0, query: TextBuf::single(), searching: false });
            vec![]
        }
        Back => match (s.view, s.focus) {
            (View::Proposals, Focus::Detail) | (View::Chats, Focus::Detail) => {
                s.focus = Focus::List;
                vec![]
            }
            (View::Chats, Focus::List) => vec![],
            // From the archived proposals, back to the inbox.
            (View::Proposals, Focus::List) if s.show_archived => toggle_archived(s),
            // Back to the view you came from, not always to Chats.
            _ if !s.back.is_empty() => view_history(s, true),
            _ => show(s, View::Chats),
        },
        FocusMode => {
            s.focus_mode = !s.focus_mode;
            vec![]
        }
        ViewBack => view_history(s, true),
        ViewForward => view_history(s, false),
        Find => {
            match s.list_rows() {
                Some((_, sel)) => s.find = Some(crate::app::state::Find { query: TextBuf::single(), origin: sel }),
                None => s.toast(ToastKind::Info, "Nothing to find in here"),
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
        LinkDevice => {
            if let Some(p) = s.settings.as_ref().and_then(|st| st.linked_to.clone()) {
                s.toast(ToastKind::Info, format!("Already one of {p}'s devices"));
                return vec![];
            }
            let mut f = WorkspaceForm::linking(false);
            f.link_asked = true;
            s.overlay = Some(Overlay::AddWorkspace(f));
            vec![Effect::LinkRequest]
        }
        Approvals => {
            let Some(w) = ws else {
                s.toast(ToastKind::Info, "Open a workspace first");
                return vec![];
            };
            s.overlay = Some(Overlay::Approvals { list: None });
            vec![Effect::LoadApprovals(w)]
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
        ChatList => {
            if s.view != View::Chats {
                let fx = go(s, View::Chats);
                s.focus = Focus::List;
                return fx;
            }
            s.focus = if s.focus == Focus::List && s.open_chat.is_some() { Focus::Composer } else { Focus::List };
            vec![]
        }
        Up | Down | PageUp | PageDown | Top | Bottom => nav_action(s, action),
        Select => select(s),
        RowMenu => {
            let (title, items) = crate::app::menu::row_menu(s);
            if items.is_empty() {
                s.toast(ToastKind::Info, "Nothing to do here");
            } else {
                s.overlay = Some(Overlay::RowMenu { title, items, sel: 0 });
            }
            vec![]
        }
        StopTurn => {
            let (Some(w), Some(c)) = (ws, s.open_chat.clone()) else { return vec![] };
            let live = s.thinking.get(&c).is_some_and(|t| !t.is_empty()) || s.transcript.iter().any(|m| m.streaming);
            if !live {
                s.toast(ToastKind::Info, "No agent is working in this chat");
                return vec![];
            }
            vec![Effect::StopTurn(w, c)]
        }
        DumpTranscript => {
            if s.open_chat.is_none() || s.transcript.is_empty() {
                return vec![];
            }
            vec![Effect::ShowText(s.transcript_text())]
        }
        ToggleTools => {
            s.tools_expanded = !s.tools_expanded;
            vec![]
        }
        AskOption1 | AskOption2 | AskOption3 | AskOption4 | AskOption5 => {
            let i = match action {
                AskOption1 => 0,
                AskOption2 => 1,
                AskOption3 => 2,
                AskOption4 => 3,
                _ => 4,
            };
            ask_pick(s, i)
        }
        AskReply => {
            let Some(a) = s.ask_target() else { return vec![] };
            if !a.allow_text {
                s.toast(ToastKind::Info, "This question takes one of its options, not a reply");
                return vec![];
            }
            let (chat, ask, agent) = (a.chat.clone(), a.id.clone(), a.agent.clone());
            s.overlay = Some(Overlay::AskReply { chat, ask, agent, text: TextBuf::single() });
            vec![]
        }
        AskNotNow => {
            let Some(a) = s.ask_target() else { return vec![] };
            let (chat, ask, v2) = (a.chat.clone(), a.id.clone(), a.v2);
            if !v2 {
                // An older question settles on its first answer for
                // everyone: put it aside here only.
                s.asks_hidden.insert(ask);
                s.toast(ToastKind::Info, "Put aside here; it stays open for the others");
                return vec![];
            }
            ws.map(|w| vec![Effect::AnswerAsk(w, chat, ask, crate::backend::model::AskReply::NotNow)]).unwrap_or_default()
        }
        AskAllow => {
            let Some(a) = s.ask_target() else { return vec![] };
            let Some(e) = a.effect.clone() else {
                s.toast(ToastKind::Info, "This is a question: pick an option with its number");
                return vec![];
            };
            if e.too_large {
                s.toast(ToastKind::Error, "Too large to review here: only Deny is offered");
                return vec![];
            }
            let (chat, ask, agent) = (a.chat.clone(), a.id.clone(), a.agent.clone());
            let on = if e.host.is_empty() { String::new() } else { format!(" on {}", e.host) };
            let how = if e.presence_required { " Your passkey confirms it." } else { "" };
            let once = if e.standing { "for the rest of its session" } else { "once" };
            confirm(s, "Allow", format!("Let {agent} run this {once}{on}?{how}"), "allow", ConfirmAction::AllowAsk { chat, ask });
            vec![]
        }
        AskDeny => {
            let Some(a) = s.ask_target() else { return vec![] };
            if a.effect.is_none() {
                s.toast(ToastKind::Info, "This is a question: pick an option with its number");
                return vec![];
            }
            let (chat, ask) = (a.chat.clone(), a.id.clone());
            ws.map(|w| vec![Effect::AnswerAsk(w, chat, ask, crate::backend::model::AskReply::Option(1))]).unwrap_or_default()
        }
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
        ChatRepo => {
            let (Some(w), Some(c)) = (ws, s.open_chat.clone()) else {
                s.toast(ToastKind::Info, "Open a chat to choose its repository");
                return vec![];
            };
            s.overlay = Some(Overlay::RepoPicker { chat: c, sel: 0, choices: None });
            vec![Effect::LoadRepoChoices(w)]
        }
        KeepScratch => {
            let Some(c) = s.open_chat_summary() else { return vec![] };
            let Some(r) = c.repo.as_ref().filter(|r| r.mine) else {
                s.toast(ToastKind::Info, "This chat's repository isn't a scratch repo this device made");
                return vec![];
            };
            let mut path = TextBuf::single();
            path.set(r.keep_to.as_deref().unwrap_or_default());
            s.overlay = Some(Overlay::KeepScratch { chat: c.id.clone(), name: r.name.clone(), path });
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
            s.sent.push(text.clone());
            s.my_chats.insert(c.clone());
            s.history_pos = None;
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
        EditInEditor => {
            if s.open_chat.is_none() {
                return vec![];
            }
            vec![Effect::EditDraft(s.composer.text.clone())]
        }
        SearchHistory => {
            if s.open_chat.is_some() {
                s.overlay = Some(Overlay::History { query: TextBuf::single(), sel: 0 });
            }
            vec![]
        }
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
        ArchiveProposal => {
            let Some(p) = s.selected_proposal() else { return vec![] };
            let restore = match p.shelf {
                Shelf::Hidden => {
                    s.toast(ToastKind::Info, "Hidden on this device: show it again first");
                    return vec![];
                }
                Shelf::Archived | Shelf::AutoArchived => true,
                Shelf::Inbox => false,
            };
            if !restore && p.waiting {
                s.toast(ToastKind::Error, "It still waits on a decision: reject it first, or hide it for yourself");
                return vec![];
            }
            if let Allowed::No(r) = &p.can_archive {
                let r = r.clone();
                s.toast(ToastKind::Error, format!("Can't {}: {r}", if restore { "restore" } else { "archive" }));
                return vec![];
            }
            let id = p.id.clone();
            ws.map(|w| vec![Effect::ArchiveProposal(w, id, !restore)]).unwrap_or_default()
        }
        HideProposal => {
            let Some(p) = s.selected_proposal() else { return vec![] };
            let (id, hide) = (p.id.clone(), p.shelf != Shelf::Hidden);
            ws.map(|w| vec![Effect::HideProposal(w, id, hide)]).unwrap_or_default()
        }
        ShowArchived => {
            if s.view != View::Proposals {
                let mut fx = go(s, View::Proposals);
                if !s.show_archived {
                    fx.extend(toggle_archived(s));
                }
                return fx;
            }
            toggle_archived(s)
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
            s.overlay = Some(Overlay::AddAgentPicker { sel: 0 });
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
                2 => main_action(s, SwitchTheme),
                3 => main_action(s, SwitchAppearance),
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
        Setup => crate::app::setup::open_list(s),
        AdmitBox => {
            if s.active_workspace().is_some_and(|w| w.your_role < Role::Admin) {
                s.toast(ToastKind::Error, "Can't admit a box: only an Admin or Owner admits one");
                return vec![];
            }
            s.overlay = Some(Overlay::AdmitBox { request: TextBuf::single(), busy: false, checked: None, result: None });
            vec![]
        }
        CopySafetyCode => {
            let code = s.device.safety_code.clone();
            vec![Effect::Copy(crate::clipboard::Clip::new(code, "select it in Settings"))]
        }
    }
}

/// Do what a palette entry says: an action as its key would, or go to an
/// entity and select it.
fn run_palette(s: &mut State, target: crate::app::palette::Target) -> Vec<Effect> {
    use crate::app::palette::Target;
    match target {
        Target::Action(a) => main_action(s, a),
        Target::Theme(id) => {
            s.theme_request = Some(crate::theme::ThemeRequest::Set { theme: Some(id), appearance: None });
            vec![]
        }
        Target::Appearance(a) => {
            s.theme_request = Some(crate::theme::ThemeRequest::Set { theme: None, appearance: Some(a) });
            vec![]
        }
        Target::SwitchWorkspace(id) => switch_workspace(s, id),
        Target::Setup(flow) => crate::app::setup::open_flow(s, flow, false),
        Target::OpenChat(id) => {
            let mut fx = go(s, View::Chats);
            if let Some(i) = s.chats.iter().position(|c| c.id == id) {
                s.chat_sel.index = i;
            }
            fx.extend(open_chat(s, id));
            fx
        }
        Target::ShowProposal(id) => {
            let mut fx = go(s, View::Proposals);
            s.show_archived = !s.proposals.iter().any(|p| p.id == id) && s.archived_proposals.iter().any(|p| p.id == id);
            if let Some(i) = s.shown_proposals().iter().position(|p| p.id == id) {
                s.proposal_sel.index = i;
            }
            // go() asked for the old selection's diff; ask for this one.
            fx.retain(|f| !matches!(f, Effect::LoadDiff(..)));
            fx.extend(load_selected_diff(s));
            fx
        }
        Target::ShowRun(id) => {
            let fx = go(s, View::Workflows);
            if let Some(i) = s.runs.iter().position(|r| r.id == id) {
                s.workflow_sel.index = s.workflows.len() + i;
            }
            fx
        }
        Target::ShowAgent(id) => {
            let fx = go(s, View::Agents);
            if let Some(i) = s.agents.iter().position(|a| a.id == id) {
                s.agent_sel.index = i;
            }
            fx
        }
    }
}

/// A number key on the card the ask keys answer: its option, sent at once
/// (a question grants nothing). A grant's Allow needs `a` and a confirm.
fn ask_pick(s: &mut State, i: u32) -> Vec<Effect> {
    let Some(a) = s.ask_target() else { return vec![] };
    if a.effect.is_some() {
        s.toast(ToastKind::Info, "A permission: a to allow once (asks first), d to deny");
        return vec![];
    }
    if i as usize >= a.options.len() {
        return vec![];
    }
    let (chat, ask) = (a.chat.clone(), a.id.clone());
    s.active.clone().map(|w| vec![Effect::AnswerAsk(w, chat, ask, AskReply::Option(i))]).unwrap_or_default()
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
pub const SETTINGS_TOGGLES: usize = 4;

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
            let n = s.shown_proposals().len();
            list_nav(&mut s.proposal_sel, n, a);
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

/// Switch the Proposals view between the inbox and the archived and hidden
/// proposals.
fn toggle_archived(s: &mut State) -> Vec<Effect> {
    s.show_archived = !s.show_archived;
    s.proposal_sel = Sel::default();
    s.focus = Focus::List;
    s.diff = None;
    load_selected_diff(s)
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
        View::Settings => main_action(s, Action::Toggle),
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
        // An empty view offers its setup flow as its one action.
        View::Agents if s.agents.is_empty() => crate::app::setup::open_flow(s, LtFlow::Engine(FlowId::Agent), false),
        View::Hosts if s.hosts.is_empty() => crate::app::setup::open_flow(s, LtFlow::Engine(FlowId::Box), false),
        View::People if s.members.len() <= 1 => crate::app::setup::open_flow(s, LtFlow::Engine(FlowId::Invite), false),
        View::People | View::Agents | View::Hosts | View::Memory => vec![],
    }
}
