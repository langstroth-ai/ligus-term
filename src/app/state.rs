//! Everything the screen shows, and nothing about how it is drawn.

use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::app::input::TextBuf;
use crate::backend::model::*;
use crate::keymap::{Context, Keymap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    Chats,
    Proposals,
    Agents,
    Hosts,
    People,
    Notifications,
    Settings,
    Workflows,
    Memory,
}

impl View {
    /// In the top bar's order (and the number keys').
    pub const ALL: [View; 9] = [
        View::Chats,
        View::Workflows,
        View::Proposals,
        View::Agents,
        View::Hosts,
        View::Memory,
        View::People,
        View::Notifications,
        View::Settings,
    ];
    /// The top bar's label when space is short.
    pub fn short(self) -> &'static str {
        match self {
            View::Notifications => "Notifs",
            View::Proposals => "Props",
            v => v.title(),
        }
    }
    /// Shorter still, for an 80-column top bar.
    pub fn tiny(self) -> &'static str {
        match self {
            View::Workflows => "Flows",
            View::Settings => "Setup",
            v => v.short(),
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            View::Chats => "Chats",
            View::Workflows => "Workflows",
            View::Memory => "Memory",
            View::Proposals => "Proposals",
            View::Agents => "Agents",
            View::Hosts => "Hosts",
            View::People => "People",
            View::Notifications => "Notifications",
            View::Settings => "Settings",
        }
    }
}

/// Which pane has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Detail,
    Composer,
}

// One of each lives at a time; boxing the form would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Loading,
    /// First run, before anything else: is this the person's first device
    /// with Ligus, or do they already use it elsewhere?
    Start(StartFork),
    Onboarding(WorkspaceForm),
    /// The recovery words, shown once after creating a workspace.
    Recovery(Created),
    Main,
}

/// The first-run question: "Is this your first device with Ligus?"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartFork {
    /// "Yes, start here" is selected (the default); else "I already use
    /// Ligus on another device".
    pub first: bool,
    /// The longer explanation, shown with `?`.
    pub about: bool,
    /// This device has no name yet (the first-device path asks for one).
    pub ask_name: bool,
}

impl StartFork {
    pub fn new(ask_name: bool) -> Self {
        Self { first: true, about: false, ask_name }
    }
}

/// Which way the workspace form was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormRoute {
    /// "+" later, or the palette: every mode.
    #[default]
    Any,
    /// First run, "Yes, start here": a name, then create, join, connect or
    /// the demo. Linking lives on the other path.
    FirstDevice,
    /// First run, "I already use Ligus": linking only. No name is asked and
    /// nothing can create or join a workspace from here.
    ExistingAccount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormMode {
    Create,
    Join,
    /// Add to my devices: this computer becomes a device of a person already
    /// in the workspace on their desktop (not a new member).
    Link,
    /// Found the workspace a deployment (ligus-local, Kubernetes) serves.
    Connect,
    /// Look around first: restart as `lt --demo` (first run only).
    Demo,
}

impl FormMode {
    pub fn next(self) -> Self {
        match self {
            FormMode::Create => FormMode::Join,
            FormMode::Join => FormMode::Link,
            FormMode::Link => FormMode::Connect,
            FormMode::Connect => FormMode::Demo,
            FormMode::Demo => FormMode::Create,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            FormMode::Create => FormMode::Demo,
            FormMode::Join => FormMode::Create,
            FormMode::Link => FormMode::Join,
            FormMode::Connect => FormMode::Link,
            FormMode::Demo => FormMode::Connect,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Name,
    Mode,
    /// The workspace name (create, connect) or the invite (join).
    Value,
    Relay,
    Room,
    Key,
    Token,
}

/// Create a workspace or join one: first run, and "+" later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceForm {
    /// First run asks for your name too.
    pub ask_name: bool,
    pub name: TextBuf,
    pub mode: FormMode,
    pub ws_name: TextBuf,
    pub invite: TextBuf,
    pub relay: TextBuf,
    pub room: TextBuf,
    pub key: TextBuf,
    pub token: TextBuf,
    pub field: FormField,
    pub error: Option<String>,
    pub busy: bool,
    /// The checked invite (join asks before joining).
    pub checked: Option<(String, InviteCheck)>,
    /// Add to my devices: the grant the desktop shows.
    pub grant: TextBuf,
    /// This computer's link request, once made.
    pub link: Option<LinkRequestView>,
    /// The request was asked for (made once per form).
    pub link_asked: bool,
    /// The checked grant (redeemed on the second Enter, once the person saw
    /// who approved it).
    pub link_checked: Option<(String, LinkGrantView)>,
    /// Add to my devices: the longer explanation, shown with `?`.
    pub link_about: bool,
    /// Add to my devices, live (code match through the approval address):
    /// the request in flight, or how it ended.
    pub live: Option<LiveLink>,
    /// The relay has no approval mailbox: the request is pasted instead.
    pub live_unavailable: bool,
    /// Redeeming a pasted code: the desktop's approval hasn't arrived yet,
    /// and lt keeps checking until this time (ms) for this reason.
    pub link_waiting: Option<(Millis, String)>,
    /// Which way the form was opened (first run's two paths, or "+").
    pub route: FormRoute,
    /// Joining with an invite here probably makes a second you: this
    /// install is linked, or the person said they already use Ligus. The
    /// Join form then suggests linking instead (a hint, never a refusal).
    pub link_hint: bool,
}

/// A live link request, as the form shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveLink {
    /// The request's id once posted (a stale event names another).
    pub rid: Option<String>,
    pub stage: LiveStage,
    /// The relay isn't answering: why, while lt tries again.
    pub retrying: Option<String>,
}

/// Where a live link request is. `yes` is which of the two answers is
/// selected (No first: nothing happens on a stray Enter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveStage {
    /// Posting the request.
    Sending,
    /// Posted: waiting for the desktop to open it.
    Waiting,
    /// The code to compare: Yes, they match / No.
    Code {
        code: MatchCodeView,
        yes: bool,
    },
    /// The person said they match: the desktop asks for its passkey.
    Matched {
        code: MatchCodeView,
        acknowledged: bool,
    },
    /// The approval arrived: checking it on the workspace log.
    Checking,
    /// Check 6: link as the verified person? Yes, link / No.
    Confirm {
        person: String,
        workspace: String,
        yes: bool,
        busy: bool,
    },
    Ended(LiveLinkEnd),
}

impl WorkspaceForm {
    pub fn new(ask_name: bool) -> Self {
        Self {
            ask_name,
            name: TextBuf::single(),
            mode: FormMode::Create,
            ws_name: TextBuf::single(),
            invite: TextBuf::single(),
            relay: TextBuf::single(),
            room: TextBuf::single(),
            key: TextBuf::single(),
            token: TextBuf::single(),
            field: if ask_name { FormField::Name } else { FormField::Mode },
            error: None,
            busy: false,
            checked: None,
            grant: TextBuf::single(),
            link: None,
            link_asked: false,
            link_checked: None,
            link_about: false,
            live: None,
            live_unavailable: false,
            link_waiting: None,
            route: FormRoute::Any,
            link_hint: false,
        }
    }

    /// First run, "Yes, start here".
    pub fn first_device(ask_name: bool) -> Self {
        Self { route: FormRoute::FirstDevice, ..Self::new(ask_name) }
    }

    /// First run, "I already use Ligus on another device": straight to
    /// Add to my devices, with no name and no other mode.
    pub fn existing_account() -> Self {
        Self { route: FormRoute::ExistingAccount, ..Self::linking(false) }
    }

    /// Show "Looks like you already use Ligus" on the Join form: this
    /// install is linked or the person said so, or the checked invite is
    /// for a workspace this install already has.
    pub fn shows_link_hint(&self) -> bool {
        self.mode == FormMode::Join
            && self.route != FormRoute::ExistingAccount
            && (self.link_hint || self.checked.as_ref().is_some_and(|(i, c)| *i == self.invite.text.trim() && c.already_joined))
    }

    /// The form opened on Add to my devices (the palette's Link this
    /// computer to my devices).
    pub fn linking(ask_name: bool) -> Self {
        let mut f = Self::new(ask_name);
        f.mode = FormMode::Link;
        f.field = FormField::Value;
        f
    }
    pub fn fields(&self) -> Vec<FormField> {
        let mut f = vec![];
        if self.route == FormRoute::ExistingAccount {
            return vec![FormField::Value];
        }
        if self.ask_name && self.mode != FormMode::Link {
            f.push(FormField::Name);
        }
        f.push(FormField::Mode);
        if self.mode == FormMode::Demo {
            return f;
        }
        if self.mode == FormMode::Connect {
            f.extend([FormField::Relay, FormField::Room, FormField::Key, FormField::Token]);
        }
        f.push(FormField::Value);
        f
    }
    /// The text field `field` edits (none for the mode row).
    pub fn buf_mut(&mut self, field: FormField) -> Option<&mut TextBuf> {
        Some(match field {
            FormField::Name => &mut self.name,
            FormField::Mode => return None,
            FormField::Value => self.value_mut(),
            FormField::Relay => &mut self.relay,
            FormField::Room => &mut self.room,
            FormField::Key => &mut self.key,
            FormField::Token => &mut self.token,
        })
    }
    pub fn value_mut(&mut self) -> &mut TextBuf {
        match self.mode {
            FormMode::Create | FormMode::Connect | FormMode::Demo => &mut self.ws_name,
            FormMode::Join => &mut self.invite,
            FormMode::Link => &mut self.grant,
        }
    }
    pub fn value(&self) -> &TextBuf {
        match self.mode {
            FormMode::Create | FormMode::Connect | FormMode::Demo => &self.ws_name,
            FormMode::Join => &self.invite,
            FormMode::Link => &self.grant,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceStage {
    Asking,
    WaitingBrowser,
    EnteringPath(TextBuf),
    /// Waiting for the person's own desktop or phone to confirm it (the
    /// code is [`PresenceSheet::handoff`]).
    Handoff,
    /// The hand-off ended without an answer (why, in one line): nothing
    /// was written; ask again, or confirm here.
    HandoffStopped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceSheet {
    pub prompt: PresencePrompt,
    pub stage: PresenceStage,
    pub error: Option<String>,
    /// No browser here: the page's address for another device's browser,
    /// and the loopback port that browser must reach.
    pub remote: Option<(String, u16)>,
    /// The code of the hand-off waiting on the person's desktop or phone,
    /// while it waits (none once they chose to confirm here instead).
    pub handoff: Option<String>,
}

impl PresenceSheet {
    pub fn new(prompt: PresencePrompt) -> Self {
        let handoff = prompt.handoff.clone();
        let stage = if handoff.is_some() { PresenceStage::Handoff } else { PresenceStage::Asking };
        PresenceSheet { prompt, stage, error: None, remote: None, handoff }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    /// The key reference: what has focus first, scrollable, and narrowed
    /// by `/` to the bindings whose keys or words match.
    Help {
        scroll: usize,
        /// The `/` search ("" shows everything).
        query: TextBuf,
        /// The search line has the keyboard.
        searching: bool,
    },
    Switcher {
        sel: usize,
    },
    AddWorkspace(WorkspaceForm),
    NewChat(TextBuf),
    HostPicker {
        proposal: ProposalId,
        sel: usize,
    },
    RolePicker {
        member: MemberId,
        sel: usize,
    },
    RuntimePicker {
        agent: AgentId,
        sel: usize,
    },
    ConfirmRemove {
        member: MemberId,
        name: String,
    },
    Invite {
        code: Option<String>,
    },
    /// A code meant for pasting elsewhere, alone on the screen with no
    /// border in its lines (a mouse selection takes just the code): `c`
    /// copies it, any other key goes back to `back`.
    Code {
        title: String,
        code: String,
        fingerprint: Option<String>,
        /// The plain-mode command that prints it, when no clipboard is
        /// reachable.
        fallback: &'static str,
        back: Option<Box<Overlay>>,
    },
    /// Admit a box: paste its join request (or leave it empty for a bearer
    /// code), compare its fingerprint with the box's terminal (before any
    /// passkey prompt), then the code to hand it.
    AdmitBox {
        request: TextBuf,
        busy: bool,
        /// The request's fingerprint, read before the passkey is asked.
        checked: Option<JoinRequestCheck>,
        result: Option<BoxAdmission>,
    },
    /// The live approval requests in the workspace's approval mailbox, read
    /// only (`None` while they load; the error when they can't be read).
    Approvals {
        list: Option<std::result::Result<Vec<ApprovalRequest>, String>>,
    },
    /// Link a device: the short code a new device of yours types (`None`
    /// while it is posted; the error when it can't be).
    LinkCode {
        view: Option<std::result::Result<crate::backend::model::LinkCodeView, String>>,
    },
    /// The Set up list (`u`): each guided flow and what is done.
    Setup {
        sel: usize,
    },
    /// One flow, one step per screen (`crate::app::setup`).
    SetupFlow(Box<crate::app::setup::Wizard>),
    /// `.` on a row: the row's actions, then the view's, the destructive
    /// ones last.
    RowMenu {
        title: String,
        items: Vec<MenuItem>,
        sel: usize,
    },
    /// The Agents view's "add an agent": what this device could run one on.
    AddAgentPicker {
        sel: usize,
    },
    /// Add an agent on a runtime of this device: its name.
    AddAgent {
        runtime: LocalRuntime,
        name: TextBuf,
    },
    /// The input for a workflow run.
    StartRun {
        workflow: WorkflowId,
        name: String,
        label: Option<String>,
        input: TextBuf,
    },
    /// The checkout this device applies the workspace's proposals in.
    BindCheckout(TextBuf),
    /// The chat's repository (`r`): a new scratch repo first, then the
    /// workspace's repositories, then none. `choices` is `None` while they
    /// load.
    RepoPicker {
        chat: ChatId,
        sel: usize,
        choices: Option<RepoChoices>,
    },
    /// Keep the chat's scratch repo as an ordinary repository (`K`): the
    /// folder to move it to.
    KeepScratch {
        chat: ChatId,
        name: String,
        path: TextBuf,
    },
    /// A one-line reply to an agent's question.
    AskReply {
        chat: ChatId,
        ask: AskId,
        agent: String,
        text: TextBuf,
    },
    /// This session's notices and errors, newest first, in full.
    Messages {
        scroll: usize,
    },
    /// The command palette: a query and the selected entry.
    Palette {
        query: TextBuf,
        sel: usize,
    },
    /// `ctrl-r` in the composer: what you sent, filtered by a query.
    History {
        query: TextBuf,
        sel: usize,
    },
    /// "Are you sure?" before something that can't be undone from here:
    /// `y` (or enter) goes ahead, any other key backs out.
    Confirm {
        title: String,
        question: String,
        /// What `y` does, as the key hint says it ("quit", "cancel the run").
        yes: String,
        action: ConfirmAction,
    },
}

/// Where an entry of the row menu sits: the row's own actions, the
/// view's, then the destructive ones (each asks first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MenuSection {
    Row,
    /// The view's own actions (what the desktop puts in a pane's header).
    Pane,
    Danger,
}

/// One entry of the row menu: what it does (as its key would), its label,
/// and whether it is allowed (the backend's reason when not).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub action: crate::keymap::Action,
    /// The context its key is shown from.
    pub context: Context,
    pub allowed: Allowed,
    pub section: MenuSection,
}

/// What a [`Overlay::Confirm`] does on `y`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    Quit,
    CancelRun(RunId),
    Reject(ProposalId),
    RejectGate {
        run: RunId,
        stage: String,
    },
    /// Allow an agent's call once (a grant card).
    AllowAsk {
        chat: ChatId,
        ask: AskId,
    },
    /// Leave this workspace on your own.
    LeaveWorkspace(WorkspaceId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Ok,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    /// Ticks left.
    pub ttl: u32,
}

/// One notice or error from this session, kept after its toast is gone so
/// it can be read again in full (the messages log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub at: Millis,
    pub kind: ToastKind,
    pub text: String,
}

/// `/` in a list: what is typed, and where the selection was, to go back
/// to on esc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Find {
    pub query: TextBuf,
    pub origin: usize,
}

/// How long `space` waits before its sheet of keys shows (ms).
pub const WHICH_KEY_DELAY: Millis = 400;

/// How many notices the messages log keeps (the oldest go first).
pub const LOG_CAP: usize = 200;

/// The selection in a list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sel {
    pub index: usize,
}

impl Sel {
    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.index = 0;
        } else if self.index >= len {
            self.index = len - 1;
        }
    }
    pub fn up(&mut self, n: usize) {
        self.index = self.index.saturating_sub(n);
    }
    pub fn down(&mut self, n: usize, len: usize) {
        self.index = (self.index + n).min(len.saturating_sub(1));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffState {
    pub proposal: ProposalId,
    pub diff: Diff,
    /// First visible row of the flattened diff.
    pub scroll: usize,
}

/// One row of a flattened diff, as the viewer draws and scrolls it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRow {
    File { path: String, added: usize, removed: usize },
    Hunk(String),
    Line(DiffLine),
    Blank,
}

impl DiffState {
    pub fn rows(&self) -> Vec<DiffRow> {
        let mut out = vec![];
        for (i, f) in self.diff.files.iter().enumerate() {
            if i > 0 {
                out.push(DiffRow::Blank);
            }
            out.push(DiffRow::File { path: f.path.clone(), added: f.added, removed: f.removed });
            for h in &f.hunks {
                out.push(DiffRow::Hunk(h.header.clone()));
                out.extend(h.lines.iter().cloned().map(DiffRow::Line));
            }
        }
        out
    }
}

pub struct State {
    pub keymap: Keymap,
    pub device: DeviceInfo,
    pub now: Millis,
    pub phase: Phase,
    pub your_name: Option<String>,
    /// The person answered the first-run question with "I already use
    /// Ligus": an invite opened here later suggests linking instead.
    pub chose_existing: bool,
    /// The recovery key screen asks "Finish later?" (its only way out
    /// before the words are confirmed).
    pub finish_later: bool,
    pub workspaces: Vec<WorkspaceSummary>,
    pub active: Option<WorkspaceId>,
    pub view: View,
    pub focus: Focus,
    pub overlay: Option<Overlay>,
    /// Passkey prompts, oldest first; the first is on screen.
    pub presence: VecDeque<PresenceSheet>,
    /// Passkey hand-offs left waiting when lt last quit (the Set up list
    /// offers them first).
    pub pending_presence: Vec<PendingPresence>,

    pub chats: Vec<ChatSummary>,
    pub chat_sel: Sel,
    pub open_chat: Option<ChatId>,
    pub transcript: Vec<Message>,
    /// The open chat's decision cards, oldest first.
    pub asks: Vec<Ask>,
    /// Older questions put aside here with Not now (they settle on their
    /// first answer for everyone, so nothing is recorded).
    pub asks_hidden: BTreeSet<AskId>,
    /// Rows scrolled up from the bottom of the transcript.
    pub transcript_scroll: usize,
    /// Tool calls show what they returned (`o`), not just one line each.
    pub tools_expanded: bool,
    /// Which agents are working on a reply, per chat.
    pub thinking: HashMap<ChatId, BTreeSet<String>>,
    /// How the last turn ended in a chat you haven't looked at since:
    /// [`SessionState::Done`] or [`SessionState::Failed`]. Opening the chat
    /// clears it.
    pub turn_outcomes: HashMap<ChatId, SessionState>,
    /// When the open chat's waiting lines are looked at again.
    pub route_refresh_at: Millis,
    pub composer: TextBuf,
    /// Drafts kept per chat when switching.
    pub drafts: HashMap<ChatId, String>,
    /// What you sent this session, oldest first (composer history).
    pub sent: Vec<String>,
    /// Browsing the history with `↑`/`↓`: how far back (0 is the newest).
    pub history_pos: Option<usize>,
    pub mentionables: Vec<Mentionable>,
    pub mention_sel: usize,
    /// The composer text at which the popup was dismissed with Esc.
    pub mention_dismissed: Option<String>,

    /// The inbox: proposals neither archived nor hidden on this device
    /// (the only ones the counts and the chat views see).
    pub proposals: Vec<Proposal>,
    /// Archived (for everyone, or by the workspace's auto-archive rule)
    /// and hidden here ("Hide for me").
    pub archived_proposals: Vec<Proposal>,
    /// The Proposals view lists the archived and hidden ones instead.
    pub show_archived: bool,
    pub proposal_sel: Sel,
    pub diff: Option<DiffState>,

    pub agents: Vec<Agent>,
    pub runtimes: Vec<Runtime>,
    /// What this device could run agents on; `None` while looking.
    pub local: Option<Vec<LocalRuntime>>,
    /// Over the agents, then the runtimes available on this device.
    pub agent_sel: Sel,
    pub hosts: Vec<Host>,
    pub host_sel: Sel,
    pub members: Vec<Member>,
    pub member_sel: Sel,
    pub notifications: Vec<Notification>,
    pub notification_sel: Sel,
    pub settings: Option<Settings>,
    pub settings_sel: Sel,

    pub workflows: Vec<Workflow>,
    pub runs: Vec<Run>,
    /// Over the definitions, then the runs.
    pub workflow_sel: Sel,
    pub memory: Memory,
    pub tasks: Vec<TaskState>,
    /// Over the task states, then the memory entries.
    pub memory_sel: Sel,
    pub handoffs: Vec<Handoff>,
    /// This device's checkout for the active workspace (apply on host).
    pub checkout: Option<String>,
    /// The Set up list's rows; `None` while loading.
    pub setup_rows: Option<Vec<crate::setup::FlowRow>>,

    /// The toast on screen: at most one, in the status bar's centre.
    pub toasts: Vec<Toast>,
    /// Every toast of this session, oldest first (at most [`LOG_CAP`]).
    pub log: Vec<LogEntry>,
    pub sync_status: HashMap<WorkspaceId, String>,
    /// Where the config file is (for Settings).
    pub config_path: Option<String>,
    pub theme_name: String,
    /// A theme picked in the app (Settings, the palette), for the runtime
    /// to switch to; this session only.
    pub theme_request: Option<crate::theme::ThemeRequest>,
    /// What `theme =` would say for the theme in use ("pollen", a theme
    /// file's name, "mono"), and the appearance.
    pub theme_id: String,
    pub appearance: crate::theme::Appearance,
    /// How Settings shows the appearance: "auto (light)".
    pub appearance_name: String,
    /// Every theme this run knows, (id, label), in the order Settings
    /// cycles them (the palette lists them too).
    pub themes: Vec<(String, String)>,
    /// The palette's selection is showing a theme it hasn't kept.
    pub theme_previewing: bool,
    /// Ring the terminal bell once (a mention arrived, a chat needs you).
    pub bell: bool,
    /// Desktop notifications to send (OSC 9) if the terminal is unfocused;
    /// the runtime takes them after each batch and sends one.
    pub signals: Vec<String>,
    /// Whether the terminal has focus (terminals that report it).
    pub focused: bool,
    /// The chats that needed you at the last look.
    pub attention: std::collections::BTreeSet<ChatId>,
    /// Which of the active workspace's lists have loaded once (their first
    /// load sets what needs you without a signal).
    pub loaded: std::collections::HashSet<&'static str>,
    /// Chats you sent to this session: their turns ending while you're
    /// away is worth a signal.
    pub my_chats: std::collections::HashSet<ChatId>,
    /// Notifications in the current burst (one toast says how many).
    pub notice_burst: usize,
    /// Focus mode: no top bar and no hint line, just the work (and one
    /// quiet badge when something needs you).
    pub focus_mode: bool,
    /// Spinners stand still (`--reduce-motion`, `LT_REDUCE_MOTION`).
    pub reduce_motion: bool,
    /// The person's offset from UTC (seconds), for the times lt shows
    /// (0 in tests: UTC).
    pub utc_offset: i32,
    /// The "Linking waiting…" notice was shown once this run.
    pub link_notice_shown: bool,
    pub quit: bool,
    /// Quitting to look around in the demo: `lt --demo` runs next.
    pub exec_demo: bool,
    /// The terminal's size (columns, rows), for scroll limits; 0×0 until
    /// the first resize message.
    pub term: (u16, u16),
    /// `/` finding in the focused list.
    pub find: Option<Find>,
    /// Views you came from (`ctrl-o` goes back) and went back from.
    pub back: Vec<View>,
    pub forward: Vec<View>,
    /// lt captures the mouse (clicks and the wheel), so selecting text
    /// takes shift-drag.
    pub mouse: bool,
    /// When `space` was pressed: the next key acts as usual, and if none
    /// comes within [`WHICH_KEY_DELAY`] a sheet shows what can be pressed.
    pub which_key: Option<Millis>,
    /// The first chat opened after start-up gets the cursor in its
    /// composer, so typing works at once (esc leaves it).
    pub compose_on_open: bool,
}

impl State {
    pub fn new(device: DeviceInfo, keymap: Keymap, now: Millis) -> Self {
        State {
            keymap,
            device,
            now,
            phase: Phase::Loading,
            your_name: None,
            chose_existing: false,
            finish_later: false,
            workspaces: vec![],
            active: None,
            view: View::Chats,
            focus: Focus::List,
            overlay: None,
            presence: VecDeque::new(),
            pending_presence: Vec::new(),
            chats: vec![],
            chat_sel: Sel::default(),
            open_chat: None,
            transcript: vec![],
            asks: vec![],
            asks_hidden: BTreeSet::new(),
            transcript_scroll: 0,
            tools_expanded: false,
            thinking: HashMap::new(),
            turn_outcomes: HashMap::new(),
            route_refresh_at: 0,
            composer: TextBuf::multi(),
            drafts: HashMap::new(),
            sent: vec![],
            history_pos: None,
            mentionables: vec![],
            mention_sel: 0,
            mention_dismissed: None,
            proposals: vec![],
            archived_proposals: vec![],
            show_archived: false,
            proposal_sel: Sel::default(),
            diff: None,
            agents: vec![],
            runtimes: vec![],
            local: None,
            agent_sel: Sel::default(),
            hosts: vec![],
            host_sel: Sel::default(),
            members: vec![],
            member_sel: Sel::default(),
            notifications: vec![],
            notification_sel: Sel::default(),
            settings: None,
            settings_sel: Sel::default(),
            workflows: vec![],
            runs: vec![],
            workflow_sel: Sel::default(),
            memory: Memory::default(),
            tasks: vec![],
            memory_sel: Sel::default(),
            handoffs: vec![],
            checkout: None,
            setup_rows: None,
            toasts: vec![],
            log: vec![],
            sync_status: HashMap::new(),
            config_path: None,
            theme_name: "Pollen".into(),
            theme_request: None,
            theme_id: "pollen".into(),
            appearance: crate::theme::Appearance::Auto,
            appearance_name: "auto (dark)".into(),
            themes: crate::theme::ThemeSet::builtin()
                .families()
                .iter()
                .map(|f| (f.id.to_string(), f.label.to_string()))
                .chain([("mono".to_string(), "Mono".to_string())])
                .collect(),
            theme_previewing: false,
            bell: false,
            signals: vec![],
            focused: true,
            attention: Default::default(),
            loaded: Default::default(),
            my_chats: Default::default(),
            reduce_motion: false,
            utc_offset: 0,
            link_notice_shown: false,
            notice_burst: 0,
            focus_mode: false,
            quit: false,
            exec_demo: false,
            term: (0, 0),
            compose_on_open: true,
            which_key: None,
            mouse: false,
            find: None,
            back: vec![],
            forward: vec![],
        }
    }

    pub fn active_workspace(&self) -> Option<&WorkspaceSummary> {
        let id = self.active.as_ref()?;
        self.workspaces.iter().find(|w| &w.id == id)
    }

    pub fn open_chat_summary(&self) -> Option<&ChatSummary> {
        let id = self.open_chat.as_ref()?;
        self.chats.iter().find(|c| &c.id == id)
    }

    /// What bears on a chat's state, from what this device has loaded.
    pub fn session_signals(&self, chat: &ChatId) -> SessionSignals {
        let summary = self.chats.iter().find(|c| &c.id == chat);
        let streaming_here = self.open_chat.as_ref() == Some(chat) && self.transcript.iter().any(|m| m.streaming);
        let outcome = self.turn_outcomes.get(chat).copied();
        SessionSignals {
            live: self.thinking.get(chat).is_some_and(|t| !t.is_empty()) || streaming_here,
            failed: outcome == Some(SessionState::Failed),
            finished_unseen: outcome == Some(SessionState::Done),
            open_asks: self.tasks.iter().filter(|t| &t.chat == chat && t.active && !t.open_questions.is_empty()).count()
                + summary.map(|c| c.asks_for_you).unwrap_or(0),
            proposals_needing_me: self
                .proposals
                .iter()
                .filter(|p| p.chat.as_ref() == Some(chat) && p.status == ProposalStatus::Open && p.can_vote.is_yes())
                .count(),
            queued: 0,
            paused: summary.is_some_and(|c| c.lockdown.is_some()),
        }
    }

    /// Where a chat stands (the chat list's chip).
    pub fn session_state(&self, chat: &ChatId) -> SessionState {
        self.session_signals(chat).state()
    }

    /// What the Proposals view lists: the inbox, or the archived and
    /// hidden ones.
    pub fn shown_proposals(&self) -> &[Proposal] {
        if self.show_archived { &self.archived_proposals } else { &self.proposals }
    }

    /// The card the ask keys answer: the oldest open card in the open chat
    /// that you may answer (and haven't put aside here).
    pub fn ask_target(&self) -> Option<&Ask> {
        self.asks.iter().find(|a| a.status == AskStatus::Open && a.can_answer.is_yes() && !self.asks_hidden.contains(&a.id))
    }

    pub fn selected_proposal(&self) -> Option<&Proposal> {
        self.shown_proposals().get(self.proposal_sel.index)
    }

    pub fn selected_member(&self) -> Option<&Member> {
        self.members.get(self.member_sel.index)
    }

    /// An invite opened here probably makes a second you: this install is
    /// linked, or the person said they already use Ligus.
    pub fn suggests_linking(&self) -> bool {
        self.chose_existing || self.settings.as_ref().is_some_and(|st| st.linked_to.is_some())
    }

    /// The "+" form (create, join, connect or link), with the Join form's
    /// "link this device instead" hint when it applies.
    pub fn add_workspace_form(&self) -> WorkspaceForm {
        WorkspaceForm { link_hint: self.suggests_linking(), ..WorkspaceForm::new(false) }
    }

    /// This install is its own member of the active workspace (not linked
    /// as someone's device), and someone else is in it too: Settings says
    /// so and offers Link to my devices.
    pub fn own_member_unlinked(&self) -> bool {
        let Some(st) = &self.settings else { return false };
        st.linked_to.is_none()
            && self.active_workspace().is_some()
            && self.members.iter().any(|m| m.you && !m.service)
            && self.members.iter().any(|m| !m.you && !m.service)
    }

    /// "Linking waiting for your desktop's code (started HH:MM; expires
    /// HH:MM)" while a link request waits, as this person's clock shows it.
    pub fn link_pending_line(&self) -> Option<String> {
        let st = self.settings.as_ref()?;
        if st.linked_to.is_some() {
            return None;
        }
        let p = st.link_pending.as_ref()?;
        Some(crate::link_cli::pending_line(p, self.now, self.utc_offset))
    }

    /// Another member that very likely is the person at this terminal on
    /// their desktop: the same name (ignoring case, spaces and
    /// punctuation), or one name a prefix of the other of at least three
    /// characters ("zaz" and "zazulam"). Only a hint: People says so and
    /// offers Link to my devices; nothing is done on its own.
    pub fn likely_me_elsewhere(&self) -> Option<&Member> {
        if !self.own_member_unlinked() {
            return None;
        }
        let me = self.members.iter().find(|m| m.you && !m.service)?;
        let key = |n: &str| n.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
        let mine = key(&me.name);
        self.members.iter().filter(|m| !m.you && !m.service).find(|m| {
            let theirs = key(&m.name);
            let (short, long) = if theirs.len() <= mine.len() { (&theirs, &mine) } else { (&mine, &theirs) };
            !short.is_empty() && (short == long || (short.chars().count() >= 3 && long.starts_with(short.as_str())))
        })
    }

    /// The selected workflow row: a definition or a run.
    pub fn selected_workflow_item(&self) -> Option<WorkflowItem<'_>> {
        let i = self.workflow_sel.index;
        match self.workflows.get(i) {
            Some(w) => Some(WorkflowItem::Def(w)),
            None => self.runs.get(i - self.workflows.len()).map(WorkflowItem::Run),
        }
    }

    pub fn workflow_rows(&self) -> usize {
        self.workflows.len() + self.runs.len()
    }

    pub fn memory_rows(&self) -> usize {
        self.tasks.len() + self.memory.entries.len()
    }

    pub fn unread_notifications(&self) -> usize {
        self.notifications.iter().filter(|n| !n.read).count()
    }

    /// The host "Apply on ‹host›" means by default: the first online
    /// worker, else this device.
    pub fn default_apply_host(&self) -> Option<&Host> {
        self.hosts.iter().find(|h| h.online && h.kind == HostKind::Worker).or_else(|| self.hosts.iter().find(|h| h.this_device))
    }

    /// What you sent, newest first and without repeats: your messages in
    /// the open chat, then what you sent elsewhere this session. `↑` on an
    /// empty draft and `ctrl-r` read it.
    pub fn my_history(&self) -> Vec<String> {
        let me = self.members.iter().find(|m| m.you).map(|m| m.id.clone());
        let mut out: Vec<String> = vec![];
        let mine = self.transcript.iter().rev().filter(|m| matches!(&m.author, Author::Member { id, .. } if Some(id) == me.as_ref()));
        for text in mine.map(|m| m.text.clone()).chain(self.sent.iter().rev().cloned()) {
            if !text.trim().is_empty() && !out.contains(&text) {
                out.push(text);
            }
        }
        out
    }

    /// The @mention candidates for what is being typed, at most 8.
    pub fn mention_candidates(&self) -> Vec<&Mentionable> {
        if self.focus != Focus::Composer || self.mention_dismissed.as_deref() == Some(self.composer.text.as_str()) {
            return vec![];
        }
        let Some((_, q)) = self.composer.mention_query() else { return vec![] };
        let q = q.to_lowercase();
        self.mentionables.iter().filter(|m| m.handle.to_lowercase().starts_with(&q)).take(8).collect()
    }

    /// Whether a text field has the keyboard.
    pub fn typing_text(&self) -> bool {
        if let Some(p) = self.presence.front() {
            return matches!(p.stage, PresenceStage::EnteringPath(_));
        }
        if self.find.is_some() && self.overlay.is_none() {
            return true;
        }
        match &self.overlay {
            Some(Overlay::AddWorkspace(f)) => f.field != FormField::Mode,
            Some(Overlay::NewChat(_))
            | Some(Overlay::StartRun { .. })
            | Some(Overlay::BindCheckout(_))
            | Some(Overlay::KeepScratch { .. }) => true,
            Some(Overlay::Help { searching, .. }) => *searching,
            Some(Overlay::Palette { .. }) | Some(Overlay::History { .. }) => true,
            Some(Overlay::SetupFlow(w)) => w.typing(),
            Some(Overlay::AdmitBox { result: None, checked: None, .. }) => true,
            Some(_) => false,
            None => match &self.phase {
                Phase::Onboarding(f) => f.field != FormField::Mode,
                Phase::Start(_) => false,
                Phase::Main => self.view == View::Chats && self.focus == Focus::Composer,
                _ => false,
            },
        }
    }

    /// The key contexts in effect, most specific first.
    pub fn contexts(&self) -> Vec<Context> {
        if self.view == View::Chats && self.focus == Focus::Composer && (self.overlay.is_some() || !self.presence.is_empty()) {
            return vec![Context::Chat, Context::Nav, Context::Global];
        }
        self.view_contexts()
    }

    /// The key contexts of the view and pane with focus, overlays aside
    /// (what the help overlay lists first).
    pub fn view_contexts(&self) -> Vec<Context> {
        use Context::*;
        if self.view == View::Chats && self.focus == Focus::Composer {
            return vec![Composer, Global];
        }
        match self.view {
            // Its keys answer only with the transcript focused, so the
            // global keys (1-9, a, n, r) keep their meaning elsewhere.
            View::Chats if self.focus == Focus::Detail && self.ask_target().is_some() => vec![Ask, Chat, Nav, Global],
            View::Chats => vec![Chat, Nav, Global],
            View::Proposals if self.focus == Focus::Detail => vec![Diff, Review, Nav, Global],
            View::Proposals => vec![Review, Nav, Global],
            View::Agents => vec![Agents, Nav, Global],
            View::Hosts => vec![Hosts, Nav, Global],
            View::Workflows => vec![Workflows, Nav, Global],
            View::Memory => vec![Nav, Global],
            View::People => vec![People, Nav, Global],
            View::Notifications => vec![Notifications, Nav, Global],
            View::Settings => vec![Settings, Nav, Global],
        }
    }

    /// Whether this device drives `r` (holds its coordinator lease).
    pub fn drives_here(&self, r: &Run) -> bool {
        r.driver
            .as_deref()
            .is_some_and(|d| d == "This device" || d == self.device.device_name || self.hosts.iter().any(|h| h.this_device && h.label == d))
    }

    /// Why quitting now would stop work on this device: a run it drives, or
    /// an agent's turn running here. `None` when nothing would stop.
    pub fn quit_guard(&self) -> Option<String> {
        if let Some(Overlay::SetupFlow(w)) = &self.overlay
            && w.stage == crate::app::setup::Stage::Running
        {
            return Some(format!(
                "\u{201c}{}\u{201d} is running. Quitting now could leave it half done: esc stops it cleanly first.",
                w.flow.title()
            ));
        }
        let runs: Vec<&Run> = self.runs.iter().filter(|r| !r.status.is_terminal() && self.drives_here(r)).collect();
        if let Some(r) = runs.first() {
            let more = if runs.len() > 1 { format!(" and {} more", runs.len() - 1) } else { String::new() };
            return Some(format!(
                "This device drives the run \u{201c}{}\u{201d}{more}. Quitting stops driving it until lt runs here again.",
                r.name
            ));
        }
        let here = |a: &&Agent| match &a.host {
            None => true,
            Some(id) => self.hosts.iter().any(|h| &h.id == id && h.this_device),
        };
        if let Some(a) = self.agents.iter().filter(here).find(|a| a.status == AgentStatus::Busy) {
            return Some(format!("@{} is working on this device. Quitting stops its turn.", a.handle));
        }
        None
    }

    /// The runtimes found on this device that no agent here uses yet.
    pub fn available_local(&self) -> Vec<&LocalRuntime> {
        self.local.iter().flatten().filter(|l| l.available()).collect()
    }

    /// Rows of the Agents view's list: the agents (what this device could
    /// add opens from "add an agent").
    pub fn agent_rows(&self) -> usize {
        self.agents.len()
    }

    /// Queue a desktop notification for when the terminal is unfocused.
    pub fn signal(&mut self, text: impl Into<String>) {
        if !self.focused {
            self.signals.push(text.into());
        }
    }

    /// Show a notice in the status bar. One shows at a time: a new one
    /// replaces it. Errors stay twice as long, and every notice is kept in
    /// the messages log to read again.
    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        self.toast_logging(kind, text.clone(), text);
    }

    /// A toast that shows `shown` and logs `logged` (a burst shows "3 new
    /// notifications" and logs each one).
    pub fn toast_logging(&mut self, kind: ToastKind, shown: String, logged: String) {
        let text = shown;
        self.log.push(LogEntry { at: self.now, kind, text: logged });
        if self.log.len() > LOG_CAP {
            self.log.remove(0);
        }
        let ttl = if kind == ToastKind::Error { 32 } else { 16 };
        self.toasts = vec![Toast { text, kind, ttl }];
        self.notice_burst = 0;
    }

    /// The focused list's rows as text (what `/` searches) and its
    /// selection; `None` where there's no list to search.
    pub fn list_rows(&self) -> Option<(Vec<String>, usize)> {
        if self.focus != Focus::List {
            return None;
        }
        Some(match self.view {
            View::Chats => {
                (self.chats.iter().map(|c| format!("{} {}", c.title, c.channel.as_deref().unwrap_or(""))).collect(), self.chat_sel.index)
            }
            View::Proposals => (
                self.shown_proposals().iter().map(|p| format!("{} {} {}", p.title, p.author, p.status.label())).collect(),
                self.proposal_sel.index,
            ),
            View::Agents => (self.agents.iter().map(|a| format!("@{} {}", a.handle, a.runtime)).collect(), self.agent_sel.index),
            View::Hosts => (self.hosts.iter().map(|h| h.label.clone()).collect(), self.host_sel.index),
            View::People => (self.members.iter().map(|m| format!("{} {}", m.name, m.role.label())).collect(), self.member_sel.index),
            View::Notifications => (self.notifications.iter().map(|n| n.text.clone()).collect(), self.notification_sel.index),
            View::Workflows => (
                self.workflows.iter().map(|w| w.name.clone()).chain(self.runs.iter().map(|r| format!("{} {}", r.name, r.input))).collect(),
                self.workflow_sel.index,
            ),
            View::Memory => (
                self.tasks
                    .iter()
                    .map(|t| format!("{} {}", t.agent, t.summary))
                    .chain(self.memory.entries.iter().map(|e| format!("{} {}", e.title, e.tags.join(" "))))
                    .collect(),
                self.memory_sel.index,
            ),
            View::Settings => return None,
        })
    }

    /// The open chat's transcript as plain text, for `$PAGER` or the
    /// scrollback: control and bidi characters removed, since it is
    /// written to the terminal as it is.
    pub fn transcript_text(&self) -> String {
        let clean = |t: &str| -> String {
            t.chars()
                .filter(|c| {
                    (*c == '\n' || *c == '\t' || !c.is_control())
                        && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
                })
                .collect()
        };
        let title = self.open_chat_summary().map(|c| c.title.clone()).unwrap_or_default();
        let mut out = format!("{}\n\n", clean(&title));
        for m in &self.transcript {
            out.push_str(&format!("{}  {}\n", clean(&m.author.display()), ago_phrase(self.now, m.at)));
            for t in &m.tools {
                out.push_str(&format!("  [{}] {} {}\n", clean(&t.name), clean(&t.preview), if t.ok { "done" } else { "error" }));
            }
            for l in clean(&m.text).lines() {
                out.push_str(&format!("  {l}\n"));
            }
            out.push('\n');
        }
        // Agents' questions and grant cards, the call in full.
        for a in &self.asks {
            out.push_str(&format!("{} asks {}: \u{201c}{}\u{201d}\n", clean(&a.agent), clean(&a.audience), clean(&a.question)));
            if let Some(e) = &a.effect {
                for l in clean(&e.detail).lines() {
                    out.push_str(&format!("  | {l}\n"));
                }
                if !e.cwd.is_empty() {
                    out.push_str(&format!("  in {}\n", clean(&e.cwd)));
                }
            } else if !a.options.is_empty() {
                out.push_str(&format!("  options: {}\n", clean(&a.options.join(" | "))));
            }
            out.push('\n');
        }
        out
    }

    /// What is still going when you quit, in one line, or nothing: runs
    /// still moving and what waits on you.
    pub fn exit_summary(&self) -> Option<String> {
        let mut parts = vec![];
        let runs: Vec<&Run> = self.runs.iter().filter(|r| !r.status.is_terminal()).collect();
        if !runs.is_empty() {
            let hosts: std::collections::BTreeSet<&str> =
                runs.iter().filter_map(|r| r.driver.as_deref()).map(|d| if d == "This device" { "this device" } else { d }).collect();
            let on =
                if hosts.is_empty() { String::new() } else { format!(" (driven by {})", hosts.into_iter().collect::<Vec<_>>().join(", ")) };
            parts.push(format!("{} run{} still going{on}", runs.len(), if runs.len() == 1 { "" } else { "s" }));
        }
        let votes = self.proposals.iter().filter(|p| p.status == ProposalStatus::Open && p.can_vote.is_yes()).count();
        if votes > 0 {
            parts.push(format!("{votes} proposal{} waiting on your vote", if votes == 1 { "" } else { "s" }));
        }
        let asks = self.tasks.iter().filter(|t| t.active && !t.open_questions.is_empty()).count();
        if asks > 0 {
            parts.push(format!("{asks} question{} from agents", if asks == 1 { "" } else { "s" }));
        }
        (!parts.is_empty()).then(|| format!("lt: {}.", parts.join("; ")))
    }

    /// Everything that needs this person in the active workspace: unread
    /// notifications (mentions of you), proposals and gates waiting on
    /// your vote, and agents' open questions. The only thing lt counts in
    /// a badge.
    pub fn needs_you(&self) -> usize {
        let votes = self.proposals.iter().filter(|p| p.status == ProposalStatus::Open && p.can_vote.is_yes()).count();
        let asks = self.tasks.iter().filter(|t| t.active && !t.open_questions.is_empty()).count();
        self.unread_notifications() + votes + asks
    }

    /// Agents working on a reply and runs moving, in this workspace (the
    /// status bar's "2 running").
    pub fn running(&self) -> usize {
        let agents = self.agents.iter().filter(|a| a.status == AgentStatus::Busy).count();
        let runs = self.runs.iter().filter(|r| r.status == RunStatus::Running).count();
        agents + runs
    }
}

/// A row of the Workflows view.
#[derive(Debug, Clone, Copy)]
pub enum WorkflowItem<'a> {
    Def(&'a Workflow),
    Run(&'a Run),
}

/// "now", "5m", "3h", "2d": how long ago `at` was, for a column of ages.
/// In a sentence use [`ago_phrase`] ("just now", "5m ago").
pub fn ago(now: Millis, at: Millis) -> String {
    let s = ((now - at) / 1000).max(0);
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// "just now", "5m ago", "3h ago": how long ago `at` was, for a sentence.
pub fn ago_phrase(now: Millis, at: Millis) -> String {
    match ago(now, at) {
        a if a == "now" => "just now".into(),
        a => format!("{a} ago"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_as_phrases() {
        assert_eq!(ago(120_000, 100_000), "now");
        assert_eq!(ago_phrase(120_000, 100_000), "just now");
        assert_eq!(ago_phrase(400_000, 100_000), "5m ago");
        assert_eq!(ago_phrase(3_700_000, 100_000), "1h ago");
    }
}
