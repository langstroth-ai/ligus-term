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
}

#[derive(Clone)]
struct MemberRec {
    id: MemberId,
    name: String,
    role: Role,
    founder: bool,
    service: bool,
    online: bool,
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
        };
        Self { world: Arc::new(Mutex::new(world)), tx, cfg }
    }

    /// A device with two workspaces, chats, proposals and people.
    pub fn demo(cfg: FakeConfig) -> Self {
        let b = Self::new(cfg);
        {
            let mut w = b.world.lock().unwrap();
            w.your_name = Some("Mara".into());
            w.settings.your_name = "Mara".into();
            seed_apiary(&mut w);
            seed_queens(&mut w);
            w.notifications.push(Notification {
                id: NotificationId::new("n1"),
                workspace: WorkspaceId::new("ws-apiary"),
                chat: Some(ChatId::new("c-log")),
                text: "Ada mentioned you in Inspection log refactor".into(),
                at: DEMO_NOW - 12 * MIN,
                read: false,
            });
        }
        b
    }

    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
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
                online: m.online,
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
            me.emit(Event::Typing { workspace: ws.clone(), chat: chat.clone(), who: who.clone(), on: true });
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
            me.emit(Event::Typing { workspace: ws.clone(), chat: chat.clone(), who, on: false });
            me.emit(Event::MessageAdded { workspace: ws.clone(), message: msg.clone() });
            let call = ToolCall { name: "read_file".into(), preview: "src/inspections.rs".into(), ok: true };
            me.emit(Event::ToolActivity { workspace: ws.clone(), chat: chat.clone(), message: msg.id.clone(), call: call.clone() });
            let reply = if files_proposal {
                format!(
                    "Looked at the inspection log. I changed the varroa threshold check and added a test.\n\nFiled it as a proposal for review, @{your_name}."
                )
            } else {
                "Here's what I found: the inspection log keeps one row per frame, so a colony summary needs a group-by. Want me to propose a change?".to_string()
            };
            for word in reply.split_inclusive(' ') {
                tokio::time::sleep(me.cfg.stream_delay).await;
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
                        title: "Tighten the varroa threshold check".into(),
                        author: format!("@{}", agent.handle),
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
                        wsd.diffs.insert(id, threshold_diff());
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
        "amber", "brood", "comb", "drone", "ember", "frame", "glade", "honey", "ivory", "jelly", "keel", "linden", "meadow", "nectar",
        "orchard", "pollen", "queen", "resin", "smoke", "thyme", "umber", "veil", "wax", "yarrow", "zephyr", "clover", "basket", "lantern",
        "harbor", "willow", "cedar", "maple",
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
    }
}

fn runtime(id: &str, kind: RuntimeKind, model: Option<&str>, configured: bool, active: bool) -> Runtime {
    Runtime { id: RuntimeId::new(id), label: kind.label().into(), kind, model: model.map(Into::into), configured, active }
}

fn member(id: &str, name: &str, role: Role, founder: bool, service: bool, online: bool) -> MemberRec {
    MemberRec { id: MemberId::new(id), name: name.into(), role, founder, service, online }
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

fn threshold_diff() -> Diff {
    Diff {
        files: vec![
            FileDiff {
                path: "src/inspections.rs".into(),
                added: 9,
                removed: 3,
                hunks: vec![
                    Hunk {
                        header: "@@ -12,7 +12,9 @@ pub struct Inspection {".into(),
                        lines: vec![
                            line_ctx("    pub colony: ColonyId,"),
                            line_ctx("    pub frames: Vec<FrameNote>,"),
                            line_del("    pub mites: u32,"),
                            line_add("    /// Mites per 100 bees, from an alcohol wash."),
                            line_add("    pub mites_per_100: f32,"),
                            line_add("    pub wash_sample: u32,"),
                            line_ctx("    pub queen_seen: bool,"),
                            line_ctx("}"),
                        ],
                    },
                    Hunk {
                        header: "@@ -40,9 +42,13 @@ impl Inspection {".into(),
                        lines: vec![
                            line_ctx("    pub fn needs_treatment(&self) -> bool {"),
                            line_del("        self.mites > 9"),
                            line_del("    }"),
                            line_add("        // 3 per 100 is the usual action threshold in late summer."),
                            line_add("        self.mites_per_100 >= TREATMENT_THRESHOLD"),
                            line_add("    }"),
                            line_add(""),
                            line_add("    pub const TREATMENT_THRESHOLD: f32 = 3.0;"),
                            line_ctx(""),
                            line_ctx("    pub fn summary(&self) -> String {"),
                        ],
                    },
                ],
            },
            FileDiff {
                path: "tests/inspections.rs".into(),
                added: 8,
                removed: 0,
                hunks: vec![Hunk {
                    header: "@@ -0,0 +1,8 @@".into(),
                    lines: vec![
                        line_add("use apiary::Inspection;"),
                        line_add(""),
                        line_add("#[test]"),
                        line_add("fn three_per_hundred_needs_treatment() {"),
                        line_add("    let i = Inspection::sample(3.0);"),
                        line_add("    assert!(i.needs_treatment());"),
                        line_add("}"),
                        line_add(""),
                    ],
                }],
            },
        ],
    }
}

fn seed_apiary(w: &mut World) {
    let id = WorkspaceId::new("ws-apiary");
    let mut ws = Ws {
        summary: Some(WorkspaceSummary {
            id: id.clone(),
            name: "Apiary Tracker".into(),
            mentioned: false,
            sync: SyncMode::Peer,
            your_role: Role::Owner,
            can_invite: Allowed::Yes,
        }),
        ..Ws::default()
    };
    ws.members = vec![
        member("m-you", "Mara", Role::Owner, true, false, true),
        member("m-ada", "Ada", Role::Admin, false, false, true),
        member("m-lin", "Lin", Role::Contributor, false, false, false),
        member("m-sam", "Sam", Role::Viewer, false, false, false),
        member("m-sff", "sff box", Role::Contributor, false, true, true),
    ];
    ws.runtimes = vec![
        runtime("claude-code", RuntimeKind::ClaudeCode, Some("default"), true, true),
        runtime("codex", RuntimeKind::Codex, None, true, false),
        runtime("ollama", RuntimeKind::Ollama, Some("qwen3-coder:30b"), true, false),
        runtime("copilot", RuntimeKind::Copilot, None, false, false),
    ];
    ws.agents = vec![
        agent("a-primary", "primary", "claude-code", None, Some("answers untagged messages")),
        agent("a-coder", "coder", "claude-code", Some("h-sff"), Some("implements proposals")),
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
        },
        Host {
            id: HostId::new("h-sff"),
            label: "sff".into(),
            kind: HostKind::Worker,
            online: true,
            last_seen: Some(DEMO_NOW - MIN),
            this_device: false,
        },
        Host {
            id: HostId::new("h-studio"),
            label: "ada-studio".into(),
            kind: HostKind::Device,
            online: false,
            last_seen: Some(DEMO_NOW - 3 * 60 * MIN),
            this_device: false,
        },
    ];
    ws.chats = vec![
        chat("c-log", "Inspection log refactor", Some("dev"), DEMO_NOW - 12 * MIN, true, true),
        chat("c-alerts", "Queen cell alerts", Some("dev"), DEMO_NOW - 50 * MIN, false, false),
        chat("c-notes", "Release notes", None, DEMO_NOW - 26 * 60 * MIN, false, false),
    ];
    ws.chats[1].lockdown =
        Some(Lockdown { reasons: vec!["prompt-injection marker in a tool result (fetch_url)".into()], clear: Allowed::Yes });
    ws.messages.insert(
        ChatId::new("c-log"),
        vec![
            msg(
                "m1",
                "c-log",
                human("m-ada", "Ada"),
                "The inspection log stores raw mite counts. Can we switch to mites per 100 bees?",
                DEMO_NOW - 40 * MIN,
            ),
            msg(
                "m2",
                "c-log",
                bot("coder", "claude-code"),
                "Yes. The wash sample size is already recorded, so it's a small change. I'll propose it.",
                DEMO_NOW - 38 * MIN,
            ),
            msg(
                "m3",
                "c-log",
                human("m-ada", "Ada"),
                "@Mara can you review the threshold? 3 per 100 is what the club uses.",
                DEMO_NOW - 12 * MIN,
            ),
        ],
    );
    ws.messages.insert(
        ChatId::new("c-alerts"),
        vec![
            msg("m4", "c-alerts", human("m-lin", "Lin"), "@scout check the forum post about swarm cells", DEMO_NOW - 55 * MIN),
            Message {
                tools: vec![ToolCall { name: "fetch_url".into(), preview: "https://forum.example/swarm-cells".into(), ok: true }],
                ..msg(
                    "m5",
                    "c-alerts",
                    bot("scout", "ollama"),
                    "The page contained instructions aimed at agents, so this chat is locked down.",
                    DEMO_NOW - 50 * MIN,
                )
            },
        ],
    );
    ws.messages.insert(
        ChatId::new("c-notes"),
        vec![msg(
            "m6",
            "c-notes",
            human("m-you", "Mara"),
            "Draft notes for 0.4: frame-level notes, export to CSV.",
            DEMO_NOW - 26 * 60 * MIN,
        )],
    );
    let p1 = ProposalId::new("p-1");
    ws.proposals = vec![
        Proposal {
            id: p1.clone(),
            chat: Some(ChatId::new("c-log")),
            title: "Record mites per 100 bees".into(),
            author: "@coder".into(),
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
            title: "Export inspections to CSV".into(),
            author: "@coder".into(),
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
            title: "Bump the frame schema version".into(),
            author: "@reviewer".into(),
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
    ws.diffs.insert(p1, threshold_diff());
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
                        line_ctx("use crate::Inspection;"),
                        line_add("pub fn to_csv(rows: &[Inspection]) -> String {"),
                        line_add("    rows.iter().map(Inspection::csv_row).collect::<Vec<_>>().join(\"\\n\")"),
                        line_add("}"),
                        line_add(""),
                    ],
                }],
            }],
        },
    );
    ws.diffs.insert(ProposalId::new("p-3"), Diff { files: vec![] });
    w.order.push(id.clone());
    w.ws.insert(id, ws);
}

fn seed_queens(w: &mut World) {
    let id = WorkspaceId::new("ws-queens");
    let mut ws = Ws {
        summary: Some(WorkspaceSummary {
            id: id.clone(),
            name: "queen-rearing".into(),
            mentioned: true,
            sync: SyncMode::Relay("https://relay.example.org".into()),
            your_role: Role::Viewer,
            can_invite: Allowed::No("only an Admin or Owner issues invites".into()),
        }),
        ..Ws::default()
    };
    ws.members = vec![member("m-jo", "Jo", Role::Owner, true, false, true), member("m-you", "Mara", Role::Viewer, false, false, true)];
    ws.runtimes = vec![runtime("claude-code", RuntimeKind::ClaudeCode, None, true, true)];
    ws.agents = vec![agent("a-primary", "primary", "claude-code", None, None)];
    ws.hosts = vec![Host {
        id: HostId::new("h-this"),
        label: "This device".into(),
        kind: HostKind::Device,
        online: true,
        last_seen: Some(DEMO_NOW),
        this_device: true,
    }];
    ws.chats = vec![chat("c-graft", "Grafting calendar", None, DEMO_NOW - 5 * MIN, true, true)];
    ws.chats[0].lockdown =
        Some(Lockdown { reasons: vec!["locked by a member".into()], clear: Allowed::No("a member can clear it".into()) });
    ws.messages.insert(
        ChatId::new("c-graft"),
        vec![msg("g1", "c-graft", human("m-jo", "Jo"), "@Mara graft day moves to Saturday.", DEMO_NOW - 5 * MIN)],
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
            members: vec![MemberRec {
                id: you,
                name: your_name.trim().into(),
                role: Role::Owner,
                founder: true,
                service: false,
                online: true,
            }],
            runtimes: vec![runtime("claude-code", RuntimeKind::ClaudeCode, None, true, true)],
            agents: vec![agent("a-primary", "primary", "claude-code", None, Some("answers untagged messages"))],
            hosts: vec![Host {
                id: HostId::new("h-this"),
                label: "This device".into(),
                kind: HostKind::Device,
                online: true,
                last_seen: Some(at),
                this_device: true,
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
                member("m-inviter", "Inviter", Role::Owner, true, false, true),
                MemberRec { id: you, name: your_name.trim().into(), role: Role::Contributor, founder: false, service: false, online: true },
            ],
            agents: vec![agent("a-primary", "primary", "claude-code", None, None)],
            runtimes: vec![runtime("claude-code", RuntimeKind::ClaudeCode, None, true, true)],
            hosts: vec![Host {
                id: HostId::new("h-this"),
                label: "This device".into(),
                kind: HostKind::Device,
                online: true,
                last_seen: Some(at),
                this_device: true,
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
        c.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
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
                responders.extend(ws.agents.iter().find(|a| a.handle == "primary").cloned());
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
            Mentionable { handle: "primary".into(), kind: "Default agent".into() },
            Mentionable { handle: "all".into(), kind: "Everyone".into() },
        ];
        out.extend(
            ws.agents.iter().filter(|a| a.handle != "primary").map(|a| Mentionable { handle: a.handle.clone(), kind: "Agent".into() }),
        );
        out.extend(ws.members.iter().map(|m| Mentionable { handle: m.name.replace(' ', ""), kind: m.role.label().into() }));
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
            p.clone()
        };
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

    async fn answer_presence(&self, id: PresenceId, answer: PresenceAnswer) -> Result<()> {
        let tx = self.lock().presence.get(&id).cloned();
        match tx {
            Some(tx) => tx.send(answer).map_err(|_| super::BackendError("that prompt has ended".into())),
            None => err("that prompt has ended"),
        }
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
        WorkspaceId::new("ws-apiary")
    }

    #[tokio::test]
    async fn agent_reply_streams_and_files_a_proposal() {
        let b = FakeBackend::demo(FakeConfig::instant());
        let mut ev = b.events();
        b.send_message(&ws(), &ChatId::new("c-log"), "@coder please fix the threshold").await.unwrap();
        let mut deltas = String::new();
        let mut saw_typing = false;
        loop {
            match ev.next().await.unwrap() {
                Event::Typing { on: true, who, .. } => saw_typing = who == "@coder",
                Event::MessageDelta { delta, .. } => deltas.push_str(&delta),
                Event::ProposalChanged { proposal, .. } => {
                    assert_eq!(proposal.title, "Tighten the varroa threshold check");
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_typing);
        assert!(deltas.contains("Filed it as a proposal"));
        let t = b.transcript(&ws(), &ChatId::new("c-log")).await.unwrap();
        assert!(!t.last().unwrap().streaming);
    }

    #[tokio::test]
    async fn untagged_message_goes_to_primary_human_mention_to_nobody() {
        let b = FakeBackend::demo(FakeConfig::instant());
        let mut ev = b.events();
        b.send_message(&ws(), &ChatId::new("c-notes"), "@Ada thoughts?").await.unwrap();
        b.send_message(&ws(), &ChatId::new("c-notes"), "what's next").await.unwrap();
        loop {
            if let Event::Typing { on: true, who, .. } = ev.next().await.unwrap() {
                assert_eq!(who, "@primary");
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
        let ms = b.members(&WorkspaceId::new("ws-queens")).await.unwrap();
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
