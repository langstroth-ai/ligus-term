//! The seam between `lt` and Ligus.
//!
//! The UI talks only to [`Backend`]: async calls for reads and actions, plus
//! one [`Event`] stream for everything that happens on its own (a message
//! arriving, an agent streaming a reply, a proposal changing,
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

pub use crate::setup::{CancelFlag, FlowRow, LtFlow, SetupForm, SetupInputs, SetupOutcome, SetupSecrets};

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
    /// An agent this device runs is working on a reply (before its first
    /// token). `agent` is its handle with the `@`. People's typing and
    /// online presence are deliberately not part of `lt`.
    Thinking {
        workspace: WorkspaceId,
        chat: ChatId,
        agent: String,
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
    /// Workflow definitions changed (one added, approved or removed).
    WorkflowsChanged {
        workspace: WorkspaceId,
    },
    /// A workflow run started or moved (a stage, a gate, its status).
    RunChanged {
        workspace: WorkspaceId,
        run: Run,
    },
    /// A chat's decision cards changed (asked, answered, closed).
    AsksChanged {
        workspace: WorkspaceId,
        chat: ChatId,
    },
    /// Workspace memory or an agent's task state changed.
    MemoryChanged {
        workspace: WorkspaceId,
    },
    WorkspacesChanged,
    /// What the setup flows know about this device changed (a harness's
    /// sign-in check finished): an open setup form asks again.
    SetupFactsChanged,
    /// A setup run is about to ask for the passkey: confirmation `step` of
    /// `of`, for `what` ("Add @coder").
    SetupConfirming {
        step: usize,
        of: usize,
        what: String,
    },
    Notification(Notification),
    /// The backend needs a passkey (or the recovery key) to go on.
    PresencePrompt(PresencePrompt),
    /// A problem the person can fix without the ceremony ending (an
    /// unreadable recovery-words file, say).
    PresenceProblem {
        id: PresenceId,
        error: String,
    },
    /// A passkey hand-off to the person's own desktop or phone moved on.
    PresenceHandoff {
        id: PresenceId,
        update: HandoffUpdate,
    },
    /// That ceremony ended.
    PresenceDone {
        id: PresenceId,
        ok: bool,
        error: Option<String>,
    },
    /// No browser here: the passkey page's address for a browser on
    /// another device, and the loopback port it must reach.
    PresenceUrl {
        id: PresenceId,
        url: String,
        port: u16,
    },
    /// Redeeming a pasted link code: the desktop's approval hasn't reached
    /// this computer yet, so it keeps checking until `until` (the code's
    /// end, ms since the epoch). `why` is the last reason, in one line.
    LinkWaiting {
        until: i64,
        why: String,
    },
    /// A live link request moved on (`rid` names it; a stale one is ignored).
    LiveLink {
        rid: String,
        update: LiveLinkUpdate,
    },
    /// Sync status, shown in the status bar.
    Sync {
        workspace: WorkspaceId,
        status: String,
    },
    /// This computer was renamed on its person's desktop (a synced name
    /// newer than the one lt keeps): lt keeps that name now.
    DeviceRenamed {
        name: String,
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
    /// What an invite says about who sent it (its founder's signature,
    /// checked against the workspaces this device shares with them).
    async fn inspect_invite(&self, invite: &str) -> Result<InviteCheck>;
    /// Join with a pasted invite code. An invite whose founder key
    /// contradicts what this device knows is refused.
    async fn join_workspace(&self, your_name: &str, invite: &str) -> Result<WorkspaceSummary>;
    /// Found a deployment's workspace (`ligus-local`, the Kubernetes chart):
    /// its relay, room and key, with this device as the founder. Refused
    /// when the relay already holds that workspace (join it with an invite).
    async fn connect_deployment(&self, your_name: &str, spec: DeploymentSpec) -> Result<Created>;
    /// Ask to be added to a person's devices ("Add to my devices"): this
    /// computer's link request, remembered as pending (no workspace write).
    async fn link_request(&self) -> Result<LinkRequestView>;
    /// Open a desktop's grant for this computer and say who approved it
    /// (checked offline; nothing is saved).
    async fn inspect_link_grant(&self, grant: &str) -> Result<LinkGrantView>;
    /// Redeem the grant once the person confirmed who approved it: connect,
    /// pull, check the desktop's certificate for this computer, and act as
    /// that person from then on.
    /// While the desktop's approval hasn't reached this computer it keeps
    /// checking, with [`Event::LinkWaiting`], until the code runs out or
    /// [`Backend::cancel_link_redeem`].
    async fn redeem_link(&self, grant: &str) -> Result<Linked>;
    /// Stop a [`Backend::redeem_link`] that is waiting for the desktop's
    /// approval (nothing is linked).
    fn cancel_link_redeem(&self) {}
    /// Accept the desktop's certificate again where a link is recorded but
    /// not confirmed here yet ([`Linked::unconfirmed`]); why it still
    /// isn't, if so.
    async fn confirm_link(&self) -> Result<()>;
    /// Code-match approval: post this computer's link request to the
    /// approval address its desktop shows (`ligusaddr1:`, My devices > Add a
    /// computer). Progress arrives as [`Event::LiveLink`]. A relay without
    /// an approval mailbox is [`LiveLinkStart::NoMailbox`] (paste instead).
    async fn link_live(&self, address: &str) -> Result<LiveLinkStart>;
    /// The person's answer to "Check your desktop shows the same code.":
    /// Yes acknowledges the desktop's offer (once it comes); No ends the
    /// request (nothing is granted).
    async fn link_live_code(&self, rid: &str, matches: bool) -> Result<()>;
    /// Link as the person the workspace log verified (`true`), or link
    /// nothing (`false`, `None`).
    async fn link_live_person(&self, rid: &str, link: bool) -> Result<Option<Linked>>;
    /// End a live request from this computer (nothing is granted).
    async fn link_live_cancel(&self, rid: &str) -> Result<()>;
    /// The live approval requests in the workspace's approval mailbox, read
    /// only: they are approved on a desktop or a phone.
    async fn approvals(&self, workspace: &WorkspaceId) -> Result<Vec<ApprovalRequest>>;
    /// The person this computer is a device of, once linked.
    fn linked_person(&self) -> Option<String>;
    /// Where linking stands: linked, or a request waiting for the
    /// desktop's code (expired requests are dropped as it reads them).
    async fn link_status(&self) -> Result<LinkStatusView>;
    /// Name this computer, as every member sees it, in every workspace it
    /// acts in (identity model v2, 6): its synced device name. Empty clears
    /// it. Returns the name as shown.
    async fn rename_this_device(&self, name: &str) -> Result<String>;
    /// The person confirmed they wrote the recovery words down.
    async fn confirm_recovery_saved(&self, workspace: &WorkspaceId) -> Result<()>;

    // ── Workspaces and chats ───────────────────────────────────────────
    async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>>;
    async fn create_invite(&self, workspace: &WorkspaceId, role: Role) -> Result<String>;
    /// Read a box's `ligusjoin1:` request (no passkey, no write): its
    /// fingerprint, to compare with the box's terminal before admitting it.
    async fn inspect_join_request(&self, join_request: &str) -> Result<JoinRequestCheck>;
    /// Admit a box as a service member (Admin, with a passkey): seal to its
    /// `ligusjoin1:` request, or (none) a bearer code for a Secret or file.
    async fn admit_box(&self, workspace: &WorkspaceId, join_request: Option<&str>) -> Result<BoxAdmission>;
    async fn chats(&self, workspace: &WorkspaceId) -> Result<Vec<ChatSummary>>;
    async fn create_chat(&self, workspace: &WorkspaceId, title: &str) -> Result<ChatSummary>;
    async fn transcript(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<Vec<Message>>;
    /// Post a message; agent replies arrive as events.
    async fn send_message(&self, workspace: &WorkspaceId, chat: &ChatId, text: &str) -> Result<Message>;
    async fn mentionables(&self, workspace: &WorkspaceId) -> Result<Vec<Mentionable>>;
    async fn mark_read(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()>;
    /// Any member may lock a chat down.
    /// Stop the agents' turns running on this device in a chat; how many
    /// were stopped (0: none runs here, so its host has to stop it).
    async fn stop_turn(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<usize>;
    /// The decision cards to show in a chat, oldest first.
    async fn asks(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<Vec<Ask>>;
    /// Answer an ask on the log of `chat` (the ask's own chat). A grant
    /// that needs a passkey asks for one ([`Event::PresencePrompt`]).
    async fn answer_ask(&self, workspace: &WorkspaceId, chat: &ChatId, ask: &AskId, reply: AskReply) -> Result<()>;
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
    /// Archive a settled proposal for everyone, or restore an archived one
    /// (a Contributor's; one that still waits on someone is refused).
    async fn archive_proposal(&self, workspace: &WorkspaceId, proposal: &ProposalId, archived: bool) -> Result<Proposal>;
    /// Hide a proposal on this device only ("Hide for me"), or show it
    /// again. Never synced.
    async fn hide_proposal(&self, workspace: &WorkspaceId, proposal: &ProposalId, hidden: bool) -> Result<Proposal>;

    // ── Agents, runtimes, hosts ────────────────────────────────────────
    async fn agents(&self, workspace: &WorkspaceId) -> Result<Vec<Agent>>;
    async fn runtimes(&self, workspace: &WorkspaceId) -> Result<Vec<Runtime>>;
    async fn hosts(&self, workspace: &WorkspaceId) -> Result<Vec<Host>>;
    /// What this device could run agents on (CLIs on `PATH`, a local model
    /// server, API keys in the environment), each with the workspace agents
    /// already using it. Never prompts for a keychain, never reads a secret's
    /// value and never sends a prompt: version checks only.
    async fn local_runtimes(&self, workspace: &WorkspaceId) -> Result<Vec<LocalRuntime>>;
    /// Add an agent named `name` on a [`LocalRuntime`]: yours, "Only me",
    /// run on this device (the desktop's add-agent path, one signed roster
    /// event). Returns its handle.
    async fn add_local_agent(&self, workspace: &WorkspaceId, runtime: &RuntimeId, name: &str) -> Result<String>;
    async fn set_agent_runtime(&self, workspace: &WorkspaceId, agent: &AgentId, runtime: &RuntimeId) -> Result<()>;
    /// Who may use an agent: only its owner, or everyone (agent access
    /// controls; its owner's to change, an Admin's on a shared agent).
    async fn set_agent_access(&self, workspace: &WorkspaceId, agent: &AgentId, everyone: bool) -> Result<()>;
    /// Recent hand-offs between agents, newest first.
    async fn handoffs(&self, workspace: &WorkspaceId) -> Result<Vec<Handoff>>;
    /// The git checkout this device applies proposals in, for the workspace.
    async fn checkout(&self, workspace: &WorkspaceId) -> Result<Option<String>>;
    /// Bind (or with `None`, unbind) that checkout.
    async fn set_checkout(&self, workspace: &WorkspaceId, path: Option<&str>) -> Result<Option<String>>;

    // ── A chat's repository ────────────────────────────────────────────
    /// What a chat's repository can be: the workspace's repositories, and
    /// whether this device may make a new scratch repo.
    async fn repo_choices(&self, workspace: &WorkspaceId) -> Result<RepoChoices>;
    /// Set the chat's repository (`None`: no repository).
    async fn set_chat_repo(&self, workspace: &WorkspaceId, chat: &ChatId, repo: Option<&str>) -> Result<()>;
    /// Make a scratch repo on this device and set it as the chat's: an
    /// empty repository (one empty commit on `main`) under this device's
    /// scratch folder, added to the workspace as scratch (an Admin's) and
    /// checked out here, named after the chat.
    async fn new_scratch_repo(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<ChatRepo>;
    /// Keep the chat's scratch repo as an ordinary repository: move it to
    /// `dest` (a new folder; `~` allowed), check it out there, and drop the
    /// scratch tag. Returns where it is now.
    async fn keep_scratch_repo(&self, workspace: &WorkspaceId, chat: &ChatId, dest: &str) -> Result<String>;

    // ── Workflows ──────────────────────────────────────────────────────
    async fn workflows(&self, workspace: &WorkspaceId) -> Result<Vec<Workflow>>;
    /// Runs, newest first.
    async fn runs(&self, workspace: &WorkspaceId) -> Result<Vec<Run>>;
    /// Start a run; this device drives it (stages go to their agents' hosts).
    async fn start_run(&self, workspace: &WorkspaceId, workflow: &WorkflowId, input: &str) -> Result<Run>;
    async fn cancel_run(&self, workspace: &WorkspaceId, run: &RunId) -> Result<Run>;
    /// Approve or reject a run's gate (a vote on its proposal).
    async fn decide_gate(&self, workspace: &WorkspaceId, run: &RunId, stage: &str, approve: bool) -> Result<Run>;

    // ── Memory and task state ──────────────────────────────────────────
    async fn memory(&self, workspace: &WorkspaceId) -> Result<Memory>;
    /// Agents' task state across the workspace's chats, newest first.
    async fn task_state(&self, workspace: &WorkspaceId) -> Result<Vec<TaskState>>;

    // ── People ─────────────────────────────────────────────────────────
    async fn members(&self, workspace: &WorkspaceId) -> Result<Vec<Member>>;
    async fn set_role(&self, workspace: &WorkspaceId, member: &MemberId, role: Role) -> Result<()>;
    async fn remove_member(&self, workspace: &WorkspaceId, member: &MemberId) -> Result<()>;
    /// What leaving `workspace` would do; nothing is written.
    async fn leave_check(&self, workspace: &WorkspaceId) -> Result<LeaveView>;
    /// Leave `workspace` on your own: others see you as asked to leave
    /// until an Admin removes you, then lt stops syncing it and forgets it
    /// here. Revokes nothing. A refused leave changes nothing.
    async fn leave_workspace(&self, workspace: &WorkspaceId) -> Result<()>;

    // ── Notifications and settings ─────────────────────────────────────
    async fn notifications(&self) -> Result<Vec<Notification>>;
    async fn mark_notifications_read(&self) -> Result<()>;
    async fn settings(&self) -> Result<Settings>;
    async fn set_setting(&self, key: SettingKey, on: bool) -> Result<Settings>;

    // ── Presence ───────────────────────────────────────────────────────
    /// Answer a [`Event::PresencePrompt`].
    async fn answer_presence(&self, id: PresenceId, answer: PresenceAnswer) -> Result<()>;
    /// The passkey hand-offs left waiting in this data dir (lt quit while
    /// one waited for the person's desktop or phone).
    async fn pending_presence(&self) -> Result<Vec<PendingPresence>> {
        Ok(vec![])
    }
    /// Wait for one again: a hand-off sheet ([`Event::PresencePrompt`] with
    /// `resumed`), then, once confirmed, write it. The summary, when
    /// written.
    async fn resume_presence(&self, _rid: &str) -> Result<String> {
        err("nothing is waiting here")
    }
    /// End one (the desktop or phone drops it; nothing is written).
    async fn cancel_pending_presence(&self, _rid: &str) -> Result<()> {
        Ok(())
    }

    // ── Guided setup (`ligus_runtime::setup`) ──────────────────────────
    /// The Set up list for `workspace` (or before there is one): each flow
    /// and what is done (cheap: no network, no probes).
    async fn setup_status(&self, workspace: Option<&WorkspaceId>) -> Result<Vec<FlowRow>>;
    /// A flow's form for these values so far (a harness's models fill in
    /// once it is chosen).
    async fn setup_form(&self, workspace: Option<&WorkspaceId>, flow: LtFlow, inputs: SetupInputs) -> Result<SetupForm>;
    /// Run a flow as a person. `cancel` stops it between steps (the engine
    /// undoes its own writes). Secrets go to the engine and nowhere else.
    async fn setup_run(
        &self,
        workspace: Option<&WorkspaceId>,
        flow: LtFlow,
        inputs: SetupInputs,
        secrets: SetupSecrets,
        dry_run: bool,
        cancel: CancelFlag,
    ) -> Result<SetupOutcome>;
    /// Whether this backend links this computer to the person's devices
    /// (the `device` flow, Add to my devices).
    fn links_devices(&self) -> bool {
        true
    }
}
