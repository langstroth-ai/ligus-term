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
    Onboarding(WorkspaceForm),
    /// The recovery words, shown once after creating a workspace.
    Recovery(Created),
    Main,
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
        }
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceSheet {
    pub prompt: PresencePrompt,
    pub stage: PresenceStage,
    pub error: Option<String>,
    /// No browser here: the page's address for another device's browser,
    /// and the loopback port that browser must reach.
    pub remote: Option<(String, u16)>,
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
    RejectGate { run: RunId, stage: String },
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
    pub workspaces: Vec<WorkspaceSummary>,
    pub active: Option<WorkspaceId>,
    pub view: View,
    pub focus: Focus,
    pub overlay: Option<Overlay>,
    /// Passkey prompts, oldest first; the first is on screen.
    pub presence: VecDeque<PresenceSheet>,

    pub chats: Vec<ChatSummary>,
    pub chat_sel: Sel,
    pub open_chat: Option<ChatId>,
    pub transcript: Vec<Message>,
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

    pub proposals: Vec<Proposal>,
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
            workspaces: vec![],
            active: None,
            view: View::Chats,
            focus: Focus::List,
            overlay: None,
            presence: VecDeque::new(),
            chats: vec![],
            chat_sel: Sel::default(),
            open_chat: None,
            transcript: vec![],
            transcript_scroll: 0,
            tools_expanded: false,
            thinking: HashMap::new(),
            turn_outcomes: HashMap::new(),
            composer: TextBuf::multi(),
            drafts: HashMap::new(),
            sent: vec![],
            history_pos: None,
            mentionables: vec![],
            mention_sel: 0,
            mention_dismissed: None,
            proposals: vec![],
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
            open_asks: self.tasks.iter().filter(|t| &t.chat == chat && t.active && !t.open_questions.is_empty()).count(),
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

    pub fn selected_proposal(&self) -> Option<&Proposal> {
        self.proposals.get(self.proposal_sel.index)
    }

    pub fn selected_member(&self) -> Option<&Member> {
        self.members.get(self.member_sel.index)
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
            Some(Overlay::NewChat(_)) | Some(Overlay::StartRun { .. }) | Some(Overlay::BindCheckout(_)) => true,
            Some(Overlay::Help { searching, .. }) => *searching,
            Some(Overlay::Palette { .. }) | Some(Overlay::History { .. }) => true,
            Some(Overlay::AdmitBox { result: None, checked: None, .. }) => true,
            Some(_) => false,
            None => match &self.phase {
                Phase::Onboarding(f) => f.field != FormField::Mode,
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
            View::Proposals => {
                (self.proposals.iter().map(|p| format!("{} {} {}", p.title, p.author, p.status.label())).collect(), self.proposal_sel.index)
            }
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
