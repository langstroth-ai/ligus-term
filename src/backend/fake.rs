//! An in-memory [`Backend`] for building and testing the UI before `lt` is
//! wired to the Ligus crates.
//!
//! It behaves like the real thing where the UI can tell: agents stream
//! their replies word by word, governance actions wait on a passkey prompt,
//! refusals come back with the rank-rule wording the crates use. It keeps
//! nothing on disk, signs nothing and syncs nothing.
//!
//! The rank rules here are a stand-in for `authz::member_action_refusal` in
//! the crates; the real backend never decides anything itself.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio::sync::{broadcast, mpsc};

use super::model::*;
use super::{Backend, Event, Result, err, presence};

/// 2026-11-18 09:00 UTC: the demo's "now", so renders are stable.
pub const DEMO_NOW: Millis = 1_794_992_400_000;
const MIN: Millis = 60_000;

#[derive(Debug, Clone)]
pub struct FakeConfig {
    /// Pause between streamed words (zero in tests).
    pub stream_delay: Duration,
    /// How long the simulated browser takes to answer a passkey prompt.
    pub browser_delay: Duration,
    pub device_name: String,
    pub data_dir: String,
}

impl Default for FakeConfig {
    fn default() -> Self {
        Self {
            stream_delay: Duration::from_millis(60),
            browser_delay: Duration::from_millis(1200),
            device_name: "lt on this terminal".into(),
            data_dir: "(in memory)".into(),
        }
    }
}

impl FakeConfig {
    pub fn instant() -> Self {
        Self { stream_delay: Duration::ZERO, browser_delay: Duration::ZERO, ..Self::default() }
    }
}

#[derive(Default)]
struct Ws {
    summary: Option<WorkspaceSummary>,
    chats: Vec<ChatSummary>,
    messages: HashMap<ChatId, Vec<Message>>,
    proposals: Vec<Proposal>,
    diffs: HashMap<ProposalId, Diff>,
    members: Vec<MemberRec>,
    agents: Vec<Agent>,
    runtimes: Vec<Runtime>,
    hosts: Vec<Host>,
    workflows: Vec<Workflow>,
    runs: Vec<Run>,
    memory: Memory,
    tasks: Vec<TaskState>,
    handoffs: Vec<Handoff>,
    checkout: Option<String>,
    /// Set up: the relay it syncs through, tools (MCP servers) and repos.
    relay: Option<String>,
    tools: Vec<ligus_runtime::setup::McpFact>,
    repos: Vec<ligus_runtime::setup::RepoFact>,
}

#[derive(Clone)]
struct MemberRec {
    id: MemberId,
    name: String,
    role: Role,
    founder: bool,
    service: bool,
}

struct World {
    your_name: Option<String>,
    you: MemberId,
    order: Vec<WorkspaceId>,
    ws: HashMap<WorkspaceId, Ws>,
    notifications: Vec<Notification>,
    settings: Settings,
    seq: u64,
    clock: Millis,
    presence: HashMap<PresenceId, mpsc::UnboundedSender<PresenceAnswer>>,
    /// What this "device" has installed (its `used_by` filled per workspace).
    local: Vec<LocalRuntime>,
    /// Replies streaming now, per chat, and the chats asked to stop.
    replying: HashMap<ChatId, usize>,
    stopping: std::collections::HashSet<ChatId>,
}

impl World {
    fn next(&mut self, prefix: &str) -> String {
        self.seq += 1;
        format!("{prefix}{}", self.seq)
    }
    fn tick(&mut self) -> Millis {
        self.clock += MIN;
        self.clock
    }
    fn ws(&mut self, id: &WorkspaceId) -> Result<&mut Ws> {
        match self.ws.get_mut(id) {
            Some(w) => Ok(w),
            None => err("no such workspace"),
        }
    }
}

#[derive(Clone)]
pub struct FakeBackend {
    world: Arc<Mutex<World>>,
    tx: broadcast::Sender<Event>,
    cfg: FakeConfig,
}

impl FakeBackend {
    /// A device that has never run: first-run onboarding.
    pub fn new(cfg: FakeConfig) -> Self {
        let (tx, _) = broadcast::channel(4096);
        let world = World {
            your_name: None,
            you: MemberId::new("m-you"),
            order: vec![],
            ws: HashMap::new(),
            notifications: vec![],
            settings: Settings {
                your_name: String::new(),
                device_name: cfg.device_name.clone(),
                notify_mentions: true,
                agents_may_edit: false,
                relay_url: None,
            },
            seq: 100,
            clock: DEMO_NOW,
            presence: HashMap::new(),
            local: demo_local(),
            replying: HashMap::new(),
            stopping: Default::default(),
        };
        Self { world: Arc::new(Mutex::new(world)), tx, cfg }
    }

    /// The same device with `local` as what is installed on it (empty:
    /// nothing found).
    pub fn with_local(self, local: Vec<LocalRuntime>) -> Self {
        self.lock().local = local;
        self
    }

    /// A device with two workspaces, chats, proposals and people.
    pub fn demo(cfg: FakeConfig) -> Self {
        let b = Self::new(cfg);
        {
            let mut w = b.world.lock().unwrap();
            w.your_name = Some("Mara".into());
            w.settings.your_name = "Mara".into();
            seed_northwind(&mut w);
            seed_docs(&mut w);
            for ws in w.ws.values_mut() {
                ws.relay = Some(ligus_core::COMMUNITY_RELAY_URL.to_string());
            }
            w.notifications.push(Notification {
                id: NotificationId::new("n1"),
                workspace: WorkspaceId::new("ws-northwind"),
                chat: Some(ChatId::new("c-checkout")),
                text: "Ada mentioned you in Fix flaky checkout test".into(),
                at: DEMO_NOW - 12 * MIN,
                read: false,
            });
        }
        b
    }

    /// The demo with `bad` in every name, id and label that comes from
    /// another member, an agent or a box: for tests that nothing printed
    /// from the network can forge a record or reach the terminal raw.
    pub fn hostile(cfg: FakeConfig, bad: &str) -> Self {
        let b = Self::demo(cfg);
        {
            let mut w = b.world.lock().unwrap();
            let bad = bad.to_string();
            for ws in w.ws.values_mut() {
                for a in &mut ws.agents {
                    a.handle = bad.clone();
                    a.runtime = RuntimeId::new(bad.clone());
                    a.host = Some(HostId::new(bad.clone()));
                }
                for h in &mut ws.hosts {
                    h.id = HostId::new(bad.clone());
                    h.agents = vec![bad.clone()];
                    h.repos = vec![bad.clone()];
                }
                for r in &mut ws.runtimes {
                    r.id = RuntimeId::new(bad.clone());
                    r.model = Some(bad.clone());
                }
                for t in &mut ws.tasks {
                    t.agent = bad.clone();
                    t.status = bad.clone();
                }
                for h in &mut ws.handoffs {
                    h.from = bad.clone();
                    h.to = bad.clone();
                }
                for d in &mut ws.workflows {
                    for st in &mut d.stages {
                        st.id = bad.clone();
                    }
                }
                for r in &mut ws.runs {
                    for st in &mut r.stages {
                        st.id = bad.clone();
                    }
                }
            }
        }
        b
    }

    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
    }

    /// A gate's vote settled (or not): fold it into its run, which the run's
    /// driver then carries on.
    fn settle_gate(&self, workspace: &WorkspaceId, p: &Proposal) {
        let outcome = match p.status {
            ProposalStatus::Approved | ProposalStatus::Applied => StageStatus::Succeeded,
            ProposalStatus::Rejected => StageStatus::Rejected,
            ProposalStatus::Open => return,
        };
        let changed = {
            let mut w = self.lock();
            let at = w.tick();
            let Ok(ws) = w.ws(workspace) else { return };
            let Some(r) = ws.runs.iter_mut().find(|r| r.stages.iter().any(|st| st.gate.as_ref() == Some(&p.id))) else { return };
            for st in r.stages.iter_mut().filter(|st| st.gate.as_ref() == Some(&p.id)) {
                st.status = outcome;
            }
            if outcome == StageStatus::Rejected {
                for st in r.stages.iter_mut().filter(|st| st.status == StageStatus::Pending) {
                    st.status = StageStatus::Skipped;
                }
                r.status = RunStatus::Halted;
            } else if r.stages.iter().all(|st| st.status == StageStatus::Succeeded) {
                r.status = RunStatus::Completed;
            } else {
                r.status = RunStatus::Running;
            }
            r.updated_at = at;
            r.clone()
        };
        self.emit(Event::RunChanged { workspace: workspace.clone(), run: changed });
    }

    /// Drive a run the demo's way: stages in order, each agent stage after a
    /// pause, each gate waiting for a vote.
    fn spawn_run(&self, workspace: WorkspaceId, run: RunId) {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(me.cfg.stream_delay * 6 + Duration::from_millis(5)).await;
                let next = {
                    let mut w = me.lock();
                    let at = w.tick();
                    let gate_id = ProposalId::new(w.next("p-gate-"));
                    let Ok(ws) = w.ws(&workspace) else { return };
                    let Some(r) = ws.runs.iter_mut().find(|r| r.id == run) else { return };
                    if r.status.is_terminal() {
                        return;
                    }
                    if r.stages.iter().any(|st| st.status == StageStatus::AwaitingApproval) {
                        continue;
                    }
                    let Some(i) = r.stages.iter().position(|st| st.status != StageStatus::Succeeded) else {
                        r.status = RunStatus::Completed;
                        r.updated_at = at;
                        let done = r.clone();
                        drop(w);
                        me.emit(Event::RunChanged { workspace: workspace.clone(), run: done });
                        return;
                    };
                    let mut gate = None;
                    let st = &mut r.stages[i];
                    if st.what.starts_with("gate") {
                        st.status = StageStatus::AwaitingApproval;
                        st.gate = Some(gate_id.clone());
                        r.status = RunStatus::AwaitingGate;
                        gate = Some(Proposal {
                            id: gate_id.clone(),
                            chat: Some(r.chat.clone()),
                            kind: ProposalKind::Gate,
                            title: format!("{}: {}", st.name, r.input),
                            author: "workflow".into(),
                            status: ProposalStatus::Open,
                            files_changed: 0,
                            approvals: 0,
                            quorum: 1,
                            votes: vec![],
                            created_at: at,
                            needs_presence: false,
                            apply: ApplyState::NotRequested,
                            can_vote: Allowed::Yes,
                        });
                    } else if st.status == StageStatus::Running {
                        st.status = StageStatus::Succeeded;
                        st.excerpt = format!("{} finished: {}", st.what, r.input);
                    } else {
                        st.status = StageStatus::Running;
                        st.attempts += 1;
                    }
                    r.updated_at = at;
                    let snapshot = r.clone();
                    if let Some(g) = &gate {
                        ws.proposals.insert(0, g.clone());
                        ws.diffs.insert(gate_id, Diff { files: vec![] });
                    }
                    (snapshot, gate)
                };
                if let Some(g) = next.1 {
                    me.emit(Event::ProposalChanged { workspace: workspace.clone(), proposal: g });
                }
                me.emit(Event::RunChanged { workspace: workspace.clone(), run: next.0 });
            }
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ask for a passkey and wait for the person's answer.
    async fn presence(&self, kind: &str, summary: String, op: PresenceOp) -> Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = {
            let mut w = self.lock();
            w.seq += 1;
            let id = PresenceId(w.seq);
            w.presence.insert(id, tx);
            id
        };
        let digest_short = format!("{:016x}", fnv(&format!("{kind}{summary}{}", id.0)));
        self.emit(Event::PresencePrompt(PresencePrompt { id, kind: kind.into(), summary, digest_short, op }));
        let outcome = loop {
            match rx.recv().await {
                None | Some(PresenceAnswer::Cancel) => break err("cancelled"),
                Some(PresenceAnswer::RecoveryWordsFile(path)) => {
                    if op == PresenceOp::Create {
                        break err("a recovery key doesn't create a passkey");
                    }
                    match presence::read_recovery_words_file(std::path::Path::new(&path)) {
                        Ok(_) => break Ok(()),
                        // Stay on the sheet so the person can fix the path.
                        Err(e) => self.emit(Event::PresenceProblem { id, error: e }),
                    }
                }
                Some(PresenceAnswer::Browser) => {
                    // The simulated browser answers after a pause, unless the
                    // person cancels or switches to the recovery words first.
                    tokio::select! {
                        _ = tokio::time::sleep(self.cfg.browser_delay) => break Ok(()),
                        a = rx.recv() => match a {
                            Some(PresenceAnswer::RecoveryWordsFile(path)) if op == PresenceOp::Get => {
                                match presence::read_recovery_words_file(std::path::Path::new(&path)) {
                                    Ok(_) => break Ok(()),
                                    // Back to waiting for an answer.
                                    Err(e) => self.emit(Event::PresenceProblem { id, error: e }),
                                }
                            }
                            Some(PresenceAnswer::Browser) => {}
                            _ => break err("cancelled"),
                        }
                    }
                }
            }
        };
        self.lock().presence.remove(&id);
        self.emit(Event::PresenceDone { id, ok: outcome.is_ok(), error: outcome.as_ref().err().map(|e| e.0.clone()) });
        outcome
    }

    fn your_role(&self, w: &mut World, ws: &WorkspaceId) -> Result<Role> {
        let you = w.you.clone();
        let wsd = w.ws(ws)?;
        Ok(wsd.members.iter().find(|m| m.id == you).map(|m| m.role).unwrap_or(Role::Viewer))
    }

    fn member_view(&self, w: &World, ws: &Ws) -> Vec<Member> {
        let me = ws.members.iter().find(|m| m.id == w.you).cloned();
        ws.members
            .iter()
            .map(|m| Member {
                id: m.id.clone(),
                name: m.name.clone(),
                role: m.role,
                founder: m.founder,
                service: m.service,
                you: m.id == w.you,
                can_remove: refusal(me.as_ref(), m, None),
                can_set_role: Role::ALL.iter().map(|&r| (r, refusal(me.as_ref(), m, Some(r)))).collect(),
            })
            .collect()
    }

    /// An agent's reply, streamed word by word, maybe filing a proposal.
    fn spawn_reply(&self, ws: WorkspaceId, chat: ChatId, agent: Agent, prompt: String) {
        let me = self.clone();
        tokio::spawn(async move {
            let who = format!("@{}", agent.handle);
            me.emit(Event::Thinking { workspace: ws.clone(), chat: chat.clone(), agent: who.clone(), on: true });
            *me.lock().replying.entry(chat.clone()).or_default() += 1;
            tokio::time::sleep(me.cfg.stream_delay * 4).await;
            let (msg, files_proposal, your_name) = {
                let mut w = me.lock();
                let id = MessageId::new(w.next("msg-"));
                let at = w.tick();
                let m = Message {
                    id,
                    chat: chat.clone(),
                    author: Author::Agent { id: agent.id.clone(), handle: agent.handle.clone(), runtime: agent.runtime.0.clone() },
                    text: String::new(),
                    at,
                    streaming: true,
                    tools: vec![],
                };
                if let Ok(wsd) = w.ws(&ws) {
                    wsd.messages.entry(chat.clone()).or_default().push(m.clone());
                }
                let lower = prompt.to_lowercase();
                let files = ["fix", "change", "add", "propose", "refactor"].iter().any(|k| lower.contains(k));
                (m, files, w.your_name.clone().unwrap_or_else(|| "you".into()))
            };
            me.emit(Event::Thinking { workspace: ws.clone(), chat: chat.clone(), agent: who, on: false });
            me.emit(Event::MessageAdded { workspace: ws.clone(), message: msg.clone() });
            let call = ToolCall {
                name: "read_file".into(),
                preview: "src/checkout.rs".into(),
                ok: true,
                duration_ms: Some(180),
                output: "pub struct Checkout {\n    pub items: Vec<LineItem>,\n    pub started: Instant,".into(),
            };
            me.emit(Event::ToolActivity { workspace: ws.clone(), chat: chat.clone(), message: msg.id.clone(), call: call.clone() });
            let reply = if files_proposal {
                format!(
                    "Looked at the checkout test. I injected a fake clock into the session check and added a test.\n\nFiled it as a proposal for review, @{your_name}."
                )
            } else {
                "Here's what I found: the checkout test reads the wall clock, so it fails when CI is slow. Want me to propose a change?"
                    .to_string()
            };
            for word in reply.split_inclusive(' ') {
                tokio::time::sleep(me.cfg.stream_delay).await;
                if me.lock().stopping.remove(&chat) {
                    break;
                }
                {
                    let mut w = me.lock();
                    if let Ok(wsd) = w.ws(&ws)
                        && let Some(m) = wsd.messages.get_mut(&chat).and_then(|v| v.iter_mut().find(|m| m.id == msg.id))
                    {
                        m.text.push_str(word);
                    }
                }
                me.emit(Event::MessageDelta { workspace: ws.clone(), chat: chat.clone(), message: msg.id.clone(), delta: word.into() });
            }
            {
                let mut w = me.lock();
                if let Ok(wsd) = w.ws(&ws)
                    && let Some(m) = wsd.messages.get_mut(&chat).and_then(|v| v.iter_mut().find(|m| m.id == msg.id))
                {
                    m.streaming = false;
                    m.tools.push(call);
                }
                if let Some(n) = w.replying.get_mut(&chat) {
                    *n = n.saturating_sub(1);
                }
            }
            me.emit(Event::MessageDone { workspace: ws.clone(), chat: chat.clone(), message: msg.id.clone(), error: None });
            if files_proposal {
                let p = {
                    let mut w = me.lock();
                    let id = ProposalId::new(w.next("p-"));
                    let at = w.clock;
                    let p = Proposal {
                        id: id.clone(),
                        chat: Some(chat.clone()),
                        title: "Inject a clock into the session check".into(),
                        author: format!("@{}", agent.handle),
                        kind: ProposalKind::Change,
                        status: ProposalStatus::Open,
                        files_changed: 1,
                        approvals: 0,
                        quorum: 1,
                        votes: vec![],
                        created_at: at,
                        needs_presence: false,
                        apply: ApplyState::NotRequested,
                        can_vote: Allowed::Yes,
                    };
                    if let Ok(wsd) = w.ws(&ws) {
                        wsd.proposals.insert(0, p.clone());
                        wsd.diffs.insert(id, clock_diff());
                    }
                    let nid = NotificationId::new(w.next("n-"));
                    let n = Notification {
                        id: nid,
                        workspace: ws.clone(),
                        chat: Some(chat.clone()),
                        text: format!("@{} mentioned you", agent.handle),
                        at,
                        read: false,
                    };
                    w.notifications.insert(0, n.clone());
                    me.emit(Event::Notification(n));
                    p
                };
                me.emit(Event::ProposalChanged { workspace: ws.clone(), proposal: p });
            }
        });
    }
}

/// The rank rules, with the crates' wording (`authz::member_action_refusal`).
/// `role` is `None` for a removal.
fn refusal(me: Option<&MemberRec>, target: &MemberRec, role: Option<Role>) -> Allowed {
    let no = |s: &str| Allowed::No(s.to_string());
    let Some(me) = me else { return no("you aren't a member of this workspace") };
    let own = me.id == target.id;
    match role {
        None => {
            if own {
                return Allowed::Yes; // leaving is always allowed
            }
            if me.service {
                return no("a service member cuts no one");
            }
            if target.founder {
                return no("nobody cuts the workspace's founder");
            }
            if target.role == Role::Owner && !me.founder {
                return no("only the founder cuts an Owner");
            }
            if me.role < Role::Admin || target.role >= me.role {
                return no("a cut goes strictly down the ranks");
            }
            Allowed::Yes
        }
        Some(r) => {
            if r == target.role {
                return no("already that role");
            }
            if me.service {
                return no("a service member changes no one's role");
            }
            if target.service && r > Role::Contributor {
                return no("a service member holds at most Contributor");
            }
            if own && r < target.role {
                return Allowed::Yes; // demoting yourself is always allowed
            }
            if r > me.role {
                return no("no one grants a role above their own");
            }
            if target.role > me.role {
                return no("no one changes the role of a member above them");
            }
            if r < target.role {
                if target.founder {
                    return no("nobody cuts the workspace's founder");
                }
                if target.role == Role::Owner && !me.founder {
                    return no("only the founder cuts an Owner");
                }
                if target.role >= me.role {
                    return no("a cut goes strictly down the ranks");
                }
            }
            if me.role < Role::Admin {
                return no("no one grants a role above their own");
            }
            Allowed::Yes
        }
    }
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// 24 words, deterministic per workspace (a demo, not a key).
fn demo_words(seed: &str) -> Vec<String> {
    const W: [&str; 32] = [
        "amber", "brook", "coral", "dune", "ember", "fern", "glade", "hazel", "ivory", "juniper", "keel", "linden", "meadow", "north",
        "orchard", "pebble", "quartz", "river", "slate", "thyme", "umber", "valley", "walnut", "yarrow", "zephyr", "clover", "basket",
        "lantern", "harbor", "willow", "cedar", "maple",
    ];
    let mut h = fnv(seed);
    (0..24)
        .map(|_| {
            h = h.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            W[(h >> 59) as usize].to_string()
        })
        .collect()
}

fn agent(id: &str, handle: &str, runtime: &str, host: Option<&str>, role: Option<&str>) -> Agent {
    Agent {
        id: AgentId::new(id),
        handle: handle.into(),
        runtime: RuntimeId::new(runtime),
        role: role.map(Into::into),
        host: host.map(HostId::new),
        shared: true,
        status: AgentStatus::Idle,
        doing: None,
        access: "everyone".into(),
        unavailable: None,
        can_manage: false,
        usage: None,
    }
}

fn runtime(id: &str, kind: RuntimeKind, model: Option<&str>, configured: bool, active: bool) -> Runtime {
    Runtime { id: RuntimeId::new(id), label: kind.label().into(), kind, model: model.map(Into::into), configured, active }
}

/// What the demo device has installed: Claude Code and Codex (signed in),
/// the Copilot CLI (not signed in) and Ollama with one model.
fn demo_local() -> Vec<LocalRuntime> {
    let l = |id: &str, kind, model: Option<&str>, version: Option<&str>, ready, detail: &str, name: &str| LocalRuntime {
        id: RuntimeId::new(id),
        kind,
        model: model.map(Into::into),
        version: version.map(Into::into),
        ready,
        detail: detail.into(),
        used_by: vec![],
        unsupported: None,
        suggested_name: name.into(),
    };
    vec![
        l("local:claude-code", RuntimeKind::ClaudeCode, None, Some("2.1.3"), true, "signed in (claude login)", "claude"),
        l("local:codex", RuntimeKind::Codex, None, Some("0.98.0"), true, "signed in (codex login)", "codex"),
        l(
            "local:copilot",
            RuntimeKind::Copilot,
            None,
            Some("1.0.106"),
            false,
            "no Copilot CLI login found: run `copilot login` in a terminal",
            "copilot",
        ),
        l("local:ollama/qwen3:8b", RuntimeKind::Ollama, Some("qwen3:8b"), None, true, "Ollama at 127.0.0.1:11434", "qwen3"),
    ]
}

/// Every agent's handle and member's name in `ws`.
fn taken_names(ws: &Ws) -> Vec<String> {
    ws.agents.iter().map(|a| a.handle.clone()).chain(ws.members.iter().map(|m| m.name.clone())).collect()
}

fn member(id: &str, name: &str, role: Role, founder: bool, service: bool) -> MemberRec {
    MemberRec { id: MemberId::new(id), name: name.into(), role, founder, service }
}

fn msg(id: &str, chat: &str, author: Author, text: &str, at: Millis) -> Message {
    Message { id: MessageId::new(id), chat: ChatId::new(chat), author, text: text.into(), at, streaming: false, tools: vec![] }
}

fn human(id: &str, name: &str) -> Author {
    Author::Member { id: MemberId::new(id), name: name.into() }
}

fn bot(handle: &str, runtime: &str) -> Author {
    Author::Agent { id: AgentId::new(format!("a-{handle}")), handle: handle.into(), runtime: runtime.into() }
}

fn chat(id: &str, title: &str, channel: Option<&str>, at: Millis, unread: bool, mentioned: bool) -> ChatSummary {
    ChatSummary {
        id: ChatId::new(id),
        title: title.into(),
        channel: channel.map(Into::into),
        updated_at: at,
        unread,
        mentioned,
        lockdown: None,
    }
}

fn line_ctx(s: &str) -> DiffLine {
    DiffLine::Context(s.into())
}
fn line_add(s: &str) -> DiffLine {
    DiffLine::Added(s.into())
}
fn line_del(s: &str) -> DiffLine {
    DiffLine::Removed(s.into())
}

fn clock_diff() -> Diff {
    Diff {
        files: vec![
            FileDiff {
                path: "src/checkout.rs".into(),
                added: 9,
                removed: 3,
                hunks: vec![
                    Hunk {
                        header: "@@ -12,7 +12,9 @@ pub struct Checkout {".into(),
                        lines: vec![
                            line_ctx("    pub cart: CartId,"),
                            line_ctx("    pub items: Vec<LineItem>,"),
                            line_del("    pub started: Instant,"),
                            line_add("    /// Injected, so tests set the time instead of reading the wall clock."),
                            line_add("    pub clock: Arc<dyn Clock>,"),
                            line_add("    pub started_at: Millis,"),
                            line_ctx("    pub paid: bool,"),
                            line_ctx("}"),
                        ],
                    },
                    Hunk {
                        header: "@@ -40,9 +42,13 @@ impl Checkout {".into(),
                        lines: vec![
                            line_ctx("    pub fn is_expired(&self) -> bool {"),
                            line_del("        self.started.elapsed().as_secs() > 30"),
                            line_del("    }"),
                            line_add("        // 30 seconds matches the payment provider's session timeout."),
                            line_add("        self.clock.now() - self.started_at >= Self::SESSION_TIMEOUT_MS"),
                            line_add("    }"),
                            line_add(""),
                            line_add("    pub const SESSION_TIMEOUT_MS: Millis = 30_000;"),
                            line_ctx(""),
                            line_ctx("    pub fn summary(&self) -> String {"),
                        ],
                    },
                ],
            },
            FileDiff {
                path: "tests/checkout.rs".into(),
                added: 8,
                removed: 0,
                hunks: vec![Hunk {
                    header: "@@ -0,0 +1,8 @@".into(),
                    lines: vec![
                        line_add("use northwind::Checkout;"),
                        line_add(""),
                        line_add("#[test]"),
                        line_add("fn expires_after_thirty_seconds() {"),
                        line_add("    let c = Checkout::at_fake_time(30_000);"),
                        line_add("    assert!(c.is_expired());"),
                        line_add("}"),
                        line_add(""),
                    ],
                }],
            },
        ],
    }
}

fn seed_northwind(w: &mut World) {
    let id = WorkspaceId::new("ws-northwind");
    let mut ws = Ws {
        summary: Some(WorkspaceSummary {
            id: id.clone(),
            name: "Northwind API".into(),
            mentioned: false,
            sync: SyncMode::Peer,
            your_role: Role::Owner,
            can_invite: Allowed::Yes,
        }),
        ..Ws::default()
    };
    ws.members = vec![
        member("m-you", "Mara", Role::Owner, true, false),
        member("m-ada", "Ada", Role::Admin, false, false),
        member("m-lin", "Lin", Role::Contributor, false, false),
        member("m-sam", "Sam", Role::Viewer, false, false),
        member("m-sff", "sff box", Role::Contributor, false, true),
    ];
    ws.runtimes = vec![
        runtime("claude-code", RuntimeKind::ClaudeCode, Some("default"), true, true),
        runtime("codex", RuntimeKind::Codex, None, true, false),
        runtime("ollama", RuntimeKind::Ollama, Some("qwen3-coder:30b"), true, false),
        runtime("copilot", RuntimeKind::Copilot, None, false, false),
    ];
    ws.agents = vec![
        agent("a-ligus", "ligus", "claude-code", None, Some("answers untagged messages")),
        Agent {
            // Yours, "Only me", with this device's counts.
            access: "owner".into(),
            shared: false,
            can_manage: true,
            usage: Some(AgentUsage {
                day: "2026-11-20".into(),
                today: vec![UsageRow { member: "Mara".into(), turns: 3, uncosted: 0, cost_micro_usd: Some(12_000) }],
                month: vec![
                    UsageRow { member: "Mara".into(), turns: 41, uncosted: 0, cost_micro_usd: Some(180_000) },
                    UsageRow { member: "Ben".into(), turns: 2, uncosted: 2, cost_micro_usd: None },
                ],
            }),
            ..agent("a-coder", "coder", "claude-code", Some("h-sff"), Some("implements proposals"))
        },
        agent("a-reviewer", "reviewer", "codex", None, Some("reviews diffs")),
        agent("a-scout", "scout", "ollama", Some("h-sff"), None),
    ];
    ws.hosts = vec![
        Host {
            id: HostId::new("h-this"),
            label: "This device".into(),
            kind: HostKind::Device,
            online: true,
            last_seen: Some(DEMO_NOW),
            this_device: true,
            busy: false,
            agents: vec![],
            repos: vec![],
        },
        Host {
            id: HostId::new("h-sff"),
            label: "sff".into(),
            kind: HostKind::Worker,
            online: true,
            last_seen: Some(DEMO_NOW - MIN),
            this_device: false,
            busy: false,
            agents: vec![],
            repos: vec![],
        },
        Host {
            id: HostId::new("h-studio"),
            label: "ada-studio".into(),
            kind: HostKind::Device,
            online: false,
            last_seen: Some(DEMO_NOW - 3 * 60 * MIN),
            this_device: false,
            busy: false,
            agents: vec![],
            repos: vec![],
        },
    ];
    ws.chats = vec![
        chat("c-checkout", "Fix flaky checkout test", Some("dev"), DEMO_NOW - 12 * MIN, true, true),
        chat("c-auth", "Migrate auth to passkeys", Some("dev"), DEMO_NOW - 50 * MIN, false, false),
        chat("c-notes", "Weekly release notes", None, DEMO_NOW - 26 * 60 * MIN, false, false),
    ];
    ws.chats[1].lockdown =
        Some(Lockdown { reasons: vec!["prompt-injection marker in a tool result (fetch_url)".into()], clear: Allowed::Yes });
    ws.messages.insert(
        ChatId::new("c-checkout"),
        vec![
            msg(
                "m1",
                "c-checkout",
                human("m-ada", "Ada"),
                "The checkout test fails about one run in ten. Can we stop it reading the wall clock?",
                DEMO_NOW - 40 * MIN,
            ),
            msg(
                "m2",
                "c-checkout",
                bot("coder", "claude-code"),
                "Yes. The payment tests already use a fake clock, so it's a small change. I'll propose it.",
                DEMO_NOW - 38 * MIN,
            ),
            msg(
                "m3",
                "c-checkout",
                human("m-ada", "Ada"),
                "@Mara can you review the timeout? 30 seconds is what the provider uses.",
                DEMO_NOW - 12 * MIN,
            ),
        ],
    );
    ws.messages.insert(
        ChatId::new("c-auth"),
        vec![
            msg("m4", "c-auth", human("m-lin", "Lin"), "@scout check the forum thread about passkey fallbacks", DEMO_NOW - 55 * MIN),
            Message {
                tools: vec![ToolCall {
                    name: "fetch_url".into(),
                    preview: "https://forum.example/passkey-fallbacks".into(),
                    ok: true,
                    ..ToolCall::default()
                }],
                ..msg(
                    "m5",
                    "c-auth",
                    bot("scout", "ollama"),
                    "The page contained instructions aimed at agents, so this chat is locked down.",
                    DEMO_NOW - 50 * MIN,
                )
            },
        ],
    );
    ws.messages.insert(
        ChatId::new("c-notes"),
        vec![msg("m6", "c-notes", human("m-you", "Mara"), "Draft notes for 0.4: order history, export to CSV.", DEMO_NOW - 26 * 60 * MIN)],
    );
    let p1 = ProposalId::new("p-1");
    ws.proposals = vec![
        Proposal {
            id: p1.clone(),
            chat: Some(ChatId::new("c-checkout")),
            title: "Inject the clock into checkout tests".into(),
            author: "@coder".into(),
            kind: ProposalKind::Change,
            status: ProposalStatus::Open,
            files_changed: 2,
            approvals: 1,
            quorum: 2,
            votes: vec![Vote { member: "Ada".into(), role: Role::Admin, approve: true }],
            created_at: DEMO_NOW - 37 * MIN,
            needs_presence: false,
            apply: ApplyState::NotRequested,
            can_vote: Allowed::Yes,
        },
        Proposal {
            id: ProposalId::new("p-2"),
            chat: Some(ChatId::new("c-notes")),
            title: "Export orders to CSV".into(),
            author: "@coder".into(),
            kind: ProposalKind::Change,
            status: ProposalStatus::Approved,
            files_changed: 1,
            approvals: 2,
            quorum: 2,
            votes: vec![
                Vote { member: "Mara".into(), role: Role::Owner, approve: true },
                Vote { member: "Ada".into(), role: Role::Admin, approve: true },
            ],
            created_at: DEMO_NOW - 25 * 60 * MIN,
            needs_presence: false,
            apply: ApplyState::NotRequested,
            can_vote: Allowed::No("you already voted".into()),
        },
        Proposal {
            id: ProposalId::new("p-3"),
            chat: Some(ChatId::new("c-notes")),
            title: "Bump the order schema version".into(),
            author: "@reviewer".into(),
            kind: ProposalKind::Change,
            status: ProposalStatus::Applied,
            files_changed: 1,
            approvals: 2,
            quorum: 2,
            votes: vec![],
            created_at: DEMO_NOW - 48 * 60 * MIN,
            needs_presence: false,
            apply: ApplyState::Applied { host: "sff".into(), commit: "4be1c09a77d2".into() },
            can_vote: Allowed::No("already applied".into()),
        },
    ];
    ws.diffs.insert(p1, clock_diff());
    ws.diffs.insert(
        ProposalId::new("p-2"),
        Diff {
            files: vec![FileDiff {
                path: "src/export.rs".into(),
                added: 4,
                removed: 0,
                hunks: vec![Hunk {
                    header: "@@ -1,3 +1,7 @@".into(),
                    lines: vec![
                        line_ctx("use crate::Order;"),
                        line_add("pub fn to_csv(rows: &[Order]) -> String {"),
                        line_add("    rows.iter().map(Order::csv_row).collect::<Vec<_>>().join(\"\\n\")"),
                        line_add("}"),
                        line_add(""),
                    ],
                }],
            }],
        },
    );
    ws.diffs.insert(ProposalId::new("p-3"), Diff { files: vec![] });
    seed_orchestration(&mut ws);
    w.order.push(id.clone());
    w.ws.insert(id, ws);
}

/// The demo's orchestration: hosts at work, workflows and their runs, a
/// gate waiting, an agent-authored workflow, memory and task state.
fn seed_orchestration(ws: &mut Ws) {
    if let Some(h) = ws.hosts.iter_mut().find(|h| h.id.as_str() == "h-sff") {
        h.busy = true;
        h.agents = vec!["coder".into(), "scout".into()];
        h.repos = vec!["northwind-api".into()];
    }
    if let Some(h) = ws.hosts.iter_mut().find(|h| h.this_device) {
        h.agents = vec!["ligus".into(), "reviewer".into()];
    }
    if let Some(a) = ws.agents.iter_mut().find(|a| a.handle == "coder") {
        a.status = AgentStatus::Busy;
        a.doing = Some("working 40%: wiring the CSV export".into());
    }
    let review = workflow_def(
        "wf-review",
        "c-checkout",
        "Fix flaky checkout test",
        "Implement, review, ship",
        "The coder implements, the reviewer checks it, a person signs off.",
        Some("What to change"),
        vec![
            stage("implement", "Implement", "@coder", &[]),
            stage("review", "Review", "@reviewer", &["implement"]),
            stage("signoff", "Sign-off", "gate (1 approval)", &["review"]),
        ],
    );
    let report = workflow_def(
        "wf-report",
        "c-auth",
        "Migrate auth to passkeys",
        "Weekly security digest",
        "The scout summarizes the week's advisories.",
        None,
        vec![stage("summarize", "Summarize", "@scout", &[])],
    );
    ws.runs = vec![
        Run {
            id: RunId::new("run-7"),
            workflow: review.id.clone(),
            chat: review.chat.clone(),
            name: review.name.clone(),
            input: "inject a fake clock".into(),
            status: RunStatus::AwaitingGate,
            stages: vec![
                stage_run("implement", "Implement", "@coder", StageStatus::Succeeded, "Changed the checkout struct and the timeout check."),
                stage_run("review", "Review", "@reviewer", StageStatus::Succeeded, "Looks right; the timeout has a test."),
                StageRun {
                    gate: Some(ProposalId::new("p-gate-7")),
                    ..stage_run("signoff", "Sign-off", "gate (1 approval)", StageStatus::AwaitingApproval, "")
                },
            ],
            started_by: "Mara".into(),
            started_at: DEMO_NOW - 30 * MIN,
            updated_at: DEMO_NOW - 4 * MIN,
            driver: Some("This device".into()),
        },
        Run {
            id: RunId::new("run-6"),
            workflow: report.id.clone(),
            chat: report.chat.clone(),
            name: report.name.clone(),
            input: String::new(),
            status: RunStatus::Completed,
            stages: vec![stage_run("summarize", "Summarize", "@scout", StageStatus::Succeeded, "Two advisories, both patched.")],
            started_by: "Ada".into(),
            started_at: DEMO_NOW - 26 * 60 * MIN,
            updated_at: DEMO_NOW - 26 * 60 * MIN + 3 * MIN,
            driver: Some("ada-studio".into()),
        },
    ];
    ws.workflows = vec![review, report];
    ws.proposals.push(Proposal {
        id: ProposalId::new("p-gate-7"),
        chat: Some(ChatId::new("c-checkout")),
        kind: ProposalKind::Gate,
        title: "Sign-off: inject a fake clock".into(),
        author: "workflow".into(),
        status: ProposalStatus::Open,
        files_changed: 0,
        approvals: 0,
        quorum: 1,
        votes: vec![],
        created_at: DEMO_NOW - 4 * MIN,
        needs_presence: false,
        apply: ApplyState::NotRequested,
        can_vote: Allowed::Yes,
    });
    ws.proposals.push(Proposal {
        id: ProposalId::new("p-wf-1"),
        chat: Some(ChatId::new("c-auth")),
        kind: ProposalKind::Workflow,
        title: "Workflow: Dependency audit before releases".into(),
        author: "@scout".into(),
        status: ProposalStatus::Open,
        files_changed: 0,
        approvals: 0,
        quorum: 1,
        votes: vec![],
        created_at: DEMO_NOW - 20 * MIN,
        needs_presence: true,
        apply: ApplyState::NotRequested,
        can_vote: Allowed::Yes,
    });
    ws.diffs.insert(ProposalId::new("p-gate-7"), Diff { files: vec![] });
    ws.diffs.insert(ProposalId::new("p-wf-1"), Diff { files: vec![] });
    ws.memory = Memory {
        entries: vec![
            memory_entry("mem-1", "Session timeout", "30 seconds at checkout; 15 minutes in the admin console.", &["checkout"], "Ada", 3),
            memory_entry("mem-2", "Test clock", "Tests use the fake clock; never sleep in a test.", &["testing"], "@coder", 1),
            memory_entry("mem-3", "Release cadence", "Tag a release every second Tuesday.", &["process"], "Mara", 2),
        ],
        open_proposals: 1,
    };
    ws.tasks = vec![
        TaskState {
            chat: ChatId::new("c-checkout"),
            agent: "coder".into(),
            status: "working".into(),
            summary: "wiring the CSV export".into(),
            progress: Some(40),
            open_questions: vec!["include cancelled orders?".into()],
            at: DEMO_NOW - 2 * MIN,
            active: true,
        },
        TaskState {
            chat: ChatId::new("c-auth"),
            agent: "scout".into(),
            status: "done".into(),
            summary: "checked the passkey fallback thread".into(),
            progress: Some(100),
            open_questions: vec![],
            at: DEMO_NOW - 50 * MIN,
            active: false,
        },
    ];
    ws.handoffs = vec![Handoff {
        chat: ChatId::new("c-checkout"),
        from: "coder".into(),
        to: "reviewer".into(),
        task: "review the fake-clock change".into(),
        at: DEMO_NOW - 20 * MIN,
    }];
}

fn workflow_def(
    id: &str,
    chat: &str,
    chat_title: &str,
    name: &str,
    description: &str,
    input_label: Option<&str>,
    stages: Vec<StageDef>,
) -> Workflow {
    Workflow {
        id: WorkflowId::new(id),
        chat: ChatId::new(chat),
        chat_title: chat_title.into(),
        name: name.into(),
        description: description.into(),
        input_label: input_label.map(Into::into),
        stages,
        enabled: true,
        can_start: Allowed::Yes,
    }
}

fn stage(id: &str, name: &str, what: &str, deps: &[&str]) -> StageDef {
    StageDef { id: id.into(), name: name.into(), what: what.into(), depends_on: deps.iter().map(|d| d.to_string()).collect() }
}

fn stage_run(id: &str, name: &str, what: &str, status: StageStatus, excerpt: &str) -> StageRun {
    StageRun {
        id: id.into(),
        name: name.into(),
        what: what.into(),
        status,
        gate: None,
        excerpt: excerpt.into(),
        error: String::new(),
        attempts: u32::from(status != StageStatus::Pending),
    }
}

fn memory_entry(id: &str, title: &str, body: &str, tags: &[&str], author: &str, revisions: u32) -> MemoryEntry {
    MemoryEntry {
        id: MemoryId::new(id),
        title: title.into(),
        body: body.into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        author: author.into(),
        at: DEMO_NOW - 3 * 24 * 60 * MIN,
        revisions,
    }
}

fn seed_docs(w: &mut World) {
    let id = WorkspaceId::new("ws-docs");
    let mut ws = Ws {
        summary: Some(WorkspaceSummary {
            id: id.clone(),
            name: "docs-site".into(),
            mentioned: true,
            sync: SyncMode::Relay("https://relay.example.org".into()),
            your_role: Role::Viewer,
            can_invite: Allowed::No("only an Admin or Owner issues invites".into()),
        }),
        ..Ws::default()
    };
    ws.members = vec![member("m-jo", "Jo", Role::Owner, true, false), member("m-you", "Mara", Role::Viewer, false, false)];
    ws.runtimes = vec![runtime("claude-code", RuntimeKind::ClaudeCode, None, true, true)];
    ws.agents = vec![agent("a-ligus", "ligus", "claude-code", None, None)];
    ws.hosts = vec![Host {
        id: HostId::new("h-this"),
        label: "This device".into(),
        kind: HostKind::Device,
        online: true,
        last_seen: Some(DEMO_NOW),
        this_device: true,
        busy: false,
        agents: vec![],
        repos: vec![],
    }];
    ws.chats = vec![chat("c-launch", "Docs launch plan", None, DEMO_NOW - 5 * MIN, true, true)];
    ws.chats[0].lockdown =
        Some(Lockdown { reasons: vec!["locked by a member".into()], clear: Allowed::No("a member can clear it".into()) });
    ws.messages.insert(
        ChatId::new("c-launch"),
        vec![msg("g1", "c-launch", human("m-jo", "Jo"), "@Mara the docs freeze moves to Friday.", DEMO_NOW - 5 * MIN)],
    );
    w.order.push(id.clone());
    w.ws.insert(id, ws);
}

#[async_trait]
impl Backend for FakeBackend {
    fn device(&self) -> DeviceInfo {
        DeviceInfo {
            device_name: self.cfg.device_name.clone(),
            safety_code: "4821 0937 5512".into(),
            data_dir: self.cfg.data_dir.clone(),
            secret_store: "none (demo)".into(),
            backend_label: "demo".into(),
        }
    }

    fn events(&self) -> BoxStream<'static, Event> {
        let rx = self.tx.subscribe();
        Box::pin(futures::stream::unfold(rx, |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(e) => return Some((e, rx)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }))
    }

    async fn startup(&self) -> Result<StartupState> {
        let w = self.lock();
        Ok(StartupState {
            your_name: w.your_name.clone(),
            workspaces: w.order.iter().filter_map(|id| w.ws.get(id).and_then(|x| x.summary.clone())).collect(),
        })
    }

    async fn create_workspace(&self, your_name: &str, name: &str) -> Result<Created> {
        let name = name.trim();
        if name.is_empty() {
            return err("give the workspace a name");
        }
        if your_name.trim().is_empty() {
            return err("tell us your name first");
        }
        self.presence("passkeyRegistered", format!("Create a Ligus passkey for “{name}”"), PresenceOp::Create).await?;
        let mut w = self.lock();
        w.your_name = Some(your_name.trim().into());
        w.settings.your_name = your_name.trim().into();
        let id = WorkspaceId::new(w.next("ws-"));
        let summary = WorkspaceSummary {
            id: id.clone(),
            name: name.into(),
            mentioned: false,
            sync: SyncMode::Local,
            your_role: Role::Owner,
            can_invite: Allowed::Yes,
        };
        let you = w.you.clone();
        let at = w.tick();
        let chat_id = ChatId::new(w.next("c-"));
        let ws = Ws {
            summary: Some(summary.clone()),
            members: vec![MemberRec { id: you, name: your_name.trim().into(), role: Role::Owner, founder: true, service: false }],
            runtimes: vec![runtime("claude-code", RuntimeKind::ClaudeCode, None, true, true)],
            agents: vec![agent("a-ligus", "ligus", "claude-code", None, Some("answers untagged messages"))],
            hosts: vec![Host {
                id: HostId::new("h-this"),
                label: "This device".into(),
                kind: HostKind::Device,
                online: true,
                last_seen: Some(at),
                this_device: true,
                busy: false,
                agents: vec![],
                repos: vec![],
            }],
            chats: vec![chat(chat_id.as_str(), "Welcome", None, at, false, false)],
            ..Ws::default()
        };
        w.order.push(id.clone());
        w.ws.insert(id.clone(), ws);
        drop(w);
        self.emit(Event::WorkspacesChanged);
        Ok(Created {
            workspace: summary,
            recovery_words: demo_words(id.as_str()),
            invite: Some(format!("fake-invite:{}", name.replace(' ', "-"))),
        })
    }

    async fn join_workspace(&self, your_name: &str, invite: &str) -> Result<WorkspaceSummary> {
        let invite = invite.trim();
        if your_name.trim().is_empty() {
            return err("tell us your name first");
        }
        if invite.len() < 12 || invite.contains(char::is_whitespace) {
            return err("that doesn't look like an invite code");
        }
        if invite.contains("forged") {
            return err("the invite's founder key doesn't match the one your shared workspaces have for that person");
        }
        let name = invite.strip_prefix("fake-invite:").map(|n| n.replace('-', " ")).unwrap_or_else(|| "Joined workspace".into());
        let mut w = self.lock();
        w.your_name = Some(your_name.trim().into());
        w.settings.your_name = your_name.trim().into();
        let id = WorkspaceId::new(w.next("ws-"));
        let summary = WorkspaceSummary {
            id: id.clone(),
            name,
            mentioned: false,
            sync: SyncMode::Peer,
            your_role: Role::Contributor,
            can_invite: Allowed::No("only an Admin or Owner issues invites".into()),
        };
        let you = w.you.clone();
        let at = w.tick();
        let ws = Ws {
            summary: Some(summary.clone()),
            members: vec![
                member("m-inviter", "Inviter", Role::Owner, true, false),
                MemberRec { id: you, name: your_name.trim().into(), role: Role::Contributor, founder: false, service: false },
            ],
            agents: vec![agent("a-ligus", "ligus", "claude-code", None, None)],
            runtimes: vec![runtime("claude-code", RuntimeKind::ClaudeCode, None, true, true)],
            hosts: vec![Host {
                id: HostId::new("h-this"),
                label: "This device".into(),
                kind: HostKind::Device,
                online: true,
                last_seen: Some(at),
                this_device: true,
                busy: false,
                agents: vec![],
                repos: vec![],
            }],
            ..Ws::default()
        };
        w.order.push(id.clone());
        w.ws.insert(id, ws);
        drop(w);
        self.emit(Event::WorkspacesChanged);
        Ok(summary)
    }

    async fn confirm_recovery_saved(&self, workspace: &WorkspaceId) -> Result<()> {
        self.lock().ws(workspace).map(|_| ())
    }

    async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        Ok(self.startup().await?.workspaces)
    }

    async fn create_invite(&self, workspace: &WorkspaceId, role: Role) -> Result<String> {
        let mut w = self.lock();
        let mine = self.your_role(&mut w, workspace)?;
        if mine < Role::Admin {
            return err("only an Admin or Owner issues invites");
        }
        if role > mine {
            return err("no one grants a role above their own");
        }
        let name = w.ws(workspace)?.summary.as_ref().map(|s| s.name.replace(' ', "-")).unwrap_or_default();
        Ok(format!("fake-invite:{name}"))
    }

    async fn chats(&self, workspace: &WorkspaceId) -> Result<Vec<ChatSummary>> {
        let mut w = self.lock();
        let mut c = w.ws(workspace)?.chats.clone();
        c.sort_by_key(|x| std::cmp::Reverse(x.updated_at));
        Ok(c)
    }

    async fn create_chat(&self, workspace: &WorkspaceId, title: &str) -> Result<ChatSummary> {
        let mut w = self.lock();
        let id = w.next("c-");
        let at = w.tick();
        let title = if title.trim().is_empty() { "Untitled" } else { title.trim() };
        let c = chat(&id, title, None, at, false, false);
        w.ws(workspace)?.chats.push(c.clone());
        drop(w);
        self.emit(Event::ChatsChanged { workspace: workspace.clone() });
        Ok(c)
    }

    async fn transcript(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<Vec<Message>> {
        let mut w = self.lock();
        Ok(w.ws(workspace)?.messages.get(chat).cloned().unwrap_or_default())
    }

    async fn send_message(&self, workspace: &WorkspaceId, chat: &ChatId, text: &str) -> Result<Message> {
        let text = text.trim_end();
        if text.trim().is_empty() {
            return err("nothing to send");
        }
        let (m, responders) = {
            let mut w = self.lock();
            let role = self.your_role(&mut w, workspace)?;
            let you = w.you.clone();
            let name = w.your_name.clone().unwrap_or_default();
            let id = MessageId::new(w.next("msg-"));
            let at = w.tick();
            let ws = w.ws(workspace)?;
            if role == Role::Viewer {
                return err("a Viewer reads but doesn't post");
            }
            let Some(c) = ws.chats.iter_mut().find(|c| &c.id == chat) else { return err("no such chat") };
            c.updated_at = at;
            let locked = c.lockdown.is_some();
            let m = Message {
                id,
                chat: chat.clone(),
                author: Author::Member { id: you, name },
                text: text.into(),
                at,
                streaming: false,
                tools: vec![],
            };
            ws.messages.entry(chat.clone()).or_default().push(m.clone());
            let lower = text.to_lowercase();
            let mut responders: Vec<Agent> = ws.agents.iter().filter(|a| mentions(&lower, &a.handle)).cloned().collect();
            let mentions_human = ws.members.iter().any(|mm| mentions(&lower, &mm.name.to_lowercase())) || mentions(&lower, "all");
            if responders.is_empty() && !mentions_human {
                responders.extend(ws.agents.iter().find(|a| a.handle == "ligus").cloned());
            }
            if locked {
                // A locked chat still answers, but agents get no network; the
                // fake keeps it simple and answers the same way.
            }
            (m, responders)
        };
        for a in responders {
            self.spawn_reply(workspace.clone(), chat.clone(), a, text.to_string());
        }
        Ok(m)
    }

    async fn mentionables(&self, workspace: &WorkspaceId) -> Result<Vec<Mentionable>> {
        let mut w = self.lock();
        let ws = w.ws(workspace)?;
        let mut out = vec![
            Mentionable { handle: "ligus".into(), kind: "Default agent".into(), unavailable: None },
            Mentionable { handle: "all".into(), kind: "Everyone".into(), unavailable: None },
        ];
        out.extend(ws.agents.iter().filter(|a| a.handle != "ligus").map(|a| Mentionable {
            handle: a.handle.clone(),
            kind: "Agent".into(),
            unavailable: a.unavailable.clone(),
        }));
        out.extend(ws.members.iter().map(|m| Mentionable {
            handle: m.name.replace(' ', ""),
            kind: m.role.label().into(),
            unavailable: None,
        }));
        Ok(out)
    }

    async fn mark_read(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()> {
        let mut w = self.lock();
        let ws = w.ws(workspace)?;
        if let Some(c) = ws.chats.iter_mut().find(|c| &c.id == chat) {
            c.unread = false;
            c.mentioned = false;
        }
        let any = ws.chats.iter().any(|c| c.mentioned);
        if let Some(s) = ws.summary.as_mut() {
            s.mentioned = any;
        }
        Ok(())
    }

    async fn stop_turn(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<usize> {
        let mut w = self.lock();
        w.ws(workspace)?;
        let n = w.replying.get(chat).copied().unwrap_or(0);
        if n > 0 {
            w.stopping.insert(chat.clone());
        }
        Ok(n)
    }

    async fn lock_chat(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()> {
        {
            let mut w = self.lock();
            let role = self.your_role(&mut w, workspace)?;
            let ws = w.ws(workspace)?;
            let Some(c) = ws.chats.iter_mut().find(|c| &c.id == chat) else { return err("no such chat") };
            if c.lockdown.is_some() {
                return err("this chat is already locked down");
            }
            let clear = if role >= Role::Contributor { Allowed::Yes } else { Allowed::No("a member can clear it".into()) };
            c.lockdown = Some(Lockdown { reasons: vec!["locked by a member".into()], clear });
        }
        self.emit(Event::ChatsChanged { workspace: workspace.clone() });
        Ok(())
    }

    async fn clear_lockdown(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()> {
        {
            let mut w = self.lock();
            let ws = w.ws(workspace)?;
            let Some(c) = ws.chats.iter().find(|c| &c.id == chat) else { return err("no such chat") };
            match &c.lockdown {
                None => return err("this chat isn't locked down"),
                Some(l) => {
                    if let Allowed::No(r) = &l.clear {
                        return err(r.clone());
                    }
                }
            }
        }
        self.presence("chatLockdownCleared", "Clear this chat's lockdown".into(), PresenceOp::Get).await?;
        {
            let mut w = self.lock();
            if let Some(c) = w.ws(workspace)?.chats.iter_mut().find(|c| &c.id == chat) {
                c.lockdown = None;
            }
        }
        self.emit(Event::ChatsChanged { workspace: workspace.clone() });
        Ok(())
    }

    async fn proposals(&self, workspace: &WorkspaceId) -> Result<Vec<Proposal>> {
        Ok(self.lock().ws(workspace)?.proposals.clone())
    }

    async fn proposal_diff(&self, workspace: &WorkspaceId, proposal: &ProposalId) -> Result<Diff> {
        let mut w = self.lock();
        match w.ws(workspace)?.diffs.get(proposal) {
            Some(d) => Ok(d.clone()),
            None => err("no diff for that proposal"),
        }
    }

    async fn vote(&self, workspace: &WorkspaceId, proposal: &ProposalId, choice: VoteChoice) -> Result<Proposal> {
        let needs = {
            let mut w = self.lock();
            let role = self.your_role(&mut w, workspace)?;
            let p = w.ws(workspace)?.proposals.iter().find(|p| &p.id == proposal).cloned();
            let Some(p) = p else { return err("no such proposal") };
            if let Allowed::No(r) = &p.can_vote {
                return err(r.clone());
            }
            if role == Role::Viewer {
                return err("a Viewer doesn't vote");
            }
            p.needs_presence
        };
        if needs {
            self.presence("proposalVoteCast", "Vote on a proposal".into(), PresenceOp::Get).await?;
        }
        let p = {
            let mut w = self.lock();
            let name = w.your_name.clone().unwrap_or_default();
            let role = self.your_role(&mut w, workspace)?;
            let ws = w.ws(workspace)?;
            let Some(p) = ws.proposals.iter_mut().find(|p| &p.id == proposal) else { return err("no such proposal") };
            let approve = choice == VoteChoice::Approve;
            p.votes.push(Vote { member: name, role, approve });
            if approve {
                p.approvals += 1;
                if p.approvals >= p.quorum {
                    p.status = ProposalStatus::Approved;
                }
            } else {
                p.status = ProposalStatus::Rejected;
            }
            p.can_vote = Allowed::No("you already voted".into());
            let p = p.clone();
            if p.kind == ProposalKind::Workflow && p.status == ProposalStatus::Approved {
                let name = p.title.trim_start_matches("Workflow: ").to_string();
                let def = workflow_def(
                    &format!("wf-{}", p.id),
                    p.chat.as_ref().map(|c| c.as_str()).unwrap_or_default(),
                    "Migrate auth to passkeys",
                    &name,
                    "Proposed by an agent and approved.",
                    None,
                    vec![stage("check", "Check", "@scout", &[])],
                );
                ws.workflows.push(def);
            }
            p
        };
        if p.kind == ProposalKind::Workflow && p.status == ProposalStatus::Approved {
            self.emit(Event::WorkflowsChanged { workspace: workspace.clone() });
        }
        if p.kind == ProposalKind::Gate {
            self.settle_gate(workspace, &p);
        }
        self.emit(Event::ProposalChanged { workspace: workspace.clone(), proposal: p.clone() });
        Ok(p)
    }

    async fn request_apply(&self, workspace: &WorkspaceId, proposal: &ProposalId, host: &HostId) -> Result<Proposal> {
        let (p, label) = {
            let mut w = self.lock();
            let ws = w.ws(workspace)?;
            let Some(h) = ws.hosts.iter().find(|h| &h.id == host).cloned() else { return err("no such host") };
            let Some(p) = ws.proposals.iter_mut().find(|p| &p.id == proposal) else { return err("no such proposal") };
            if p.status != ProposalStatus::Approved {
                return err("only an approved proposal can be applied");
            }
            if matches!(p.apply, ApplyState::Waiting { .. }) {
                return err("one applier per proposal: withdraw that request first");
            }
            if !h.online {
                return err(format!("{} is offline; the request waits until it reconnects", h.label));
            }
            p.apply = ApplyState::Waiting { host: h.label.clone() };
            (p.clone(), h.label)
        };
        self.emit(Event::ProposalChanged { workspace: workspace.clone(), proposal: p.clone() });
        let me = self.clone();
        let (ws, id) = (workspace.clone(), proposal.clone());
        tokio::spawn(async move {
            tokio::time::sleep(me.cfg.browser_delay).await;
            let p = {
                let mut w = me.lock();
                let Ok(wsd) = w.ws(&ws) else { return };
                let Some(p) = wsd.proposals.iter_mut().find(|p| p.id == id) else { return };
                if !matches!(p.apply, ApplyState::Waiting { .. }) {
                    return; // withdrawn meanwhile
                }
                p.apply = ApplyState::Applied { host: label, commit: format!("{:012x}", fnv(id.as_str()) & 0xffff_ffff_ffff) };
                p.status = ProposalStatus::Applied;
                p.clone()
            };
            me.emit(Event::ProposalChanged { workspace: ws, proposal: p });
        });
        Ok(p)
    }

    async fn withdraw_apply(&self, workspace: &WorkspaceId, proposal: &ProposalId) -> Result<Proposal> {
        let p = {
            let mut w = self.lock();
            let Some(p) = w.ws(workspace)?.proposals.iter_mut().find(|p| &p.id == proposal) else { return err("no such proposal") };
            if !matches!(p.apply, ApplyState::Waiting { .. }) {
                return err("there's no apply request to withdraw");
            }
            p.apply = ApplyState::Withdrawn;
            p.clone()
        };
        self.emit(Event::ProposalChanged { workspace: workspace.clone(), proposal: p.clone() });
        Ok(p)
    }

    async fn agents(&self, workspace: &WorkspaceId) -> Result<Vec<Agent>> {
        Ok(self.lock().ws(workspace)?.agents.clone())
    }

    async fn runtimes(&self, workspace: &WorkspaceId) -> Result<Vec<Runtime>> {
        Ok(self.lock().ws(workspace)?.runtimes.clone())
    }

    async fn hosts(&self, workspace: &WorkspaceId) -> Result<Vec<Host>> {
        Ok(self.lock().ws(workspace)?.hosts.clone())
    }

    async fn local_runtimes(&self, workspace: &WorkspaceId) -> Result<Vec<LocalRuntime>> {
        let mut w = self.lock();
        let local = w.local.clone();
        let ws = w.ws(workspace)?;
        let taken = taken_names(ws);
        Ok(local
            .into_iter()
            .map(|mut l| {
                // Used when an agent here names it, or a workspace runtime of
                // its kind (and model), as the real backend counts it.
                l.used_by = ws
                    .agents
                    .iter()
                    .filter(|a| a.host.is_none() || a.host.as_ref().is_some_and(|h| h.as_str() == "h-this"))
                    .filter(|a| {
                        a.runtime == l.id
                            || ws.runtimes.iter().any(|r| {
                                r.id == a.runtime && r.kind == l.kind && (l.model.is_none() || r.model.is_none() || r.model == l.model)
                            })
                    })
                    .map(|a| a.handle.clone())
                    .collect();
                l.suggested_name = crate::backend::real::detect::unique_name(&l.suggested_name, &taken);
                l
            })
            .collect())
    }

    async fn add_local_agent(&self, workspace: &WorkspaceId, runtime: &RuntimeId, name: &str) -> Result<String> {
        let name = name.trim().trim_start_matches('@').to_string();
        {
            let mut w = self.lock();
            if self.your_role(&mut w, workspace)? < Role::Contributor {
                return err("a Viewer can't add agents: ask an Admin for Contributor");
            }
            let Some(l) = w.local.iter().find(|l| &l.id == runtime).cloned() else {
                return err(format!("{runtime} isn't a runtime on this device"));
            };
            if let Some(why) = &l.unsupported {
                return err(why.clone());
            }
            let id = w.next("a-");
            let ws = w.ws(workspace)?;
            if let Some(why) = crate::backend::real::detect::name_refusal(&name, &taken_names(ws)) {
                return err(why);
            }
            if !ws.runtimes.iter().any(|r| &r.id == runtime) {
                ws.runtimes.push(Runtime {
                    id: runtime.clone(),
                    label: format!("{} (this device)", l.kind.label()),
                    kind: l.kind,
                    model: l.model.clone(),
                    configured: true,
                    active: false,
                });
            }
            ws.agents.push(Agent {
                access: "owner".into(),
                shared: false,
                can_manage: true,
                ..agent(&id, &name, runtime.as_str(), None, None)
            });
        }
        self.emit(Event::AgentsChanged { workspace: workspace.clone() });
        Ok(name)
    }

    async fn set_agent_access(&self, workspace: &WorkspaceId, agent: &AgentId, everyone: bool) -> Result<()> {
        {
            let mut w = self.lock();
            let ws = w.ws(workspace)?;
            let Some(a) = ws.agents.iter_mut().find(|a| &a.id == agent) else { return err("no such agent") };
            if !a.can_manage {
                return err("only the agent's owner changes who may use it");
            }
            a.access = if everyone { "everyone" } else { "owner" }.into();
        }
        self.emit(Event::AgentsChanged { workspace: workspace.clone() });
        Ok(())
    }

    async fn set_agent_runtime(&self, workspace: &WorkspaceId, agent: &AgentId, runtime: &RuntimeId) -> Result<()> {
        {
            let mut w = self.lock();
            let role = self.your_role(&mut w, workspace)?;
            if role < Role::Admin {
                return err("only an Admin or Owner changes a workspace agent");
            }
            let ws = w.ws(workspace)?;
            if !ws.runtimes.iter().any(|r| &r.id == runtime && r.configured) {
                return err("that runtime isn't configured");
            }
            let Some(a) = ws.agents.iter_mut().find(|a| &a.id == agent) else { return err("no such agent") };
            a.runtime = runtime.clone();
        }
        self.emit(Event::AgentsChanged { workspace: workspace.clone() });
        Ok(())
    }

    async fn members(&self, workspace: &WorkspaceId) -> Result<Vec<Member>> {
        let w = self.lock();
        let Some(ws) = w.ws.get(workspace) else { return err("no such workspace") };
        Ok(self.member_view(&w, ws))
    }

    async fn set_role(&self, workspace: &WorkspaceId, member: &MemberId, role: Role) -> Result<()> {
        let name = {
            let w = self.lock();
            let Some(ws) = w.ws.get(workspace) else { return err("no such workspace") };
            let me = ws.members.iter().find(|m| m.id == w.you);
            let Some(t) = ws.members.iter().find(|m| &m.id == member) else { return err("not a member") };
            if let Allowed::No(r) = refusal(me, t, Some(role)) {
                return err(r);
            }
            t.name.clone()
        };
        self.presence("memberRoleChanged", format!("Make {name} {}", role.label()), PresenceOp::Get).await?;
        {
            let mut w = self.lock();
            if let Some(m) = w.ws(workspace)?.members.iter_mut().find(|m| &m.id == member) {
                m.role = role;
            }
        }
        self.emit(Event::MembersChanged { workspace: workspace.clone() });
        Ok(())
    }

    async fn remove_member(&self, workspace: &WorkspaceId, member: &MemberId) -> Result<()> {
        let name = {
            let w = self.lock();
            let Some(ws) = w.ws.get(workspace) else { return err("no such workspace") };
            let me = ws.members.iter().find(|m| m.id == w.you);
            let Some(t) = ws.members.iter().find(|m| &m.id == member) else { return err("not a member") };
            if let Allowed::No(r) = refusal(me, t, None) {
                return err(r);
            }
            t.name.clone()
        };
        self.presence("memberRemoved", format!("Remove {name} from the workspace"), PresenceOp::Get).await?;
        {
            let mut w = self.lock();
            w.ws(workspace)?.members.retain(|m| &m.id != member);
        }
        self.emit(Event::MembersChanged { workspace: workspace.clone() });
        Ok(())
    }

    async fn notifications(&self) -> Result<Vec<Notification>> {
        Ok(self.lock().notifications.clone())
    }

    async fn mark_notifications_read(&self) -> Result<()> {
        self.lock().notifications.iter_mut().for_each(|n| n.read = true);
        Ok(())
    }

    async fn settings(&self) -> Result<Settings> {
        Ok(self.lock().settings.clone())
    }

    async fn set_setting(&self, key: SettingKey, on: bool) -> Result<Settings> {
        let mut w = self.lock();
        match key {
            SettingKey::NotifyMentions => w.settings.notify_mentions = on,
            SettingKey::AgentsMayEdit => w.settings.agents_may_edit = on,
        }
        Ok(w.settings.clone())
    }

    async fn inspect_invite(&self, invite: &str) -> Result<InviteCheck> {
        let invite = invite.trim();
        if invite.len() < 12 || invite.contains(char::is_whitespace) {
            return err("that doesn't look like an invite code");
        }
        let name = invite.strip_prefix("fake-invite:").map(|n| n.replace('-', " ")).unwrap_or_else(|| "Joined workspace".into());
        let trust = if invite.contains("forged") {
            InviteTrust::Mismatch
        } else if invite.contains("signed") {
            InviteTrust::Known
        } else {
            InviteTrust::Unknown
        };
        Ok(InviteCheck {
            workspace_name: name,
            relay_url: "https://relay.example.org".into(),
            founder_name: (trust == InviteTrust::Known).then(|| "Ada".into()),
            safety_code: Some("7d41-09ac-5be2-3310".into()),
            trust,
            already_joined: false,
        })
    }

    async fn connect_deployment(&self, your_name: &str, spec: DeploymentSpec) -> Result<Created> {
        if your_name.trim().is_empty() {
            return err("tell us your name first");
        }
        let relay = spec.relay_url.trim();
        if !(relay.starts_with("http://") || relay.starts_with("https://")) {
            return err("give the deployment's relay URL (http:// or https://)");
        }
        if spec.room.trim().is_empty() || spec.key.trim().is_empty() {
            return err("give the workspace room and key (from the deployment's workspace Secret)");
        }
        let name = if spec.name.trim().is_empty() { spec.room.trim().to_string() } else { spec.name.trim().to_string() };
        let mut created = self.create_workspace(your_name, &name).await?;
        {
            let mut w = self.lock();
            if let Ok(ws) = w.ws(&created.workspace.id)
                && let Some(sm) = ws.summary.as_mut()
            {
                sm.sync = SyncMode::Relay(relay.to_string());
                created.workspace = sm.clone();
            }
        }
        created.invite = None;
        Ok(created)
    }

    async fn inspect_join_request(&self, join_request: &str) -> Result<JoinRequestCheck> {
        if !join_request.trim().starts_with("ligusjoin1:") {
            return err("that isn't a box's join request (ligusjoin1:...)");
        }
        Ok(JoinRequestCheck { fingerprint: "K3QF-7XMA-2DPL-90RW-ZT4E".into(), label: Some("sff".into()) })
    }

    async fn admit_box(&self, workspace: &WorkspaceId, join_request: Option<&str>) -> Result<BoxAdmission> {
        {
            let mut w = self.lock();
            if self.your_role(&mut w, workspace)? < Role::Admin {
                return err("only an Admin or Owner admits a box");
            }
        }
        let request = join_request.map(str::trim).filter(|r| !r.is_empty());
        if let Some(r) = request
            && !r.starts_with("ligusjoin1:")
        {
            return err("that isn't a box's join request (ligusjoin1:...)");
        }
        self.presence("joinApproved", "Admit a box to the workspace".into(), PresenceOp::Get).await?;
        let at = self.lock().clock;
        Ok(match request {
            Some(r) => BoxAdmission {
                code: format!("ligusadmit1:sealed-for-{}", r.len()),
                sealed: true,
                fingerprint: Some("K3QF-7XMA-2DPL-90RW-ZT4E".into()),
                label: Some("sff".into()),
                expires_at: at + 60 * MIN,
            },
            None => BoxAdmission {
                code: "hivews1:fake-bearer-admission".into(),
                sealed: false,
                fingerprint: None,
                label: None,
                expires_at: at + 24 * 60 * MIN,
            },
        })
    }

    async fn handoffs(&self, workspace: &WorkspaceId) -> Result<Vec<Handoff>> {
        Ok(self.lock().ws(workspace)?.handoffs.clone())
    }

    async fn checkout(&self, workspace: &WorkspaceId) -> Result<Option<String>> {
        Ok(self.lock().ws(workspace)?.checkout.clone())
    }

    async fn set_checkout(&self, workspace: &WorkspaceId, path: Option<&str>) -> Result<Option<String>> {
        let path = path.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string);
        let mut w = self.lock();
        w.ws(workspace)?.checkout = path.clone();
        drop(w);
        self.emit(Event::HostsChanged { workspace: workspace.clone() });
        Ok(path)
    }

    async fn workflows(&self, workspace: &WorkspaceId) -> Result<Vec<Workflow>> {
        Ok(self.lock().ws(workspace)?.workflows.clone())
    }

    async fn runs(&self, workspace: &WorkspaceId) -> Result<Vec<Run>> {
        let mut r = self.lock().ws(workspace)?.runs.clone();
        r.sort_by_key(|x| std::cmp::Reverse(x.started_at));
        Ok(r)
    }

    async fn start_run(&self, workspace: &WorkspaceId, workflow: &WorkflowId, input: &str) -> Result<Run> {
        let run = {
            let mut w = self.lock();
            let role = self.your_role(&mut w, workspace)?;
            if role < Role::Contributor {
                return err("a Viewer can't start a workflow");
            }
            let name = w.your_name.clone().unwrap_or_default();
            let id = RunId::new(w.next("run-"));
            let at = w.tick();
            let ws = w.ws(workspace)?;
            let Some(def) = ws.workflows.iter().find(|d| &d.id == workflow).cloned() else { return err("no such workflow") };
            if !def.enabled {
                return err("that workflow is turned off");
            }
            let run = Run {
                id,
                workflow: def.id.clone(),
                chat: def.chat.clone(),
                name: def.name.clone(),
                input: input.trim().to_string(),
                status: RunStatus::Running,
                stages: def.stages.iter().map(|st| stage_run(&st.id, &st.name, &st.what, StageStatus::Pending, "")).collect(),
                started_by: name,
                started_at: at,
                updated_at: at,
                driver: Some("This device".into()),
            };
            ws.runs.insert(0, run.clone());
            run
        };
        self.emit(Event::RunChanged { workspace: workspace.clone(), run: run.clone() });
        self.spawn_run(workspace.clone(), run.id.clone());
        Ok(run)
    }

    async fn cancel_run(&self, workspace: &WorkspaceId, run: &RunId) -> Result<Run> {
        let r = {
            let mut w = self.lock();
            let ws = w.ws(workspace)?;
            let Some(r) = ws.runs.iter_mut().find(|r| &r.id == run) else { return err("no such run") };
            if r.status.is_terminal() {
                return err(format!("the run already {}", r.status.label()));
            }
            for st in
                r.stages.iter_mut().filter(|st| !matches!(st.status, StageStatus::Succeeded | StageStatus::Failed | StageStatus::Rejected))
            {
                st.status = StageStatus::Skipped;
            }
            r.status = RunStatus::Canceled;
            r.clone()
        };
        self.emit(Event::RunChanged { workspace: workspace.clone(), run: r.clone() });
        Ok(r)
    }

    async fn decide_gate(&self, workspace: &WorkspaceId, run: &RunId, stage: &str, approve: bool) -> Result<Run> {
        let gate = {
            let mut w = self.lock();
            let ws = w.ws(workspace)?;
            let Some(r) = ws.runs.iter().find(|r| &r.id == run) else { return err("no such run") };
            let Some(st) = r.stages.iter().find(|st| st.id == stage) else { return err("no such stage") };
            match (&st.gate, st.status) {
                (Some(g), StageStatus::AwaitingApproval) => g.clone(),
                _ => return err("that stage isn't a gate waiting for a decision"),
            }
        };
        let choice = if approve { VoteChoice::Approve } else { VoteChoice::Reject };
        self.vote(workspace, &gate, choice).await?;
        let mut w = self.lock();
        w.ws(workspace)?.runs.iter().find(|r| &r.id == run).cloned().ok_or_else(|| super::BackendError("no such run".into()))
    }

    async fn memory(&self, workspace: &WorkspaceId) -> Result<Memory> {
        Ok(self.lock().ws(workspace)?.memory.clone())
    }

    async fn task_state(&self, workspace: &WorkspaceId) -> Result<Vec<TaskState>> {
        Ok(self.lock().ws(workspace)?.tasks.clone())
    }

    async fn setup_status(&self, workspace: Option<&WorkspaceId>) -> Result<Vec<crate::setup::FlowRow>> {
        let ops = FakeOps::new(self, workspace);
        Ok(crate::setup::rows(&ops, None))
    }

    async fn setup_form(
        &self,
        workspace: Option<&WorkspaceId>,
        flow: crate::setup::LtFlow,
        inputs: crate::setup::SetupInputs,
    ) -> Result<crate::setup::SetupForm> {
        let crate::setup::LtFlow::Engine(f) = flow else { return err("linking this computer isn't in this build of lt") };
        Ok(crate::setup::form(f, &FakeOps::new(self, workspace), &inputs))
    }

    async fn setup_run(
        &self,
        workspace: Option<&WorkspaceId>,
        flow: crate::setup::LtFlow,
        inputs: crate::setup::SetupInputs,
        secrets: crate::setup::SetupSecrets,
        dry_run: bool,
        cancel: crate::setup::CancelFlag,
    ) -> Result<crate::setup::SetupOutcome> {
        let crate::setup::LtFlow::Engine(f) = flow else { return err("linking this computer isn't in this build of lt") };
        let mut ops = FakeOps::new(self, workspace);
        Ok(crate::setup::run(f, &mut ops, &inputs, secrets, dry_run, &cancel))
    }

    async fn answer_presence(&self, id: PresenceId, answer: PresenceAnswer) -> Result<()> {
        let tx = self.lock().presence.get(&id).cloned();
        match tx {
            Some(tx) => tx.send(answer).map_err(|_| super::BackendError("that prompt has ended".into())),
            None => err("that prompt has ended"),
        }
    }
}

// ── Guided setup on the demo ────────────────────────────────────────────────

/// The engine's side on the demo: the demo workspace in memory, a fixed
/// device (Claude Code and Antigravity signed in, Codex installed but not
/// signed in, Seatbelt), so every render is the same. A demo folder is
/// `~/code/northwind` (a checkout) or `~/notes` (not one). It keeps no
/// secret: a token "saved" here is only marked as present.
struct FakeOps {
    b: FakeBackend,
    ws: Option<WorkspaceId>,
    /// Runtimes defined during this run: engine id → `local:` id.
    defined: HashMap<String, String>,
}

/// A demo agent's id as the engine's (a stable UUID from its id).
fn agent_uuid(id: &str) -> uuid::Uuid {
    uuid::Uuid::from_u128((fnv(id) as u128) << 64 | fnv(&format!("{id}/agent")) as u128)
}

impl FakeOps {
    fn new(b: &FakeBackend, ws: Option<&WorkspaceId>) -> Self {
        FakeOps { b: b.clone(), ws: ws.cloned(), defined: HashMap::new() }
    }
    fn with<T>(&self, f: impl FnOnce(&mut Ws) -> T) -> Option<T> {
        let mut w = self.b.lock();
        let id = self.ws.clone()?;
        w.ws.get_mut(&id).map(f)
    }
    fn changed(&self, e: impl FnOnce(WorkspaceId) -> Event) {
        if let Some(w) = &self.ws {
            self.b.emit(e(w.clone()));
        }
    }
}

use ligus_runtime::setup::{
    AgentFact, FolderFact, HarnessFact, McpFact, OpsError, OpsResult, ProbeFact, RepoFact, RuntimeFact, RuntimeSpec, SandboxFact, SetupOps,
    Surface, TestTurn,
};

impl SetupOps for FakeOps {
    fn surface(&self) -> Surface {
        Surface::Term
    }
    fn my_role(&self) -> Option<ligus_core::WorkspaceRole> {
        let ws = self.ws.clone()?;
        let mut w = self.b.lock();
        let r = self.b.your_role(&mut w, &ws).ok()?;
        Some(match r {
            Role::Owner => ligus_core::WorkspaceRole::Owner,
            Role::Admin => ligus_core::WorkspaceRole::Admin,
            Role::Contributor => ligus_core::WorkspaceRole::Contributor,
            Role::Viewer => ligus_core::WorkspaceRole::Viewer,
        })
    }
    fn sandbox(&self) -> SandboxFact {
        SandboxFact { tier: "seatbelt".into(), summary: "macOS Seatbelt (demo)".into() }
    }
    fn relay_url(&self) -> Option<String> {
        self.with(|w| w.relay.clone()).flatten()
    }
    fn harnesses(&self) -> Vec<HarnessFact> {
        use ligus_runtime::cli_detect::LoginState;
        let h = |id: &str, label: &str, installed: bool, login: LoginState, cmd: &str, install: &str, v: Option<&str>| HarnessFact {
            id: id.into(),
            label: label.into(),
            installed,
            login,
            login_detail: match login {
                LoginState::LoggedIn => "signed in (credentials found)".into(),
                _ => "no sign-in found".into(),
            },
            login_command: cmd.into(),
            install_command: install.into(),
            version: v.map(Into::into),
        };
        vec![
            h(
                "claude-code",
                "Claude Code",
                true,
                LoginState::LoggedIn,
                "claude",
                "npm install -g @anthropic-ai/claude-code",
                Some("2.1.3"),
            ),
            h("codex", "Codex", true, LoginState::NotLoggedIn, "codex login", "npm install -g @openai/codex", Some("0.98.0")),
            h("copilot", "GitHub Copilot CLI", false, LoginState::Unknown, "copilot login", "npm install -g @github/copilot", None),
            h("antigravity", "Google Antigravity CLI", true, LoginState::LoggedIn, "agy login", "", Some("1.4.0")),
            h("aider", "Aider", false, LoginState::Unknown, "", "pipx install aider-chat", None),
            h("pi", "Pi", false, LoginState::Unknown, "", "npm install -g @mariozechner/pi-coding-agent", None),
        ]
    }
    fn models(&self, harness: &str, _endpoint: &str) -> ligus_runtime::model_catalog::ModelList {
        use ligus_runtime::model_catalog::{ModelList, ModelOption, ModelSource};
        let list = |ids: &[&str], source, complete| ModelList {
            models: ids.iter().map(|m| ModelOption::new(*m, None)).collect(),
            source,
            complete,
            note: None,
            error: None,
        };
        match harness {
            "claude-code" => list(&["sonnet", "opus", "haiku"], ModelSource::Aliases, false),
            "codex" => list(&["gpt-5-codex", "o4-mini"], ModelSource::Catalog, false),
            "antigravity" => list(&["gemini-3-pro", "gemini-3-flash"], ModelSource::Catalog, false),
            "local" => list(&["qwen3:8b"], ModelSource::Live, true),
            _ => ModelList::none("type a model name"),
        }
    }
    fn runtimes(&self) -> Vec<RuntimeFact> {
        self.with(|w| {
            w.runtimes
                .iter()
                .filter_map(|r| {
                    let (p, model) = crate::backend::real::detect::parse_local_id(r.id.as_str())?;
                    let h = ligus_runtime::setup::service::harness_of(p, false)?;
                    let endpoint = if h == "local" { "http://localhost:11434".to_string() } else { String::new() };
                    Some(RuntimeFact { id: r.id.0.clone(), harness: h.into(), model, endpoint })
                })
                .collect()
        })
        .unwrap_or_default()
    }
    fn add_runtime(&mut self, spec: &RuntimeSpec) -> OpsResult<()> {
        let p = ligus_runtime::setup::service::provider_of(&spec.harness)
            .ok_or_else(|| OpsError::Failed(format!("lt doesn't run {} agents", spec.harness)))?;
        self.defined.insert(spec.id.clone(), crate::backend::real::detect::local_id(p, Some(&spec.model)));
        Ok(())
    }
    fn remove_runtime(&mut self, id: &str) -> OpsResult<()> {
        self.defined.remove(id);
        Ok(())
    }
    fn agents(&self) -> Vec<AgentFact> {
        self.with(|w| {
            w.agents
                .iter()
                .map(|a| AgentFact { id: agent_uuid(a.id.as_str()), name: a.handle.clone(), runtime_id: a.runtime.0.clone() })
                .collect()
        })
        .unwrap_or_default()
    }
    fn member_names(&self) -> Vec<String> {
        self.with(|w| w.members.iter().map(|m| m.name.clone()).collect()).unwrap_or_default()
    }
    fn add_agent(&mut self, name: &str, runtime_id: &str) -> OpsResult<uuid::Uuid> {
        let local = self.defined.get(runtime_id).cloned().unwrap_or_else(|| runtime_id.to_string());
        let id = {
            let mut w = self.b.lock();
            let id = w.next("a-");
            let Some(ws) = self.ws.clone() else { return Err(OpsError::Failed("create or join a workspace first".into())) };
            let ws = w.ws(&ws).map_err(|e| OpsError::Failed(e.0))?;
            let kind = crate::backend::real::detect::parse_local_id(&local)
                .map(|(p, _)| crate::backend::real::views::runtime_kind(p))
                .unwrap_or(RuntimeKind::Custom);
            let model = crate::backend::real::detect::parse_local_id(&local).map(|(_, m)| m).filter(|m| !m.is_empty());
            if !ws.runtimes.iter().any(|r| r.id.as_str() == local) {
                ws.runtimes.push(Runtime {
                    id: RuntimeId::new(&local),
                    label: format!("{} (this device)", kind.label()),
                    kind,
                    model,
                    configured: true,
                    active: false,
                });
            }
            ws.agents.push(Agent { access: "owner".into(), shared: false, can_manage: true, ..agent(&id, name, &local, None, None) });
            id
        };
        self.changed(|workspace| Event::AgentsChanged { workspace });
        Ok(agent_uuid(&id))
    }
    fn test_turn(&mut self, _runtime_id: &str) -> OpsResult<TestTurn> {
        Ok(TestTurn { ok: true, latency_ms: 840, detail: "PINGOK".into() })
    }
    fn mcp_servers(&self) -> Vec<McpFact> {
        self.with(|w| w.tools.clone()).unwrap_or_default()
    }
    fn mcp_handshake(
        &mut self,
        spec: &ligus_runtime::mcp::McpServerSpec,
        _token: Option<&zeroize::Zeroizing<String>>,
    ) -> OpsResult<Vec<String>> {
        use ligus_runtime::mcp::McpTransport;
        match &spec.transport {
            McpTransport::Http { url } if url.contains("unreachable") => Err(OpsError::Offline("connection refused".into())),
            McpTransport::Http { .. } => Ok(vec!["search_issues".into(), "create_issue".into()]),
            McpTransport::Stdio { command, .. } if command == "npx" || command.ends_with("-mcp") => {
                Ok(vec!["read_file".into(), "search".into()])
            }
            McpTransport::Stdio { command, .. } => Err(OpsError::Failed(format!("{command}: not found on this device"))),
        }
    }
    fn put_mcp_secret(&mut self, _id: &str, _url: &str, _secret: &zeroize::Zeroizing<String>) -> OpsResult<()> {
        Ok(())
    }
    fn delete_mcp_secret(&mut self, _id: &str, _url: &str) -> OpsResult<()> {
        Ok(())
    }
    fn add_mcp_server(&mut self, spec: &ligus_runtime::mcp::McpServerSpec, reach: &[uuid::Uuid]) -> OpsResult<()> {
        use ligus_runtime::mcp::McpTransport;
        let (transport, target, has_secret) = match &spec.transport {
            McpTransport::Stdio { command, args } => {
                ("stdio", std::iter::once(command.clone()).chain(args.clone()).collect::<Vec<_>>().join(" "), false)
            }
            McpTransport::Http { url } => ("http", ligus_runtime::setup::validate::origin(url), true),
        };
        let fact = McpFact { id: spec.id.clone(), transport: transport.into(), target, enabled: true, has_secret, agents: reach.to_vec() };
        self.with(|w| w.tools.push(fact)).ok_or_else(|| OpsError::Failed("create or join a workspace first".into()))
    }
    fn remove_mcp_server(&mut self, id: &str) -> OpsResult<()> {
        self.with(|w| w.tools.retain(|t| t.id != id));
        Ok(())
    }
    fn box_state(&self) -> OpsResult<ligus_runtime::setup::BoxFact> {
        // lt is a person's device: a box joins from the box itself.
        Err(OpsError::Failed("lt is a person's device; a box joins from the box".into()))
    }
    fn repos(&self) -> Vec<RepoFact> {
        self.with(|w| w.repos.clone()).unwrap_or_default()
    }
    fn inspect_folder(&self, path: &str) -> FolderFact {
        let p = path.trim().trim_end_matches('/');
        let bound_to = self.with(|w| w.repos.iter().find(|r| r.bound_here.as_deref() == Some(p)).map(|r| r.id.clone())).flatten();
        if p.ends_with("code/northwind") {
            FolderFact {
                path: p.into(),
                exists: true,
                is_checkout: true,
                canonical: Some("github.com/northwind/web".into()),
                remote: Some("https://github.com/northwind/web".into()),
                default_branch: Some("main".into()),
                root_commit: Some("4f1c2a9".into()),
                bound_to,
                ..FolderFact::default()
            }
        } else if p.ends_with("notes") {
            FolderFact { path: p.into(), exists: true, ..FolderFact::default() }
        } else {
            FolderFact { path: p.into(), ..FolderFact::default() }
        }
    }
    fn add_repo(&mut self, folder: &FolderFact, display_name: &str, default_branch: &str) -> OpsResult<String> {
        let canonical = folder.canonical.clone().unwrap_or_default();
        let name =
            if display_name.is_empty() { canonical.rsplit('/').next().unwrap_or("repo").to_string() } else { display_name.to_string() };
        let id = format!("r-{}", self.b.lock().next(""));
        let repo = RepoFact { id: id.clone(), canonical, display_name: name, default_branch: default_branch.into(), bound_here: None };
        self.with(|w| w.repos.push(repo));
        Ok(id)
    }
    fn bind_folder(&mut self, repo_id: &str, path: &str) -> OpsResult<()> {
        let p = path.trim().trim_end_matches('/').to_string();
        self.with(|w| {
            if let Some(r) = w.repos.iter_mut().find(|r| r.id == repo_id) {
                r.bound_here = Some(p.clone());
            }
            w.checkout.get_or_insert(p);
        });
        self.changed(|workspace| Event::HostsChanged { workspace });
        Ok(())
    }
    fn probe_relay(&self, url: &str, token: Option<&zeroize::Zeroizing<String>>) -> ProbeFact {
        if url.contains("localhost") || url.contains("127.0.0.1") {
            ProbeFact::Unreachable("connection refused".into())
        } else if url.contains("private") && token.is_none() {
            ProbeFact::Unauthorized
        } else {
            ProbeFact::Ok
        }
    }
    fn switch_relay(&mut self, url: &str, _token: Option<&zeroize::Zeroizing<String>>) -> OpsResult<()> {
        self.with(|w| w.relay = Some(url.to_string())).ok_or_else(|| OpsError::Failed("create or join a workspace first".into()))
    }
}

/// `@handle` as a whole word.
fn mentions(lower: &str, handle: &str) -> bool {
    let needle = format!("@{}", handle.to_lowercase());
    lower.match_indices(&needle).any(|(i, _)| {
        let end = i + needle.len();
        lower[end..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn ws() -> WorkspaceId {
        WorkspaceId::new("ws-northwind")
    }

    #[tokio::test]
    async fn agent_reply_streams_and_files_a_proposal() {
        let b = FakeBackend::demo(FakeConfig::instant());
        let mut ev = b.events();
        b.send_message(&ws(), &ChatId::new("c-checkout"), "@coder please fix the flaky test").await.unwrap();
        let mut deltas = String::new();
        let mut saw_typing = false;
        loop {
            match ev.next().await.unwrap() {
                Event::Thinking { on: true, agent, .. } => saw_typing = agent == "@coder",
                Event::MessageDelta { delta, .. } => deltas.push_str(&delta),
                Event::ProposalChanged { proposal, .. } => {
                    assert_eq!(proposal.title, "Inject a clock into the session check");
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_typing);
        assert!(deltas.contains("Filed it as a proposal"));
        let t = b.transcript(&ws(), &ChatId::new("c-checkout")).await.unwrap();
        assert!(!t.last().unwrap().streaming);
    }

    #[tokio::test]
    async fn untagged_message_goes_to_the_default_agent_human_mention_to_nobody() {
        let b = FakeBackend::demo(FakeConfig::instant());
        let mut ev = b.events();
        b.send_message(&ws(), &ChatId::new("c-notes"), "@Ada thoughts?").await.unwrap();
        b.send_message(&ws(), &ChatId::new("c-notes"), "what's next").await.unwrap();
        loop {
            if let Event::Thinking { on: true, agent, .. } = ev.next().await.unwrap() {
                assert_eq!(agent, "@ligus");
                break;
            }
        }
    }

    #[tokio::test]
    async fn rank_rules_refuse_with_reasons() {
        let b = FakeBackend::demo(FakeConfig::instant());
        let ms = b.members(&ws()).await.unwrap();
        let me = ms.iter().find(|m| m.you).unwrap();
        assert!(me.founder);
        let ada = ms.iter().find(|m| m.name == "Ada").unwrap();
        assert_eq!(ada.can_remove, Allowed::Yes);
        let sff = ms.iter().find(|m| m.service).unwrap();
        let admin = sff.can_set_role.iter().find(|(r, _)| *r == Role::Admin).unwrap();
        assert_eq!(admin.1, Allowed::No("a service member holds at most Contributor".into()));
        // In the other workspace I'm a Viewer.
        let ms = b.members(&WorkspaceId::new("ws-docs")).await.unwrap();
        let jo = ms.iter().find(|m| m.name == "Jo").unwrap();
        assert_eq!(jo.can_remove, Allowed::No("nobody cuts the workspace's founder".into()));
        let me = ms.iter().find(|m| m.you).unwrap();
        assert_eq!(me.can_remove, Allowed::Yes, "leaving is always allowed");
    }

    #[tokio::test]
    async fn removal_waits_for_presence_and_cancel_aborts() {
        let b = FakeBackend::demo(FakeConfig::instant());
        let mut ev = b.events();
        let b2 = b.clone();
        let task = tokio::spawn(async move { b2.remove_member(&ws(), &MemberId::new("m-lin")).await });
        let Event::PresencePrompt(p) = ev.next().await.unwrap() else { panic!() };
        assert_eq!(p.kind, "memberRemoved");
        b.answer_presence(p.id, PresenceAnswer::Cancel).await.unwrap();
        assert_eq!(task.await.unwrap(), err("cancelled"));
        assert_eq!(b.members(&ws()).await.unwrap().len(), 5);

        let b2 = b.clone();
        let task = tokio::spawn(async move { b2.remove_member(&ws(), &MemberId::new("m-lin")).await });
        let p = loop {
            if let Event::PresencePrompt(p) = ev.next().await.unwrap() {
                break p;
            }
        };
        b.answer_presence(p.id, PresenceAnswer::Browser).await.unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(b.members(&ws()).await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn recovery_words_file_answers_presence() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w");
        std::fs::write(&p, "amber ".repeat(24)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let b = FakeBackend::demo(FakeConfig::instant());
        let mut ev = b.events();
        let b2 = b.clone();
        let task = tokio::spawn(async move { b2.set_role(&ws(), &MemberId::new("m-sam"), Role::Contributor).await });
        let Event::PresencePrompt(pr) = ev.next().await.unwrap() else { panic!() };
        b.answer_presence(pr.id, PresenceAnswer::RecoveryWordsFile(p.display().to_string())).await.unwrap();
        task.await.unwrap().unwrap();
        let sam = b.members(&ws()).await.unwrap().into_iter().find(|m| m.name == "Sam").unwrap();
        assert_eq!(sam.role, Role::Contributor);
    }

    #[tokio::test]
    async fn apply_on_host() {
        let b = FakeBackend::demo(FakeConfig::instant());
        let p = b.request_apply(&ws(), &ProposalId::new("p-2"), &HostId::new("h-sff")).await.unwrap();
        assert_eq!(p.apply, ApplyState::Waiting { host: "sff".into() });
        assert!(b.request_apply(&ws(), &ProposalId::new("p-2"), &HostId::new("h-sff")).await.is_err());
        assert_eq!(
            b.request_apply(&ws(), &ProposalId::new("p-1"), &HostId::new("h-sff")).await,
            err("only an approved proposal can be applied")
        );
    }

    #[test]
    fn mention_matching() {
        assert!(mentions("hi @coder!", "coder"));
        assert!(!mentions("hi @coders", "coder"));
        assert!(mentions("@all", "all"));
    }

    #[test]
    fn words() {
        assert_eq!(demo_words("x").len(), 24);
        assert_eq!(demo_words("x"), demo_words("x"));
    }
}
