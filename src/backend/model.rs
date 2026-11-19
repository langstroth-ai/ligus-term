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

id_type!(WorkspaceId, ChatId, MessageId, ProposalId, MemberId, AgentId, HostId, RuntimeId, NotificationId, WorkflowId, RunId, MemoryId);

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
    /// Whether you may issue invites, and why not.
    pub can_invite: Allowed,
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
    /// Busy on a reply, idle, or its host is offline.
    pub status: AgentStatus,
    /// What it says it is doing (its live task state), if anything.
    pub doing: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    /// Streaming a reply now.
    Busy,
    Idle,
    /// Its host isn't online.
    Offline,
}

impl AgentStatus {
    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Busy => "busy",
            AgentStatus::Idle => "idle",
            AgentStatus::Offline => "offline",
        }
    }
}

/// One agent handing work to another (a cascade, a tool hand-off or a
/// workflow stage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub chat: ChatId,
    pub from: String,
    pub to: String,
    pub task: String,
    pub at: Millis,
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
    /// Where it runs: a CLI on a host, a local model server, or a remote API.
    pub fn place(self) -> &'static str {
        match self {
            RuntimeKind::ClaudeCode
            | RuntimeKind::Codex
            | RuntimeKind::Antigravity
            | RuntimeKind::Copilot
            | RuntimeKind::Pi
            | RuntimeKind::Aider => "local CLI",
            RuntimeKind::Ollama => "local server",
            _ => "API",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub id: HostId,
    pub label: String,
    pub kind: HostKind,
    /// A heartbeat within the online window.
    pub online: bool,
    pub last_seen: Option<Millis>,
    /// This device.
    pub this_device: bool,
    /// One of its agents is streaming a reply.
    pub busy: bool,
    /// The agents that run here.
    pub agents: Vec<String>,
    /// The workspace repos it covers (from its heartbeat).
    pub repos: Vec<String>,
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
    pub kind: ProposalKind,
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

/// What a proposal asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalKind {
    /// A code change (a diff), applied on a host.
    Change,
    /// A decision to vote on.
    Decision,
    /// An agent-authored workflow: approving it adds the definition.
    Workflow,
    /// A workflow run's approval gate.
    Gate,
}

impl ProposalKind {
    pub fn label(self) -> &'static str {
        match self {
            ProposalKind::Change => "change",
            ProposalKind::Decision => "decision",
            ProposalKind::Workflow => "workflow",
            ProposalKind::Gate => "gate",
        }
    }
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
#[derive(Clone, PartialEq, Eq)]
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

// ── Deployments ─────────────────────────────────────────────────────────

/// A deployment's workspace, as `ligus-local` or the Kubernetes chart
/// configure it: the relay, the room and its key (the chart's workspace
/// Secret), and a relay access token when the relay is gated.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct DeploymentSpec {
    /// The name to show for the workspace.
    pub name: String,
    pub relay_url: String,
    pub room: String,
    pub key: String,
    pub token: Option<String>,
}

/// What an invite says about who sent it, checked before joining.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteCheck {
    pub workspace_name: String,
    pub relay_url: String,
    pub founder_name: Option<String>,
    /// The founder's safety code as the invite carries it.
    pub safety_code: Option<String>,
    pub trust: InviteTrust,
    pub already_joined: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteTrust {
    /// A co-member whose device signed it, with the key your workspaces know.
    Known,
    /// Names a co-member and their key but isn't signed by their device.
    Claimed,
    /// A co-member with a different key: not from them (joining is refused).
    Mismatch,
    /// Someone you share no workspace with: compare the safety code.
    Unknown,
    /// The invite carries no founder fingerprint.
    NoFingerprint,
}

impl InviteTrust {
    pub fn label(self) -> &'static str {
        match self {
            InviteTrust::Known => "known",
            InviteTrust::Claimed => "claimed",
            InviteTrust::Mismatch => "mismatch",
            InviteTrust::Unknown => "unknown",
            InviteTrust::NoFingerprint => "none",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            InviteTrust::Known => "signed by the founder's device, and the key matches the one your workspaces know",
            InviteTrust::Claimed => "it says it's from them, but their device didn't sign it",
            InviteTrust::Mismatch => "the founder's key doesn't match the one your workspaces know: not from them",
            InviteTrust::Unknown => "you share no workspace with them yet: compare the safety code",
            InviteTrust::NoFingerprint => "the invite carries no founder fingerprint",
        }
    }
}

/// A box's join request, read before anything is asked of the Admin: its
/// fingerprint is compared with the box's terminal before the passkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequestCheck {
    pub fingerprint: String,
    pub label: Option<String>,
}

/// An admission for a box (`ligus serve` or `ligus worker`): it joins as
/// its own service member (Contributor).
#[derive(Clone, PartialEq, Eq)]
pub struct BoxAdmission {
    /// `ligusadmit1:` (sealed to the box's join request) or a bearer
    /// `hivews1:` code (for a Kubernetes Secret or `LIGUS_ADMISSION_FILE`).
    pub code: String,
    pub sealed: bool,
    /// The join request's fingerprint, to compare with the box's terminal.
    pub fingerprint: Option<String>,
    pub label: Option<String>,
    pub expires_at: Millis,
}

// ── Workflows ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workflow {
    pub id: WorkflowId,
    /// Definitions belong to the chat they were made in.
    pub chat: ChatId,
    pub chat_title: String,
    pub name: String,
    pub description: String,
    /// What the run's input is for, if it asks for one.
    pub input_label: Option<String>,
    pub stages: Vec<StageDef>,
    pub enabled: bool,
    /// Whether you may start it, and why not.
    pub can_start: Allowed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageDef {
    pub id: String,
    pub name: String,
    /// `@agent`, or `gate (N approvals)`.
    pub what: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    AwaitingGate,
    Completed,
    Failed,
    Halted,
    Canceled,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::AwaitingGate => "awaiting-gate",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Halted => "halted",
            RunStatus::Canceled => "canceled",
        }
    }
    pub fn is_terminal(self) -> bool {
        !matches!(self, RunStatus::Running | RunStatus::AwaitingGate)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStatus {
    Pending,
    Running,
    AwaitingApproval,
    Succeeded,
    Failed,
    Rejected,
    Skipped,
}

impl StageStatus {
    pub fn label(self) -> &'static str {
        match self {
            StageStatus::Pending => "pending",
            StageStatus::Running => "running",
            StageStatus::AwaitingApproval => "awaiting-approval",
            StageStatus::Succeeded => "succeeded",
            StageStatus::Failed => "failed",
            StageStatus::Rejected => "rejected",
            StageStatus::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: RunId,
    pub workflow: WorkflowId,
    pub chat: ChatId,
    pub name: String,
    pub input: String,
    pub status: RunStatus,
    pub stages: Vec<StageRun>,
    pub started_by: String,
    pub started_at: Millis,
    pub updated_at: Millis,
    /// The device driving it (its coordinator lease), if known.
    pub driver: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRun {
    pub id: String,
    pub name: String,
    pub what: String,
    pub status: StageStatus,
    /// A gate's proposal (vote on it to decide the gate).
    pub gate: Option<ProposalId>,
    /// The start of the stage's output.
    pub excerpt: String,
    pub error: String,
    pub attempts: u32,
}

// ── Memory and task state ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntry {
    pub id: MemoryId,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub author: String,
    pub at: Millis,
    pub revisions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Memory {
    pub entries: Vec<MemoryEntry>,
    /// Agents' proposed edits waiting for a person.
    pub open_proposals: usize,
}

/// An agent's live task state in a chat ("working, 40%: …").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskState {
    pub chat: ChatId,
    pub agent: String,
    /// working, blocked, waiting, done or abandoned.
    pub status: String,
    pub summary: String,
    pub progress: Option<u8>,
    pub open_questions: Vec<String>,
    pub at: Millis,
    /// Still within its time to live and not finished.
    pub active: bool,
}

// Secrets never show in `{:?}` (a panic, a test failure, a debug log).
const REDACTED: &str = "<redacted>";

impl fmt::Debug for DeploymentSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeploymentSpec")
            .field("name", &self.name)
            .field("relay_url", &self.relay_url)
            .field("room", &self.room)
            .field("key", &REDACTED)
            .field("token", &self.token.as_ref().map(|_| REDACTED))
            .finish()
    }
}

impl fmt::Debug for Created {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Created")
            .field("workspace", &self.workspace)
            .field("recovery_words", &REDACTED)
            .field("invite", &self.invite)
            .finish()
    }
}

impl fmt::Debug for BoxAdmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxAdmission")
            .field("code", &REDACTED)
            .field("sealed", &self.sealed)
            .field("fingerprint", &self.fingerprint)
            .field("label", &self.label)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod secret_debug_tests {
    use super::*;

    /// A secret never shows in `{:?}` (a panic message, a test failure, a
    /// debug log of a `Msg` or an `Effect`).
    #[test]
    fn debug_output_never_shows_a_secret() {
        let spec = DeploymentSpec {
            name: "n".into(),
            relay_url: "http://r".into(),
            room: "room".into(),
            key: "the-workspace-key".into(),
            token: Some("hrt1-relay-token".into()),
        };
        let created = Created {
            workspace: WorkspaceSummary {
                id: WorkspaceId::new("w"),
                name: "w".into(),
                mentioned: false,
                sync: SyncMode::Local,
                your_role: Role::Owner,
                can_invite: Allowed::Yes,
            },
            recovery_words: vec!["abandonword".into(); 24],
            invite: Some("invite-code".into()),
        };
        let bearer = BoxAdmission { code: "hivews1:bearer-secret".into(), sealed: false, fingerprint: None, label: None, expires_at: 0 };
        let shown = format!("{spec:?} {created:?} {bearer:?}");
        for secret in ["the-workspace-key", "hrt1-relay-token", "abandonword", "bearer-secret"] {
            assert!(!shown.contains(secret), "{secret} in {shown}");
        }
        assert!(shown.contains("room") && shown.contains("http://r"), "{shown}");
    }
}
