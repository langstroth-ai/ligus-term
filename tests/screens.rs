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

fn snap(name: &str, title: &str, screen: String) {
    snap_at(name, title, W, H, screen);
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
    snap("onboarding", "First run: create or join", h.screen(W, H));
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
    snap("onboarding_mono", "First run in the mono theme: the mark as a silhouette", screen);
}

#[tokio::test]
async fn onboarding_join_with_invite() {
    let mut h = Harness::new(false).await;
    h.chars("Mara").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Right).await;
    h.key(KeyCode::Tab).await;
    h.send(Msg::Paste("fake-invite:Acme-Web".into())).await;
    snap("onboarding_join", "First run: join with an invite", h.screen(W, H));
    h.key(KeyCode::Enter).await;
    snap("onboarding_join_checked", "Join: who the invite is from, checked before joining", h.screen(W, H));
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
    snap("onboarding_connect", "First run: connect a deployment (ligus-local, Kubernetes)", h.screen(W, H));
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
    snap("workflows", "Workflows: definitions and runs", h.screen(W, H));
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    snap("workflow_run_gate", "A run waiting on its gate", h.screen(W, H));
    h.key(KeyCode::Char('A')).await;
    snap("workflow_run_done", "The gate approved: the run completes", h.screen(W, H));
    h.key(KeyCode::Char('g')).await;
    h.key(KeyCode::Char('S')).await;
    h.chars("export the order history").await;
    snap("workflow_start", "Starting a run", h.screen(W, H));
    h.key(KeyCode::Enter).await;
    snap("workflow_run_live", "A run started here, live", h.screen(W, H));
}

#[tokio::test]
async fn memory_and_task_state() {
    let mut h = demo().await;
    h.key(KeyCode::Char('y')).await;
    snap("memory", "Task state and memory", h.screen(W, H));
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    snap("memory_entry", "A memory entry", h.screen(W, H));
}

#[tokio::test]
async fn admit_a_box() {
    let mut h = demo().await;
    h.key(KeyCode::Char('m')).await;
    h.key(KeyCode::Char('B')).await;
    h.send(Msg::Paste("ligusjoin1:eyJ2IjoxLCJsYWJlbCI6InNmZiJ9".into())).await;
    snap("admit_box", "Admit a box: its join request", h.screen(W, H));
    h.key(KeyCode::Enter).await;
    // The fingerprint is on screen before any passkey prompt.
    let checked = h.screen(W, H);
    assert!(checked.contains("K3QF-7XMA-2DPL-90RW-ZT4E"), "{checked}");
    assert!(!checked.contains("Confirm with passkey"), "{checked}");
    snap("admit_box_fingerprint", "Admit a box: compare its fingerprint", checked);
    h.key(KeyCode::Char('y')).await;
    h.key(KeyCode::Enter).await; // the (simulated) browser confirms
    snap("admit_box_sealed", "Admit a box: the sealed admission", h.screen(W, H));
}

#[tokio::test]
async fn bind_a_checkout() {
    let mut h = demo().await;
    h.key(KeyCode::Char('h')).await;
    h.key(KeyCode::Char('b')).await;
    h.chars("/srv/northwind-api").await;
    snap("bind_checkout", "Binding this device's checkout for apply on host", h.screen(W, H));
    h.key(KeyCode::Enter).await;
    snap("hosts_bound", "Hosts with a bound checkout", h.screen(W, H));
}

#[tokio::test]
async fn onboarding_error_without_a_name() {
    let mut h = Harness::new(false).await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Tab).await;
    h.chars("Northwind").await;
    h.key(KeyCode::Enter).await;
    snap("onboarding_error", "First run: a validation error", h.screen(W, H));
}

#[tokio::test]
async fn passkey_create_then_recovery_key() {
    let mut h = Harness::new(false).await;
    h.chars("Mara").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Tab).await;
    h.chars("Acme Web").await;
    h.key(KeyCode::Enter).await;
    snap("passkey_create", "Creating a workspace asks for a passkey", h.screen(W, H));
    h.key(KeyCode::Enter).await; // open the (simulated) browser
    snap("recovery_key", "The recovery key, shown once", h.screen(W, H));
    h.key(KeyCode::Enter).await;
    snap("new_workspace", "A new workspace", h.screen(W, H));
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
    snap("start", "Start-up: the cursor in the composer", h.screen(W, H));
    h.chars("hi").await;
    assert_eq!(h.state.composer.text, "hi");
}

#[tokio::test]
async fn chat_view() {
    let h = demo().await;
    snap("chat", "Chats: list and transcript", h.screen(W, H));
}

#[tokio::test]
async fn chat_composer_with_mentions() {
    let mut h = demo().await;
    h.key(KeyCode::Char('i')).await;
    h.chars("Thanks! @co").await;
    snap("chat_mentions", "Composer with the @mention popup", h.screen(W, H));
}

#[tokio::test]
async fn chat_multiline_composer() {
    let mut h = demo().await;
    h.key(KeyCode::Char('i')).await;
    h.chars("First line").await;
    h.ctrl('j').await;
    h.chars("second line").await;
    snap("chat_multiline", "Multi-line composer", h.screen(W, H));
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
        call: ToolCall { name: "read_file".into(), preview: "src/checkout.rs".into(), ok: true },
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
    snap("chat_streaming", "An agent streaming its reply", h.screen(W, H));
}

#[tokio::test]
async fn chat_lockdown_bar() {
    let mut h = demo().await;
    h.key(KeyCode::Down).await;
    snap("chat_lockdown", "A locked-down chat", h.screen(W, H));
}

#[tokio::test]
async fn workspace_switcher() {
    let mut h = demo().await;
    h.key(KeyCode::Char('w')).await;
    snap("switcher", "Workspace switcher", h.screen(W, H));
}

#[tokio::test]
async fn add_workspace_overlay() {
    let mut h = demo().await;
    h.key(KeyCode::Char('+')).await;
    snap("add_workspace", "Create or join another workspace", h.screen(W, H));
}

#[tokio::test]
async fn proposals_and_diff() {
    let mut h = demo().await;
    h.key(KeyCode::Char('p')).await;
    h.key(KeyCode::Enter).await;
    snap("proposals", "Proposals with the diff viewer", h.screen(W, H));
    h.key(KeyCode::Char(']')).await;
    h.key(KeyCode::Char(']')).await;
    snap("proposals_hunk", "Diff viewer: jumped to a later hunk", h.screen(W, H));
}

#[tokio::test]
async fn apply_on_host_picker() {
    let mut h = demo().await;
    h.key(KeyCode::Char('p')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Char('O')).await;
    snap("apply_host", "Apply on ‹host›", h.screen(W, H));
}

#[tokio::test]
async fn agents_and_runtimes() {
    let mut h = demo().await;
    h.key(KeyCode::Char('a')).await;
    snap("agents", "Agents and runtimes", h.screen(W, H));
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
    snap("agents_add", "Agents: add one on this device", h.screen(W, H));
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
    snap("agents_empty", "Agents: nothing found on this device", screen.clone());
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
    snap("hosts", "Hosts", h.screen(W, H));
}

#[tokio::test]
async fn people_as_owner() {
    let mut h = demo().await;
    h.key(KeyCode::Char('m')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    snap("people", "People: roles and actions", h.screen(W, H));
}

#[tokio::test]
async fn people_refusals_as_viewer() {
    let mut h = demo().await;
    h.key(KeyCode::Char('w')).await;
    h.key(KeyCode::Char('2')).await;
    h.key(KeyCode::Char('m')).await;
    snap("people_refused", "People: actions the rank rules refuse, with the reason", h.screen(W, H));
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
    snap("role_picker", "Changing a role (service member: refused roles crossed out)", h.screen(W, H));
}

#[tokio::test]
async fn presence_confirm_and_recovery_file() {
    let mut h = demo().await;
    h.key(KeyCode::Char('m')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Char('X')).await;
    snap("confirm_remove", "Confirm a removal", h.screen(W, H));
    h.key(KeyCode::Char('y')).await;
    snap("presence", "Passkey sheet", h.screen(W, H));
    h.key(KeyCode::Char('f')).await;
    h.chars("~/ligus-words.txt").await;
    snap("presence_recovery", "Passkey sheet: the recovery-words file", h.screen(W, H));
}

#[tokio::test]
async fn notifications() {
    let mut h = demo().await;
    h.key(KeyCode::Char('n')).await;
    snap("notifications", "Notifications", h.screen(W, H));
}

#[tokio::test]
async fn settings() {
    let mut h = demo().await;
    h.key(KeyCode::Char('s')).await;
    snap("settings", "Settings", h.screen(W, H));
}

#[tokio::test]
async fn help_overlay() {
    let mut h = demo().await;
    h.key(KeyCode::Char('?')).await;
    snap_at("help", "Help: keybindings", W, 44, h.screen(W, 44));
}

#[tokio::test]
async fn narrow_terminal_still_renders() {
    let h = demo().await;
    let s = h.screen(60, 16);
    assert!(s.contains("Chats"));
    // No panic at tiny sizes either.
    let _ = h.screen(10, 4);
}

/// An empty chat shows the mark above its welcome when the pane has room.
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
    assert!(h.screen(W, H).contains('▀'), "{}", h.screen(W, H));
    let short = h.screen(W, 18);
    assert!(!short.contains('▀') && short.contains("Start a conversation"), "{short}");
    snap_at("new_workspace_short", "A new workspace in a short terminal: the welcome without the mark", W, 18, short);
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
        tools: vec![ToolCall { name: format!("read\u{202e}{bad}"), preview: bad.into(), ok: true }],
    };
    h.send(Msg::Event(Event::MessageAdded { workspace: ws.clone(), message })).await;
    let screen = h.screen(W, H);
    assert!(screen.contains("gnp.exe"), "{screen}");
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
    assert!(h.screen(80, 24).contains("for what has focus first"));
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
    snap("confirm_quit", "Quitting while this device drives a run", h.screen(W, H));
    h.key(KeyCode::Esc).await;
    h.key(KeyCode::Char('f')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Char('X')).await;
    snap("confirm_cancel_run", "Cancelling a run asks first", h.screen(W, H));
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
    snap("presence_no_browser", "Passkey sheet with no browser here: the address for another device", screen);
}
