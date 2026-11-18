//! The data `lt` shows, as the [`Backend`](super::Backend) hands it over.
//!
//! These are view models: plain, cloneable snapshots shaped for the
//! terminal. The real backend fills them from the Ligus crates' projections
//! (see `docs/ARCHITECTURE.md` for the mapping); the fake fills them from
//! memory. Nothing here decides authorization: whether an action is allowed
//! comes from the backend, with the reason it is refused.

use std::fmt;

macro_rules! id_type {
    ($($name:ident),* $(,)?) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
        pub struct $name(pub String);
        impl $name {
            pub fn new(s: impl Into<String>) -> Self { Self(s.into()) }
            pub fn as_str(&self) -> &str { &self.0 }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
        }
    )*};
}

id_type!(WorkspaceId, ChatId, MessageId, ProposalId, MemberId, AgentId, HostId, RuntimeId, NotificationId);

/// A pending passkey ceremony, numbered by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PresenceId(pub u64);

/// Milliseconds since the Unix epoch.
pub type Millis = i64;

/// This device: `lt` is its own device with its own keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub device_name: String,
    /// The account's safety code, shown so people can compare it out of band.
    pub safety_code: String,
    /// Where this device keeps its identity and database.
    pub data_dir: String,
    /// Which OS secret store holds the keys ("macOS keychain", "file", ...).
    pub secret_store: String,
    /// A label for the backend, shown in the status bar ("demo" for the fake).
    pub backend_label: String,
}

/// What the app needs to know before showing anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupState {
    /// The person's display name, if this device has an identity.
    pub your_name: Option<String>,
    pub workspaces: Vec<WorkspaceSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSummary {
    pub id: WorkspaceId,
    pub name: String,
    /// Holds an unread @-mention of you.
    pub mentioned: bool,
    /// Synced through a relay, or peer-to-peer/local only.
    pub sync: SyncMode,
    pub your_role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncMode {
    /// This device only.
    Local,
    /// Peer to peer, with no server.
    Peer,
    /// Through a relay at this URL.
    Relay(String),
}

impl SyncMode {
    pub fn label(&self) -> String {
        match self {
            SyncMode::Local => "this device only".into(),
            SyncMode::Peer => "peer to peer".into(),
            SyncMode::Relay(url) => format!("relay {url}"),
        }
    }
}

/// The two-letter tile the workspace switcher shows (the desktop's rule:
/// first letters of the first two words, else the first two characters).
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split([' ', '-', '_']).filter(|w| !w.is_empty()).collect();
    let s: String = match words.as_slice() {
        [] => return "#".into(),
        [one] => one.chars().take(2).collect(),
        [a, b, ..] => a.chars().take(1).chain(b.chars().take(1)).collect(),
    };
    s.to_uppercase()
}

/// A chat's lockdown: agents get no network and every proposal needs a human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lockdown {
    pub reasons: Vec<String>,
    /// Whether you may clear it, and why not.
    pub clear: Allowed,
}

/// Whether an action is allowed, with the backend's reason when it isn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Allowed {
    Yes,
    No(String),
}

impl Allowed {
    pub fn is_yes(&self) -> bool {
        matches!(self, Allowed::Yes)
    }
    pub fn reason(&self) -> Option<&str> {
        match self {
            Allowed::Yes => None,
            Allowed::No(r) => Some(r),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSummary {
    pub id: ChatId,
    pub title: String,
    /// The channel it is filed under, if any.
    pub channel: Option<String>,
    pub updated_at: Millis,
    pub unread: bool,
    pub mentioned: bool,
    /// Lockdown is per chat.
    pub lockdown: Option<Lockdown>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Author {
    Member { id: MemberId, name: String },
    Agent { id: AgentId, handle: String, runtime: String },
    System,
}

impl Author {
    pub fn display(&self) -> String {
        match self {
            Author::Member { name, .. } => name.clone(),
            Author::Agent { handle, .. } => format!("@{handle}"),
            Author::System => "ligus".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub chat: ChatId,
    pub author: Author,
    pub text: String,
    pub at: Millis,
    /// Still streaming in.
    pub streaming: bool,
    pub tools: Vec<ToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub preview: String,
    pub ok: bool,
}

/// Who can be @mentioned in a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mentionable {
    /// Inserted as `@handle `.
    pub handle: String,
    /// "Agent", "Everyone", "Default agent", or a member's role.
    pub kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Viewer,
    Contributor,
    Admin,
    Owner,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::Owner, Role::Admin, Role::Contributor, Role::Viewer];
    pub fn label(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Contributor => "contributor",
            Role::Viewer => "viewer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: MemberId,
    pub name: String,
    pub role: Role,
    pub founder: bool,
    /// A service member (a box).
    pub service: bool,
    pub you: bool,
    pub online: bool,
    /// What you may do to this member; refused actions carry the reason.
    pub can_remove: Allowed,
    /// One entry per role, in [`Role::ALL`] order.
    pub can_set_role: Vec<(Role, Allowed)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub id: AgentId,
    pub handle: String,
    pub runtime: RuntimeId,
    pub role: Option<String>,
    /// `None` is this device.
    pub host: Option<HostId>,
    /// Shared with the workspace, or owned by one member.
    pub shared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub id: RuntimeId,
    pub label: String,
    pub kind: RuntimeKind,
    pub model: Option<String>,
    pub configured: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    ClaudeCode,
    Codex,
    Antigravity,
    Copilot,
    Pi,
    Aider,
    Ollama,
    Anthropic,
    OpenAi,
    OpenRouter,
    Custom,
}

impl RuntimeKind {
    pub fn label(self) -> &'static str {
        match self {
            RuntimeKind::ClaudeCode => "Claude Code",
            RuntimeKind::Codex => "Codex",
            RuntimeKind::Antigravity => "Google Antigravity CLI",
            RuntimeKind::Copilot => "GitHub Copilot CLI",
            RuntimeKind::Pi => "Pi",
            RuntimeKind::Aider => "Aider",
            RuntimeKind::Ollama => "Ollama (local)",
            RuntimeKind::Anthropic => "Anthropic API",
            RuntimeKind::OpenAi => "OpenAI",
            RuntimeKind::OpenRouter => "OpenRouter",
            RuntimeKind::Custom => "OpenAI-compatible API",
        }
    }
    /// Runs as a local CLI on a host (vs an HTTP provider).
    pub fn local(self) -> bool {
        matches!(
            self,
            RuntimeKind::ClaudeCode | RuntimeKind::Codex | RuntimeKind::Antigravity | RuntimeKind::Copilot | RuntimeKind::Pi | RuntimeKind::Aider
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub id: HostId,
    pub label: String,
    pub kind: HostKind,
    pub online: bool,
    pub last_seen: Option<Millis>,
    /// This device.
    pub this_device: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    Device,
    Worker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalStatus {
    Open,
    Approved,
    Rejected,
    Applied,
}

impl ProposalStatus {
    pub fn label(self) -> &'static str {
        match self {
            ProposalStatus::Open => "open",
            ProposalStatus::Approved => "approved",
            ProposalStatus::Rejected => "rejected",
            ProposalStatus::Applied => "applied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: ProposalId,
    pub chat: Option<ChatId>,
    pub title: String,
    pub author: String,
    pub status: ProposalStatus,
    pub files_changed: usize,
    pub approvals: usize,
    pub quorum: usize,
    pub votes: Vec<Vote>,
    pub created_at: Millis,
    pub needs_presence: bool,
    pub apply: ApplyState,
    /// Whether you may vote, and why not.
    pub can_vote: Allowed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vote {
    pub member: String,
    pub role: Role,
    pub approve: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyState {
    NotRequested,
    Waiting { host: String },
    Applied { host: String, commit: String },
    Failed { host: String, reason: String },
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -a,b +c,d @@` header.
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Context(String),
    Added(String),
    Removed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteChoice {
    Approve,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: NotificationId,
    pub workspace: WorkspaceId,
    pub chat: Option<ChatId>,
    pub text: String,
    pub at: Millis,
    pub read: bool,
}

/// What creating a workspace hands back: the recovery key is shown once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub workspace: WorkspaceSummary,
    /// The 24 recovery words.
    pub recovery_words: Vec<String>,
    /// An invite code to share.
    pub invite: Option<String>,
}

/// A passkey ceremony the backend is waiting on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresencePrompt {
    pub id: PresenceId,
    /// The event kind (`memberRemoved`, `proposalVoteCast`, ...).
    pub kind: String,
    /// What is being authorized, in words.
    pub summary: String,
    /// The first 16 hex characters of the event digest.
    pub digest_short: String,
    /// `get` (confirm) or `create` (register a passkey).
    pub op: PresenceOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceOp {
    Get,
    Create,
}

/// How the person answers a presence prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceAnswer {
    /// Open the system browser to the hand-off page and wait on loopback.
    Browser,
    /// Sign with the recovery key read from this file (24 words).
    RecoveryWordsFile(String),
    Cancel,
}

/// Device-local settings `lt` shows and edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub your_name: String,
    pub device_name: String,
    pub notify_mentions: bool,
    pub agents_may_edit: bool,
    pub relay_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    NotifyMentions,
    AgentsMayEdit,
}
