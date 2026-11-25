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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolCall {
    pub name: String,
    pub preview: String,
    pub ok: bool,
    /// How long it took, when this device timed it (live turns here).
    pub duration_ms: Option<u64>,
    /// The start of what it returned (a few lines), when it ran here.
    pub output: String,
}

/// Who can be @mentioned in a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mentionable {
    /// Inserted as `@handle `.
    pub handle: String,
    /// "Agent", "Everyone", "Default agent", or a member's role.
    pub kind: String,
    /// An agent you may not use (agent access controls): why. Shown dimmed
    /// and never completed.
    pub unavailable: Option<String>,
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
    /// Who may use it: "owner" (only its owner; on a shared agent, Admins)
    /// or "everyone".
    pub access: String,
    /// You may not use it: why.
    pub unavailable: Option<String>,
    /// You may change who may use it and see its usage (its owner; an
    /// Admin on a shared agent).
    pub can_manage: bool,
    /// Its usage as this device counted it (only for an agent you manage).
    pub usage: Option<AgentUsage>,
}

/// One agent's usage on this device: today and this month (UTC days).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentUsage {
    /// Today, `YYYY-MM-DD` (UTC).
    pub day: String,
    pub today: Vec<UsageRow>,
    pub month: Vec<UsageRow>,
}

/// One member's use of an agent over a span of days.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRow {
    /// The member's name ("Unattributed" when no person could be traced).
    pub member: String,
    pub turns: u64,
    /// Turns whose runtime reported no token usage (not in the cost).
    pub uncosted: u64,
    /// Estimated cost in millionths of a US dollar, when any turn reported
    /// usage.
    pub cost_micro_usd: Option<u64>,
}

impl UsageRow {
    /// "3 turns, ~$0.0120" / "1 turn (no usage reported)".
    pub fn summary(&self) -> String {
        let turns = format!("{} turn{}", self.turns, if self.turns == 1 { "" } else { "s" });
        match self.cost_micro_usd {
            Some(c) if self.uncosted == 0 => format!("{turns}, ~${:.4}", c as f64 / 1_000_000.0),
            Some(c) => format!("{turns}, ~${:.4} ({} without reported usage)", c as f64 / 1_000_000.0, self.uncosted),
            None => format!("{turns} (no usage reported)"),
        }
    }
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

/// A runtime found on this device that an agent here could run on: a CLI
/// agent on `PATH`, a local model server, or a hosted API whose key is in
/// the environment. Found the way the desktop's onboarding finds them, and
/// never by sending a prompt (version checks only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRuntime {
    /// The id an agent names it by: `local:<provider>[/<model>]`.
    pub id: RuntimeId,
    pub kind: RuntimeKind,
    pub model: Option<String>,
    /// The CLI's `--version`, when it reported one.
    pub version: Option<String>,
    /// Installed and signed in (or its sign-in can't be checked without
    /// asking, which counts as ready), or a key or a model is there.
    pub ready: bool,
    /// How readiness was decided, or what to run to fix it. Never a secret.
    pub detail: String,
    /// The agents in this workspace that already run on it, here.
    pub used_by: Vec<String>,
    /// Ligus can't drive it (the reason), so it can't become an agent.
    pub unsupported: Option<String>,
    /// A name for an agent on it (`claude`, `codex`, ...).
    pub suggested_name: String,
}

impl LocalRuntime {
    /// Detected and not yet used by an agent in this workspace.
    pub fn available(&self) -> bool {
        self.used_by.is_empty()
    }
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
    /// The person this install is a device of ("Add to my devices"); none
    /// while it is its own member.
    pub linked_to: Option<String>,
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

/// This computer's request to be added to its person's devices: the code to
/// paste on their desktop (no secret in it) and the fingerprint both screens
/// show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRequestView {
    /// `liguslink1:…`.
    pub code: String,
    /// `XXXX-XXXX-XXXX-XXXX-XXXX`.
    pub fingerprint: String,
    pub label: String,
    /// The install was already in use, so it asks with a new device key
    /// (switched to only once the grant is redeemed).
    pub new_device: bool,
}

/// Who a desktop's grant says approved this computer, read before anything
/// is saved: the person confirms it is them and their desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkGrantView {
    pub person_name: String,
    pub workspace_name: String,
    /// The approving desktop's device fingerprint (its sheet shows it as
    /// This desktop).
    pub approver_fingerprint: String,
}

/// How a live link request (code-match approval through the workspace's
/// approval address) started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveLinkStart {
    /// Posted: the desktop sees it in My devices. `request` is the link
    /// request it carries, which is also the paste text for an older desktop.
    Posted { rid: String, request: LinkRequestView },
    /// The relay has no approval mailbox: paste the request instead (the
    /// fingerprint compare, as before).
    NoMailbox { request: LinkRequestView },
}

/// Said when 3 live requests ended without an approval in 15 minutes (the
/// requester stops there).
pub const TOO_MANY_DEAD: &str = "3 requests ended without an approval in the last 15 minutes, so lt stops here. Check the computer \
                                 you're adding, start again on a trusted network in a while, and tell an Admin if it keeps happening";

/// The 8-symbol code both screens show (`K7Q4 M9X2`). Not a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchCodeView {
    /// `K7Q4 M9X2`.
    pub shown: String,
    /// What a screen reader reads (`K, 7, Q, 4. M, 9, X, 2.`).
    pub spelled: String,
    /// Who the desktop says it is, unverified until the approval is on the
    /// workspace log ("Alex's desktop").
    pub claimed: String,
    /// When the code stops being valid (5 minutes after the desktop opened
    /// the request).
    pub expires_at: Millis,
}

/// Why a live link request ended without a link. Each is said in words;
/// nothing was granted in any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveLinkEnd {
    /// The person said the codes don't match (this side posted the abort).
    Mismatch,
    /// The desktop said the codes don't match, or ended it (unverified).
    EndedByDesktop,
    /// The desktop declined it.
    Declined,
    /// The code (5 minutes) or the request (15 minutes) ran out.
    Expired,
    /// The grant or the approval on the log was refused: why.
    Refused(String),
    /// The person said No to the verified person (check 6): nothing linked.
    NotLinked { person: String },
    /// Cancelled from this computer.
    Cancelled,
}

impl LiveLinkEnd {
    /// The one line the screen and plain mode show.
    pub fn describe(&self) -> String {
        match self {
            Self::Mismatch => "Request didn't match. Nothing was granted.".into(),
            Self::EndedByDesktop => "Your desktop ended the request. Nothing was granted.".into(),
            Self::Declined => "Your desktop declined the request. Nothing was granted.".into(),
            Self::Expired => "The request ran out of time. Nothing was granted.".into(),
            Self::Refused(why) => format!("Not linked: {why}. Nothing was granted."),
            Self::NotLinked { person } => {
                format!("Nothing was linked. The approval came from “{person}”: if that isn't you, tell an Admin.")
            }
            Self::Cancelled => "Request cancelled. Nothing was granted.".into(),
        }
    }

    /// What to do next.
    pub fn next(&self) -> &'static str {
        match self {
            Self::Mismatch | Self::EndedByDesktop => {
                "Someone may have tried to intercept it. Check the computer you're adding, start again on a trusted network, and tell an Admin if it happens twice."
            }
            _ => "Start again: a new request gets a new code.",
        }
    }
}

/// Progress of a live link request ([`crate::backend::Event::LiveLink`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveLinkUpdate {
    /// The desktop opened it: compare the code.
    Opened(MatchCodeView),
    /// The person said the codes match and the desktop's offer was
    /// acknowledged: the desktop asks for its passkey now.
    Acknowledged,
    /// The approval arrived: checking it on the workspace log (up to 2
    /// minutes while it syncs).
    Checking,
    /// The approval is on the workspace log, by this person: link as them?
    Verified {
        person: String,
        workspace: String,
    },
    Ended(LiveLinkEnd),
}

/// What kind of device an approval request is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalKind {
    Box,
    Computer,
    Phone,
}

impl ApprovalKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Computer => "computer",
            Self::Phone => "phone",
        }
    }

    /// What a request of this kind asks, verb first, before its quoted
    /// label (the kind word comes from the request's kind, never from its
    /// label).
    pub fn asks(self) -> &'static str {
        match self {
            Self::Box => "A box asks to join",
            Self::Computer => "A computer asks to be your device",
            Self::Phone => "A phone asks to be your device",
        }
    }
}

impl ApprovalRequest {
    /// `A box asks to join: “build-1”`.
    pub fn statement(&self) -> String {
        format!("{}: “{}”", self.kind.asks(), self.label)
    }

    /// Who is checking it, the name quoted (`“Ben's desktop” is checking
    /// it`), `waiting`, or the contested warning.
    pub fn checking_line(&self) -> String {
        if self.contested {
            return CONTESTED.to_string();
        }
        if self.mine {
            return "this device is checking it".into();
        }
        match &self.checking {
            Some(n) => format!("“{n}” is checking it"),
            None => "waiting".into(),
        }
    }
}

/// A live approval request in the workspace's mailbox, read only: lt can't
/// approve one, and shows no code (the exchange is sealed to its requester
/// and its approver).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub kind: ApprovalKind,
    /// The device's own label (`build-1`, `lt on mbp`).
    pub label: String,
    /// When it was posted.
    pub at: Millis,
    /// Who says they are checking it ("Ben's desktop"), as its claim says.
    pub checking: Option<String>,
    /// This device claimed it.
    pub mine: bool,
    /// Opened or claimed by a device that can't approve this kind (or
    /// opened with no claim): it may be an attempt to intercept it.
    pub contested: bool,
}

/// What a contested approval request says (design note 6.4).
pub const CONTESTED: &str = "Opened by a device that can't approve this. It may be an attempt to intercept it.";

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

// ── Session state ───────────────────────────────────────────────────────

/// Where a chat stands, in one word: the same states, precedence and
/// glyphs as the desktop's chat rows, so every surface agrees. Derived
/// only from what lt already receives (live turns, agents' open
/// questions, proposals waiting on you, a lockdown).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionState {
    /// Nothing going on.
    Idle,
    /// An agent is replying.
    Working,
    /// An agent asked a question, or a proposal waits on your vote.
    NeedsYou,
    /// Waiting for the agent's host to pick it up.
    Queued,
    /// A turn finished since you last looked.
    Done,
    /// The last turn failed.
    Failed,
    /// A lockdown holds the chat's agents until a person clears it.
    Paused,
}

impl SessionState {
    /// The word shown next to the glyph.
    pub fn word(self) -> &'static str {
        match self {
            SessionState::Idle => "",
            SessionState::Working => "working",
            SessionState::NeedsYou => "needs you",
            SessionState::Queued => "queued",
            SessionState::Done => "done",
            SessionState::Failed => "failed",
            SessionState::Paused => "paused",
        }
    }

    /// A sentence for help and screen readers.
    pub fn describe(self) -> &'static str {
        match self {
            SessionState::Idle => "Nothing going on",
            SessionState::Working => "An agent is working in this chat",
            SessionState::NeedsYou => "An agent asked a question or a change waits on your decision",
            SessionState::Queued => "Waiting for the agent's host to pick it up",
            SessionState::Done => "Finished since you last looked",
            SessionState::Failed => "The last turn failed",
            SessionState::Paused => "Agents are paused here until a person clears the lockdown",
        }
    }

    /// The glyph, which carries the meaning on its own (colour never
    /// does). `ascii` is for fonts without the symbols. Working is a
    /// spinner the UI animates; this is its resting frame.
    pub fn glyph(self, ascii: bool) -> &'static str {
        match (self, ascii) {
            (SessionState::Idle, _) => " ",
            (SessionState::Working, false) => "⠹",
            (SessionState::Working, true) => "*",
            (SessionState::NeedsYou, false) => "◆",
            (SessionState::NeedsYou, true) => "<>",
            (SessionState::Queued, false) => "⧗",
            (SessionState::Queued, true) => "~",
            (SessionState::Done, false) => "✓",
            (SessionState::Done, true) => "ok",
            (SessionState::Failed, false) => "✕",
            (SessionState::Failed, true) => "x",
            (SessionState::Paused, false) => "‖",
            (SessionState::Paused, true) => "=",
        }
    }

    /// Whether it counts as something that needs this person (badges).
    pub fn needs_you(self) -> bool {
        self == SessionState::NeedsYou
    }
}

/// Everything known about one chat that bears on its state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionSignals {
    /// A turn is streaming (or an agent is thinking) in this chat.
    pub live: bool,
    /// The last turn died and nobody has looked since.
    pub failed: bool,
    /// A turn finished while the chat wasn't open.
    pub finished_unseen: bool,
    /// Agent questions still waiting for an answer.
    pub open_asks: usize,
    /// Proposals (and gates) waiting on this person's vote.
    pub proposals_needing_me: usize,
    /// Work waiting on a host.
    pub queued: usize,
    /// A lockdown holds the chat's agents.
    pub paused: bool,
}

impl SessionSignals {
    /// Precedence, highest first: something waiting on you outranks
    /// everything (an agent that asked mid-turn is stopped until you
    /// answer); a failure is next (it won't fix itself), then a lockdown,
    /// then live work. Queued work only shows while nothing runs, and done
    /// is the quietest: it clears when you open the chat.
    pub fn state(&self) -> SessionState {
        if self.open_asks > 0 || self.proposals_needing_me > 0 {
            SessionState::NeedsYou
        } else if self.failed {
            SessionState::Failed
        } else if self.paused {
            SessionState::Paused
        } else if self.live {
            SessionState::Working
        } else if self.queued > 0 {
            SessionState::Queued
        } else if self.finished_unseen {
            SessionState::Done
        } else {
            SessionState::Idle
        }
    }
}

#[cfg(test)]
mod session_state_tests {
    use super::*;

    /// The desktop's precedence (web/src/lib/sessionState.ts), case by case.
    #[test]
    fn precedence_matches_the_desktop() {
        let s = |f: fn(&mut SessionSignals)| {
            let mut x = SessionSignals::default();
            f(&mut x);
            x.state()
        };
        assert_eq!(s(|_| {}), SessionState::Idle);
        assert_eq!(s(|x| x.finished_unseen = true), SessionState::Done);
        assert_eq!(
            s(|x| {
                x.queued = 2;
                x.finished_unseen = true
            }),
            SessionState::Queued
        );
        assert_eq!(
            s(|x| {
                x.live = true;
                x.queued = 1
            }),
            SessionState::Working
        );
        assert_eq!(
            s(|x| {
                x.paused = true;
                x.live = true
            }),
            SessionState::Paused
        );
        assert_eq!(
            s(|x| {
                x.failed = true;
                x.paused = true
            }),
            SessionState::Failed
        );
        assert_eq!(
            s(|x| {
                x.open_asks = 1;
                x.failed = true
            }),
            SessionState::NeedsYou
        );
        assert_eq!(s(|x| x.proposals_needing_me = 1), SessionState::NeedsYou);
    }

    /// Every state but idle has a word and two glyphs, and no two states
    /// share a glyph.
    #[test]
    fn glyphs_are_distinct() {
        use SessionState::*;
        let all = [Working, NeedsYou, Queued, Done, Failed, Paused];
        for ascii in [false, true] {
            let g: std::collections::HashSet<_> = all.iter().map(|x| x.glyph(ascii)).collect();
            assert_eq!(g.len(), all.len());
            assert!(g.iter().all(|g| g.is_ascii() == ascii || !ascii));
        }
        assert!(all.iter().all(|x| !x.word().is_empty() && !x.describe().is_empty()));
    }
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
