//! `lt --plain`: a line-oriented client for screen readers and scripts.
//!
//! It reads one command per line on stdin and writes one record per line
//! on stdout: a keyword, then tab-separated fields, text last (newlines in
//! text are written as `\n`). Backend events print as they arrive, so a
//! screen reader hears new messages and passkey prompts without a screen to
//! scan. `help` lists the commands; EOF waits for running actions, then
//! exits.
//!
//! One-shot: `lt --plain COMMAND [ARGS…]` runs one command and exits, for
//! scripts and other agents. It prints only that command's records (and any
//! passkey prompt it raises), and exits 0 on success, 1 when it printed an
//! `error` record (or a run it followed didn't complete), 2 for a command
//! it doesn't know. `run` and `watch` follow the run until it ends: this
//! device drives the runs it starts, so `run` stays until its run is done.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::backend::model::*;
use crate::backend::{Backend, Event};

pub const HELP: &str = "\
commands (N is a number from the last list, or an id):
  name TEXT            set your name (first run)
  create NAME          create a workspace (no server needed)
  join CODE            join with an invite code
  workspaces           list workspaces          use N        switch workspace
  chats                list chats               open N       open a chat and print it
  new TITLE            start a chat             say TEXT     post to the open chat
  lock / unlock        lock the open chat down / clear its lockdown
  proposals            list proposals           diff N       print a proposal's diff
  approve N / reject N vote                     apply N [H]  apply on host H (default: first online worker)
  agents / runtimes / hosts / members / notifications / settings
  role M ROLE          change a member's role   remove M     remove a member
  invite               create an invite code    admit [REQUEST]  admit a box (ligusjoin1:, or none: a bearer code)
  check CODE           who an invite is from    connect RELAY ROOM KEY [TOKEN]  found a deployment's workspace
                       (KEY and TOKEN may be @FILE to read them from a file)
  workflows            list workflows           runs         list runs
  run W [INPUT]        start workflow W and follow it until it ends
  start W [INPUT]      start workflow W         watch R      follow run R until it ends
  gate R approve|reject  decide run R's waiting gate      cancel-run R  cancel run R
  tasks / memory / handoffs  agents' task state, workspace memory, hand-offs
  checkout [PATH|none] show or set this device's checkout for apply on host
  confirm ID           answer a passkey prompt in the browser
  confirm ID file PATH answer it with a recovery-words file
  cancel ID            cancel a passkey prompt
  wait [SECS]          wait until events go quiet (default up to 30 s)
  help / quit";

/// Escape a field: tabs become spaces, newlines `\n`.
fn esc(s: &str) -> String {
    s.replace('\t', " ").replace('\\', "\\\\").replace('\n', "\\n")
}

struct Session {
    ws: Option<WorkspaceId>,
    chat: Option<ChatId>,
    your_name: Option<String>,
    workspaces: Vec<WorkspaceSummary>,
    chats: Vec<ChatSummary>,
    proposals: Vec<Proposal>,
    members: Vec<Member>,
    hosts: Vec<Host>,
    workflows: Vec<Workflow>,
    runs: Vec<Run>,
}

struct Output {
    w: Mutex<Box<dyn AsyncWrite + Unpin + Send>>,
    /// An `error` record was printed (the one-shot exit code).
    failed: std::sync::atomic::AtomicBool,
}

type Out = Arc<Output>;

async fn emit(out: &Out, line: String) {
    if line.starts_with("error\t") {
        out.failed.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    let mut o = out.w.lock().await;
    let _ = o.write_all(line.as_bytes()).await;
    let _ = o.write_all(b"\n").await;
    let _ = o.flush().await;
}

/// How plain mode runs.
#[derive(Debug, Clone, Default)]
pub struct PlainOpts {
    /// Run this one command and exit (`lt --plain COMMAND ARGS…`).
    pub one_shot: Option<String>,
    /// The workspace to use (a name, an id, or a number), else the first.
    pub workspace: Option<String>,
    /// Answer every passkey prompt this way (one-shot: the browser, or a
    /// recovery-words file).
    pub presence: Option<PresenceAnswer>,
}

/// The exit code for a usage error (an unknown one-shot command).
pub const EXIT_USAGE: i32 = 2;

const COMMANDS: &[&str] = &[
    "help",
    "name",
    "create",
    "join",
    "check",
    "connect",
    "workspaces",
    "use",
    "chats",
    "open",
    "new",
    "say",
    "lock",
    "unlock",
    "proposals",
    "diff",
    "approve",
    "reject",
    "apply",
    "agents",
    "runtimes",
    "hosts",
    "members",
    "role",
    "remove",
    "invite",
    "admit",
    "notifications",
    "settings",
    "confirm",
    "cancel",
    "workflows",
    "runs",
    "run",
    "start",
    "watch",
    "gate",
    "cancel-run",
    "tasks",
    "memory",
    "handoffs",
    "checkout",
];

fn run_line(i: usize, r: &Run) -> String {
    let done = r.stages.iter().filter(|s| s.status == StageStatus::Succeeded).count();
    format!(
        "run\t{}\t{}\t{}\t{}\t{done}/{}\t{}\t{}",
        i + 1,
        r.id,
        r.status.label(),
        esc(&r.name),
        r.stages.len(),
        esc(r.driver.as_deref().unwrap_or("")),
        esc(&r.input)
    )
}

fn stage_lines(r: &Run) -> Vec<String> {
    r.stages
        .iter()
        .map(|st| {
            let note = if st.error.is_empty() { &st.excerpt } else { &st.error };
            format!(
                "stage\t{}\t{}\t{}\t{}\t{}\t{}",
                r.id,
                st.id,
                st.status.label(),
                esc(&st.what),
                st.gate.as_ref().map(|g| g.0.as_str()).unwrap_or(""),
                esc(note)
            )
        })
        .collect()
}

/// Format a backend event as a line; `None` for events plain mode skips
/// (streaming deltas: the whole message prints when it is done).
pub fn event_line(e: &Event, names: &dyn Fn(&WorkspaceId) -> String) -> Option<String> {
    Some(match e {
        Event::MessageAdded { workspace, message } if !message.streaming => {
            format!("message\t{}\t{}\t{}\t{}", esc(&names(workspace)), message.chat, esc(&message.author.display()), esc(&message.text))
        }
        Event::MessageAdded { .. } | Event::MessageDelta { .. } => return None,
        Event::MessageDone { error: Some(e), chat, .. } => format!("error\t{chat}\t{}", esc(e)),
        Event::MessageDone { .. } => return None,
        Event::ToolActivity { chat, call, .. } => format!("tool\t{chat}\t{}\t{}", esc(&call.name), esc(&call.preview)),
        Event::Thinking { chat, agent, on: true, .. } => format!("thinking\t{chat}\t{}", esc(agent)),
        Event::Thinking { .. } => return None,
        Event::ProposalChanged { proposal: p, .. } => {
            format!("proposal-changed\t{}\t{}\t{}/{}\t{}", p.id, p.status.label(), p.approvals, p.quorum, esc(&p.title))
        }
        Event::Notification(n) => format!("notification\t{}\t{}", esc(&names(&n.workspace)), esc(&n.text)),
        Event::PresencePrompt(p) => {
            let how = if p.op == PresenceOp::Get { "confirm ID | confirm ID file PATH | cancel ID" } else { "confirm ID | cancel ID" };
            format!("presence\t{}\t{}\t{}\t{}\t{how}", p.id.0, esc(&p.kind), p.digest_short, esc(&p.summary))
        }
        Event::PresenceProblem { id, error } => format!("presence-problem\t{}\t{}", id.0, esc(error)),
        Event::PresenceDone { id, ok, error } => {
            format!("presence-done\t{}\t{}\t{}", id.0, if *ok { "ok" } else { "failed" }, esc(error.as_deref().unwrap_or("")))
        }
        Event::Sync { workspace, status } => format!("sync\t{}\t{}", esc(&names(workspace)), esc(status)),
        Event::RunChanged { run: r, .. } => {
            let done = r.stages.iter().filter(|s| s.status == StageStatus::Succeeded).count();
            format!("run-changed\t{}\t{}\t{}\t{done}/{}", r.id, r.status.label(), esc(&r.name), r.stages.len())
        }
        Event::WorkflowsChanged { .. } | Event::MemoryChanged { .. } => return None,
        Event::ChatsChanged { .. }
        | Event::MembersChanged { .. }
        | Event::HostsChanged { .. }
        | Event::AgentsChanged { .. }
        | Event::WorkspacesChanged => {
            return None;
        }
    })
}

/// Run plain mode until EOF or `quit`.
pub async fn run<R>(backend: Arc<dyn Backend>, input: R, out: Box<dyn AsyncWrite + Unpin + Send>) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
{
    run_with(backend, input, out, PlainOpts::default()).await.map(|_| ())
}

/// Run plain mode; the exit code (see the module docs).
pub async fn run_with<R>(
    backend: Arc<dyn Backend>,
    input: R,
    out: Box<dyn AsyncWrite + Unpin + Send>,
    opts: PlainOpts,
) -> anyhow::Result<i32>
where
    R: AsyncBufRead + Unpin,
{
    let out: Out = Arc::new(Output { w: Mutex::new(out), failed: std::sync::atomic::AtomicBool::new(false) });
    let one_shot = opts.one_shot.is_some();
    if let Some(cmd) = &opts.one_shot {
        let name = cmd.split_whitespace().next().unwrap_or("");
        if !COMMANDS.contains(&name) {
            emit(&out, format!("error\tunknown command {name}: lt --plain help lists them")).await;
            return Ok(EXIT_USAGE);
        }
    }
    let d = backend.device();
    if !one_shot {
        emit(&out, format!("device\t{}\t{}\t{}", esc(&d.device_name), esc(&d.data_dir), d.backend_label)).await;
    }

    let st = backend.startup().await.map_err(|e| anyhow::anyhow!(e))?;
    let chosen = match &opts.workspace {
        Some(w) => match pick(&st.workspaces, w, |x| x.id.as_str()).or_else(|| st.workspaces.iter().find(|x| x.name == *w)) {
            Some(x) => Some(x.id.clone()),
            None => {
                emit(&out, format!("error\tno workspace {}", esc(w))).await;
                return Ok(1);
            }
        },
        None => st.workspaces.first().map(|w| w.id.clone()),
    };
    let session = Arc::new(Mutex::new(Session {
        ws: chosen,
        chat: None,
        your_name: st.your_name.clone(),
        workspaces: st.workspaces.clone(),
        chats: vec![],
        proposals: vec![],
        members: vec![],
        hosts: vec![],
        workflows: vec![],
        runs: vec![],
    }));
    if !one_shot {
        if st.workspaces.is_empty() {
            emit(&out, "first-run\tset your name with: name TEXT; then: create NAME or join CODE".into()).await;
        } else {
            for (i, w) in st.workspaces.iter().enumerate() {
                emit(&out, ws_line(i, w)).await;
            }
        }
        emit(&out, "ready\ttype help for commands".into()).await;
    }

    // Events print as they come; `last_event` lets `wait` see quiet.
    let last_event = Arc::new(std::sync::Mutex::new(tokio::time::Instant::now()));
    let presence_answer = opts.presence.clone().or_else(|| one_shot.then_some(PresenceAnswer::Browser));
    let ev_task = {
        let (out, backend, session, last) = (out.clone(), backend.clone(), session.clone(), last_event.clone());
        let presence_answer = presence_answer.clone();
        tokio::spawn(async move {
            let mut events = backend.events();
            // Streaming replies are collected and printed whole when done.
            let mut streaming: std::collections::HashMap<MessageId, (WorkspaceId, Message)> = Default::default();
            while let Some(e) = events.next().await {
                *last.lock().unwrap() = tokio::time::Instant::now();
                let names = { session.lock().await.workspaces.clone() };
                let name_of = |id: &WorkspaceId| names.iter().find(|w| &w.id == id).map(|w| w.name.clone()).unwrap_or_else(|| id.0.clone());
                let e = match e {
                    Event::MessageAdded { workspace, message } if message.streaming => {
                        streaming.insert(message.id.clone(), (workspace, message));
                        continue;
                    }
                    Event::MessageDelta { message, delta, .. } => {
                        if let Some((_, m)) = streaming.get_mut(&message) {
                            m.text.push_str(&delta);
                        }
                        continue;
                    }
                    Event::MessageDone { message, error: None, .. } => match streaming.remove(&message) {
                        Some((workspace, mut message)) => {
                            message.streaming = false;
                            Event::MessageAdded { workspace, message }
                        }
                        None => continue,
                    },
                    other => other,
                };
                // One-shot prints only what the command asked for, and the
                // passkey prompts it raises (answered as the options say).
                if let Event::PresencePrompt(p) = &e
                    && let Some(a) = presence_answer.clone()
                {
                    let answer = if p.op == PresenceOp::Create { PresenceAnswer::Browser } else { a };
                    let _ = backend.answer_presence(p.id, answer).await;
                }
                let quiet = one_shot && !matches!(e, Event::PresencePrompt(_) | Event::PresenceProblem { .. } | Event::PresenceDone { .. });
                if !quiet && let Some(line) = event_line(&e, &name_of) {
                    emit(&out, line).await;
                }
            }
        })
    };

    if let Some(cmd) = opts.one_shot {
        command(&backend, &out, &session, &cmd).await;
        ev_task.abort();
        return Ok(i32::from(out.failed.load(std::sync::atomic::Ordering::SeqCst)));
    }
    let mut actions = tokio::task::JoinSet::new();
    let mut lines = input.lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "quit" || line == "exit" {
            break;
        }
        if let Some(rest) = line.strip_prefix("wait") {
            let secs: u64 = rest.trim().parse().unwrap_or(30);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
            while let Some(r) = tokio::time::timeout_at(deadline, actions.join_next()).await.ok().flatten() {
                let _ = r;
            }
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let quiet = last_event.lock().unwrap().elapsed() >= Duration::from_millis(500);
                if quiet || tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
            continue;
        }
        let (b, o, s) = (backend.clone(), out.clone(), session.clone());
        // Actions that may wait on a passkey run in the background so
        // `confirm`/`cancel` can still be typed.
        let background =
            ["role ", "remove ", "unlock", "create ", "approve ", "reject ", "admit", "connect ", "run ", "watch ", "gate ", "cancel-run "]
                .iter()
                .any(|p| line.starts_with(p));
        if background {
            actions.spawn(async move { command(&b, &o, &s, &line).await });
        } else {
            command(&b, &o, &s, &line).await;
        }
    }
    while actions.join_next().await.is_some() {}
    ev_task.abort();
    Ok(0)
}

/// Print a run and its stages, then each change, until it ends; an
/// unfinished run is an error (the exit code).
async fn follow(
    b: &Arc<dyn Backend>,
    out: &Out,
    w: &WorkspaceId,
    mut r: Run,
    mut events: futures::stream::BoxStream<'static, crate::backend::Event>,
) {
    let print = |r: Run| async move {
        emit(out, run_line(0, &r)).await;
        for l in stage_lines(&r) {
            emit(out, l).await;
        }
    };
    // The latest state, in case it moved before we subscribed.
    if let Ok(now) = b.runs(w).await
        && let Some(x) = now.into_iter().find(|x| x.id == r.id)
    {
        r = x;
    }
    print(r.clone()).await;
    let mut last = format!("{:?}", r);
    while !r.status.is_terminal() {
        // Events as they come, and a look every few seconds besides.
        let next = tokio::select! {
            e = events.next() => match e {
                Some(crate::backend::Event::RunChanged { workspace, run }) if workspace == *w && run.id == r.id => Some(run),
                Some(_) => None,
                None => break,
            },
            _ = tokio::time::sleep(Duration::from_secs(2)) => b.runs(w).await.ok().and_then(|x| x.into_iter().find(|x| x.id == r.id)),
        };
        if let Some(run) = next {
            let key = format!("{run:?}");
            if key != last {
                last = key;
                r = run;
                print(r.clone()).await;
            }
        }
    }
    if r.status == RunStatus::Completed {
        emit(out, format!("run-done\t{}\t{}", r.id, r.status.label())).await;
    } else {
        emit(out, format!("error\trun {} {}", r.id, r.status.label())).await;
    }
}

fn ws_line(i: usize, w: &WorkspaceSummary) -> String {
    format!("workspace\t{}\t{}\t{}\t{}\t{}", i + 1, w.id, esc(&w.name), w.your_role.label(), w.sync.label())
}

fn pick<'a, T>(items: &'a [T], arg: &str, id: impl Fn(&T) -> &str) -> Option<&'a T> {
    let arg = arg.trim();
    if let Ok(n) = arg.parse::<usize>() {
        return n.checked_sub(1).and_then(|i| items.get(i));
    }
    items.iter().find(|x| id(x) == arg)
}

async fn command(b: &Arc<dyn Backend>, out: &Out, s: &Arc<Mutex<Session>>, line: &str) {
    let (cmd, arg) = line.split_once(' ').map(|(c, a)| (c, a.trim())).unwrap_or((line, ""));
    let say = |l: String| emit(out, l);
    let fail = |e: String| emit(out, format!("error\t{}", esc(&e)));
    let ws = { s.lock().await.ws.clone() };
    let need_ws = || async {
        if ws.is_none() {
            fail("no workspace yet: create NAME or join CODE".into()).await;
        }
    };
    match cmd {
        "help" => {
            for l in HELP.lines() {
                say(format!("help\t{l}")).await;
            }
        }
        "name" => {
            s.lock().await.your_name = Some(arg.to_string());
            say(format!("ok\tname\t{}", esc(arg))).await;
        }
        "create" | "join" => {
            let your = { s.lock().await.your_name.clone().unwrap_or_default() };
            if cmd == "create" {
                match b.create_workspace(&your, arg).await {
                    Ok(c) => {
                        say(format!("created\t{}\t{}", c.workspace.id, esc(&c.workspace.name))).await;
                        say(format!("recovery-key\t{}\twrite these 24 words down; they are shown once", c.recovery_words.join(" "))).await;
                        if let Some(i) = &c.invite {
                            say(format!("invite\t{i}")).await;
                        }
                        let _ = b.confirm_recovery_saved(&c.workspace.id).await;
                        let mut g = s.lock().await;
                        g.ws = Some(c.workspace.id.clone());
                        g.workspaces.push(c.workspace);
                    }
                    Err(e) => fail(e.0).await,
                }
            } else {
                match b.join_workspace(&your, arg).await {
                    Ok(w) => {
                        say(format!("joined\t{}\t{}", w.id, esc(&w.name))).await;
                        let mut g = s.lock().await;
                        g.ws = Some(w.id.clone());
                        g.workspaces.push(w);
                    }
                    Err(e) => fail(e.0).await,
                }
            }
        }
        "workspaces" => match b.workspaces().await {
            Ok(w) => {
                for (i, x) in w.iter().enumerate() {
                    say(ws_line(i, x)).await;
                }
                s.lock().await.workspaces = w;
            }
            Err(e) => fail(e.0).await,
        },
        "use" => {
            let mut g = s.lock().await;
            let Some(w) = pick(&g.workspaces, arg, |w| w.id.as_str()).cloned() else {
                drop(g);
                return fail(format!("no workspace {arg}")).await;
            };
            g.ws = Some(w.id.clone());
            g.chat = None;
            drop(g);
            say(format!("using\t{}\t{}", w.id, esc(&w.name))).await;
        }
        "check" => match b.inspect_invite(arg).await {
            Ok(c) => {
                say(format!(
                    "invite-check\t{}\t{}\t{}\t{}\t{}\t{}",
                    c.trust.label(),
                    esc(&c.workspace_name),
                    esc(c.founder_name.as_deref().unwrap_or("")),
                    c.safety_code.as_deref().unwrap_or(""),
                    esc(&c.relay_url),
                    c.trust.describe()
                ))
                .await
            }
            Err(e) => fail(e.0).await,
        },
        "connect" => {
            let parts: Vec<&str> = arg.split_whitespace().collect();
            let [relay, room, key, rest @ ..] = parts.as_slice() else {
                return fail("usage: connect RELAY ROOM KEY [TOKEN] (KEY and TOKEN may be @FILE)".into()).await;
            };
            let read = |v: &str| -> std::result::Result<String, String> {
                match v.strip_prefix('@') {
                    Some(path) => std::fs::read_to_string(path).map(|t| t.trim().to_string()).map_err(|e| format!("reading {path}: {e}")),
                    None => Ok(v.to_string()),
                }
            };
            let (key, token) = match (read(key), rest.first().map(|t| read(t)).transpose()) {
                (Ok(k), Ok(t)) => (k, t),
                (Err(e), _) | (_, Err(e)) => return fail(e).await,
            };
            let your = { s.lock().await.your_name.clone().unwrap_or_default() };
            let spec = DeploymentSpec { name: String::new(), relay_url: relay.to_string(), room: room.to_string(), key, token };
            match b.connect_deployment(&your, spec).await {
                Ok(c) => {
                    say(format!("connected\t{}\t{}\t{}", c.workspace.id, esc(&c.workspace.name), c.workspace.sync.label())).await;
                    say(format!("recovery-key\t{}\twrite these 24 words down; they are shown once", c.recovery_words.join(" "))).await;
                    let _ = b.confirm_recovery_saved(&c.workspace.id).await;
                    let mut g = s.lock().await;
                    g.ws = Some(c.workspace.id.clone());
                    g.workspaces.push(c.workspace);
                }
                Err(e) => fail(e.0).await,
            }
        }
        _ if ws.is_none() && cmd != "notifications" && cmd != "settings" && cmd != "confirm" && cmd != "cancel" => need_ws().await,
        "admit" => match b.admit_box(ws.as_ref().unwrap(), Some(arg).filter(|a| !a.is_empty())).await {
            Ok(a) => {
                say(format!(
                    "admission\t{}\t{}\t{}\t{}",
                    if a.sealed { "sealed" } else { "bearer" },
                    a.fingerprint.as_deref().unwrap_or(""),
                    esc(a.label.as_deref().unwrap_or("")),
                    a.code
                ))
                .await
            }
            Err(e) => fail(e.0).await,
        },
        "workflows" => match b.workflows(ws.as_ref().unwrap()).await {
            Ok(w) => {
                for (i, d) in w.iter().enumerate() {
                    let start = match &d.can_start {
                        Allowed::Yes => "startable".to_string(),
                        Allowed::No(r) => format!("refused ({r})"),
                    };
                    let stages: Vec<String> = d.stages.iter().map(|st| format!("{}:{}", st.id, st.what)).collect();
                    say(format!(
                        "workflow\t{}\t{}\t{}\t{}\t{}\t{}",
                        i + 1,
                        d.id,
                        esc(&d.name),
                        esc(&d.chat_title),
                        esc(&stages.join(" ")),
                        esc(&start)
                    ))
                    .await;
                }
                s.lock().await.workflows = w;
            }
            Err(e) => fail(e.0).await,
        },
        "runs" => match b.runs(ws.as_ref().unwrap()).await {
            Ok(r) => {
                for (i, x) in r.iter().enumerate() {
                    say(run_line(i, x)).await;
                }
                s.lock().await.runs = r;
            }
            Err(e) => fail(e.0).await,
        },
        "run" | "start" => {
            let w = ws.unwrap();
            if s.lock().await.workflows.is_empty()
                && let Ok(x) = b.workflows(&w).await
            {
                s.lock().await.workflows = x;
            }
            let (which, input) = arg.split_once(' ').map(|(a, b)| (a, b.trim())).unwrap_or((arg, ""));
            let def = {
                let g = s.lock().await;
                pick(&g.workflows, which, |d| d.id.as_str()).or_else(|| g.workflows.iter().find(|d| d.name == which)).cloned()
            };
            let Some(def) = def else { return fail(format!("no workflow {which}")).await };
            // Follow it from before it starts, so no change is missed.
            let events = b.events();
            match b.start_run(&w, &def.id, &input.replace("\\n", "\n")).await {
                Ok(r) => {
                    say(run_line(0, &r)).await;
                    if cmd == "run" {
                        follow(b, out, &w, r, events).await;
                    }
                }
                Err(e) => fail(e.0).await,
            }
        }
        "watch" | "gate" | "cancel-run" => {
            let w = ws.unwrap();
            let mut parts = arg.split_whitespace();
            let which = parts.next().unwrap_or("");
            let events = b.events();
            let runs = match b.runs(&w).await {
                Ok(r) => r,
                Err(e) => return fail(e.0).await,
            };
            s.lock().await.runs = runs.clone();
            let Some(r) = pick(&runs, which, |r| r.id.as_str()).cloned() else { return fail(format!("no run {which}")).await };
            match cmd {
                "watch" => follow(b, out, &w, r, events).await,
                "gate" => {
                    let approve = match parts.next() {
                        Some("approve") => true,
                        Some("reject") => false,
                        _ => return fail("usage: gate RUN approve|reject".into()).await,
                    };
                    let Some(stage) = r.stages.iter().find(|st| st.status == StageStatus::AwaitingApproval) else {
                        return fail(format!("run {} isn't waiting on a gate", r.id)).await;
                    };
                    match b.decide_gate(&w, &r.id, &stage.id, approve).await {
                        Ok(r) => say(format!("gate\t{}\t{}\t{}", r.id, stage.id, if approve { "approved" } else { "rejected" })).await,
                        Err(e) => fail(e.0).await,
                    }
                }
                _ => match b.cancel_run(&w, &r.id).await {
                    Ok(r) => say(run_line(0, &r)).await,
                    Err(e) => fail(e.0).await,
                },
            }
        }
        "tasks" => match b.task_state(ws.as_ref().unwrap()).await {
            Ok(t) => {
                for x in t {
                    say(format!(
                        "task\t{}\t@{}\t{}\t{}\t{}\t{}",
                        x.chat,
                        x.agent,
                        x.status,
                        x.progress.map(|p| p.to_string()).unwrap_or_default(),
                        if x.active { "active" } else { "inactive" },
                        esc(&x.summary)
                    ))
                    .await;
                }
            }
            Err(e) => fail(e.0).await,
        },
        "memory" => match b.memory(ws.as_ref().unwrap()).await {
            Ok(m) => {
                for e in &m.entries {
                    say(format!("memory\t{}\t{}\t{}\t{}\t{}", e.id, esc(&e.title), esc(&e.tags.join(" ")), esc(&e.author), esc(&e.body)))
                        .await;
                }
                say(format!("memory-proposals\t{}", m.open_proposals)).await;
            }
            Err(e) => fail(e.0).await,
        },
        "handoffs" => match b.handoffs(ws.as_ref().unwrap()).await {
            Ok(h) => {
                for x in h {
                    say(format!("handoff\t{}\t@{}\t@{}\t{}", x.chat, x.from, x.to, esc(&x.task))).await;
                }
            }
            Err(e) => fail(e.0).await,
        },
        "checkout" => {
            let w = ws.unwrap();
            let r = match arg {
                "" => b.checkout(&w).await,
                "none" => b.set_checkout(&w, None).await,
                p => b.set_checkout(&w, Some(p)).await,
            };
            match r {
                Ok(c) => say(format!("checkout\t{}", esc(c.as_deref().unwrap_or("")))).await,
                Err(e) => fail(e.0).await,
            }
        }
        "chats" => match b.chats(ws.as_ref().unwrap()).await {
            Ok(c) => {
                for (i, x) in c.iter().enumerate() {
                    let flags = format!(
                        "{}{}{}",
                        if x.unread { "unread " } else { "" },
                        if x.mentioned { "mentioned " } else { "" },
                        if x.lockdown.is_some() { "locked" } else { "" }
                    );
                    say(format!("chat\t{}\t{}\t{}\t{}", i + 1, x.id, esc(&x.title), flags.trim())).await;
                }
                s.lock().await.chats = c;
            }
            Err(e) => fail(e.0).await,
        },
        "open" => {
            let w = ws.unwrap();
            if s.lock().await.chats.is_empty()
                && let Ok(c) = b.chats(&w).await
            {
                s.lock().await.chats = c;
            }
            let chat = { pick(&s.lock().await.chats, arg, |c| c.id.as_str()).cloned() };
            let Some(chat) = chat else { return fail(format!("no chat {arg}")).await };
            s.lock().await.chat = Some(chat.id.clone());
            say(format!("opened\t{}\t{}", chat.id, esc(&chat.title))).await;
            if let Some(l) = &chat.lockdown {
                say(format!("lockdown\t{}\t{}", esc(&l.reasons.join("; ")), esc(l.clear.reason().unwrap_or("you can clear it: unlock"))))
                    .await;
            }
            let ws_name = { s.lock().await.workspaces.iter().find(|x| x.id == w).map(|x| x.name.clone()).unwrap_or_default() };
            match b.transcript(&w, &chat.id).await {
                Ok(t) => {
                    for m in t {
                        say(format!("message\t{}\t{}\t{}\t{}", esc(&ws_name), chat.id, esc(&m.author.display()), esc(&m.text))).await;
                    }
                }
                Err(e) => fail(e.0).await,
            }
            let _ = b.mark_read(&w, &chat.id).await;
        }
        "new" => match b.create_chat(ws.as_ref().unwrap(), arg).await {
            Ok(c) => {
                s.lock().await.chat = Some(c.id.clone());
                say(format!("opened\t{}\t{}", c.id, esc(&c.title))).await;
            }
            Err(e) => fail(e.0).await,
        },
        "say" => {
            let chat = { s.lock().await.chat.clone() };
            let Some(chat) = chat else { return fail("open a chat first: open N".into()).await };
            match b.send_message(ws.as_ref().unwrap(), &chat, &arg.replace("\\n", "\n")).await {
                Ok(m) => say(format!("sent\t{}\t{}", chat, m.id)).await,
                Err(e) => fail(e.0).await,
            }
        }
        "lock" | "unlock" => {
            let chat = { s.lock().await.chat.clone() };
            let Some(chat) = chat else { return fail("open a chat first: open N".into()).await };
            let w = ws.unwrap();
            let r = if cmd == "lock" { b.lock_chat(&w, &chat).await } else { b.clear_lockdown(&w, &chat).await };
            match r {
                Ok(()) => say(format!("ok\t{cmd}")).await,
                Err(e) => fail(e.0).await,
            }
        }
        "proposals" => match b.proposals(ws.as_ref().unwrap()).await {
            Ok(p) => {
                for (i, x) in p.iter().enumerate() {
                    let apply = crate::ui::apply_text(&x.apply).unwrap_or_default();
                    say(format!(
                        "proposal\t{}\t{}\t{}\t{}/{}\t{}\t{}\t{}",
                        i + 1,
                        x.id,
                        x.status.label(),
                        x.approvals,
                        x.quorum,
                        esc(&x.title),
                        esc(&apply),
                        x.kind.label()
                    ))
                    .await;
                }
                s.lock().await.proposals = p;
            }
            Err(e) => fail(e.0).await,
        },
        "diff" | "approve" | "reject" | "apply" => {
            let w = ws.unwrap();
            if s.lock().await.proposals.is_empty()
                && let Ok(p) = b.proposals(&w).await
            {
                s.lock().await.proposals = p;
            }
            let mut parts = arg.split_whitespace();
            let which = parts.next().unwrap_or("");
            let p = { pick(&s.lock().await.proposals, which, |p| p.id.as_str()).cloned() };
            let Some(p) = p else { return fail(format!("no proposal {which}")).await };
            match cmd {
                "diff" => match b.proposal_diff(&w, &p.id).await {
                    Ok(d) => {
                        for f in d.files {
                            say(format!("file\t{}\t+{}\t-{}", esc(&f.path), f.added, f.removed)).await;
                            for h in f.hunks {
                                say(format!("hunk\t{}", esc(&h.header))).await;
                                for l in h.lines {
                                    let (k, t) = match l {
                                        DiffLine::Added(t) => ("+", t),
                                        DiffLine::Removed(t) => ("-", t),
                                        DiffLine::Context(t) => (" ", t),
                                    };
                                    say(format!("line\t{k}\t{}", esc(&t))).await;
                                }
                            }
                        }
                    }
                    Err(e) => fail(e.0).await,
                },
                "approve" | "reject" => {
                    let c = if cmd == "approve" { VoteChoice::Approve } else { VoteChoice::Reject };
                    match b.vote(&w, &p.id, c).await {
                        Ok(p) => {
                            say(format!("voted\t{}\t{}\t{}/{}\t{}", p.id, p.status.label(), p.approvals, p.quorum, esc(&p.title))).await
                        }
                        Err(e) => fail(e.0).await,
                    }
                }
                _ => {
                    let hosts = match b.hosts(&w).await {
                        Ok(h) => h,
                        Err(e) => return fail(e.0).await,
                    };
                    s.lock().await.hosts = hosts.clone();
                    let host = match parts.next() {
                        Some(h) => pick(&hosts, h, |x| x.id.as_str()).or_else(|| hosts.iter().find(|x| x.label == h)).cloned(),
                        None => hosts
                            .iter()
                            .find(|h| h.online && h.kind == HostKind::Worker)
                            .or_else(|| hosts.iter().find(|h| h.this_device))
                            .cloned(),
                    };
                    let Some(host) = host else { return fail("no such host".into()).await };
                    match b.request_apply(&w, &p.id, &host.id).await {
                        Ok(p) => say(format!("apply\t{}\t{}", p.id, esc(&crate::ui::apply_text(&p.apply).unwrap_or_default()))).await,
                        Err(e) => fail(e.0).await,
                    }
                }
            }
        }
        "agents" => match b.agents(ws.as_ref().unwrap()).await {
            Ok(a) => {
                for (i, x) in a.iter().enumerate() {
                    let host = x.host.as_ref().map(|h| h.0.clone()).unwrap_or_else(|| "this device".into());
                    say(format!(
                        "agent\t{}\t@{}\t{}\t{}\t{}\t{}\t{}",
                        i + 1,
                        x.handle,
                        x.runtime,
                        host,
                        esc(x.role.as_deref().unwrap_or("agent")),
                        x.status.label(),
                        esc(x.doing.as_deref().unwrap_or(""))
                    ))
                    .await;
                }
            }
            Err(e) => fail(e.0).await,
        },
        "runtimes" => match b.runtimes(ws.as_ref().unwrap()).await {
            Ok(r) => {
                for x in r {
                    let st = if x.active {
                        "active"
                    } else if x.configured {
                        "configured"
                    } else {
                        "not configured"
                    };
                    say(format!("runtime\t{}\t{}\t{}\t{st}", x.id, x.kind.label(), x.model.as_deref().unwrap_or("default"))).await;
                }
            }
            Err(e) => fail(e.0).await,
        },
        "hosts" => match b.hosts(ws.as_ref().unwrap()).await {
            Ok(h) => {
                for (i, x) in h.iter().enumerate() {
                    let kind = if x.kind == HostKind::Worker { "worker" } else { "device" };
                    let me = if x.this_device { "this device" } else { "" };
                    say(format!(
                        "host\t{}\t{}\t{}\t{kind}\t{}\t{me}\t{}\t{}\t{}",
                        i + 1,
                        x.id,
                        esc(&x.label),
                        if x.online { "online" } else { "offline" },
                        if x.busy { "busy" } else { "idle" },
                        x.agents.iter().map(|a| format!("@{a}")).collect::<Vec<_>>().join(" "),
                        x.repos.join(" ")
                    ))
                    .await;
                }
                s.lock().await.hosts = h;
            }
            Err(e) => fail(e.0).await,
        },
        "members" => match b.members(ws.as_ref().unwrap()).await {
            Ok(m) => {
                for (i, x) in m.iter().enumerate() {
                    let mut tags = vec![];
                    if x.you {
                        tags.push("you");
                    }
                    if x.founder {
                        tags.push("founder");
                    }
                    if x.service {
                        tags.push("service");
                    }
                    let remove = match &x.can_remove {
                        Allowed::Yes => "remove: allowed".to_string(),
                        Allowed::No(r) => format!("remove: refused ({r})"),
                    };
                    say(format!("member\t{}\t{}\t{}\t{}\t{}\t{remove}", i + 1, x.id, esc(&x.name), x.role.label(), tags.join(" "))).await;
                }
                s.lock().await.members = m;
            }
            Err(e) => fail(e.0).await,
        },
        "role" | "remove" => {
            let w = ws.unwrap();
            if s.lock().await.members.is_empty()
                && let Ok(m) = b.members(&w).await
            {
                s.lock().await.members = m;
            }
            let mut parts = arg.split_whitespace();
            let who = parts.next().unwrap_or("");
            let m = { pick(&s.lock().await.members, who, |m| m.id.as_str()).cloned() };
            let Some(m) = m else { return fail(format!("no member {who}")).await };
            let r = if cmd == "role" {
                let role = match parts.next().unwrap_or("") {
                    "owner" => Role::Owner,
                    "admin" => Role::Admin,
                    "contributor" => Role::Contributor,
                    "viewer" => Role::Viewer,
                    other => return fail(format!("unknown role {other} (owner, admin, contributor, viewer)")).await,
                };
                b.set_role(&w, &m.id, role).await
            } else {
                b.remove_member(&w, &m.id).await
            };
            match r {
                Ok(()) => say(format!("ok\t{cmd}\t{}", esc(&m.name))).await,
                Err(e) => fail(e.0).await,
            }
        }
        "invite" => match b.create_invite(ws.as_ref().unwrap(), Role::Contributor).await {
            Ok(code) => say(format!("invite\t{code}")).await,
            Err(e) => fail(e.0).await,
        },
        "notifications" => match b.notifications().await {
            Ok(n) => {
                for x in n {
                    say(format!("notification\t{}\t{}\t{}", if x.read { "read" } else { "unread" }, x.workspace, esc(&x.text))).await;
                }
            }
            Err(e) => fail(e.0).await,
        },
        "settings" => match b.settings().await {
            Ok(st) => {
                say(format!("setting\tname\t{}", esc(&st.your_name))).await;
                say(format!("setting\tdevice\t{}", esc(&st.device_name))).await;
                say(format!("setting\tnotify_mentions\t{}", st.notify_mentions)).await;
                say(format!("setting\tagents_may_edit\t{}", st.agents_may_edit)).await;
            }
            Err(e) => fail(e.0).await,
        },
        "confirm" | "cancel" => {
            let mut parts = arg.split_whitespace();
            let Some(id) = parts.next().and_then(|i| i.parse::<u64>().ok()) else { return fail(format!("usage: {cmd} ID")).await };
            let answer = match (cmd, parts.next()) {
                ("cancel", _) => PresenceAnswer::Cancel,
                (_, Some("file")) => PresenceAnswer::RecoveryWordsFile(parts.collect::<Vec<_>>().join(" ")),
                _ => PresenceAnswer::Browser,
            };
            if let Err(e) = b.answer_presence(PresenceId(id), answer).await {
                fail(e.0).await;
            }
        }
        _ => fail(format!("unknown command {cmd}: type help")).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::{FakeBackend, FakeConfig};

    struct Sink(Arc<std::sync::Mutex<Vec<u8>>>);
    impl AsyncWrite for Sink {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.0.lock().unwrap().extend_from_slice(buf);
            std::task::Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    async fn run_script(b: FakeBackend, script: &str) -> String {
        let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
        run(Arc::new(b), script.as_bytes(), Box::new(Sink(buf.clone()))).await.unwrap();
        String::from_utf8(buf.lock().unwrap().clone()).unwrap()
    }

    #[tokio::test]
    async fn script_lists_opens_and_streams() {
        let out =
            run_script(FakeBackend::demo(FakeConfig::instant()), "chats\nopen 1\nsay @coder fix it\nwait 5\nproposals\nmembers\n").await;
        assert!(out.contains("chat\t1\tc-log\tInspection log refactor\tunread mentioned"), "{out}");
        assert!(out.contains("message\tApiary Tracker\tc-log\tAda\tThe inspection log stores raw mite counts."), "{out}");
        assert!(out.contains("thinking\tc-log\t@coder"), "{out}");
        assert!(out.contains("message\tApiary Tracker\tc-log\t@coder\tLooked at the inspection log."), "{out}");
        assert!(out.contains("Tighten the varroa threshold check"), "{out}");
        assert!(out.contains("member\t4\tm-sam\tSam\tviewer\t\tremove: allowed"), "{out}");
    }

    #[tokio::test]
    async fn first_run_create_prints_recovery_words() {
        let out =
            run_script(FakeBackend::new(FakeConfig::instant()), "workspaces\nname Mara\ncreate Bee Yard\nwait 1\nconfirm 101\nwait 2\n")
                .await;
        assert!(out.contains("first-run"), "{out}");
        assert!(out.contains("presence\t101\tpasskeyRegistered"), "{out}");
        assert!(out.contains("created\t"), "{out}");
        let words = out.lines().find(|l| l.starts_with("recovery-key")).unwrap().split('\t').nth(1).unwrap();
        assert_eq!(words.split(' ').count(), 24);
    }

    async fn one_shot(b: FakeBackend, cmd: &str) -> (i32, String) {
        let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
        let opts = PlainOpts { one_shot: Some(cmd.into()), ..Default::default() };
        let code = run_with(Arc::new(b), &b""[..], Box::new(Sink(buf.clone())), opts).await.unwrap();
        (code, String::from_utf8(buf.lock().unwrap().clone()).unwrap())
    }

    /// One-shot: only the command's records, tab-separated, and an exit code.
    #[tokio::test]
    async fn one_shot_lists_workflows_runs_and_hosts() {
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "workflows").await;
        assert_eq!(code, 0);
        assert_eq!(
            out,
            "workflow\t1\twf-review\tImplement, review, ship\tInspection log refactor\timplement:@coder review:@reviewer signoff:gate (1 approval)\tstartable\n\
             workflow\t2\twf-report\tWeekly swarm report\tQueen cell alerts\tsummarize:@scout\tstartable\n"
        );
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "runs").await;
        assert_eq!(code, 0);
        assert!(
            out.starts_with("run\t1\trun-7\tawaiting-gate\tImplement, review, ship\t2/3\tThis device\tcount mites per 100 bees\n"),
            "{out}"
        );
        let (_, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "hosts").await;
        assert!(out.contains("host\t2\th-sff\tsff\tworker\tonline\t\tbusy\t@coder @scout\tapiary\n"), "{out}");
        let (_, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "proposals").await;
        assert!(out.lines().any(|l| l.starts_with("proposal\t") && l.ends_with("\tgate")), "{out}");
        let (_, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "tasks").await;
        assert!(out.contains("task\tc-log\t@coder\tworking\t40\tactive\twiring the CSV export"), "{out}");
    }

    /// `run` starts a workflow and follows it to the end: exit 0 when it
    /// completes. Unknown commands exit 2; errors exit 1.
    #[tokio::test]
    async fn one_shot_run_follows_to_the_end_with_exit_codes() {
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "run 2 this week").await;
        assert_eq!(code, 0, "{out}");
        assert!(out.lines().next().unwrap().starts_with("run\t1\trun-"), "{out}");
        assert!(out.contains("\trunning\tWeekly swarm report\t0/1\tThis device\tthis week"), "{out}");
        assert!(out.contains("\tsummarize\tsucceeded\t@scout\t\t@scout finished: this week"), "{out}");
        assert!(out.trim_end().lines().last().unwrap().starts_with("run-done\trun-"), "{out}");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "frobnicate").await;
        assert_eq!(code, EXIT_USAGE, "{out}");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "gate 2 approve").await;
        assert_eq!(code, 1);
        assert!(out.contains("error\trun run-6 isn't waiting on a gate"), "{out}");
        let (code, _) = one_shot(FakeBackend::demo(FakeConfig::instant()), "watch nope").await;
        assert_eq!(code, 1);
    }

    /// Interactive: a gate decided from plain mode lets the run complete.
    #[tokio::test]
    async fn a_gate_is_decided_from_plain_mode() {
        let out = run_script(FakeBackend::demo(FakeConfig::instant()), "runs\ngate 1 approve\nwait 2\nruns\n").await;
        assert!(out.contains("gate\trun-7\tsignoff\tapproved"), "{out}");
        assert!(out.contains("run-changed\trun-7\tcompleted\tImplement, review, ship\t3/3"), "{out}");
        assert!(out.contains("run\t1\trun-7\tcompleted\t"), "{out}");
    }

    #[tokio::test]
    async fn refusals_print_reasons() {
        let out = run_script(FakeBackend::demo(FakeConfig::instant()), "use 2\nmembers\nremove 1\nrole 2 owner\n").await;
        assert!(out.contains("remove: refused (nobody cuts the workspace's founder)"), "{out}");
        assert!(out.contains("error\tnobody cuts the workspace's founder"), "{out}");
        assert!(out.contains("error\tno one grants a role above their own"), "{out}");
    }
}
