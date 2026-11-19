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
}

impl View {
    pub const ALL: [View; 7] = [View::Chats, View::Proposals, View::Agents, View::Hosts, View::People, View::Notifications, View::Settings];
    pub fn title(self) -> &'static str {
        match self {
            View::Chats => "Chats",
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Name,
    Mode,
    Value,
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
    pub field: FormField,
    pub error: Option<String>,
    pub busy: bool,
}

impl WorkspaceForm {
    pub fn new(ask_name: bool) -> Self {
        Self {
            ask_name,
            name: TextBuf::single(),
            mode: FormMode::Create,
            ws_name: TextBuf::single(),
            invite: TextBuf::single(),
            field: if ask_name { FormField::Name } else { FormField::Mode },
            error: None,
            busy: false,
        }
    }
    pub fn fields(&self) -> Vec<FormField> {
        let mut f = vec![];
        if self.ask_name {
            f.push(FormField::Name);
        }
        f.push(FormField::Mode);
        f.push(FormField::Value);
        f
    }
    pub fn value_mut(&mut self) -> &mut TextBuf {
        match self.mode {
            FormMode::Create => &mut self.ws_name,
            FormMode::Join => &mut self.invite,
        }
    }
    pub fn value(&self) -> &TextBuf {
        match self.mode {
            FormMode::Create => &self.ws_name,
            FormMode::Join => &self.invite,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    Help { scroll: usize },
    Switcher { sel: usize },
    AddWorkspace(WorkspaceForm),
    NewChat(TextBuf),
    HostPicker { proposal: ProposalId, sel: usize },
    RolePicker { member: MemberId, sel: usize },
    RuntimePicker { agent: AgentId, sel: usize },
    ConfirmRemove { member: MemberId, name: String },
    Invite { code: Option<String> },
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
    /// Which agents are working on a reply, per chat.
    pub thinking: HashMap<ChatId, BTreeSet<String>>,
    pub composer: TextBuf,
    /// Drafts kept per chat when switching.
    pub drafts: HashMap<ChatId, String>,
    pub mentionables: Vec<Mentionable>,
    pub mention_sel: usize,
    /// The composer text at which the popup was dismissed with Esc.
    pub mention_dismissed: Option<String>,

    pub proposals: Vec<Proposal>,
    pub proposal_sel: Sel,
    pub diff: Option<DiffState>,

    pub agents: Vec<Agent>,
    pub runtimes: Vec<Runtime>,
    pub agent_sel: Sel,
    pub hosts: Vec<Host>,
    pub host_sel: Sel,
    pub members: Vec<Member>,
    pub member_sel: Sel,
    pub notifications: Vec<Notification>,
    pub notification_sel: Sel,
    pub settings: Option<Settings>,
    pub settings_sel: Sel,

    pub toasts: Vec<Toast>,
    pub sync_status: HashMap<WorkspaceId, String>,
    /// Where the config file is (for Settings).
    pub config_path: Option<String>,
    pub theme_name: String,
    /// Ring the terminal bell once (a mention arrived).
    pub bell: bool,
    pub quit: bool,
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
            thinking: HashMap::new(),
            composer: TextBuf::multi(),
            drafts: HashMap::new(),
            mentionables: vec![],
            mention_sel: 0,
            mention_dismissed: None,
            proposals: vec![],
            proposal_sel: Sel::default(),
            diff: None,
            agents: vec![],
            runtimes: vec![],
            agent_sel: Sel::default(),
            hosts: vec![],
            host_sel: Sel::default(),
            members: vec![],
            member_sel: Sel::default(),
            notifications: vec![],
            notification_sel: Sel::default(),
            settings: None,
            settings_sel: Sel::default(),
            toasts: vec![],
            sync_status: HashMap::new(),
            config_path: None,
            theme_name: "dark".into(),
            bell: false,
            quit: false,
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

    pub fn selected_proposal(&self) -> Option<&Proposal> {
        self.proposals.get(self.proposal_sel.index)
    }

    pub fn selected_member(&self) -> Option<&Member> {
        self.members.get(self.member_sel.index)
    }

    pub fn unread_notifications(&self) -> usize {
        self.notifications.iter().filter(|n| !n.read).count()
    }

    /// The host "Apply on ‹host›" means by default: the first online
    /// worker, else this device.
    pub fn default_apply_host(&self) -> Option<&Host> {
        self.hosts.iter().find(|h| h.online && h.kind == HostKind::Worker).or_else(|| self.hosts.iter().find(|h| h.this_device))
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
        match &self.overlay {
            Some(Overlay::AddWorkspace(f)) => f.field != FormField::Mode,
            Some(Overlay::NewChat(_)) => true,
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
        use Context::*;
        if self.view == View::Chats && self.focus == Focus::Composer && self.overlay.is_none() && self.presence.is_empty() {
            return vec![Composer, Global];
        }
        match self.view {
            View::Chats => vec![Chat, Nav, Global],
            View::Proposals if self.focus == Focus::Detail => vec![Diff, Review, Nav, Global],
            View::Proposals => vec![Review, Nav, Global],
            View::Agents => vec![Agents, Nav, Global],
            View::Hosts => vec![Nav, Global],
            View::People => vec![People, Nav, Global],
            View::Notifications => vec![Notifications, Nav, Global],
            View::Settings => vec![Settings, Nav, Global],
        }
    }

    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), kind, ttl: 16 });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }
}

/// "now", "5m", "3h", "2d": how long ago `at` was.
pub fn ago(now: Millis, at: Millis) -> String {
    let s = ((now - at) / 1000).max(0);
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}
