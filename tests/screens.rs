//! Snapshot tests: every screen rendered into a ratatui `TestBackend` and
//! compared as text (insta). `LT_WRITE_SCREENS=1 cargo test --test screens`
//! also writes each screen to `docs/screens/<name>.md`.
//!
//! Accept changed snapshots with `INSTA_UPDATE=always cargo test` (or
//! `cargo insta review`), and look at the diff before committing.

mod common;

use common::Harness;
use crossterm::event::KeyCode;
use ligus_term::app::Msg;
use ligus_term::backend::Event;
use ligus_term::backend::fake::FakeBackend;
use ligus_term::backend::model::*;

const W: u16 = 110;
const H: u16 = 32;

/// A screen at 110×32 (with its capture in `docs/screens/`), and the same
/// screen at the default terminal (80×24) and a large one (200×50). The main
/// views are checked at those sizes by `every_view_at_every_size`, which
/// reaches them from the demo's start.
fn snap(h: &Harness, name: &str, title: &str) {
    snap_at(name, title, W, H, h.screen(W, H));
    if VIEWS.contains(&name) {
        return;
    }
    for (w, ht) in [(80, 24), (200, 50)] {
        let screen = h.screen(w, ht);
        fits(name, w, ht, &screen);
        insta::assert_snapshot!(format!("{name}_{w}x{ht}"), screen);
    }
}

/// What every screen keeps at any size: something drawn, and a status bar
/// that never runs into the device label.
fn fits(name: &str, w: u16, ht: u16, screen: &str) {
    let lines: Vec<&str> = screen.lines().collect();
    assert_eq!(lines.len(), ht as usize, "{name} at {w}x{ht}");
    assert!(lines.iter().any(|l| !l.trim().is_empty()), "{name} at {w}x{ht}: nothing drawn");
    let status = lines[ht as usize - 1];
    assert!(!status.contains("helplt"), "{name} at {w}x{ht}: {status}");
}

/// A snapshot, and with `LT_WRITE_SCREENS` its capture in `docs/screens/`
/// (rendered at `w`×`h`).
fn snap_at(name: &str, title: &str, w: u16, h: u16, screen: String) {
    if std::env::var_os("LT_WRITE_SCREENS").is_some() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/screens");
        std::fs::create_dir_all(&dir).unwrap();
        let md = format!("# {title}\n\nRendered by `tests/screens.rs` ({w}×{h}, the in-memory demo backend).\n\n```text\n{screen}```\n");
        std::fs::write(dir.join(format!("{name}.md")), md).unwrap();
    }
    insta::assert_snapshot!(name, screen);
}

#[tokio::test]
async fn onboarding_first_run() {
    let h = Harness::new(false).await;
    snap(&h, "onboarding", "First run: create or join");
}

/// The Ligus mark beside the first-run form when the terminal is wide
/// enough (`onboarding` above), above it when only tall enough, and left
/// out when neither.
#[tokio::test]
async fn onboarding_splash_by_terminal_size() {
    const MARK: [char; 3] = ['▀', '▄', '█'];
    let h = Harness::new(false).await;
    let wide = h.screen(W, H);
    assert!(wide.contains(MARK), "{wide}");
    let narrow = h.screen(80, H);
    assert!(!narrow.contains(MARK), "{narrow}");
    snap_at("onboarding_narrow", "First run in a narrow terminal: no mark", 80, H, narrow);
    let tall = h.screen(80, 40);
    assert!(tall.contains(MARK), "{tall}");
    snap_at("onboarding_narrow_tall", "First run in a narrow, tall terminal: the mark above the form", 80, 40, tall);
    // Too small for the form and the mark: the form alone.
    assert!(!h.screen(60, 20).contains(MARK));
}

/// Mono (`NO_COLOR` or `--theme mono`): the same mark as a silhouette, with
/// no colour anywhere on screen.
#[tokio::test]
async fn onboarding_splash_mono() {
    use ligus_term::theme::Theme;
    let h = Harness::new(false).await;
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(W, H)).unwrap();
    term.draw(|f| ligus_term::ui::draw(f, &h.state, &Theme::MONO)).unwrap();
    let buf = term.backend().buffer();
    assert!(buf.content().iter().all(|c| c.fg == ratatui::style::Color::Reset && c.bg == ratatui::style::Color::Reset));
    let screen = common::buffer_text(buf);
    assert!(screen.contains('█'), "{screen}");
    // The colour themes put two pixels in one cell (`▀` on a background);
    // mono uses full and half blocks instead, so its text differs.
    snap_at("onboarding_mono", "First run in the mono theme: the mark as a silhouette", W, H, screen);
}

#[tokio::test]
async fn onboarding_join_with_invite() {
    let mut h = Harness::new(false).await;
    h.chars("Mara").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Right).await;
    h.key(KeyCode::Tab).await;
    h.send(Msg::Paste("fake-invite:Acme-Web".into())).await;
    snap(&h, "onboarding_join", "First run: join with an invite");
    h.key(KeyCode::Enter).await;
    snap(&h, "onboarding_join_checked", "Join: who the invite is from, checked before joining");
}

/// Add to my devices: this computer's request and fingerprint, then who
/// approved the desktop's code, then linked (one of Alex's devices).
#[tokio::test]
async fn onboarding_add_to_my_devices() {
    let mut h = Harness::new(false).await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Char('a')).await;
    h.key(KeyCode::Tab).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("7F3A-91C0-77DE-0B14-5E62") && screen.contains("Add a computer"), "{screen}");
    assert!(!screen.contains("Your name"), "the person's name comes from the desktop: {screen}");
    snap(&h, "onboarding_link_request", "First run: Add to my devices (the request for your desktop)");
    h.send(Msg::Paste("ligusgrant1:demo-grant-from-the-desktop".into())).await;
    h.key(KeyCode::Enter).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("Approved by Alex's desktop") && screen.contains("2C41-88E0-1B9D-04AF-6E73"), "{screen}");
    snap(&h, "onboarding_link_confirm", "Add to my devices: who approved it, confirmed before linking");
    h.key(KeyCode::Enter).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("one of Alex's devices"), "{screen}");
    snap(&h, "onboarding_link_done", "Add to my devices: this computer is now one of Alex's devices");
}

/// The palette's Link this computer to my devices, on an install in use.
#[tokio::test]
async fn palette_link_this_computer() {
    let mut h = demo().await;
    h.key(KeyCode::Char(':')).await;
    h.chars("link this").await;
    h.key(KeyCode::Enter).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("new device key") && screen.contains("7F3A-91C0-77DE-0B14-5E62"), "{screen}");
    snap(&h, "link_from_palette", "Palette: Link this computer to my devices (an install already in use)");
}

/// The first run's fourth choice: look around in the demo first.
#[tokio::test]
async fn onboarding_demo_choice() {
    let mut h = Harness::new(false).await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Left).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("(•) Look around first") && screen.contains("Nothing is saved"), "{screen}");
    snap(&h, "onboarding_demo", "First run: look around in the demo first");
}

#[tokio::test]
async fn onboarding_connect_a_deployment() {
    let mut h = Harness::new(false).await;
    h.chars("Mara").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Char('d')).await;
    h.key(KeyCode::Tab).await;
    h.chars("http://127.0.0.1:18443").await;
    h.key(KeyCode::Tab).await;
    h.chars("k8s-trial-7f3a").await;
    snap(&h, "onboarding_connect", "First run: connect a deployment (ligus-local, Kubernetes)");
    // The workspace key and the relay token never show on screen.
    h.key(KeyCode::Tab).await;
    h.chars("s3cret-workspace-key").await;
    h.key(KeyCode::Tab).await;
    h.chars("hrt1-token-xyz").await;
    let screen = h.screen(W, H);
    assert!(!screen.contains("s3cret") && !screen.contains("hrt1-token"), "{screen}");
    assert!(screen.contains("••••"), "{screen}");
}

#[tokio::test]
async fn workflows_and_runs() {
    let mut h = demo().await;
    h.key(KeyCode::Char('f')).await;
    snap(&h, "workflows", "Workflows: definitions and runs");
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    snap(&h, "workflow_run_gate", "A run waiting on its gate");
    h.key(KeyCode::Char('A')).await;
    snap(&h, "workflow_run_done", "The gate approved: the run completes");
    h.key(KeyCode::Char('g')).await;
    h.key(KeyCode::Char('S')).await;
    h.chars("export the order history").await;
    snap(&h, "workflow_start", "Starting a run");
    h.key(KeyCode::Enter).await;
    snap(&h, "workflow_run_live", "A run started here, live");
}

#[tokio::test]
async fn memory_and_task_state() {
    let mut h = demo().await;
    h.key(KeyCode::Char('y')).await;
    snap(&h, "memory", "Task state and memory");
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    snap(&h, "memory_entry", "A memory entry");
}

#[tokio::test]
async fn admit_a_box() {
    let mut h = demo().await;
    h.key(KeyCode::Char('m')).await;
    h.key(KeyCode::Char('B')).await;
    h.send(Msg::Paste("ligusjoin1:eyJ2IjoxLCJsYWJlbCI6InNmZiJ9".into())).await;
    snap(&h, "admit_box", "Admit a box: its join request");
    h.key(KeyCode::Enter).await;
    // The fingerprint is on screen before any passkey prompt.
    let checked = h.screen(W, H);
    assert!(checked.contains("K3QF-7XMA-2DPL-90RW-ZT4E"), "{checked}");
    assert!(!checked.contains("Confirm with passkey"), "{checked}");
    snap(&h, "admit_box_fingerprint", "Admit a box: compare its fingerprint");
    h.key(KeyCode::Char('y')).await;
    h.key(KeyCode::Enter).await; // the (simulated) browser confirms
    snap(&h, "admit_box_sealed", "Admit a box: the sealed admission");
}

#[tokio::test]
async fn bind_a_checkout() {
    let mut h = demo().await;
    h.key(KeyCode::Char('h')).await;
    h.key(KeyCode::Char('b')).await;
    h.chars("/srv/northwind-api").await;
    snap(&h, "bind_checkout", "Binding this device's checkout for apply on host");
    h.key(KeyCode::Enter).await;
    snap(&h, "hosts_bound", "Hosts with a bound checkout");
}

#[tokio::test]
async fn onboarding_error_without_a_name() {
    let mut h = Harness::new(false).await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Tab).await;
    h.chars("Northwind").await;
    h.key(KeyCode::Enter).await;
    snap(&h, "onboarding_error", "First run: a validation error");
}

#[tokio::test]
async fn passkey_create_then_recovery_key() {
    let mut h = Harness::new(false).await;
    h.chars("Mara").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Tab).await;
    h.chars("Acme Web").await;
    h.key(KeyCode::Enter).await;
    snap(&h, "passkey_create", "Creating a workspace asks for a passkey");
    h.key(KeyCode::Enter).await; // open the (simulated) browser
    snap(&h, "recovery_key", "The recovery key, shown once");
    h.key(KeyCode::Enter).await;
    snap(&h, "new_workspace", "A new workspace");
}

/// The demo, with the cursor moved from the composer (where it starts) to
/// the chat list, so the keys below drive views.
async fn demo() -> Harness {
    let mut h = Harness::new(true).await;
    assert_eq!(h.state.focus, ligus_term::app::state::Focus::Composer);
    h.key(KeyCode::Esc).await;
    h
}

/// lt starts with the cursor in the open chat's composer: typing works at
/// once, and esc leaves it for the keys.
#[tokio::test]
async fn starts_in_the_composer() {
    let mut h = Harness::new(true).await;
    snap(&h, "start", "Start-up: the cursor in the composer");
    h.chars("hi").await;
    assert_eq!(h.state.composer.text, "hi");
}

#[tokio::test]
async fn chat_view() {
    let h = demo().await;
    snap(&h, "chat", "Chats: list and transcript");
}

#[tokio::test]
async fn chat_composer_with_mentions() {
    let mut h = demo().await;
    h.key(KeyCode::Char('i')).await;
    h.chars("Thanks! @co").await;
    snap(&h, "chat_mentions", "Composer with the @mention popup");
}

#[tokio::test]
async fn chat_multiline_composer() {
    let mut h = demo().await;
    h.key(KeyCode::Char('i')).await;
    h.chars("First line").await;
    h.ctrl('j').await;
    h.chars("second line").await;
    snap(&h, "chat_multiline", "Multi-line composer");
}

#[tokio::test]
async fn chat_streaming_reply() {
    let mut h = demo().await;
    let ws = WorkspaceId::new("ws-northwind");
    let chat = ChatId::new("c-checkout");
    h.send(Msg::Event(Event::Thinking { workspace: ws.clone(), chat: chat.clone(), agent: "@reviewer".into(), on: true })).await;
    let message = Message {
        id: MessageId::new("s1"),
        chat: chat.clone(),
        author: Author::Agent { id: AgentId::new("a-coder"), handle: "coder".into(), runtime: "claude-code".into() },
        text: String::new(),
        at: ligus_term::backend::fake::DEMO_NOW,
        streaming: true,
        tools: vec![],
    };
    h.send(Msg::Event(Event::MessageAdded { workspace: ws.clone(), message })).await;
    h.send(Msg::Event(Event::ToolActivity {
        workspace: ws.clone(),
        chat: chat.clone(),
        message: MessageId::new("s1"),
        call: ToolCall { name: "read_file".into(), preview: "src/checkout.rs".into(), ok: true, ..ToolCall::default() },
    }))
    .await;
    for d in ["Thirty ", "seconds ", "matches ", "the ", "provider's "] {
        h.send(Msg::Event(Event::MessageDelta {
            workspace: ws.clone(),
            chat: chat.clone(),
            message: MessageId::new("s1"),
            delta: d.into(),
        }))
        .await;
    }
    snap(&h, "chat_streaming", "An agent streaming its reply");
}

#[tokio::test]
async fn chat_lockdown_bar() {
    let mut h = demo().await;
    h.key(KeyCode::Down).await;
    snap(&h, "chat_lockdown", "A locked-down chat");
}

#[tokio::test]
async fn workspace_switcher() {
    let mut h = demo().await;
    h.key(KeyCode::Char('w')).await;
    snap(&h, "switcher", "Workspace switcher");
}

#[tokio::test]
async fn add_workspace_overlay() {
    let mut h = demo().await;
    h.key(KeyCode::Char('+')).await;
    snap(&h, "add_workspace", "Create or join another workspace");
}

#[tokio::test]
async fn proposals_and_diff() {
    let mut h = demo().await;
    h.key(KeyCode::Char('p')).await;
    h.key(KeyCode::Enter).await;
    snap(&h, "proposals", "Proposals with the diff viewer");
    h.key(KeyCode::Char(']')).await;
    h.key(KeyCode::Char(']')).await;
    snap(&h, "proposals_hunk", "Diff viewer: jumped to a later hunk");
}

#[tokio::test]
async fn apply_on_host_picker() {
    let mut h = demo().await;
    h.key(KeyCode::Char('p')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Char('O')).await;
    snap(&h, "apply_host", "Apply on ‹host›");
}

#[tokio::test]
async fn agents_and_runtimes() {
    let mut h = demo().await;
    h.key(KeyCode::Char('a')).await;
    snap(&h, "agents", "Agents and runtimes");
}

/// Adding an agent from what this device has: the selected runtime under
/// "Available on this device", its name prompt, and the agent in the list.
#[tokio::test]
async fn agents_add_from_this_device() {
    let mut h = demo().await;
    h.key(KeyCode::Char('a')).await;
    for _ in 0..4 {
        h.key(KeyCode::Down).await;
    }
    h.key(KeyCode::Enter).await;
    snap(&h, "agents_add", "Agents: add one on this device");
    h.key(KeyCode::Enter).await;
    let screen = h.screen(W, H);
    assert!(screen.lines().any(|l| l.contains("@copilot") && l.contains("Only me") && l.contains("This device")), "{screen}");
    assert!(screen.contains("Added @copilot"), "{screen}");
    let left: Vec<String> = h.state.available_local().iter().map(|l| l.id.to_string()).collect();
    assert_eq!(left, vec!["local:ollama/qwen3:8b"]);
}

/// Nothing found on this device: what to install or set.
#[tokio::test]
async fn agents_nothing_on_this_device() {
    let fake = FakeBackend::demo(Harness::cfg()).with_local(vec![]);
    let mut h = Harness::with(fake).await;
    h.key(KeyCode::Esc).await;
    h.key(KeyCode::Char('a')).await;
    let screen = h.screen(W, H);
    snap(&h, "agents_empty", "Agents: nothing found on this device");
    assert!(screen.contains("No agent runtime found on this device"), "{screen}");
}

/// Agent access controls: the agent you own shows this device's usage
/// (per member, today and this month); `E` toggles who may use it.
#[tokio::test]
async fn agents_usage_and_who_can_use() {
    let mut h = demo().await;
    h.key(KeyCode::Char('a')).await;
    h.key(KeyCode::Down).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("Mara: 3 turns, ~$0.0120"), "{screen}");
    assert!(screen.contains("Ben: 2 turns (no usage reported)"), "{screen}");
    assert!(screen.contains("count turns only"), "{screen}");
    h.key(KeyCode::Char('E')).await;
    let screen = h.screen(W, H);
    assert!(screen.lines().any(|l| l.contains("@coder") && l.contains("Everyone")), "{screen}");
    // Someone else's (here: shared, not yours to manage) is refused.
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Char('E')).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("Only an Admin changes who can use @reviewer") || screen.contains("Only its owner"), "{screen}");
}

#[tokio::test]
async fn hosts() {
    let mut h = demo().await;
    h.key(KeyCode::Char('h')).await;
    snap(&h, "hosts", "Hosts");
}

#[tokio::test]
async fn people_as_owner() {
    let mut h = demo().await;
    h.key(KeyCode::Char('m')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    snap(&h, "people", "People: roles and actions");
}

#[tokio::test]
async fn people_refusals_as_viewer() {
    let mut h = demo().await;
    h.key(KeyCode::Char('w')).await;
    h.key(KeyCode::Char('2')).await;
    h.key(KeyCode::Char('m')).await;
    snap(&h, "people_refused", "People: actions the rank rules refuse, with the reason");
}

#[tokio::test]
async fn role_picker() {
    let mut h = demo().await;
    h.key(KeyCode::Char('m')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Char('r')).await;
    snap(&h, "role_picker", "Changing a role (service member: refused roles crossed out)");
}

#[tokio::test]
async fn presence_confirm_and_recovery_file() {
    let mut h = demo().await;
    h.key(KeyCode::Char('m')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Char('X')).await;
    snap(&h, "confirm_remove", "Confirm a removal");
    h.key(KeyCode::Char('y')).await;
    snap(&h, "presence", "Passkey sheet");
    h.key(KeyCode::Char('f')).await;
    h.chars("~/ligus-words.txt").await;
    snap(&h, "presence_recovery", "Passkey sheet: the recovery-words file");
}

#[tokio::test]
async fn notifications() {
    let mut h = demo().await;
    h.key(KeyCode::Char('n')).await;
    snap(&h, "notifications", "Notifications");
}

#[tokio::test]
async fn settings() {
    let mut h = demo().await;
    h.key(KeyCode::Char('s')).await;
    snap(&h, "settings", "Settings");
}

#[tokio::test]
async fn help_overlay() {
    let mut h = demo().await;
    h.key(KeyCode::Char('?')).await;
    snap_at("help", "Help: keybindings", W, 44, h.screen(W, 44));
}

/// `/` in the help: only the bindings that match, under the search line.
#[tokio::test]
async fn help_overlay_search() {
    let mut h = demo().await;
    h.key(KeyCode::Char('3')).await;
    h.key(KeyCode::Char('?')).await;
    h.key(KeyCode::Char('/')).await;
    h.chars("reject").await;
    let screen = h.screen(W, H);
    assert!(screen.contains("/ reject") && screen.contains("Reject the run's waiting gate"), "{screen}");
    assert!(!screen.contains("Next hunk"), "{screen}");
    snap(&h, "help_search", "Help: / narrows the keys to what matches");
}

/// The status bar's right zone reads the relay's state in words, and a
/// toast sits in its centre: one at a time, never over a pane's border.
#[tokio::test]
async fn status_bar_sync_and_toasts() {
    let mut h = demo().await;
    let w = h.state.active.clone().unwrap();
    h.state.workspaces[0].sync = SyncMode::Relay("https://relay.example.com".into());
    let status = |h: &Harness| h.screen(W, H).lines().last().unwrap_or("").to_string();
    assert!(status(&h).contains("⇅ relay"), "{}", status(&h));
    h.send(Msg::Event(Event::Sync { workspace: w.clone(), status: "synced".into() })).await;
    assert!(status(&h).contains("⇅ relay ok"), "{}", status(&h));
    h.send(Msg::Event(Event::Sync { workspace: w.clone(), status: "sync off: no relay token".into() })).await;
    assert!(status(&h).contains("⚠ sync off"), "{}", status(&h));
    h.send(Msg::Event(Event::Sync { workspace: w.clone(), status: "sync failed: connection refused".into() })).await;
    assert!(status(&h).contains("⚠ offline"), "{}", status(&h));
    h.state.toast(ligus_term::app::state::ToastKind::Ok, "Runtime changed");
    h.state.toast(ligus_term::app::state::ToastKind::Error, "turn failed: no runtime for @coder");
    let screen = h.screen(W, H);
    let last = screen.lines().last().unwrap();
    assert!(last.contains("✗ turn failed") && !screen.contains("Runtime changed"), "{screen}");
    assert!(screen.lines().take(H as usize - 1).all(|l| !l.contains("turn failed")), "the toast stays in the status bar: {screen}");
    snap(&h, "status_offline_toast", "Status bar: offline, with an error toast in its centre");
}

/// The command palette: everything for the view first, with its key.
#[tokio::test]
async fn command_palette() {
    let mut h = demo().await;
    h.key(KeyCode::Char('3')).await;
    h.key(KeyCode::Char(':')).await;
    snap(&h, "palette", "Command palette: the view's actions first, each with its key");
    h.chars("appr").await;
    let screen = h.screen(W, H);
    assert!(screen.contains("› appr") && screen.contains("Approve"), "{screen}");
    snap(&h, "palette_query", "Command palette: typing filters");
}

/// `space`, then a pause: the sheet of single keys for the view, the
/// views, and what works anywhere. Before the pause, nothing shows.
#[tokio::test]
async fn which_key_sheet() {
    use ligus_term::backend::fake::DEMO_NOW;
    let mut h = demo().await;
    h.key(KeyCode::Char('3')).await;
    h.key(KeyCode::Char(' ')).await;
    assert!(!h.screen(W, H).contains("press a key"), "no sheet before the pause");
    h.send(Msg::Tick(DEMO_NOW + 500)).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("press a key") && screen.contains("A approve") && screen.contains("p proposals"), "{screen}");
    snap(&h, "which_key", "space, then a pause: the keys you can press here");
}

/// `ctrl-r` in the composer: what you sent, narrowed as you type.
#[tokio::test]
async fn composer_history_search() {
    let mut h = demo().await;
    h.key(KeyCode::Char('i')).await;
    for m in ["@coder can you rerun the flaky test?", "Thanks, that fixed it"] {
        h.chars(m).await;
        h.key(KeyCode::Enter).await;
    }
    h.ctrl('r').await;
    snap(&h, "history_search", "ctrl-r in the composer: what you sent");
}

/// The transcript: Markdown (bold, inline code, lists, a fenced block with
/// its language), tool calls with their time, collapsed and then expanded
/// with `o`, a reply still streaming with its spinner and elapsed time.
#[tokio::test]
async fn transcript_markdown_and_tools() {
    use ligus_term::backend::fake::DEMO_NOW;
    let mut h = demo().await;
    let chat = h.state.open_chat.clone().unwrap();
    let agent = Author::Agent { id: AgentId::new("a-coder"), handle: "coder".into(), runtime: "claude-code".into() };
    h.state.transcript.push(Message {
        id: MessageId::new("md-1"),
        chat: chat.clone(),
        author: agent.clone(),
        text: "## The fix\nThe test reads the **wall clock**, so `Checkout::is_expired` flips under load. Two changes:\n- inject a `Clock`\n- set the time in the test\n```rust\npub const CHECKOUT_TIMEOUT: Duration = Duration::from_secs(30);\n```\nThat's all.".into(),
        at: DEMO_NOW - 60_000,
        streaming: false,
        tools: vec![
            ToolCall {
                name: "run_command".into(),
                preview: "cargo test -p checkout".into(),
                ok: true,
                duration_ms: Some(9_400),
                output: "test checkout::expires ... ok\ntest result: ok. 18 passed; 0 failed".into(),
            },
            ToolCall { name: "edit_file".into(), preview: "src/checkout.rs".into(), ok: false, duration_ms: Some(120), output: String::new() },
        ],
    });
    h.state.transcript.push(Message {
        id: MessageId::new("md-2"),
        chat,
        author: agent,
        text: "Running the suite again".into(),
        at: DEMO_NOW - 12_000,
        streaming: true,
        tools: vec![],
    });
    let screen = h.screen(W, 44);
    assert!(screen.contains("▸ run_command") && screen.contains("done 9s") && screen.contains("error 0.1s"), "{screen}");
    assert!(screen.contains("┌ rust") && screen.contains("• inject a Clock") && !screen.contains("**"), "{screen}");
    assert!(screen.contains("00:12"), "elapsed time on the streaming reply: {screen}");
    assert!(!screen.contains("18 passed"), "collapsed by default: {screen}");
    h.key(KeyCode::Char('o')).await;
    let open = h.screen(W, 44);
    assert!(open.contains("▾ run_command") && open.contains("18 passed"), "{open}");
    snap_at("transcript_markdown", "The transcript: Markdown, tool calls with times, expanded with o", W, 44, open);
    for (w, ht) in [(80, 24), (200, 50)] {
        insta::assert_snapshot!(format!("transcript_markdown_{w}x{ht}"), h.screen(w, ht));
    }
}

/// `--ascii`: borders, glyphs and state chips in plain ASCII, the mark
/// left out, and the words unchanged.
#[tokio::test]
async fn ascii_mode() {
    use ligus_term::theme::Theme;
    let mut h = demo().await;
    h.state.toast(ligus_term::app::state::ToastKind::Ok, "Runtime changed");
    let ascii = Theme { ascii: true, ..Theme::DARK };
    for (w, ht) in [(80, 24), (110, 32), (200, 50)] {
        let screen = common::render(&h.state, w, ht, &ascii);
        let symbols: Vec<char> = screen.chars().filter(|c| !c.is_ascii()).collect();
        assert!(symbols.is_empty(), "non-ASCII at {w}x{ht}: {symbols:?}\n{screen}");
        assert!(screen.contains("+-") && screen.contains("<> Fix flaky"), "{screen}");
        assert!(screen.contains("ok Runtime changed"), "{screen}");
        let name = if (w, ht) == (110, 32) { "ascii".to_string() } else { format!("ascii_{w}x{ht}") };
        insta::assert_snapshot!(name, screen);
    }
}

/// `--reduce-motion`: the working chip stands still from tick to tick.
#[tokio::test]
async fn reduce_motion_stills_the_spinner() {
    use ligus_term::backend::fake::DEMO_NOW;
    let mut h = demo().await;
    let w = h.state.active.clone().unwrap();
    let chat = h.state.chats[2].id.clone();
    h.send(Msg::Event(Event::Thinking { workspace: w, chat, agent: "@coder".into(), on: true })).await;
    let at = |h: &mut Harness, now| {
        h.state.now = now;
        h.screen(W, H).lines().find(|l| l.contains("Weekly")).unwrap_or("").to_string()
    };
    assert_ne!(at(&mut h, DEMO_NOW), at(&mut h, DEMO_NOW + 250), "it spins by default");
    h.state.reduce_motion = true;
    let still = at(&mut h, DEMO_NOW);
    assert_eq!(still, at(&mut h, DEMO_NOW + 250));
    assert!(still.contains("⠹ Weekly"), "{still}");
}

/// `/` in a list: the selection jumps to the match, and the status bar
/// says what is typed and how many rows match.
#[tokio::test]
async fn find_in_proposals() {
    let mut h = demo().await;
    h.key(KeyCode::Char('p')).await;
    h.key(KeyCode::Char('/')).await;
    h.chars("csv").await;
    let screen = h.screen(W, H);
    let status = screen.lines().last().unwrap();
    assert!(status.contains("/csv") && status.contains("1 match"), "{status}");
    assert_eq!(h.state.selected_proposal().map(|p| p.title.as_str()), Some("Export orders to CSV"));
    snap(&h, "find", "/ in a list: the selection jumps to the match");
}

/// Focus mode: no top bar, no key hints; one quiet badge on the top
/// border while something needs you.
#[tokio::test]
async fn focus_mode() {
    let mut h = demo().await;
    h.ctrl('f').await;
    let screen = h.screen(W, H);
    let first = screen.lines().next().unwrap();
    assert!(!first.contains("Chats Workflows") && !first.contains("│ Chats"), "no top bar: {first}");
    assert!(first.contains("◆"), "one badge when something needs you: {first}");
    assert!(!screen.lines().last().unwrap().contains("? help"), "no hint line");
    snap(&h, "focus_mode", "Focus mode: just the work, and one quiet badge");
}

/// The chat list's chips: one glyph per state, and nothing for an idle
/// chat.
#[tokio::test]
async fn chat_list_states() {
    let mut h = demo().await;
    let w = h.state.active.clone().unwrap();
    let ids: Vec<ChatId> = h.state.chats.iter().map(|c| c.id.clone()).collect();
    h.send(Msg::Event(Event::Thinking { workspace: w.clone(), chat: ids[2].clone(), agent: "@coder".into(), on: true })).await;
    h.state.proposals.clear();
    h.state.tasks.clear();
    h.state.turn_outcomes.insert(ids[0].clone(), SessionState::Failed);
    let screen = h.screen(W, H);
    assert!(screen.contains("✕ Fix flaky"), "{screen}");
    assert!(screen.contains("‖ Migrate auth"), "the locked-down chat is paused: {screen}");
    assert!(screen.contains("Weekly release"), "{screen}");
    snap(&h, "chat_states", "Chats: failed, paused and working chips");
}

/// `!` opens this session's notices and errors in full, newest first.
#[tokio::test]
async fn messages_log() {
    let mut h = demo().await;
    h.key(KeyCode::Char('!')).await;
    snap(&h, "messages_empty", "Messages: nothing yet");
    h.key(KeyCode::Esc).await;
    h.state.toast(ligus_term::app::state::ToastKind::Ok, "Runtime changed");
    h.state.toast(
        ligus_term::app::state::ToastKind::Error,
        "apply: the checkout at ~/src/northwind has uncommitted changes in src/checkout.rs; commit or stash them and ask again",
    );
    h.state.toast(ligus_term::app::state::ToastKind::Info, "Cancelled");
    h.key(KeyCode::Char('!')).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("commit or stash them and ask again"), "the error is there in full: {screen}");
    assert!(screen.find("Cancelled").unwrap() < screen.find("Runtime changed").unwrap(), "newest first: {screen}");
    snap(&h, "messages", "Messages: this session's notices and errors, in full");
}

#[tokio::test]
async fn narrow_terminal_still_renders() {
    let mut h = demo().await;
    let s = h.screen(60, 20);
    assert!(s.contains("Chats"), "{s}");
    // Below 60×20 one line says so, instead of a broken layout.
    let small = h.screen(52, 18);
    assert!(small.contains("Terminal too small") && small.contains("52×18"), "{small}");
    snap_at("too_small", "A terminal below 60×20", 52, 18, small);
    // No panic at tiny sizes either.
    let _ = h.screen(10, 4);
    // A passkey prompt waits on you, so it shows at any size.
    h.key(KeyCode::Char('m')).await;
    h.key(KeyCode::Char('r')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Enter).await;
    if !h.state.presence.is_empty() {
        assert!(h.screen(52, 18).contains("passkey"), "{}", h.screen(52, 18));
    }
}

/// Under 100 columns the chat list is an overlay: hidden while you write,
/// shown with esc or ctrl-l; from 160 a third column shows what the chat
/// has going on; the transcript never runs wider than 100 columns.
#[tokio::test]
async fn chat_layout_by_width() {
    let mut h = Harness::new(true).await;
    let narrow = h.screen(80, 24);
    assert!(!narrow.contains("┏ Chats") && !narrow.contains("╭ Chats"), "the list is out of the way while writing: {narrow}");
    assert!(narrow.contains("1/3"), "the title says where the chat sits: {narrow}");
    h.key(KeyCode::Esc).await;
    assert!(h.screen(80, 24).contains("Chats"), "esc shows the list");
    h.key(KeyCode::Enter).await;
    assert_eq!(h.state.focus, ligus_term::app::state::Focus::Composer);
    h.ctrl('l').await;
    assert_eq!(h.state.focus, ligus_term::app::state::Focus::List, "ctrl-l shows it from the composer");
    h.ctrl('l').await;
    assert_eq!(h.state.focus, ligus_term::app::state::Focus::Composer, "and puts it away");
    let wide = h.screen(200, 50);
    assert!(wide.contains("This chat"), "{wide}");
    let longest = wide.lines().filter(|l| l.contains("The checkout test")).map(|l| l.trim().len()).max().unwrap_or(0);
    assert!(longest > 0, "{wide}");
    snap_at("chat_wide_rail", "Chats at 200×50: list, transcript at reading width, and this chat's work", 200, 50, wide);
}

/// A new workspace's empty chat shows its getting-started list (from real
/// state: no agents, nobody invited, no boxes, no runs), with the mark
/// above it when the pane has room.
#[tokio::test]
async fn empty_chat_splash() {
    let mut h = Harness::new(false).await;
    h.chars("Mara").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Tab).await;
    h.chars("Acme Web").await;
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    h.key(KeyCode::Enter).await;
    let screen = h.screen(W, H);
    assert!(screen.contains('▀'), "{screen}");
    assert!(screen.contains("✓ Workspace created") && screen.contains("○ Invite someone") && screen.contains("m then I"), "{screen}");
    assert!(screen.contains("needs a relay"), "a workspace on this device only can't invite yet: {screen}");
    snap(&h, "new_workspace_checklist", "A new workspace: the getting-started list");
    let short = h.screen(W, 20);
    assert!(!short.contains('▀') && short.contains("getting started"), "{short}");
    snap_at("new_workspace_short", "A new workspace in a short terminal: the getting-started list without the mark", W, 20, short);
}

#[tokio::test]
async fn light_and_mono_themes_change_styles_not_text() {
    use ligus_term::theme::Theme;
    let h = demo().await;
    let dark = common::render(&h.state, W, H, &Theme::DARK);
    let light = common::render(&h.state, W, H, &Theme::LIGHT);
    let mono = common::render(&h.state, W, H, &Theme::MONO);
    assert_eq!(dark, light);
    assert_eq!(dark, mono);
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(W, H)).unwrap();
    term.draw(|f| ligus_term::ui::draw(f, &h.state, &Theme::LIGHT)).unwrap();
    let fg = term.backend().buffer().content().iter().find(|c| c.symbol() == "A").map(|c| c.fg);
    assert_eq!(fg, Some(Theme::LIGHT.accent), "the active workspace name uses the light accent");
}

/// Remote text reaches the terminal as text: no escape sequences (ratatui
/// drops control characters) and no bidi controls or other invisible
/// format characters that would reorder or hide what is shown.
#[tokio::test]
async fn remote_text_cannot_reorder_or_hide_what_is_shown() {
    let mut h = demo().await;
    let ws = WorkspaceId::new("ws-northwind");
    let chat = ChatId::new("c-checkout");
    let bad = "ok \u{1b}]52;c;eA==\u{7}\u{1b}[2J \u{202e}gnp.exe\u{202c} \u{2067}x\u{2069} a\u{200b}b \u{feff}\u{9b}";
    let message = Message {
        id: MessageId::new("hostile"),
        chat: chat.clone(),
        author: Author::Agent { id: AgentId::new("a-coder"), handle: format!("coder\u{202e}{bad}"), runtime: "claude-code".into() },
        text: bad.into(),
        at: ligus_term::backend::fake::DEMO_NOW,
        streaming: false,
        tools: vec![ToolCall {
            name: format!("read\u{202e}{bad}"),
            preview: bad.into(),
            ok: true,
            output: bad.into(),
            ..ToolCall::default()
        }],
    };
    h.send(Msg::Event(Event::MessageAdded { workspace: ws.clone(), message })).await;
    // The tool's output too, shown with o.
    h.state.tools_expanded = true;
    let screen = h.screen(W, H);
    assert!(screen.contains("gnp.exe"), "{screen}");
    assert!(screen.matches("gnp.exe").count() >= 3, "the text, the preview and the output: {screen}");
    for c in screen.chars() {
        let format_or_control = c.is_control() && c != '\n'
            || matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}');
        assert!(!format_or_control, "{c:?} reached the terminal: {screen:?}");
    }
}

/// Every overlay renders at any terminal size, however small (no
/// arithmetic underflow on a narrow pane).
#[tokio::test]
async fn overlays_render_in_a_tiny_terminal() {
    use ligus_term::app::state::Overlay;
    let mut h = demo().await;
    h.state.overlay =
        Some(Overlay::AdmitBox { request: ligus_term::app::input::TextBuf::single(), busy: false, checked: None, result: None });
    for (w, ht) in [(1, 1), (3, 3), (5, 4), (8, 6)] {
        let _ = h.screen(w, ht);
    }
}

/// The terminal sizes every main view is checked at besides 110×32: the
/// default terminal (80×24), a small one (60×20) and a large one (200×50).
const SIZES: [(u16, u16); 3] = [(80, 24), (60, 20), (200, 50)];

/// Each main view, as reached by keys from the demo's start.
async fn view_at(name: &str) -> Harness {
    let mut h = demo().await;
    let keys: &[KeyCode] = match name {
        "chat" => &[],
        "composer" => &[KeyCode::Char('i'), KeyCode::Char('@'), KeyCode::Char('c'), KeyCode::Char('o')],
        "workflows" => &[KeyCode::Char('f')],
        "workflow_run" => &[KeyCode::Char('f'), KeyCode::Down, KeyCode::Down],
        "proposals" => &[KeyCode::Char('p')],
        "diff" => &[KeyCode::Char('p'), KeyCode::Enter],
        "agents" => &[KeyCode::Char('a')],
        "hosts" => &[KeyCode::Char('h')],
        "memory" => &[KeyCode::Char('y')],
        "people" => &[KeyCode::Char('m')],
        "notifications" => &[KeyCode::Char('n')],
        "settings" => &[KeyCode::Char('s')],
        "help" => &[KeyCode::Char('?')],
        _ => unreachable!("{name}"),
    };
    for k in keys {
        h.key(*k).await;
    }
    // Extra quiet windows, for a loaded machine running every test at once.
    for _ in 0..3 {
        h.settle().await;
    }
    h
}

const VIEWS: [&str; 13] = [
    "chat",
    "composer",
    "workflows",
    "workflow_run",
    "proposals",
    "diff",
    "agents",
    "hosts",
    "memory",
    "people",
    "notifications",
    "settings",
    "help",
];

/// Every main view at 80×24, 60×20 and 200×50: the tabs never lose a view
/// (People, Notifications with its badge and Settings were cut off at 80
/// columns), the status bar never runs into the device label, and nothing
/// panics.
#[tokio::test]
async fn every_view_at_every_size() {
    for view in VIEWS {
        let mut h = view_at(view).await;
        for (w, ht) in SIZES {
            h.send(Msg::Resize(w, ht)).await;
            let screen = h.screen(w, ht);
            let top = screen.lines().next().unwrap_or("");
            // Every view stays reachable from the top bar: by name, or by
            // its digit when the bar is down to digits.
            let all_named = ["Chats", "Props", "Agents", "Hosts", "People", "Notifs", "Setup"].iter().all(|v| top.contains(v))
                || ["Chats", "Proposals", "Agents", "Hosts", "People", "Notifications", "Settings"].iter().all(|v| top.contains(v));
            let digits = (1..=9).all(|d| top.contains(&d.to_string()));
            assert!(all_named || digits || top.contains('‹'), "{view} at {w}x{ht}: views cut off: {top}");
            // The unread badge survives.
            let n = h.state.unread_notifications();
            assert!(
                n > 0 && (top.contains(&format!(" {n}")) || top.contains(&format!("({n})"))),
                "{view} at {w}x{ht}: no unread badge: {top}"
            );
            // The status bar keeps the help key, two columns from the right zone.
            let status = screen.lines().nth(ht as usize - 1).unwrap_or("");
            assert!(status.contains("help") || status.contains("close") || view == "composer", "{view} at {w}x{ht}: {status}");
            assert!(!status.contains("helplt"), "{view} at {w}x{ht}: {status}");
            let name = format!("{view}_{w}x{ht}");
            let title = format!("{view} at {w}×{ht}");
            if (w, ht) == (80, 24) {
                snap_at(&name, &title, w, ht, screen);
            } else {
                insta::assert_snapshot!(name, screen);
            }
        }
    }
}

/// Help scrolls to its end and back, with no dead zone past the end.
#[tokio::test]
async fn help_scrolls_to_the_end() {
    let mut h = view_at("help").await;
    h.send(Msg::Resize(80, 24)).await;
    h.key(KeyCode::Char('G')).await;
    let end = h.screen(80, 24);
    assert!(end.contains("Cancel"), "the passkey keys close the list: {end}");
    h.key(KeyCode::Up).await;
    let up = h.screen(80, 24);
    assert_ne!(end, up, "one up from the end moves at once");
    h.key(KeyCode::Char('g')).await;
    assert!(h.screen(80, 24).contains("What has focus comes first"));
    // The view's own keys come first: Chats, then Lists.
    let top = h.screen(80, 24);
    let chats = top.find("Write a message").unwrap();
    assert!(top.find("Quit").is_none_or(|q| q > chats), "{top}");
}

/// Quitting while this device drives a run asks first; so does cancelling a
/// run.
#[tokio::test]
async fn confirm_quit_and_cancel_run() {
    let mut h = demo().await;
    h.key(KeyCode::Char('q')).await;
    assert!(!h.state.quit);
    snap(&h, "confirm_quit", "Quitting while this device drives a run");
    h.key(KeyCode::Esc).await;
    h.key(KeyCode::Char('f')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Char('X')).await;
    snap(&h, "confirm_cancel_run", "Cancelling a run asks first");
    h.key(KeyCode::Char('y')).await;
    assert!(h.screen(W, H).contains("canceled"), "{}", h.screen(W, H));
}

/// No browser on this device (LT_NO_BROWSER): the prompt shows the page's
/// address below the sheet, full width, for a browser on another device.
#[tokio::test]
async fn presence_without_a_browser_here() {
    use ligus_term::app::state::PresenceStage;
    let mut h = demo().await;
    h.key(KeyCode::Char('m')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Char('X')).await;
    h.key(KeyCode::Char('y')).await;
    let p = h.state.presence.front_mut().expect("a passkey prompt");
    p.stage = PresenceStage::WaitingBrowser;
    let url = "https://ligus.ai/presence#op=get&rp=ligus.ai&c=Y2hhbGxlbmdl&cred=&k=memberRemoved&s=Remove%20Lin&p=8765&n=bm9uY2Utbm9uY2U";
    p.remote = Some((url.into(), 8765));
    let screen = h.screen(W, H);
    assert!(screen.lines().any(|l| l.starts_with("https://ligus.ai/presence#op=get")), "{screen}");
    assert!(screen.contains("8765:127.0.0.1:8765"), "{screen}");
    snap(&h, "presence_no_browser", "Passkey sheet with no browser here: the address for another device");
}
