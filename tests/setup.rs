//! Guided setup in the full-screen app: the Set up list, a flow's popup
//! with each kind of field, the run's checks (a failure with its fix, a
//! person's hand-off), cancel, and that a secret never reaches the screen,
//! the state's `Debug`, the effects or the messages log. The demo backend
//! runs the real engine (`ligus_runtime::setup`) over an in-memory device.
//!
//! `LT_WRITE_SCREENS=1` also writes each screen to `docs/screens/`.

mod common;

use common::Harness;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ligus_runtime::setup::FlowId;
use ligus_term::app::setup::{Stage, Wizard};
use ligus_term::app::state::Overlay;
use ligus_term::app::{Effect, Msg, update};
use ligus_term::setup::{CancelFlag, LtFlow, SetupInputs, SetupSecrets};

const W: u16 = 110;
const H: u16 = 32;
const CANARY: &str = "tok-canary-7f3a9";

fn snap(h: &Harness, name: &str, title: &str) {
    let screen = h.screen(W, H);
    if std::env::var_os("LT_WRITE_SCREENS").is_some() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/screens");
        std::fs::create_dir_all(&dir).unwrap();
        let md = format!("# {title}\n\nRendered by `tests/setup.rs` ({W}×{H}, the in-memory demo backend).\n\n```text\n{screen}```\n");
        std::fs::write(dir.join(format!("{name}.md")), md).unwrap();
    }
    insta::assert_snapshot!(name, screen);
    // The default terminal keeps the popup's keys.
    let small = h.screen(80, 24);
    assert!(small.lines().any(|l| l.contains("esc") || l.contains("enter") || l.contains("any key")), "{name} at 80x24:\n{small}");
}

fn wizard(h: &Harness) -> &Wizard {
    match &h.state.overlay {
        Some(Overlay::SetupFlow(w)) => w,
        o => panic!("no setup popup: {o:?}"),
    }
}

/// The demo, with the composer left (the first chat opens with it).
async fn demo() -> Harness {
    let mut h = Harness::new(true).await;
    h.key(KeyCode::Esc).await;
    h
}

/// `u`, then the flow's row.
async fn open(h: &mut Harness, row: usize) {
    h.key(KeyCode::Char('u')).await;
    for _ in 0..row {
        h.key(KeyCode::Down).await;
    }
    h.key(KeyCode::Enter).await;
    formed(h).await;
}

/// Wait for the flow's form (a slow machine answers after the harness's
/// quiet spell).
async fn formed(h: &mut Harness) {
    for _ in 0..100 {
        if matches!(&h.state.overlay, Some(Overlay::SetupFlow(w)) if w.form.is_some()) {
            return;
        }
        h.settle().await;
    }
}

/// Wait for a run to finish.
async fn finished(h: &mut Harness) {
    for _ in 0..100 {
        if !matches!(&h.state.overlay, Some(Overlay::SetupFlow(w)) if matches!(w.stage, Stage::Running)) {
            return;
        }
        h.settle().await;
    }
}

#[tokio::test]
async fn the_list_shows_each_flow_and_what_is_done() {
    let mut h = demo().await;
    h.key(KeyCode::Char('u')).await;
    let s = h.screen(W, H);
    for t in ["Add an agent", "Connect a tool", "Add a code folder", "Add a box", "Add a phone", "Invite a person", "Set up a relay"] {
        assert!(s.contains(t), "{t}\n{s}");
    }
    assert!(s.contains("4 agents") && s.contains("✓"), "{s}");
    assert!(s.contains("Add a computer") && s.contains("not linked to your devices"), "the device flow: {s}");
    snap(&h, "setup_list", "Set up: the guided flows and what is done");
    h.key(KeyCode::Esc).await;
    assert!(h.state.overlay.is_none());
}

/// The Set up Ligus rows lead the list: exactly the shared fixture's rows
/// (the same ones the desktop, `ligus setup` and the phone render), each
/// with its mark, title and detail, grey for what was never set up.
#[tokio::test]
async fn the_list_leads_with_the_set_up_ligus_rows() {
    use ligus_runtime::setup::checklist::{Checklist, fixture};
    let mut h = demo().await;
    h.key(KeyCode::Char('u')).await;
    loaded(&mut h).await;
    let want: Checklist = serde_json::from_str(fixture::SETTLED_TEAM_CHECKLIST).unwrap();
    assert_eq!(h.state.setup_checklist.as_ref(), Some(&want), "the shared fixture's rows");
    let s = h.screen(W, H);
    let mut at = 0;
    for r in &want.rows {
        let mark = ligus_runtime::setup::checklist::mark(r.state);
        let line = s.lines().enumerate().skip(at).find(|(_, l)| l.contains(&format!("{mark} {}", r.title)));
        let (i, l) = line.unwrap_or_else(|| panic!("{} in order\n{s}", r.title));
        let head: String = r.detail.chars().take(16).collect();
        assert!(l.contains(&head), "{}: {head}\n{l}", r.title);
        at = i + 1;
    }
    assert!(s.contains("x clean up"), "{s}");
    snap(&h, "setup_checklist", "Set up: the Set up Ligus checklist above the guided flows");
}

async fn loaded(h: &mut Harness) {
    for _ in 0..50 {
        if h.state.setup_checklist.is_some() {
            return;
        }
        h.settle().await;
    }
}

/// Clean up: Remove from view writes its key only after the confirm; a
/// removal from the workspace is the desktop's and changes nothing here.
#[tokio::test]
async fn clean_up_hides_only_after_the_confirm() {
    let mut h = demo().await;
    h.key(KeyCode::Char('u')).await;
    loaded(&mut h).await;
    h.key(KeyCode::Char('x')).await;
    h.settle().await;
    assert!(matches!(h.state.overlay, Some(Overlay::Cleanup { sel: 0, confirming: false })), "{:?}", h.state.overlay);
    let items = h.state.setup_checklist.as_ref().unwrap().cleanup.clone();
    let count = items.len();
    let revoked = items.iter().position(|i| i.key.starts_with("deviceRevoked:")).unwrap();
    let remove = items.iter().position(|i| i.key.starts_with("memberLeft:")).unwrap();

    // A removal: said where it's done, nothing hidden.
    for _ in 0..remove {
        h.key(KeyCode::Down).await;
    }
    h.key(KeyCode::Enter).await;
    h.settle().await;
    assert!(h.state.toasts.iter().any(|t| t.text.contains("on your desktop")), "{:?}", h.state.toasts);
    assert_eq!(h.state.setup_checklist.as_ref().unwrap().cleanup.len(), count);

    // Remove from view: the confirm, declined, keeps it.
    for _ in 0..remove {
        h.key(KeyCode::Up).await;
    }
    for _ in 0..revoked {
        h.key(KeyCode::Down).await;
    }
    h.key(KeyCode::Enter).await;
    assert!(matches!(h.state.overlay, Some(Overlay::Cleanup { confirming: true, .. })));
    snap(&h, "setup_cleanup_confirm", "Set up: Clean up asks before removing a leftover from view");
    h.key(KeyCode::Char('n')).await;
    h.settle().await;
    assert_eq!(h.state.setup_checklist.as_ref().unwrap().cleanup.len(), count, "declined: nothing hidden");
    // Accepted: hidden here, gone from the list.
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Char('y')).await;
    for _ in 0..50 {
        if h.state.setup_checklist.as_ref().unwrap().cleanup.len() < count {
            break;
        }
        h.settle().await;
    }
    let left = &h.state.setup_checklist.as_ref().unwrap().cleanup;
    assert_eq!(left.len(), count - 1);
    assert!(left.iter().all(|i| !i.key.starts_with("deviceRevoked:")));
}

#[tokio::test]
async fn the_palette_opens_a_flow_by_name() {
    let mut h = demo().await;
    h.key(KeyCode::Char(':')).await;
    h.chars("connect a tool").await;
    h.key(KeyCode::Enter).await;
    assert_eq!(wizard(&h).flow, LtFlow::Engine(FlowId::Tool));
    assert!(!wizard(&h).from_list);
}

/// An agent, step by step: text (the name), a choice (what answers, with
/// sign-in hints and lt's limits), a suggestion (the model), where it runs
/// (with its one line), a toggle (the test turn), the review with the one
/// button's words, then done.
#[tokio::test]
async fn an_agent_step_by_step() {
    let mut h = demo().await;
    open(&mut h, 0).await;
    assert_eq!(wizard(&h).stage, Stage::Fill);
    h.chars("helper").await;
    snap(&h, "setup_agent_name", "Add an agent: its name");
    h.key(KeyCode::Enter).await;
    let s = h.screen(W, H);
    assert!(s.contains("Claude Code") && s.contains("signed in · this comp"), "{s}");
    assert!(s.contains("not in lt yet"), "Aider and Pi are shown disabled: {s}");
    assert!(s.contains("gw-home (OpenAI API)") && s.contains("offered by build-box"), "a box's gateway: {s}");
    snap(&h, "setup_agent_source", "Add an agent: what answers");
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Down).await;
    assert_eq!(wizard(&h).inputs().get("model").map(String::as_str), Some("sonnet"), "↓ takes a suggestion");
    snap(&h, "setup_agent_model", "Add an agent: a model (suggestions)");
    h.key(KeyCode::Enter).await;
    assert_eq!(wizard(&h).inputs().get("runs-on").map(String::as_str), Some("here"));
    assert!(h.screen(W, H).contains("Turns run on this device while Ligus is open."));
    snap(&h, "setup_agent_runs_on", "Add an agent: where it runs");
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Char(' ')).await;
    snap(&h, "setup_agent_test", "Add an agent: a test turn (a toggle)");
    h.key(KeyCode::Enter).await;
    assert_eq!(wizard(&h).stage, Stage::Review);
    assert!(h.screen(W, H).contains("enter Add @helper"), "the one button's words");
    snap(&h, "setup_agent_review", "Add an agent: ready to run");
    h.key(KeyCode::Enter).await;
    let Stage::Done(o) = &wizard(&h).stage else { panic!("{:?}", wizard(&h).stage) };
    assert_eq!(o.exit, 0, "{o:?}");
    snap(&h, "setup_agent_done", "Add an agent: done");
    assert!(h.state.agents.iter().any(|a| a.handle == "helper"), "the agents list reloaded");
    h.key(KeyCode::Enter).await;
    assert!(matches!(h.state.overlay, Some(Overlay::Setup { .. })), "back to the list it came from");
}

/// A gateway a box offers: its own models (auto first), Runs on is the box
/// with its one line, and one agent is written placed on it.
#[tokio::test]
async fn a_gateway_on_a_box_in_one_step() {
    let mut h = demo().await;
    open(&mut h, 0).await;
    h.chars("builder").await;
    h.key(KeyCode::Enter).await;
    // The CLIs come first; the gateway after them.
    while wizard(&h).inputs().get("source").map(String::as_str) != Some("gateway:gw-home") {
        h.key(KeyCode::Down).await;
    }
    h.key(KeyCode::Enter).await;
    assert_eq!(wizard(&h).inputs().get("model").map(String::as_str), Some("auto"), "the gateway's first model");
    assert!(h.screen(W, H).contains("qwen3-coder"));
    h.key(KeyCode::Enter).await;
    assert_eq!(wizard(&h).inputs().get("runs-on").map(String::as_str), Some("h-build-box"), "the one host that offers it");
    let s = h.screen(W, H);
    assert!(s.contains("Turns run on build-box. When it's offline, @builder waits."), "{s}");
    assert!(!s.contains("This device"), "a gateway never runs here: {s}");
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    assert_eq!(wizard(&h).stage, Stage::Review);
    assert!(h.screen(W, H).contains("enter Add @builder"));
    h.key(KeyCode::Enter).await;
    finished(&mut h).await;
    let Stage::Done(o) = &wizard(&h).stage else { panic!("{:?}", wizard(&h).stage) };
    assert_eq!(o.exit, 0, "{o:?}");
    let ws = h.state.active.clone().unwrap();
    let agents = h.backend.agents(&ws).await.unwrap();
    let builder: Vec<_> = agents.iter().filter(|a| a.handle == "builder").collect();
    assert_eq!(builder.len(), 1, "one agent");
    assert_eq!(builder[0].host.as_ref().map(|h| h.as_str()), Some("h-build-box"), "placed on the box");
}

/// A passkey run that takes two confirmations says which one it is.
#[tokio::test]
async fn a_run_says_which_confirmation_it_asks_for() {
    let mut h = demo().await;
    let mut w = Wizard::new(LtFlow::Engine(FlowId::Agent), false);
    w.stage = Stage::Running;
    h.state.overlay = Some(Overlay::SetupFlow(Box::new(w)));
    let ev = |step, of| ligus_term::backend::Event::SetupConfirming { step, of, what: "Add @coder".into() };
    update(&mut h.state, Msg::Event(ev(2, 2)));
    assert!(h.screen(W, H).contains("Confirm 2 of 2: Add @coder"));
    // One confirmation needs no count.
    let mut w = Wizard::new(LtFlow::Engine(FlowId::Agent), false);
    w.stage = Stage::Running;
    h.state.overlay = Some(Overlay::SetupFlow(Box::new(w)));
    update(&mut h.state, Msg::Event(ev(1, 1)));
    assert!(!h.screen(W, H).contains("Confirm 1 of 1"));
}

/// A failed check shows its fix line; `r` goes back to the steps with the
/// values kept.
#[tokio::test]
async fn a_failed_check_shows_its_fix() {
    let mut h = demo().await;
    open(&mut h, 0).await;
    h.chars("coder2").await;
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Down).await; // Codex: installed here, not signed in
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    // Codex is signed in on the box, so Runs on starts there: pick here.
    while wizard(&h).inputs().get("runs-on").map(String::as_str) != Some("here") {
        h.key(KeyCode::Up).await;
    }
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    let Stage::Done(o) = &wizard(&h).stage else { panic!("{:?}", wizard(&h).stage) };
    assert_eq!(o.exit, 3);
    let s = h.screen(W, H);
    assert!(s.contains("Stopped") && s.contains("fix: run `codex login`"), "{s}");
    snap(&h, "setup_agent_failed", "Add an agent: a failed check and its fix");
    h.key(KeyCode::Char('r')).await;
    assert_eq!(wizard(&h).stage, Stage::Fill);
    assert_eq!(wizard(&h).inputs().get("name").map(String::as_str), Some("coder2"));
}

/// A tool with a bearer token: the token is typed into a secret field,
/// drawn as dots, and never appears on screen, in the state's Debug, in
/// the run's effect or in the messages log.
#[tokio::test]
async fn a_tool_token_is_masked_and_never_shown() {
    let mut h = demo().await;
    open(&mut h, 1).await;
    h.chars("issues").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Down).await; // a URL
    h.key(KeyCode::Tab).await;
    h.chars("https://mcp.example.com/v1").await;
    snap(&h, "setup_tool_server", "Connect a tool: the server");
    h.key(KeyCode::Enter).await;
    h.chars(CANARY).await;
    h.send(Msg::Paste("-more".into())).await;
    let s = h.screen(W, H);
    assert!(s.contains("••••••••"), "{s}");
    assert!(!s.contains(CANARY) && !s.contains("canary"), "{s}");
    snap(&h, "setup_tool_token", "Connect a tool: the token, masked");
    let dbg = format!("{:?}", h.state.overlay);
    assert!(!dbg.contains("canary"), "{dbg}");
    h.key(KeyCode::Enter).await; // reach: every agent
    snap(&h, "setup_tool_reach", "Connect a tool: which agents may use it");
    h.key(KeyCode::Enter).await;
    let s = h.screen(W, H);
    assert!(s.contains("•••••••• (only to https://mcp.example.com)") && !s.contains("canary"), "{s}");
    // The run's effect carries the secret only in its redacted secrets.
    let Some(Overlay::SetupFlow(w)) = h.state.overlay.as_mut() else { panic!() };
    let mut probe = (**w).clone();
    let fx = format!("{:?}", probe.take_secrets());
    assert!(fx.contains("***") && !fx.contains("canary"), "{fx}");
    h.key(KeyCode::Enter).await;
    let Stage::Done(o) = &wizard(&h).stage else { panic!("{:?}", wizard(&h).stage) };
    assert_eq!(o.exit, 0, "{o:?}");
    assert!(!o.json.to_string().contains("canary") && !format!("{o:?}").contains("canary"));
    assert!(!h.screen(W, H).contains("canary"));
    assert!(h.state.log.iter().all(|l| !l.text.contains("canary")));
    let Some(Overlay::SetupFlow(w)) = &h.state.overlay else { panic!() };
    assert!(w.secrets.values().all(|b| b.is_empty()), "the popup keeps no secret after the run");
}

/// A person's step: the invite is created where its code is shown, so the
/// run stops with exit 4 and says where; lt's own Invite is one key away.
#[tokio::test]
async fn an_invite_hands_off_to_a_person() {
    let mut h = demo().await;
    open(&mut h, 5).await;
    let s = h.screen(W, H);
    assert!(s.contains("A person does this"), "{s}");
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    let Stage::Done(o) = &wizard(&h).stage else { panic!("{:?}", wizard(&h).stage) };
    assert_eq!(o.exit, 4);
    let s = h.screen(W, H);
    assert!(s.contains("Waiting for a person") && s.contains("i create an invite here"), "{s}");
    snap(&h, "setup_invite_handoff", "Invite a person: the hand-off");
    h.key(KeyCode::Char('i')).await;
    assert!(matches!(h.state.overlay, Some(Overlay::Invite { .. })));
}

/// Pairing a phone prints a secret: it happens in the person's own
/// terminal (`ligus pair`), never here.
#[tokio::test]
async fn a_phone_pairs_in_your_own_terminal() {
    let mut h = demo().await;
    open(&mut h, 4).await;
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    let Stage::Done(o) = &wizard(&h).stage else { panic!("{:?}", wizard(&h).stage) };
    assert_eq!(o.exit, 4);
    assert!(h.screen(W, H).contains("ligus pair"));
    snap(&h, "setup_phone_handoff", "Add a phone: pairing in your own terminal");
}

/// A path field completes folders with tab (an effect: the reducer never
/// reads the disk).
#[tokio::test]
async fn a_folder_completes_and_binds() {
    let mut h = demo().await;
    open(&mut h, 2).await;
    // "Start with a scratch repo" comes first; the folder is next.
    h.key(KeyCode::Down).await;
    let fx = update(&mut h.state, Msg::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
    assert!(matches!(fx.as_slice(), [Effect::CompletePath(f, _)] if f == "path"), "{fx:?}");
    h.chars("~/code/northwind").await;
    snap(&h, "setup_folder_path", "Add a code folder: the folder");
    h.key(KeyCode::Enter).await;
    formed(&mut h).await;
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    finished(&mut h).await;
    let Stage::Done(o) = &wizard(&h).stage else { panic!("{:?}", wizard(&h).stage) };
    assert_eq!(o.exit, 0, "{o:?}");
}

/// "Start with a scratch repo": no folder needed; the device makes an
/// empty repository, adds it to the workspace as scratch and binds it.
#[tokio::test]
async fn a_folder_can_start_with_a_scratch_repo() {
    let mut h = demo().await;
    open(&mut h, 2).await;
    assert!(h.screen(W, H).contains("Start with a scratch repo"));
    h.key(KeyCode::Char(' ')).await;
    h.settle().await;
    snap(&h, "setup_folder_scratch", "Add a code folder: start with a scratch repo");
    h.key(KeyCode::Enter).await;
    formed(&mut h).await;
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    finished(&mut h).await;
    let Stage::Done(o) = &wizard(&h).stage else { panic!("{:?}\n{}", wizard(&h).stage, h.screen(W, H)) };
    assert_eq!(o.exit, 0, "{o:?}");
    assert_eq!(o.results.get("scratch").map(String::as_str), Some("true"), "{o:?}");
    // It is in the workspace now, tagged scratch, and checked out here.
    let ws = h.state.active.clone().unwrap();
    let c = h.backend.repo_choices(&ws).await.unwrap();
    assert!(c.repos.iter().any(|r| r.scratch && r.here && r.id == o.results["repoId"]), "{:?}", c.repos);
}

/// Esc before the run closes with nothing written; esc during it asks the
/// engine to stop, which undoes the run's own writes.
#[tokio::test]
async fn esc_cancels_cleanly() {
    let mut h = demo().await;
    open(&mut h, 0).await;
    let fx = update(&mut h.state, Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(
        fx.iter().all(|f| matches!(f, Effect::LoadSetup(_) | Effect::LoadChecklist(_) | Effect::LoadPendingPresence)),
        "nothing runs: {fx:?}"
    );
    assert!(matches!(h.state.overlay, Some(Overlay::Setup { .. })));

    // While running: esc is a cancel, once.
    let mut w = Wizard::new(LtFlow::Engine(FlowId::Agent), false);
    w.stage = Stage::Running;
    let flag = w.cancel.clone();
    h.state.overlay = Some(Overlay::SetupFlow(Box::new(w)));
    let fx = update(&mut h.state, Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert_eq!(fx, vec![Effect::CancelSetup(flag.clone())]);
    assert!(update(&mut h.state, Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))).is_empty());
    assert!(h.screen(W, H).contains("Stopping"));

    // The engine stops between steps and says so.
    let cancel = CancelFlag::default();
    cancel.cancel();
    let inputs: SetupInputs =
        [("harness", "claude-code"), ("name", "late")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let ws = h.state.active.clone();
    let o = h.backend.setup_run(ws.as_ref(), LtFlow::Engine(FlowId::Agent), inputs, SetupSecrets::new(), false, cancel).await.unwrap();
    assert_eq!(o.exit, 1);
    assert!(o.checks.iter().any(|c| c.id == "agent.cancelled"), "{o:?}");
    assert!(!h.backend.agents(ws.as_ref().unwrap()).await.unwrap().iter().any(|a| a.handle == "late"), "nothing was added");
}

/// A dry run checks and writes nothing.
#[tokio::test]
async fn a_dry_run_writes_nothing() {
    let mut h = demo().await;
    open(&mut h, 0).await;
    h.chars("dryrun").await;
    for _ in 0..5 {
        h.key(KeyCode::Enter).await;
    }
    h.key(KeyCode::Char('d')).await;
    h.key(KeyCode::Enter).await;
    let Stage::Done(o) = &wizard(&h).stage else { panic!() };
    assert!(o.dry_run && o.exit == 0, "{o:?}");
    assert!(h.screen(W, H).contains("Dry run: nothing was written"));
    assert!(!h.state.agents.iter().any(|a| a.handle == "dryrun"));
}

/// An empty view offers its flow as its one action.
#[tokio::test]
async fn an_empty_view_offers_its_flow() {
    let mut h = demo().await;
    h.key(KeyCode::Char('a')).await;
    h.state.agents.clear();
    assert!(h.screen(W, H).contains("No agents yet · enter set one up"));
    h.key(KeyCode::Enter).await;
    assert_eq!(wizard(&h).flow, LtFlow::Engine(FlowId::Agent));
}

/// A token typed for one URL never goes to another: changing the URL after
/// typing it clears it, and says so.
#[tokio::test]
async fn a_token_typed_for_one_url_is_cleared_when_the_url_changes() {
    let mut h = demo().await;
    open(&mut h, 1).await;
    h.chars("issues").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Tab).await;
    h.chars("https://mcp.example.com/v1").await;
    h.key(KeyCode::Enter).await;
    h.chars(CANARY).await;
    h.send(Msg::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE))).await;
    // Back on the server screen: point it elsewhere.
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Tab).await;
    for _ in 0.."https://mcp.example.com/v1".len() {
        h.key(KeyCode::Backspace).await;
    }
    h.chars("https://evil.example/v1").await;
    h.key(KeyCode::Enter).await;
    let w = wizard(&h);
    assert!(w.secrets.values().all(|b| b.is_empty()), "cleared: {:?}", w.inputs());
    assert!(w.error.as_deref().is_some_and(|e| e.contains("cleared")), "{:?}", w.error);
}

/// Quitting while a flow runs asks first (it would skip the run's undo).
#[tokio::test]
async fn quitting_during_a_run_asks_first() {
    let mut h = demo().await;
    let mut w = Wizard::new(LtFlow::Engine(FlowId::Tool), false);
    w.stage = Stage::Running;
    h.state.overlay = Some(Overlay::SetupFlow(Box::new(w)));
    assert!(h.state.quit_guard().is_some_and(|g| g.contains("Connect a tool")));
}

/// The device row opens lt's Add to my devices screens (the link request
/// first), not an engine form.
#[tokio::test]
async fn the_device_row_opens_the_link_screens() {
    let mut h = demo().await;
    open(&mut h, 7).await;
    assert!(
        matches!(&h.state.overlay, Some(Overlay::AddWorkspace(f)) if f.mode == ligus_term::app::state::FormMode::Link),
        "{:?}",
        h.state.overlay
    );
}

/// A sign-in check that finishes while a form is being filled in asks for
/// the form again (values kept); outside a form it does nothing.
#[tokio::test]
async fn a_finished_sign_in_check_refreshes_the_open_form() {
    let mut h = demo().await;
    let fx = update(&mut h.state, Msg::Event(ligus_term::backend::Event::SetupFactsChanged));
    assert!(fx.is_empty(), "{fx:?}");
    open(&mut h, 0).await;
    assert_eq!(wizard(&h).stage, Stage::Fill);
    let flow = wizard(&h).flow;
    let fx = update(&mut h.state, Msg::Event(ligus_term::backend::Event::SetupFactsChanged));
    assert!(matches!(fx.as_slice(), [Effect::SetupForm(_, f, _)] if *f == flow), "{fx:?}");
}
