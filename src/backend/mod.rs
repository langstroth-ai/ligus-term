//! The seam between `lt` and Ligus.
//!
//! The UI talks only to [`Backend`]: async calls for reads and actions, plus
//! one [`Event`] stream for everything that happens on its own (a message
//! arriving, an agent streaming a reply, someone typing, a proposal changing,
//! a passkey prompt). Today [`fake::FakeBackend`] implements it in memory; the
//! real implementation wraps the Ligus crates (`docs/ARCHITECTURE.md`).
//!
//! Rules the UI relies on:
//! * Authorization is the backend's: [`model::Allowed`] carries the refusal
//!   reason, and the UI only shows it.
//! * Actions that need a passkey don't fail for want of one: the backend
//!   emits [`Event::PresencePrompt`] and waits for
//!   [`Backend::answer_presence`] before the action's future resolves.

pub mod fake;
pub mod model;
pub mod presence;
pub mod real;

use async_trait::async_trait;
use futures::stream::BoxStream;

use model::*;

/// An error the UI shows as is (one line, no trailing period needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError(pub String);

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BackendError {}

pub type Result<T> = std::result::Result<T, BackendError>;

pub fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(BackendError(msg.into()))
}

/// Something that happened without the UI asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A whole message (someone else's, or an agent's first chunk).
    MessageAdded {
        workspace: WorkspaceId,
        message: Message,
    },
    /// More text for a streaming message.
    MessageDelta {
        workspace: WorkspaceId,
        chat: ChatId,
        message: MessageId,
        delta: String,
    },
    /// A streaming message finished (or failed with `error`).
    MessageDone {
        workspace: WorkspaceId,
        chat: ChatId,
        message: MessageId,
        error: Option<String>,
    },
    /// A tool call an agent made during its turn.
    ToolActivity {
        workspace: WorkspaceId,
        chat: ChatId,
        message: MessageId,
        call: ToolCall,
    },
    /// Someone (or an agent, before its first token) is typing or thinking.
    Typing {
        workspace: WorkspaceId,
        chat: ChatId,
        who: String,
        on: bool,
    },
    ChatsChanged {
        workspace: WorkspaceId,
    },
    ProposalChanged {
        workspace: WorkspaceId,
        proposal: Proposal,
    },
    MembersChanged {
        workspace: WorkspaceId,
    },
    HostsChanged {
        workspace: WorkspaceId,
    },
    AgentsChanged {
        workspace: WorkspaceId,
    },
    WorkspacesChanged,
    Notification(Notification),
    /// The backend needs a passkey (or the recovery key) to go on.
    PresencePrompt(PresencePrompt),
    /// A problem the person can fix without the ceremony ending (an
    /// unreadable recovery-words file, say).
    PresenceProblem {
        id: PresenceId,
        error: String,
    },
    /// That ceremony ended.
    PresenceDone {
        id: PresenceId,
        ok: bool,
        error: Option<String>,
    },
    /// Sync status, shown in the status bar.
    Sync {
        workspace: WorkspaceId,
        status: String,
    },
}

/// Everything `lt` needs from Ligus.
#[async_trait]
pub trait Backend: Send + Sync {
    /// This device.
    fn device(&self) -> DeviceInfo;

    /// The live event stream. Each call returns a fresh subscription.
    fn events(&self) -> BoxStream<'static, Event>;

    // ── First run ──────────────────────────────────────────────────────
    async fn startup(&self) -> Result<StartupState>;
    /// Create a workspace (no server needed); hands back the recovery words
    /// to show once. Creating the founder's passkey raises a presence prompt.
    async fn create_workspace(&self, your_name: &str, name: &str) -> Result<Created>;
    /// Join with a pasted invite code.
    async fn join_workspace(&self, your_name: &str, invite: &str) -> Result<WorkspaceSummary>;
    /// The person confirmed they wrote the recovery words down.
    async fn confirm_recovery_saved(&self, workspace: &WorkspaceId) -> Result<()>;

    // ── Workspaces and chats ───────────────────────────────────────────
    async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>>;
    async fn create_invite(&self, workspace: &WorkspaceId, role: Role) -> Result<String>;
    async fn chats(&self, workspace: &WorkspaceId) -> Result<Vec<ChatSummary>>;
    async fn create_chat(&self, workspace: &WorkspaceId, title: &str) -> Result<ChatSummary>;
    async fn transcript(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<Vec<Message>>;
    /// Post a message; agent replies arrive as events.
    async fn send_message(&self, workspace: &WorkspaceId, chat: &ChatId, text: &str) -> Result<Message>;
    async fn mentionables(&self, workspace: &WorkspaceId) -> Result<Vec<Mentionable>>;
    async fn mark_read(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()>;
    /// Any member may lock a chat down.
    async fn lock_chat(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()>;
    /// A human Contributor or above clears it, with a passkey.
    async fn clear_lockdown(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()>;

    // ── Proposals ──────────────────────────────────────────────────────
    async fn proposals(&self, workspace: &WorkspaceId) -> Result<Vec<Proposal>>;
    async fn proposal_diff(&self, workspace: &WorkspaceId, proposal: &ProposalId) -> Result<Diff>;
    async fn vote(&self, workspace: &WorkspaceId, proposal: &ProposalId, choice: VoteChoice) -> Result<Proposal>;
    /// Ask one host to apply an approved proposal on its checkout.
    async fn request_apply(&self, workspace: &WorkspaceId, proposal: &ProposalId, host: &HostId) -> Result<Proposal>;
    async fn withdraw_apply(&self, workspace: &WorkspaceId, proposal: &ProposalId) -> Result<Proposal>;

    // ── Agents, runtimes, hosts ────────────────────────────────────────
    async fn agents(&self, workspace: &WorkspaceId) -> Result<Vec<Agent>>;
    async fn runtimes(&self, workspace: &WorkspaceId) -> Result<Vec<Runtime>>;
    async fn hosts(&self, workspace: &WorkspaceId) -> Result<Vec<Host>>;
    async fn set_agent_runtime(&self, workspace: &WorkspaceId, agent: &AgentId, runtime: &RuntimeId) -> Result<()>;

    // ── People ─────────────────────────────────────────────────────────
    async fn members(&self, workspace: &WorkspaceId) -> Result<Vec<Member>>;
    async fn set_role(&self, workspace: &WorkspaceId, member: &MemberId, role: Role) -> Result<()>;
    async fn remove_member(&self, workspace: &WorkspaceId, member: &MemberId) -> Result<()>;

    // ── Notifications and settings ─────────────────────────────────────
    async fn notifications(&self) -> Result<Vec<Notification>>;
    async fn mark_notifications_read(&self) -> Result<()>;
    async fn settings(&self) -> Result<Settings>;
    async fn set_setting(&self, key: SettingKey, on: bool) -> Result<Settings>;

    // ── Presence ───────────────────────────────────────────────────────
    /// Answer a [`Event::PresencePrompt`].
    async fn answer_presence(&self, id: PresenceId, answer: PresenceAnswer) -> Result<()>;
}
