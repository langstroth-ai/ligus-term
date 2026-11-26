//! `lt --plain`: a line-oriented client for screen readers and scripts.
//!
//! It reads one command per line on stdin and writes one record per line
//! on stdout: a keyword, then tab-separated fields, text last (newlines in
//! text are written as `\n`). Backend events print as they arrive, so a
//! screen reader hears new messages and passkey prompts without a screen to
//! scan. `help` lists the commands; EOF waits for running actions, then
//! exits.
//!
//! One-shot: `lt COMMAND [ARGS…]` (`--plain` optional) runs one command and exits, for
//! scripts and other agents. It prints only that command's records (and any
//! passkey prompt it raises), and exits 0 on success, or the first error
//! record's code from the shared table ([`error_code`]: 1 failed, 2 usage,
//! 3 a precondition, 4 a person must act, 5 not found, 6 refused, 7
//! offline); a run it followed that didn't complete is 1. `run` and `watch` follow the run until it ends: this
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
  join CODE            join with an invite code (a NEW member)
  link-request         this computer's request to be added to your devices (on your desktop:
                       My devices > Add a computer); prints its fingerprint
  link-redeem CODE --approver-fingerprint FP  link with the code your desktop shows, after
                       checking the approving desktop's fingerprint (FP) is your desktop's
  link-to ADDRESS      send this computer's request to the approval address your desktop shows
                       (ligusaddr1:); then compare the code: match yes|no; then link-as yes|no
  match yes|no         the code on your desktop is the same (yes) or not (no: nothing is granted)
  link-as yes|no       link as the person the workspace log verified, or link nothing
  link-cancel          end the request link-to sent
  approvals            approval requests waiting in this workspace (approve on your desktop or phone)
  link-status          whether this computer is linked, or a link is waiting for the desktop's code
  device-rename NAME   name this computer, as every member sees it
  workspaces           list workspaces          use N        switch workspace
  chats                list chats               open N       open a chat and print it
  new TITLE            start a chat             say TEXT     post to the open chat
  stop                 stop the agents' turns running here in the open chat
  lock / unlock        lock the open chat down / clear its lockdown
  proposals            list proposals           diff N       print a proposal's diff
  approve N / reject N vote                     apply N [H]  apply on host H (default: first online worker)
  agents / runtimes / hosts / members / notifications / settings
  access N everyone|owner  who can use agent N (yours: everyone, or only you)
  available            what this device could run agents on (CLIs, Ollama, API keys)
  add-agent N [NAME]   add runtime N from available as an agent: yours, only you, on this device
  role M ROLE          change a member's role   remove M     remove a member
  invite               create an invite code    admit [REQUEST] [--out FILE]  admit a box (ligusjoin1:,
                       first prints its fingerprint to compare with the box before the passkey prompt;
                       or none: a bearer code, written owner-only to FILE, never printed)
  check CODE           who an invite is from    connect RELAY ROOM KEY [TOKEN] [--out FILE]  found a deployment's
                       workspace (KEY and TOKEN may be @FILE, owner-only; one-shot: only @FILE, and the
                       recovery words go to the owner-only FILE; so does create NAME --out FILE)
  workflows            list workflows           runs         list runs
  run W [INPUT]        start workflow W and follow it until it ends
  start W [INPUT]      start workflow W         watch R      follow run R until it ends
  gate R approve|reject  decide run R's waiting gate      cancel-run R  cancel run R
  tasks / memory / handoffs  agents' task state, workspace memory, hand-offs
  checkout [PATH|none] show or set this device's checkout for apply on host
  setup [FLOW [--FIELD VALUE ...]]  the guided setup flows (agent tool folder box phone invite relay);
                       secrets only as @FILE here; lt help setup has every flag
  confirm ID           answer a passkey prompt in the browser
  confirm ID file PATH answer it with a recovery-words file
  cancel ID            cancel a passkey prompt
  wait [SECS]          wait until events go quiet (default up to 30 s)
  help [COMMAND|TOPIC] / quit   (topics: quickstart keys plain agents kubernetes troubleshooting env)";

/// Escape a field: tabs become spaces, newlines `\n`.
fn esc(s: impl std::fmt::Display) -> String {
    s.to_string().replace('\t', " ").replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r")
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
    local: Vec<LocalRuntime>,
    /// The live link request `link-to` sent.
    live: Option<String>,
    /// One-shot (`lt --plain COMMAND`): a secret never rides argv or stdout.
    one_shot: bool,
}

struct Output {
    w: Mutex<Box<dyn AsyncWrite + Unpin + Send>>,
    /// An `error` record was printed (the one-shot exit code).
    failed: std::sync::atomic::AtomicBool,
    /// The first error's exit code (see [`error_code`]).
    code: std::sync::atomic::AtomicI32,
    /// Write each record as a JSON object (`--json`).
    json: bool,
    /// Also write each error to stderr as `lt: <what>` (one-shot).
    stderr: bool,
}

type Out = Arc<Output>;

/// Bidi controls: they reorder what a terminal shows (U+061C, U+200E/F,
/// U+202A..U+202E, U+2066..U+2069).
pub(crate) fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// A record as it may reach a terminal: tabs separate fields; a line break
/// inside a field is written `\n` (it would forge a record); every other
/// control character (escape sequences: ANSI, OSC, C1) and every bidi
/// control becomes U+FFFD. Much of a record comes from the network.
pub fn clean(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        match c {
            '\t' => out.push('\t'),
            '\n' | '\u{2028}' | '\u{2029}' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() || is_bidi(c) => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

async fn emit(out: &Out, line: String) {
    let line = clean(&line);
    if line.starts_with("error\t") {
        let first = !out.failed.swap(true, std::sync::atomic::Ordering::SeqCst);
        if first {
            let msg = line.rsplit('\t').next().unwrap_or("");
            out.code.store(error_code(msg).1, std::sync::atomic::Ordering::SeqCst);
        }
        if out.stderr {
            eprintln!("lt: {}", line.rsplit('\t').next().unwrap_or(""));
        }
    }
    let line = if out.json { crate::help::record_json(&line) } else { line };
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
    /// Also write each error to stderr (`lt: <what>`), as one-shot does,
    /// while the `error` record stays on stdout.
    pub errors_to_stderr: bool,
    /// The workspace to use (a name, an id, or a number), else the first.
    pub workspace: Option<String>,
    /// Answer every passkey prompt this way (one-shot: the browser, or a
    /// recovery-words file).
    pub presence: Option<PresenceAnswer>,
    /// Write each record as one JSON object per line (`--json`).
    pub json: bool,
    /// Your name for a device with no identity yet (`--name`).
    pub name: Option<String>,
}

/// The exit code for a usage error (an unknown one-shot command).
pub const EXIT_USAGE: i32 = 2;

/// An error's stable code and exit code, from the shared table (`lt help
/// exit-codes`): 1 failed, 2 usage, 3 a precondition you can fix, 4 a
/// person must act, 5 not found, 6 refused by workspace rules, 7 offline.
pub fn error_code(message: &str) -> (&'static str, i32) {
    let m = message.trim().to_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| m.contains(w));
    let starts = |words: &[&str]| words.iter().any(|w| m.starts_with(w));
    if starts(&["usage:", "unknown command", "unknown role", "unknown access", "unknown option"]) || any(&["add --out"]) {
        ("usage", 2)
    } else if any(&[" yet:", "first:", "open a chat first", "needs a relay", "set your name"]) {
        ("precondition", 3)
    } else if any(&["passkey", "recovery words", "cancelled", "confirm who approved", "a person must"]) {
        ("needs_person", 4)
    } else if starts(&[
        "no such ",
        "no chat ",
        "no run ",
        "no proposal ",
        "no member ",
        "no agent ",
        "no workflow ",
        "no workspace ",
        "no runtime ",
        "no host ",
    ]) || any(&["not found"])
    {
        ("not_found", 5)
    } else if any(&["offline", "unreachable", "timed out", "timeout", "connection refused", "error sending request", "couldn't reach"]) {
        ("offline", 7)
    } else if any(&[
        "can't ",
        "cannot ",
        "only an admin",
        "only the ",
        "only its owner",
        "only a ",
        "refused",
        "not allowed",
        "a viewer",
        "lockdown",
        "you may not",
    ]) {
        ("refused", 6)
    } else {
        ("error", 1)
    }
}

pub const COMMANDS: &[&str] = &[
    "help",
    "name",
    "create",
    "join",
    "link-request",
    "link-redeem",
    "link-to",
    "match",
    "link-as",
    "link-cancel",
    "approvals",
    "link-status",
    "device-rename",
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
    "stop",
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
    "access",
    "available",
    "add-agent",
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
    "setup",
];

fn run_line(i: usize, r: &Run) -> String {
    let done = r.stages.iter().filter(|s| s.status == StageStatus::Succeeded).count();
    format!(
        "run\t{}\t{}\t{}\t{}\t{done}/{}\t{}\t{}",
        i + 1,
        esc(&r.id),
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
                esc(&r.id),
                esc(&st.id),
                st.status.label(),
                esc(&st.what),
                esc(st.gate.as_ref().map(|g| g.0.as_str()).unwrap_or("")),
                esc(note)
            )
        })
        .collect()
}

/// Write the passkey page's address straight to the controlling terminal
/// (unit tests have none to write to).
fn to_tty(url: &str) -> std::io::Result<()> {
    use std::io::Write;
    if cfg!(test) {
        return Err(std::io::Error::other("no terminal in tests"));
    }
    let mut tty = std::fs::OpenOptions::new().write(true).open("/dev/tty")?;
    writeln!(tty, "\npasskey page (open it on the other device):\n{}\n", clean(url))
}

/// Format a backend event as a line; `None` for events plain mode skips
/// (streaming deltas: the whole message prints when it is done).
pub fn event_line(e: &Event, names: &dyn Fn(&WorkspaceId) -> String) -> Option<String> {
    Some(match e {
        Event::MessageAdded { workspace, message } if !message.streaming => {
            format!("message\t{}\t{}\t{}\t{}", esc(names(workspace)), esc(&message.chat), esc(message.author.display()), esc(&message.text))
        }
        Event::MessageAdded { .. } | Event::MessageDelta { .. } => return None,
        Event::MessageDone { error: Some(e), chat, .. } => format!("error\t{}\t{}", esc(chat), esc(e)),
        Event::MessageDone { .. } => return None,
        Event::ToolActivity { chat, call, .. } => format!("tool\t{}\t{}\t{}", esc(chat), esc(&call.name), esc(&call.preview)),
        Event::Thinking { chat, agent, on: true, .. } => format!("thinking\t{}\t{}", esc(chat), esc(agent)),
        Event::Thinking { .. } => return None,
        Event::ProposalChanged { proposal: p, .. } => {
            format!("proposal-changed\t{}\t{}\t{}/{}\t{}", esc(&p.id), p.status.label(), p.approvals, p.quorum, esc(&p.title))
        }
        Event::Notification(n) => format!("notification\t{}\t{}", esc(names(&n.workspace)), esc(&n.text)),
        Event::PresencePrompt(p) => {
            let how = if p.op == PresenceOp::Get { "confirm ID | confirm ID file PATH | cancel ID" } else { "confirm ID | cancel ID" };
            format!("presence\t{}\t{}\t{}\t{}\t{how}", p.id.0, esc(&p.kind), esc(&p.digest_short), esc(&p.summary))
        }
        Event::PresenceProblem { id, error } => format!("presence-problem\t{}\t{}", id.0, esc(error)),
        // The address can answer a passkey creation, so it goes to the
        // person's terminal, never to stdout (often a log or a pipe).
        Event::PresenceUrl { id, url, port } => match to_tty(url) {
            Ok(()) => format!(
                "presence-url\t{}\t{port}\t(on your terminal)\tthe address is on your terminal: open it in a browser on a \
                 device whose 127.0.0.1:{port} reaches this one (ssh -L 127.0.0.1:{port}:127.0.0.1:{port} THIS-HOST); it is a \
                 one-time secret for this prompt",
                id.0
            ),
            Err(e) => format!(
                "presence-url\t{}\t{port}\t-\tno terminal to show the passkey page's address on ({e}); it is never written to \
                 stdout: run lt in a terminal, or cancel {} and use a browser here",
                id.0, id.0
            ),
        },
        Event::PresenceDone { id, ok, error } => {
            format!("presence-done\t{}\t{}\t{}", id.0, if *ok { "ok" } else { "failed" }, esc(error.as_deref().unwrap_or("")))
        }
        Event::Sync { workspace, status } => format!("sync\t{}\t{}", esc(names(workspace)), esc(status)),
        Event::LiveLink { rid, update } => live_line(rid, update),
        Event::RunChanged { run: r, .. } => {
            let done = r.stages.iter().filter(|s| s.status == StageStatus::Succeeded).count();
            format!("run-changed\t{}\t{}\t{}\t{done}/{}", esc(&r.id), r.status.label(), esc(&r.name), r.stages.len())
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

/// A live link request's progress, as a record.
fn live_line(rid: &str, u: &LiveLinkUpdate) -> String {
    match u {
        LiveLinkUpdate::Opened(c) => format!(
            "match-code\t{rid}\t{}\t{}\t{}\t{}",
            esc(&c.shown),
            esc(&c.spelled),
            esc(format!("says it's “{}” (checked after approval)", c.claimed)),
            esc("check your desktop shows the same code: match yes | match no")
        ),
        LiveLinkUpdate::Acknowledged => format!("link-matched\t{rid}\t{}", esc("the codes match: waiting for your desktop's passkey")),
        LiveLinkUpdate::Checking => {
            format!("link-checking\t{rid}\t{}", esc("checking the approval on the workspace log (up to 2 minutes)"))
        }
        LiveLinkUpdate::Verified { person, workspace } => format!(
            "link-verified\t{rid}\t{}\t{}\t{}",
            esc(person),
            esc(workspace),
            esc(format!("link this computer as “{person}” (verified on the workspace log)? link-as yes | link-as no"))
        ),
        LiveLinkUpdate::Ended(e) => format!("link-ended\t{rid}\t{}\t{}", esc(e.describe()), esc(e.next())),
    }
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
    let out: Out = Arc::new(Output {
        w: Mutex::new(out),
        failed: std::sync::atomic::AtomicBool::new(false),
        code: std::sync::atomic::AtomicI32::new(0),
        json: opts.json,
        stderr: opts.errors_to_stderr,
    });
    let one_shot = opts.one_shot.is_some();
    if let Some(cmd) = &opts.one_shot {
        let name = cmd.split_whitespace().next().unwrap_or("");
        if !COMMANDS.contains(&name) {
            emit(&out, format!("error\tunknown command {name}: lt help lists them")).await;
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
        your_name: st.your_name.clone().filter(|n| !n.is_empty()).or_else(|| opts.name.clone()),
        workspaces: st.workspaces.clone(),
        chats: vec![],
        proposals: vec![],
        members: vec![],
        hosts: vec![],
        workflows: vec![],
        runs: vec![],
        local: vec![],
        live: None,
        one_shot,
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
        // Subscribe now, not when the task first runs: a one-shot command
        // starts at once, and a passkey prompt it raises before the task
        // subscribed would never be answered (the command would hang).
        let mut events = backend.events();
        tokio::spawn(async move {
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
                let quiet = one_shot
                    && !matches!(
                        e,
                        Event::PresencePrompt(_) | Event::PresenceProblem { .. } | Event::PresenceUrl { .. } | Event::PresenceDone { .. }
                    );
                if !quiet && let Some(line) = event_line(&e, &name_of) {
                    emit(&out, line).await;
                }
            }
        })
    };

    if let Some(cmd) = opts.one_shot {
        command(&backend, &out, &session, &cmd).await;
        ev_task.abort();
        return Ok(if out.failed.load(std::sync::atomic::Ordering::SeqCst) {
            out.code.load(std::sync::atomic::Ordering::SeqCst).max(1)
        } else {
            0
        });
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
        emit(out, format!("run-done\t{}\t{}", esc(&r.id), r.status.label())).await;
    } else {
        emit(out, format!("error\trun {} {}", esc(&r.id), r.status.label())).await;
    }
}

fn ws_line(i: usize, w: &WorkspaceSummary) -> String {
    format!("workspace\t{}\t{}\t{}\t{}\t{}", i + 1, esc(&w.id), esc(&w.name), w.your_role.label(), w.sync.label())
}

fn pick<'a, T>(items: &'a [T], arg: &str, id: impl Fn(&T) -> &str) -> Option<&'a T> {
    let arg = arg.trim();
    if let Ok(n) = arg.parse::<usize>() {
        return n.checked_sub(1).and_then(|i| items.get(i));
    }
    items.iter().find(|x| id(x) == arg)
}

/// `ARGS --out FILE`: the args without the option, and the file.
fn take_out(arg: &str) -> (String, Option<std::path::PathBuf>) {
    let parts: Vec<&str> = arg.split_whitespace().collect();
    match parts.iter().position(|p| *p == "--out") {
        Some(i) => {
            let file = parts.get(i + 1).map(std::path::PathBuf::from);
            let rest: Vec<&str> = parts.iter().enumerate().filter(|(j, _)| *j != i && *j != i + 1).map(|(_, p)| *p).collect();
            (rest.join(" "), file)
        }
        None => (arg.to_string(), None),
    }
}

/// Write a secret to a new owner-only file. An existing path (or a link)
/// is refused, never overwritten or followed.
pub fn write_private(path: &std::path::Path, text: &str) -> std::result::Result<(), String> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path).map_err(|e| format!("writing {}: {e} (give a new file)", path.display()))?;
    f.write_all(text.as_bytes()).and_then(|_| f.write_all(b"\n")).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Refuse an `--out FILE` that already exists, before anything is made.
fn fresh_out(file: Option<&std::path::Path>) -> std::result::Result<(), String> {
    match file {
        Some(f) if std::fs::symlink_metadata(f).is_ok() => Err(format!("{} already exists: give --out a new file", f.display())),
        _ => Ok(()),
    }
}

/// Read a secret from `@FILE`: owner-only, small.
pub fn read_secret_file(path: &str) -> std::result::Result<String, String> {
    crate::backend::presence::read_owner_only(std::path::Path::new(path), 64 * 1024).map(|t| t.trim().to_string())
}

/// Where a new workspace's recovery words go: printed for a person at the
/// terminal, written owner-only to `--out FILE` by a one-shot command.
async fn recovery_words(out: &Out, words: &[String], file: Option<&std::path::Path>) {
    let words = words.join(" ");
    match file {
        None => emit(out, format!("recovery-key\t{words}\twrite these 24 words down; they are shown once")).await,
        Some(f) => match write_private(f, &words) {
            Ok(()) => {
                emit(
                    out,
                    format!(
                        "recovery-key\t@{}\tthe 24 words are in this owner-only file: keep them offline, then delete it",
                        esc(f.display())
                    ),
                )
                .await
            }
            // The only copy: better on the terminal than lost.
            Err(e) => emit(out, format!("recovery-key\t{words}\t{e}; write these 24 words down")).await,
        },
    }
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
        "help" if arg.is_empty() => {
            for l in HELP.lines() {
                say(format!("help\t{l}")).await;
            }
        }
        "help" => match crate::help::topic(arg, &crate::cli::flags()) {
            Some(text) => {
                for l in text.lines() {
                    say(format!("help\t{}", esc(l))).await;
                }
            }
            None => fail(crate::help::unknown(arg)).await,
        },
        "name" => {
            s.lock().await.your_name = Some(arg.to_string());
            say(format!("ok\tname\t{}", esc(arg))).await;
        }
        "create" | "join" => {
            let (your, one_shot) = {
                let g = s.lock().await;
                (g.your_name.clone().unwrap_or_default(), g.one_shot)
            };
            if cmd == "create" {
                let (name, file) = take_out(arg);
                if one_shot && file.is_none() {
                    return fail("a one-shot create writes the recovery words to a file: create NAME --out FILE".into()).await;
                }
                if let Err(e) = fresh_out(file.as_deref()) {
                    return fail(e).await;
                }
                match b.create_workspace(&your, &name).await {
                    Ok(c) => {
                        say(format!("created\t{}\t{}", esc(&c.workspace.id), esc(&c.workspace.name))).await;
                        recovery_words(out, &c.recovery_words, file.as_deref()).await;
                        if let Some(i) = &c.invite {
                            say(format!("invite\t{}", esc(i))).await;
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
                        say(format!("joined\t{}\t{}", esc(&w.id), esc(&w.name))).await;
                        let mut g = s.lock().await;
                        g.ws = Some(w.id.clone());
                        g.workspaces.push(w);
                    }
                    Err(e) => fail(e.0).await,
                }
            }
        }
        "link-request" => match b.link_request().await {
            Ok(r) => {
                say(format!("link-request\t{}\t{}\t{}", esc(&r.fingerprint), esc(&r.label), esc(&r.code))).await;
                say(format!(
                    "ok\t{}",
                    esc("on your desktop: Settings > Profile > My devices > Add a computer; paste the request, compare the fingerprint, confirm with your passkey")
                ))
                .await;
            }
            Err(e) => fail(e.0).await,
        },
        "link-redeem" => {
            let parts: Vec<&str> = arg.split_whitespace().collect();
            let fp = parts.iter().position(|p| *p == "--approver-fingerprint").and_then(|i| parts.get(i + 1)).map(|s| s.to_string());
            let grant: Vec<&str> = parts
                .iter()
                .enumerate()
                .filter(|(i, p)| **p != "--approver-fingerprint" && (*i == 0 || parts[*i - 1] != "--approver-fingerprint"))
                .map(|(_, p)| *p)
                .collect();
            let grant = grant.join("");
            if grant.is_empty() {
                return fail("usage: link-redeem CODE --approver-fingerprint FP".into()).await;
            }
            // The code out of whatever was pasted around it.
            let grant = match crate::link_code::grant_in(&grant) {
                crate::link_code::Pasted::Grant(g) => g,
                other => return fail(crate::link_code::not_a_grant(&other, false)).await,
            };
            let check = match b.inspect_link_grant(&grant).await {
                Ok(c) => c,
                Err(e) => return fail(e.0).await,
            };
            say(format!("link-grant\t{}\t{}\t{}", esc(&check.person_name), esc(&check.workspace_name), esc(&check.approver_fingerprint)))
                .await;
            match fp {
                Some(f) if crate::link_code::same_fingerprint(&f, &check.approver_fingerprint) => {}
                Some(f) => {
                    return fail(format!("approved by desktop {}, not {}: don't use this code", check.approver_fingerprint, f.trim()))
                        .await;
                }
                None => {
                    return fail(format!(
                        "confirm who approved this ({}'s desktop {} for {}): link-redeem CODE --approver-fingerprint FP",
                        check.person_name, check.approver_fingerprint, check.workspace_name
                    ))
                    .await;
                }
            }
            match b.redeem_link(&grant).await {
                Ok(w) => {
                    say(format!("linked\t{}\t{}\t{}", esc(&w.id), esc(&w.name), esc(&check.person_name))).await;
                    let mut g = s.lock().await;
                    g.ws = Some(w.id.clone());
                    g.workspaces.push(w);
                }
                Err(e) => fail(e.0).await,
            }
        }
        "device-rename" => match b.rename_this_device(arg).await {
            Ok(name) => say(format!("device-name\t{}", esc(&name))).await,
            Err(e) => fail(e.0).await,
        },
        "link-to" => {
            if s.lock().await.one_shot {
                // A person compares the code on this computer: no one-shot.
                return fail(
                    "a person must compare the code on this computer: run lt --plain and type link-to ADDRESS, then match yes|no".into(),
                )
                .await;
            }
            let address: String = arg.split_whitespace().collect();
            if address.is_empty() {
                return fail("usage: link-to ADDRESS (ligusaddr1:..., from your desktop's My devices > Add a computer)".into()).await;
            }
            match b.link_live(&address).await {
                Ok(LiveLinkStart::Posted { rid, request: r }) => {
                    s.lock().await.live = Some(rid.clone());
                    say(format!("link-request\t{}\t{}\t{}", esc(&r.fingerprint), esc(&r.label), esc(&r.code))).await;
                    say(format!(
                        "link-waiting\t{rid}\t{}",
                        esc("waiting for your desktop (My devices); an older desktop takes the request above instead")
                    ))
                    .await;
                }
                Ok(LiveLinkStart::NoMailbox { request: r }) => {
                    say(format!("link-request\t{}\t{}\t{}", esc(&r.fingerprint), esc(&r.label), esc(&r.code))).await;
                    say(format!(
                        "ok\t{}",
                        esc("that relay has no approval mailbox: paste the request on your desktop, compare the fingerprint, then link-redeem CODE --approver-fingerprint FP")
                    ))
                    .await;
                }
                Err(e) => fail(e.0).await,
            }
        }
        "match" | "link-as" => {
            let yes = match arg {
                "yes" | "y" => true,
                "no" | "n" => false,
                _ => return fail(format!("usage: {cmd} yes|no")).await,
            };
            let Some(rid) = s.lock().await.live.clone() else {
                return fail("no request yet: link-to ADDRESS first".into()).await;
            };
            if cmd == "match" {
                match b.link_live_code(&rid, yes).await {
                    Ok(()) => say(format!("ok\tmatch {}", if yes { "yes" } else { "no" })).await,
                    Err(e) => fail(e.0).await,
                }
                return;
            }
            match b.link_live_person(&rid, yes).await {
                Ok(Some(w)) => {
                    s.lock().await.live = None;
                    let person = b.linked_person().unwrap_or_default();
                    say(format!("linked\t{}\t{}\t{}", esc(&w.id), esc(&w.name), esc(&person))).await;
                    let mut g = s.lock().await;
                    g.ws = Some(w.id.clone());
                    g.workspaces.push(w);
                }
                Ok(None) => {
                    s.lock().await.live = None;
                    say("ok\tnothing was linked".into()).await;
                }
                Err(e) => fail(e.0).await,
            }
        }
        "link-cancel" => {
            let Some(rid) = s.lock().await.live.take() else { return say("ok\tno request to end".into()).await };
            match b.link_live_cancel(&rid).await {
                Ok(()) => say("ok\trequest ended".into()).await,
                Err(e) => fail(e.0).await,
            }
        }
        "approvals" => {
            need_ws().await;
            let Some(w) = ws.clone() else { return };
            match b.approvals(&w).await {
                Ok(list) => {
                    let now = ligus_core::flow::now_ms();
                    for (i, r) in list.iter().enumerate() {
                        let who = r.checking_line();
                        say(format!(
                            "approval\t{}\t{}\t{}\t{}\t{}",
                            i + 1,
                            r.kind.label(),
                            esc(&r.label),
                            crate::app::state::ago(now.max(r.at), r.at),
                            esc(&who)
                        ))
                        .await;
                    }
                    say(format!("ok\t{}", esc("approve on your desktop or phone"))).await;
                }
                Err(e) => fail(e.0).await,
            }
        }
        "link-status" => match b.link_status().await {
            Ok(v) => {
                let now = crate::app::runtime::now_ms();
                let text = crate::link_cli::status_line(&v, now, crate::link_code::local_offset_secs());
                let state = if v.linked_to.is_some() {
                    "linked"
                } else if v.pending.is_some() {
                    "waiting"
                } else {
                    "none"
                };
                let (started, expires) = v.pending.map(|p| (p.started_ms.to_string(), p.approve_by_ms.to_string())).unwrap_or_default();
                say(format!("link-status\t{state}\t{}\t{started}\t{expires}\t{}", esc(v.linked_to.as_deref().unwrap_or("")), esc(&text)))
                    .await;
            }
            Err(e) => fail(e.0).await,
        },
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
            say(format!("using\t{}\t{}", esc(&w.id), esc(&w.name))).await;
        }
        "check" => match b.inspect_invite(arg).await {
            Ok(c) => {
                say(format!(
                    "invite-check\t{}\t{}\t{}\t{}\t{}\t{}",
                    c.trust.label(),
                    esc(&c.workspace_name),
                    esc(c.founder_name.as_deref().unwrap_or("")),
                    esc(c.safety_code.as_deref().unwrap_or("")),
                    esc(&c.relay_url),
                    c.trust.describe()
                ))
                .await
            }
            Err(e) => fail(e.0).await,
        },
        "connect" => {
            let one_shot = s.lock().await.one_shot;
            let (arg, file) = take_out(arg);
            let parts: Vec<&str> = arg.split_whitespace().collect();
            let [relay, room, key, rest @ ..] = parts.as_slice() else {
                return fail("usage: connect RELAY ROOM KEY [TOKEN] [--out FILE] (KEY and TOKEN may be @FILE)".into()).await;
            };
            // A one-shot's arguments are its argv, which every local process
            // can list: the key and token come from owner-only files there.
            let read = |v: &str| -> std::result::Result<String, String> {
                match v.strip_prefix('@') {
                    Some(path) => read_secret_file(path),
                    None if one_shot => {
                        Err("a one-shot connect takes the key and token as @FILE (owner-only), never in its arguments".into())
                    }
                    None => Ok(v.to_string()),
                }
            };

            let (key, token) = match (read(key), rest.first().map(|t| read(t)).transpose()) {
                (Ok(k), Ok(t)) => (k, t),
                (Err(e), _) | (_, Err(e)) => return fail(e).await,
            };
            if one_shot && file.is_none() {
                return fail("a one-shot connect writes the recovery words to a file: add --out FILE".into()).await;
            }
            if let Err(e) = fresh_out(file.as_deref()) {
                return fail(e).await;
            }
            let your = { s.lock().await.your_name.clone().unwrap_or_default() };
            let spec = DeploymentSpec { name: String::new(), relay_url: relay.to_string(), room: room.to_string(), key, token };
            match b.connect_deployment(&your, spec).await {
                Ok(c) => {
                    say(format!("connected\t{}\t{}\t{}", esc(&c.workspace.id), esc(&c.workspace.name), c.workspace.sync.label())).await;
                    recovery_words(out, &c.recovery_words, file.as_deref()).await;
                    let _ = b.confirm_recovery_saved(&c.workspace.id).await;
                    let mut g = s.lock().await;
                    g.ws = Some(c.workspace.id.clone());
                    g.workspaces.push(c.workspace);
                }
                Err(e) => fail(e.0).await,
            }
        }
        "setup" => {
            for line in crate::setup::cli::plain(b, ws.as_ref(), arg).await {
                let rec = line.split('\t').map(esc).collect::<Vec<_>>().join("\t");
                say(rec).await;
            }
        }
        _ if ws.is_none() && cmd != "notifications" && cmd != "settings" && cmd != "confirm" && cmd != "cancel" => need_ws().await,
        "admit" => {
            let (request, file) = take_out(arg);
            // A bearer code carries the workspace key: it goes to an
            // owner-only file, never to the terminal or a log.
            if request.is_empty() && file.is_none() {
                return fail("a bearer code carries the workspace key, so it isn't printed: admit --out FILE writes it owner-only".into())
                    .await;
            }
            if let Err(e) = fresh_out(file.as_deref()) {
                return fail(e).await;
            }
            // The box's fingerprint comes first, so it's compared with the
            // box's terminal before the passkey prompt is answered.
            if !request.is_empty() {
                match b.inspect_join_request(&request).await {
                    Ok(c) => say(format!("joinrequest\t{}\t{}", esc(&c.fingerprint), esc(c.label.as_deref().unwrap_or("")))).await,
                    Err(e) => return fail(e.0).await,
                }
            }
            match b.admit_box(ws.as_ref().unwrap(), Some(request.as_str()).filter(|a| !a.is_empty())).await {
                Ok(a) => {
                    let code = match &file {
                        Some(f) => match write_private(f, &a.code) {
                            Ok(()) => format!("@{}", f.display()),
                            Err(e) => return fail(e).await,
                        },
                        None => a.code.clone(),
                    };
                    say(format!(
                        "admission\t{}\t{}\t{}\t{}",
                        if a.sealed { "sealed" } else { "bearer" },
                        esc(a.fingerprint.as_deref().unwrap_or("")),
                        esc(a.label.as_deref().unwrap_or("")),
                        esc(&code)
                    ))
                    .await
                }
                Err(e) => fail(e.0).await,
            }
        }
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
                        esc(&d.id),
                        esc(&d.name),
                        esc(&d.chat_title),
                        esc(stages.join(" ")),
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
                        Ok(r) => {
                            say(format!("gate\t{}\t{}\t{}", esc(&r.id), esc(&stage.id), if approve { "approved" } else { "rejected" }))
                                .await
                        }
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
                        esc(&x.chat),
                        esc(&x.agent),
                        esc(&x.status),
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
                    say(format!(
                        "memory\t{}\t{}\t{}\t{}\t{}",
                        esc(&e.id),
                        esc(&e.title),
                        esc(e.tags.join(" ")),
                        esc(&e.author),
                        esc(&e.body)
                    ))
                    .await;
                }
                say(format!("memory-proposals\t{}", m.open_proposals)).await;
            }
            Err(e) => fail(e.0).await,
        },
        "handoffs" => match b.handoffs(ws.as_ref().unwrap()).await {
            Ok(h) => {
                for x in h {
                    say(format!("handoff\t{}\t@{}\t@{}\t{}", esc(&x.chat), esc(&x.from), esc(&x.to), esc(&x.task))).await;
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
                // A chat with a proposal waiting on your vote needs you.
                let props = b.proposals(ws.as_ref().unwrap()).await.unwrap_or_default();
                for (i, x) in c.iter().enumerate() {
                    let needs_you =
                        props.iter().any(|p| p.chat.as_ref() == Some(&x.id) && p.status == ProposalStatus::Open && p.can_vote.is_yes());
                    let flags = format!(
                        "{}{}{}{}",
                        if x.unread { "unread " } else { "" },
                        if x.mentioned { "mentioned " } else { "" },
                        if needs_you { "needs-you " } else { "" },
                        if x.lockdown.is_some() { "locked" } else { "" }
                    );
                    say(format!("chat\t{}\t{}\t{}\t{}", i + 1, esc(&x.id), esc(&x.title), flags.trim())).await;
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
            say(format!("opened\t{}\t{}", esc(&chat.id), esc(&chat.title))).await;
            if let Some(l) = &chat.lockdown {
                say(format!("lockdown\t{}\t{}", esc(l.reasons.join("; ")), esc(l.clear.reason().unwrap_or("you can clear it: unlock"))))
                    .await;
            }
            let ws_name = { s.lock().await.workspaces.iter().find(|x| x.id == w).map(|x| x.name.clone()).unwrap_or_default() };
            match b.transcript(&w, &chat.id).await {
                Ok(t) => {
                    for m in t {
                        say(format!("message\t{}\t{}\t{}\t{}", esc(&ws_name), esc(&chat.id), esc(m.author.display()), esc(&m.text))).await;
                    }
                }
                Err(e) => fail(e.0).await,
            }
            let _ = b.mark_read(&w, &chat.id).await;
        }
        "new" => match b.create_chat(ws.as_ref().unwrap(), arg).await {
            Ok(c) => {
                s.lock().await.chat = Some(c.id.clone());
                say(format!("opened\t{}\t{}", esc(&c.id), esc(&c.title))).await;
            }
            Err(e) => fail(e.0).await,
        },
        "say" => {
            let chat = { s.lock().await.chat.clone() };
            let Some(chat) = chat else { return fail("open a chat first: open N".into()).await };
            match b.send_message(ws.as_ref().unwrap(), &chat, &arg.replace("\\n", "\n")).await {
                Ok(m) => say(format!("sent\t{}\t{}", esc(&chat), esc(&m.id))).await,
                Err(e) => fail(e.0).await,
            }
        }
        "stop" => {
            let chat = { s.lock().await.chat.clone() };
            let Some(chat) = chat else { return fail("open a chat first: open N".into()).await };
            match b.stop_turn(ws.as_ref().unwrap(), &chat).await {
                Ok(0) => fail("nothing runs on this device in this chat: the turn's host stops it".into()).await,
                Ok(n) => say(format!("stopped\t{}\t{n}", esc(&chat))).await,
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
                        esc(&x.id),
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
                            say(format!("voted\t{}\t{}\t{}/{}\t{}", esc(&p.id), p.status.label(), p.approvals, p.quorum, esc(&p.title)))
                                .await
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
                        Ok(p) => say(format!("apply\t{}\t{}", esc(&p.id), esc(crate::ui::apply_text(&p.apply).unwrap_or_default()))).await,
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
                        "agent\t{}\t@{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        i + 1,
                        esc(&x.handle),
                        esc(&x.runtime),
                        esc(&host),
                        esc(x.role.as_deref().unwrap_or("agent")),
                        x.status.label(),
                        esc(x.doing.as_deref().unwrap_or("")),
                        esc(&x.access),
                        esc(x.unavailable.as_ref().map(|r| format!("unavailable: {r}")).unwrap_or_else(|| "available".into())),
                    ))
                    .await;
                    // This device's counts, for an agent you manage (UTC
                    // days; a runtime reporting no usage counts turns only).
                    if let Some(u) = &x.usage {
                        for (period, rows) in [(u.day.as_str(), &u.today), ("month", &u.month)] {
                            for r in rows {
                                let cost = r.cost_micro_usd.map(|c| format!("{:.4}", c as f64 / 1_000_000.0)).unwrap_or_else(|| "-".into());
                                say(format!(
                                    "usage\t@{}\t{}\t{}\t{}\t{}\t{}",
                                    esc(&x.handle),
                                    esc(period),
                                    esc(&r.member),
                                    r.turns,
                                    r.uncosted,
                                    cost
                                ))
                                .await;
                            }
                        }
                    }
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
                    say(format!("runtime\t{}\t{}\t{}\t{st}", esc(&x.id), x.kind.label(), esc(x.model.as_deref().unwrap_or("default"))))
                        .await;
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
                        esc(&x.id),
                        esc(&x.label),
                        if x.online { "online" } else { "offline" },
                        if x.busy { "busy" } else { "idle" },
                        esc(x.agents.iter().map(|a| format!("@{a}")).collect::<Vec<_>>().join(" ")),
                        esc(x.repos.join(" "))
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
                    say(format!("member\t{}\t{}\t{}\t{}\t{}\t{remove}", i + 1, esc(&x.id), esc(&x.name), x.role.label(), tags.join(" ")))
                        .await;
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
        "access" => {
            let Some(w) = ws.clone() else { return fail("no workspace yet: create NAME or join CODE".into()).await };
            let mut parts = arg.split_whitespace();
            let (which, to) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let everyone = match to {
                "everyone" => true,
                "owner" | "me" => false,
                other => return fail(format!("unknown access {other} (everyone, owner)")).await,
            };
            let agents = match b.agents(&w).await {
                Ok(a) => a,
                Err(e) => return fail(e.0).await,
            };
            let Some(a) = pick(&agents, which, |a| a.handle.as_str()).cloned() else { return fail(format!("no agent {which}")).await };
            match b.set_agent_access(&w, &a.id, everyone).await {
                Ok(()) => say(format!("ok\taccess\t@{}\t{}", esc(&a.handle), if everyone { "everyone" } else { "owner" })).await,
                Err(e) => fail(e.0).await,
            }
        }
        "available" => match b.local_runtimes(ws.as_ref().unwrap()).await {
            Ok(l) => {
                if l.is_empty() {
                    say("available\tnone\tinstall Claude Code, Codex, the Copilot CLI or agy, start Ollama, or set an API key".into())
                        .await;
                }
                for (i, x) in l.iter().enumerate() {
                    let state = match (&x.unsupported, x.ready) {
                        (Some(_), _) => "unsupported",
                        (None, true) => "ready",
                        (None, false) => "not ready",
                    };
                    let used = if x.used_by.is_empty() {
                        "unused".to_string()
                    } else {
                        format!("used by {}", x.used_by.iter().map(|a| format!("@{a}")).collect::<Vec<_>>().join(" "))
                    };
                    say(format!(
                        "available\t{}\t{}\t{}\t{}\t{}\t{state}\t{}\t{used}\t{}",
                        i + 1,
                        esc(&x.id),
                        x.kind.label(),
                        esc(x.model.as_deref().unwrap_or("default")),
                        esc(x.version.as_deref().unwrap_or("")),
                        esc(x.unsupported.as_deref().unwrap_or(&x.detail)),
                        esc(&x.suggested_name),
                    ))
                    .await;
                }
                s.lock().await.local = l;
            }
            Err(e) => fail(e.0).await,
        },
        "add-agent" => {
            let w = ws.unwrap();
            if s.lock().await.local.is_empty() {
                match b.local_runtimes(&w).await {
                    Ok(l) => s.lock().await.local = l,
                    Err(e) => return fail(e.0).await,
                }
            }
            let mut parts = arg.split_whitespace();
            let which = parts.next().unwrap_or("");
            let l = { pick(&s.lock().await.local, which, |l| l.id.as_str()).cloned() };
            let Some(l) = l else { return fail(format!("no runtime {which} (list them with available)")).await };
            let name = parts.next().map(str::to_string).unwrap_or_else(|| l.suggested_name.clone());
            match b.add_local_agent(&w, &l.id, &name).await {
                Ok(handle) => {
                    s.lock().await.local.clear();
                    say(format!("ok\tadd-agent\t@{}\t{}\towner\tthis device", esc(&handle), esc(&l.id))).await
                }
                Err(e) => fail(e.0).await,
            }
        }
        "invite" => match b.create_invite(ws.as_ref().unwrap(), Role::Contributor).await {
            Ok(code) => say(format!("invite\t{}", esc(&code))).await,
            Err(e) => fail(e.0).await,
        },
        "notifications" => match b.notifications().await {
            Ok(n) => {
                for x in n {
                    say(format!("notification\t{}\t{}\t{}", if x.read { "read" } else { "unread" }, esc(&x.workspace), esc(&x.text))).await;
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

    /// Errors map to the shared exit codes.
    #[test]
    fn errors_map_to_the_exit_code_table() {
        for (msg, code) in [
            ("usage: gate RUN approve|reject", 2),
            ("no workspace yet: create NAME or join CODE", 3),
            ("open a chat first: open N", 3),
            ("cancelled", 4),
            ("no chat 9", 5),
            ("no such host", 5),
            ("relay unreachable: connection refused", 7),
            ("Can't remove Ada: nobody cuts the workspace's founder", 6),
            ("a Viewer reads but doesn't post", 6),
            ("run run-6 isn't waiting on a gate", 1),
        ] {
            assert_eq!(error_code(msg).1, code, "{msg}");
        }
    }

    /// stop with nothing running here says so; it needs an open chat.
    #[tokio::test]
    async fn stop_needs_a_turn_running_here() {
        let out = run_script(FakeBackend::demo(FakeConfig::instant()), "stop\nopen 1\nstop\n").await;
        assert!(out.contains("open a chat first"), "{out}");
        assert!(out.contains("the turn's host stops it"), "{out}");
    }

    #[tokio::test]
    async fn script_lists_opens_and_streams() {
        let out =
            run_script(FakeBackend::demo(FakeConfig::instant()), "chats\nopen 1\nsay @coder fix it\nwait 5\nproposals\nmembers\n").await;
        assert!(out.contains("chat\t1\tc-checkout\tFix flaky checkout test\tunread mentioned"), "{out}");
        assert!(out.contains("message\tNorthwind API\tc-checkout\tAda\tThe checkout test fails about one run in ten."), "{out}");
        assert!(out.contains("thinking\tc-checkout\t@coder"), "{out}");
        assert!(out.contains("message\tNorthwind API\tc-checkout\t@coder\tLooked at the checkout test."), "{out}");
        assert!(out.contains("Inject a clock into the session check"), "{out}");
        assert!(out.contains("member\t4\tm-sam\tSam\tviewer\t\tremove: allowed"), "{out}");
    }

    /// Add to my devices through the approval address: the request (for an
    /// older desktop too), the code, match yes, the verified person,
    /// link-as yes. A one-shot link-to exits 4: a person compares the code.
    #[tokio::test]
    async fn link_to_compares_the_code_and_links_as_the_verified_person() {
        let addr = crate::backend::fake::DEMO_ADDRESS;
        let script = format!("link-to {addr}\nwait 1\nmatch yes\nwait 2\nlink-as yes\n");
        let out = run_script(FakeBackend::new(FakeConfig::instant()), &script).await;
        assert!(out.contains("link-request\t7F3A-91C0-77DE-0B14-5E62\t"), "{out}");
        assert!(out.contains("link-waiting\trid-"), "{out}");
        assert!(out.contains("match-code\trid-") && out.contains("\tK7Q4 M9X2\t"), "{out}");
        assert!(out.contains("link-verified\trid-") && out.contains("\tAlex\tNorthwind API\t"), "{out}");
        assert!(out.contains("linked\t") && out.contains("\tAlex"), "{out}");

        let script = format!("link-to {addr}\nwait 1\nmatch no\nwait 1\nlink-as yes\n");
        let out = run_script(FakeBackend::new(FakeConfig::instant()), &script).await;
        assert!(out.contains("link-ended\trid-") && out.contains("Request didn't match. Nothing was granted."), "{out}");
        assert!(!out.contains("linked\t"), "{out}");

        let (code, out) = one_shot(FakeBackend::new(FakeConfig::instant()), &format!("link-to {addr}")).await;
        assert_eq!(code, 4, "{out}");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "approvals").await;
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("approval\t1\tbox\tbuild-2\t") && out.contains("“Ada's desktop” is checking it"), "{out}");
        assert!(out.contains("approve on your desktop or phone"), "{out}");
    }

    #[tokio::test]
    async fn first_run_create_prints_recovery_words() {
        let out =
            run_script(FakeBackend::new(FakeConfig::instant()), "workspaces\nname Mara\ncreate Acme Web\nwait 1\nconfirm 101\nwait 2\n")
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

    /// The passkey page's address (a one-time secret that can answer a
    /// passkey creation) never reaches stdout.
    #[test]
    fn the_passkey_address_never_reaches_stdout() {
        let url = "https://ligus.ai/presence#op=create&c=Y2hhbA&p=8765&n=c2VjcmV0LW5vbmNl".to_string();
        let line = event_line(&Event::PresenceUrl { id: PresenceId(3), url, port: 8765 }, &|_| String::new()).unwrap();
        assert!(line.starts_with("presence-url\t3\t8765\t-\t"), "{line}");
        assert!(!line.contains("c2VjcmV0") && !line.contains("Y2hhbA"), "{line}");
    }

    /// A one-shot create on a new device: refused without a name, done with
    /// --name.
    #[tokio::test]
    async fn one_shot_create_takes_its_name_from_the_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let cmd = format!("create Acme Web --out {}/w", tmp.path().display());
        let (code, out) = one_shot(FakeBackend::new(FakeConfig::instant()), &cmd).await;
        assert_eq!(code, 1);
        assert!(out.contains("tell us your name first"), "{out}");
        let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
        let opts = PlainOpts { one_shot: Some(cmd), name: Some("Ada".into()), ..Default::default() };
        let code = run_with(Arc::new(FakeBackend::new(FakeConfig::instant())), &b""[..], Box::new(Sink(buf.clone())), opts).await.unwrap();
        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("created\t"), "{out}");
    }

    /// One-shot: only the command's records, tab-separated, and an exit code.
    #[tokio::test]
    async fn one_shot_lists_workflows_runs_and_hosts() {
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "workflows").await;
        assert_eq!(code, 0);
        assert_eq!(
            out,
            "workflow\t1\twf-review\tImplement, review, ship\tFix flaky checkout test\timplement:@coder review:@reviewer signoff:gate (1 approval)\tstartable\n\
             workflow\t2\twf-report\tWeekly security digest\tMigrate auth to passkeys\tsummarize:@scout\tstartable\n"
        );
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "runs").await;
        assert_eq!(code, 0);
        assert!(out.starts_with("run\t1\trun-7\tawaiting-gate\tImplement, review, ship\t2/3\tThis device\tinject a fake clock\n"), "{out}");
        let (_, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "hosts").await;
        assert!(out.contains("host\t2\th-build-box\tbuild-box\tworker\tonline\t\tbusy\t@coder @scout\tnorthwind-api\n"), "{out}");
        let (_, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "proposals").await;
        assert!(out.lines().any(|l| l.starts_with("proposal\t") && l.ends_with("\tgate")), "{out}");
        let (_, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "tasks").await;
        assert!(out.contains("task\tc-checkout\t@coder\tworking\t40\tactive\twiring the CSV export"), "{out}");
    }

    /// Agent access controls: each agent says who may use it and whether
    /// you may; an agent you manage lists this device's counts.
    #[tokio::test]
    async fn agents_say_who_can_use_them_and_their_usage() {
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "agents").await;
        assert_eq!(code, 0);
        let coder = out.lines().find(|l| l.contains("\t@coder\t")).unwrap();
        assert!(coder.ends_with("\towner\tavailable"), "{coder}");
        assert!(out.lines().any(|l| l.ends_with("\teveryone\tavailable") && l.contains("@scout")), "{out}");
        assert!(out.contains("usage\t@coder\t2026-11-20\tMara\t3\t0\t0.0120\n"), "{out}");
        assert!(out.contains("usage\t@coder\tmonth\tBen\t2\t2\t-\n"), "{out}");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "access coder everyone").await;
        assert_eq!((code, out.as_str()), (0, "ok\taccess\t@coder\teveryone\n"));
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "access scout owner").await;
        assert_eq!(code, 6, "only its owner, refused: {out}");
        let (code, _) = one_shot(FakeBackend::demo(FakeConfig::instant()), "access coder nobody").await;
        assert_eq!(code, 2, "a usage error");
    }

    /// `available` lists what this device could run agents on; `add-agent`
    /// adds one (yours, only you, this device); nothing found says so.
    #[tokio::test]
    async fn available_runtimes_and_add_agent() {
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "available").await;
        assert_eq!(code, 0);
        assert!(
            out.contains("available\t1\tlocal:claude-code\tClaude Code\tdefault\t2.1.3\tready\tsigned in (claude login)\tused by @ligus\t"),
            "{out}"
        );
        assert!(out.lines().any(|l| l.contains("local:copilot") && l.contains("\tnot ready\t") && l.contains("\tunused\t")), "{out}");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "add-agent 3").await;
        assert_eq!((code, out.as_str()), (0, "ok\tadd-agent\t@copilot\tlocal:copilot\towner\tthis device\n"));
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "add-agent local:codex reviewer").await;
        assert_eq!(code, 1, "a taken name is refused: {out}");
        assert!(out.contains("already taken"), "{out}");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()).with_local(vec![]), "available").await;
        assert_eq!(code, 0);
        assert!(out.starts_with("available\tnone\t"), "{out}");
        let (code, _) = one_shot(FakeBackend::demo(FakeConfig::instant()), "add-agent 9").await;
        assert_eq!(code, 5, "not found");
    }

    /// `run` starts a workflow and follows it to the end: exit 0 when it
    /// completes. Unknown commands exit 2; errors exit 1.
    #[tokio::test]
    async fn one_shot_run_follows_to_the_end_with_exit_codes() {
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "run 2 this week").await;
        assert_eq!(code, 0, "{out}");
        assert!(out.lines().next().unwrap().starts_with("run\t1\trun-"), "{out}");
        assert!(out.contains("\trunning\tWeekly security digest\t0/1\tThis device\tthis week"), "{out}");
        assert!(out.contains("\tsummarize\tsucceeded\t@scout\t\t@scout finished: this week"), "{out}");
        assert!(out.trim_end().lines().last().unwrap().starts_with("run-done\trun-"), "{out}");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "frobnicate").await;
        assert_eq!(code, EXIT_USAGE, "{out}");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "gate 2 approve").await;
        assert_eq!(code, 1);
        assert!(out.contains("error\trun run-6 isn't waiting on a gate"), "{out}");
        let (code, _) = one_shot(FakeBackend::demo(FakeConfig::instant()), "watch nope").await;
        assert_eq!(code, 5, "not found");
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

    /// Nothing from the network reaches the terminal raw: no escape
    /// sequences (ANSI, OSC), no bidi overrides, no stray line breaks
    /// that would forge a record.
    #[tokio::test]
    async fn network_text_is_sanitised_before_printing() {
        let out = run_script(
            FakeBackend::demo(FakeConfig::instant()),
            "open 1\nsay hi \u{1b}]52;c;cm0gLXJmIH4=\u{7} \u{1b}[2J\u{202e}gnp.exe\r\u{9b}31m\u{2066}x\nopen 1\n",
        )
        .await;
        assert!(out.contains("hi "), "{out}");
        for bad in ['\u{1b}', '\u{7}', '\u{202e}', '\u{2066}', '\u{9b}', '\r'] {
            assert!(!out.contains(bad), "{bad:?} reached the terminal: {out:?}");
        }
    }

    /// Every field that came from another member, an agent or a box is
    /// escaped: a tab in an agent's handle, a host's id, a stage's id or a
    /// hand-off's names can't forge a field, a newline can't forge a record.
    #[tokio::test]
    async fn no_network_field_forges_a_field_or_a_record() {
        let bad = "x\tforged\nfake-record\t\u{1b}[31m";
        for cmd in ["agents", "hosts", "runtimes", "tasks", "handoffs", "workflows", "runs", "run 2 this week"] {
            let (_, out) = one_shot(FakeBackend::hostile(FakeConfig::instant(), bad), cmd).await;
            assert!(!out.contains("\tforged"), "{cmd}: a field was forged: {out:?}");
            assert!(!out.lines().any(|l| l.starts_with("fake-record")), "{cmd}: a record was forged: {out:?}");
            assert!(!out.contains('\u{1b}'), "{cmd}: {out:?}");
        }
    }

    fn private(dir: &std::path::Path, name: &str, text: &str, mode: u32) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let _ = mode;
        p
    }

    #[cfg(unix)]
    fn mode_of(p: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    /// One-shot `connect`: the key never rides argv (only @FILE), a key file
    /// others can read is refused, and the recovery words go to an
    /// owner-only file, never to stdout.
    #[tokio::test]
    async fn one_shot_connect_keeps_the_key_and_words_off_argv_and_stdout() {
        let tmp = tempfile::tempdir().unwrap();
        let demo = || FakeBackend::demo(FakeConfig::instant());
        let (code, out) = one_shot(demo(), "connect http://127.0.0.1:18443 k8s-room the-workspace-key").await;
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("@FILE"), "{out}");
        #[cfg(unix)]
        {
            let open = private(tmp.path(), "open-key", "the-workspace-key\n", 0o644);
            let (code, out) =
                one_shot(demo(), &format!("connect http://127.0.0.1:18443 k8s-room @{} --out {}/w1", open.display(), tmp.path().display()))
                    .await;
            assert_eq!(code, 1, "{out}");
            assert!(out.contains("others can read"), "{out}");
        }
        let key = private(tmp.path(), "key", "the-workspace-key\n", 0o600);
        let (code, out) = one_shot(demo(), &format!("connect http://127.0.0.1:18443 k8s-room @{}", key.display())).await;
        assert_eq!(code, 2, "a usage error: {out}");
        assert!(out.contains("--out"), "{out}");
        let words = tmp.path().join("words.txt");
        let (code, out) =
            one_shot(demo(), &format!("connect http://127.0.0.1:18443 k8s-room @{} --out {}", key.display(), words.display())).await;
        assert_eq!(code, 0, "{out}");
        let saved = std::fs::read_to_string(&words).unwrap();
        assert_eq!(saved.split_whitespace().count(), 24);
        assert!(!out.contains(saved.trim()), "{out}");
        assert!(!out.contains("the-workspace-key"), "{out}");
        #[cfg(unix)]
        assert_eq!(mode_of(&words), 0o600);
        // An existing file is never overwritten (or followed).
        let (code, out) =
            one_shot(demo(), &format!("connect http://127.0.0.1:18443 other-room @{} --out {}", key.display(), words.display())).await;
        assert_eq!(code, 1, "{out}");
    }

    /// A bearer admission carries the workspace key: plain mode never prints
    /// it, it writes it owner-only where `--out` says.
    #[tokio::test]
    async fn a_bearer_admission_is_never_printed() {
        let tmp = tempfile::tempdir().unwrap();
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "admit").await;
        assert_eq!(code, 1, "{out}");
        assert!(!out.contains("fake-bearer-admission"), "{out}");
        let out_script = run_script(FakeBackend::demo(FakeConfig::instant()), "admit\nwait 1\n").await;
        assert!(!out_script.contains("fake-bearer-admission"), "{out_script}");
        let file = tmp.path().join("admission");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), &format!("admit --out {}", file.display())).await;
        assert_eq!(code, 0, "{out}");
        assert!(!out.contains("fake-bearer-admission"), "{out}");
        assert!(out.lines().any(|l| l.starts_with("admission\tbearer\t")), "{out}");
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), "hivews1:fake-bearer-admission");
        #[cfg(unix)]
        assert_eq!(mode_of(&file), 0o600);
        // A sealed code opens only on its box: printed as before.
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "admit ligusjoin1:abc").await;
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("ligusadmit1:sealed-for-"), "{out}");
    }

    /// The box's fingerprint is printed before the passkey prompt, so it's
    /// compared with the box's terminal before anyone confirms; a request
    /// that doesn't read raises no prompt at all.
    #[tokio::test]
    async fn admitting_prints_the_fingerprint_before_the_passkey_prompt() {
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "admit ligusjoin1:abc").await;
        assert_eq!(code, 0, "{out}");
        let fp = out.find("joinrequest\tK3QF-7XMA-2DPL-90RW-ZT4E\tbuild-box").expect(&out);
        let prompt = out.find("presence\t").expect(&out);
        assert!(fp < prompt, "fingerprint after the passkey prompt: {out}");
        let (code, out) = one_shot(FakeBackend::demo(FakeConfig::instant()), "admit nonsense").await;
        assert_eq!(code, 1, "{out}");
        assert!(!out.contains("presence\t"), "{out}");
    }
}
