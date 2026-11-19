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
use ligus_term::backend::model::*;

const W: u16 = 110;
const H: u16 = 32;

fn snap(name: &str, title: &str, screen: String) {
    if std::env::var_os("LT_WRITE_SCREENS").is_some() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/screens");
        std::fs::create_dir_all(&dir).unwrap();
        let md = format!("# {title}\n\nRendered by `tests/screens.rs` ({W}×{H}, the in-memory demo backend).\n\n```text\n{screen}```\n");
        std::fs::write(dir.join(format!("{name}.md")), md).unwrap();
    }
    insta::assert_snapshot!(name, screen);
}

#[tokio::test]
async fn onboarding_first_run() {
    let h = Harness::new(false).await;
    snap("onboarding", "First run: create or join", h.screen(W, H));
}

#[tokio::test]
async fn onboarding_join_with_invite() {
    let mut h = Harness::new(false).await;
    h.chars("Mara").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Right).await;
    h.key(KeyCode::Tab).await;
    h.send(Msg::Paste("fake-invite:Bee-Yard".into())).await;
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
    h.chars("export the wash counts").await;
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
    h.key(KeyCode::Enter).await; // the (simulated) browser confirms
    snap("admit_box_sealed", "Admit a box: the sealed admission", h.screen(W, H));
}

#[tokio::test]
async fn bind_a_checkout() {
    let mut h = demo().await;
    h.key(KeyCode::Char('h')).await;
    h.key(KeyCode::Char('b')).await;
    h.chars("/srv/apiary").await;
    snap("bind_checkout", "Binding this device's checkout for apply on host", h.screen(W, H));
    h.key(KeyCode::Enter).await;
    snap("hosts_bound", "Hosts with a bound checkout", h.screen(W, H));
}

#[tokio::test]
async fn onboarding_error_without_a_name() {
    let mut h = Harness::new(false).await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Tab).await;
    h.chars("Apiary").await;
    h.key(KeyCode::Enter).await;
    snap("onboarding_error", "First run: a validation error", h.screen(W, H));
}

#[tokio::test]
async fn passkey_create_then_recovery_key() {
    let mut h = Harness::new(false).await;
    h.chars("Mara").await;
    h.key(KeyCode::Tab).await;
    h.key(KeyCode::Tab).await;
    h.chars("Bee Yard").await;
    h.key(KeyCode::Enter).await;
    snap("passkey_create", "Creating a workspace asks for a passkey", h.screen(W, H));
    h.key(KeyCode::Enter).await; // open the (simulated) browser
    snap("recovery_key", "The recovery key, shown once", h.screen(W, H));
    h.key(KeyCode::Enter).await;
    snap("new_workspace", "A new workspace", h.screen(W, H));
}

async fn demo() -> Harness {
    Harness::new(true).await
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
    let ws = WorkspaceId::new("ws-apiary");
    let chat = ChatId::new("c-log");
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
        call: ToolCall { name: "read_file".into(), preview: "src/inspections.rs".into(), ok: true },
    }))
    .await;
    for d in ["Three ", "per ", "100 ", "matches ", "the ", "club's "] {
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
    snap("help", "Help: keybindings", h.screen(W, 44));
}

#[tokio::test]
async fn narrow_terminal_still_renders() {
    let h = demo().await;
    let s = h.screen(60, 16);
    assert!(s.contains("Chats"));
    // No panic at tiny sizes either.
    let _ = h.screen(10, 4);
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
