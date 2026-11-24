//! Dev-only: render lt's screens (the demo backend, through ratatui's
//! `TestBackend`) as SVG terminal images for the README, into
//! `assets/screens/`. Not part of the `lt` binary.
//!
//! ```sh
//! cargo run --example screenshots
//! cargo run --example screenshots -- --gallery DIR   # every view, its menu and popups
//! cargo run --example screenshots -- --setup DIR     # the guided setup flows
//! ```
//!
//! Each buffer becomes a monospace grid in the dark theme's colours on the
//! brand's dark tile colour, in a window frame. Box-drawing and block
//! characters are drawn as shapes, so frames join up whatever font the
//! viewer has; the rest is text placed glyph by glyph on the grid.

#[path = "../tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use common::Harness;
use crossterm::event::KeyCode;
use ligus_term::app::Msg;
use ligus_term::backend::Event;
use ligus_term::backend::model::*;
use ligus_term::theme::Theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

/// ligus-brand-tokens.json: tileBottomDark (the terminal), tileTopDark
/// (the title bar), soft (the title).
const TERM_BG: &str = "#1a1206";
const BAR_BG: &str = "#3a2812";
const SOFT: &str = "#8a8073";

const CW: f32 = 8.4; // cell width (0.6em of 14px)
const CH: f32 = 18.0; // cell height: two half-block pixels make a square
const FONT: f32 = 14.0;
const PAD: f32 = 14.0;
const BAR: f32 = 30.0;

fn hex(c: Color, theme: &Theme, fg: bool) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset if fg => hex(theme.text, theme, true),
        Color::Reset => TERM_BG.into(),
        other => panic!("the demo themes are all RGB, got {other:?}"),
    }
}

/// Light/heavy arms (up, right, down, left) of a box-drawing character,
/// and whether its corner is rounded.
fn box_arms(sym: &str) -> Option<([u8; 4], bool)> {
    Some(match sym {
        "─" => ([0, 1, 0, 1], false),
        "│" => ([1, 0, 1, 0], false),
        "━" => ([0, 2, 0, 2], false),
        "┃" => ([2, 0, 2, 0], false),
        "┌" => ([0, 1, 1, 0], false),
        "┐" => ([0, 0, 1, 1], false),
        "└" => ([1, 1, 0, 0], false),
        "┘" => ([1, 0, 0, 1], false),
        "┏" => ([0, 2, 2, 0], false),
        "┓" => ([0, 0, 2, 2], false),
        "┗" => ([2, 2, 0, 0], false),
        "┛" => ([2, 0, 0, 2], false),
        "├" => ([1, 1, 1, 0], false),
        "┤" => ([1, 0, 1, 1], false),
        "┬" => ([0, 1, 1, 1], false),
        "┴" => ([1, 1, 0, 1], false),
        "┼" => ([1, 1, 1, 1], false),
        "╭" => ([0, 1, 1, 0], true),
        "╮" => ([0, 0, 1, 1], true),
        "╰" => ([1, 1, 0, 0], true),
        "╯" => ([1, 0, 0, 1], true),
        _ => return None,
    })
}

/// Block elements as (x0, y0, x1, y1) fractions of the cell, and opacity.
fn block(sym: &str) -> Option<(f32, f32, f32, f32, f32)> {
    Some(match sym {
        "█" => (0.0, 0.0, 1.0, 1.0, 1.0),
        "▀" => (0.0, 0.0, 1.0, 0.5, 1.0),
        "▄" => (0.0, 0.5, 1.0, 1.0, 1.0),
        "▌" => (0.0, 0.0, 0.5, 1.0, 1.0),
        "▍" => (0.0, 0.0, 0.375, 1.0, 1.0),
        "▏" => (0.0, 0.0, 0.125, 1.0, 1.0),
        "░" => (0.0, 0.0, 1.0, 1.0, 0.3),
        _ => return None,
    })
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn n(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// One cell's resolved colours and text attributes.
struct Look {
    fg: String,
    bg: String,
    bold: bool,
    dim: bool,
    underline: bool,
    strike: bool,
}

fn look(cell: &ratatui::buffer::Cell, theme: &Theme) -> Look {
    let m = cell.modifier;
    let (mut fg, mut bg) = (hex(cell.fg, theme, true), hex(cell.bg, theme, false));
    if m.contains(Modifier::REVERSED) {
        std::mem::swap(&mut fg, &mut bg);
    }
    Look {
        fg,
        bg,
        bold: m.contains(Modifier::BOLD),
        dim: m.contains(Modifier::DIM),
        underline: m.contains(Modifier::UNDERLINED),
        strike: m.contains(Modifier::CROSSED_OUT),
    }
}

fn to_svg(buf: &Buffer, theme: &Theme, title: &str) -> String {
    let (w, h) = (buf.area.width, buf.area.height);
    let (gw, gh) = (w as f32 * CW, h as f32 * CH);
    let (width, height) = (gw + 2.0 * PAD, gh + 2.0 * PAD + BAR);
    let (ox, oy) = (PAD, BAR + PAD);
    let mut bgs = String::new();
    let mut shapes: BTreeMap<String, String> = BTreeMap::new(); // fill colour -> path data
    let mut lines: BTreeMap<(String, u8), String> = BTreeMap::new(); // (stroke, weight) -> path data
    let mut text = String::new();

    for y in 0..h {
        // Background runs.
        let mut x = 0;
        while x < w {
            let bg = look(&buf[(x, y)], theme).bg;
            let start = x;
            while x < w && look(&buf[(x, y)], theme).bg == bg {
                x += 1;
            }
            if bg != TERM_BG {
                let _ = write!(
                    bgs,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{bg}"/>"#,
                    n(ox + start as f32 * CW),
                    n(oy + y as f32 * CH),
                    n((x - start) as f32 * CW),
                    n(CH)
                );
            }
        }
        // Glyphs: shapes for box and block characters, text runs for the rest.
        let mut x = 0;
        while x < w {
            let cell = &buf[(x, y)];
            let sym = cell.symbol();
            let l = look(cell, theme);
            let (cx, cy) = (ox + x as f32 * CW, oy + y as f32 * CH);
            if let Some((arms, round)) = box_arms(sym) {
                let (mx, my) = (cx + CW / 2.0, cy + CH / 2.0);
                let weight = *arms.iter().max().unwrap();
                let d = lines.entry((l.fg.clone(), weight)).or_default();
                if round {
                    // A quarter arc from the vertical arm to the horizontal one.
                    let r = CW / 2.0;
                    let (vy, hx) = (if arms[0] > 0 { cy } else { cy + CH }, if arms[1] > 0 { cx + CW } else { cx });
                    let (sy, ex) = (if arms[0] > 0 { my - r } else { my + r }, if arms[1] > 0 { mx + r } else { mx - r });
                    let _ = write!(d, "M{} {}V{}Q{} {} {} {}H{}", n(mx), n(vy), n(sy), n(mx), n(my), n(ex), n(my), n(hx));
                } else {
                    let ends = [(mx, cy), (cx + CW, my), (mx, cy + CH), (cx, my)];
                    for (i, a) in arms.iter().enumerate() {
                        if *a > 0 {
                            let (ex, ey) = ends[i];
                            let _ = write!(d, "M{} {}L{} {}", n(mx), n(my), n(ex), n(ey));
                        }
                    }
                }
                x += 1;
                continue;
            }
            if let Some((x0, y0, x1, y1, op)) = block(sym) {
                let mut fill = l.fg.clone();
                if op < 1.0 {
                    fill = format!("{fill}4d");
                }
                // One rect for a run of the same block in the same colour.
                let mut cols = 1;
                while x + cols < w && buf[(x + cols, y)].symbol() == sym && look(&buf[(x + cols, y)], theme).fg == l.fg {
                    cols += 1;
                }
                let x1 = if x1 < 1.0 { x1 } else { cols as f32 };
                // A hair of overlap, so neighbouring blocks show no seam.
                let (bw, bh) = ((x1 - x0) * CW + 0.4, (y1 - y0) * CH + 0.4);
                let _ = write!(shapes.entry(fill).or_default(), "M{} {}h{}v{}h-{}z", n(cx + x0 * CW), n(cy + y0 * CH), n(bw), n(bh), n(bw));
                x += if x1 < 1.0 { 1 } else { cols };
                continue;
            }
            if sym.trim().is_empty() {
                x += 1;
                continue;
            }
            // A run of same-looking, non-space text cells.
            let mut run = String::new();
            let mut xs = vec![];
            while x < w {
                let c = &buf[(x, y)];
                let s = c.symbol();
                let cl = look(c, theme);
                if s.trim().is_empty()
                    || box_arms(s).is_some()
                    || block(s).is_some()
                    || cl.fg != l.fg
                    || cl.bold != l.bold
                    || cl.dim != l.dim
                    || cl.underline != l.underline
                    || cl.strike != l.strike
                {
                    break;
                }
                xs.push(n(ox + x as f32 * CW));
                run.push_str(s);
                x += unicode_width::UnicodeWidthStr::width(s).max(1) as u16;
            }
            let mut attrs = format!(r#" fill="{}""#, l.fg);
            if l.bold {
                attrs.push_str(r#" font-weight="700""#);
            }
            if l.dim {
                attrs.push_str(r#" opacity=".6""#);
            }
            match (l.underline, l.strike) {
                (true, _) => attrs.push_str(r#" text-decoration="underline""#),
                (_, true) => attrs.push_str(r#" text-decoration="line-through""#),
                _ => {}
            }
            let _ = write!(text, r#"<text x="{}" y="{}"{attrs}>{}</text>"#, xs.join(" "), n(oy + y as f32 * CH + CH * 0.72), esc(&run));
        }
    }

    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="{t}">"#,
        W = n(width),
        H = n(height),
        t = esc(title)
    );
    let _ = write!(out, "<title>{}</title>", esc(title));
    let _ = write!(
        out,
        r#"<rect width="{W}" height="{H}" rx="10" fill="{TERM_BG}"/><path d="M10 0H{R}a10 10 0 0 1 10 10V{BAR}H0V10A10 10 0 0 1 10 0z" fill="{BAR_BG}"/>"#,
        W = n(width),
        H = n(height),
        R = n(width - 10.0),
    );
    for (i, c) in ["#f0e0b8", "#f0c25a", "#d98a44"].iter().enumerate() {
        let _ = write!(out, r#"<circle cx="{}" cy="15" r="5.5" fill="{c}" opacity=".85"/>"#, 20 + i * 18);
    }
    let _ = write!(
        out,
        r#"<text x="{}" y="20" text-anchor="middle" fill="{SOFT}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,'DejaVu Sans Mono','Liberation Mono',monospace" font-size="12">lt --demo</text>"#,
        n(width / 2.0)
    );
    out.push_str(&bgs);
    for (fill, d) in &shapes {
        let _ = write!(out, r#"<path fill="{fill}" d="{d}"/>"#);
    }
    for ((stroke, weight), d) in &lines {
        let sw = if *weight == 2 { 2.2 } else { 1.1 };
        let _ = write!(out, r#"<path fill="none" stroke="{stroke}" stroke-width="{}" stroke-linecap="square" d="{d}"/>"#, n(sw));
    }
    let _ = write!(
        out,
        r#"<g font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,'DejaVu Sans Mono','Liberation Mono',monospace" font-size="{}">{text}</g></svg>"#,
        n(FONT)
    );
    out.push('\n');
    out
}

fn buffer(h: &Harness, w: u16, ht: u16, theme: &Theme) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(w, ht)).unwrap();
    term.draw(|f| ligus_term::ui::draw(f, &h.state, theme)).unwrap();
    term.backend().buffer().clone()
}

fn write(dir: &Path, name: &str, title: &str, buf: &Buffer) {
    let path = dir.join(format!("{name}.svg"));
    std::fs::write(&path, to_svg(buf, &Theme::DARK, title)).unwrap();
    println!("{}", path.display());
}

const W: u16 = 104;
const H: u16 = 26;

/// `--gallery DIR`: every view, its row menu and its add popup, at two
/// sizes, into `DIR` (for reviewing a UI change side by side; nothing in
/// the repo uses these).
async fn gallery(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let t = Theme::DARK;
    let shots: &[(&str, &[KeyCode])] = &[
        ("chats", &[]),
        ("workflows", &[KeyCode::Char('f')]),
        ("workflows-run", &[KeyCode::Char('f'), KeyCode::Down, KeyCode::Down]),
        ("workflows-run-menu", &[KeyCode::Char('f'), KeyCode::Down, KeyCode::Down, KeyCode::Char('.')]),
        ("proposals", &[KeyCode::Char('p')]),
        ("proposals-menu", &[KeyCode::Char('p'), KeyCode::Char('.')]),
        ("agents", &[KeyCode::Char('a')]),
        ("agents-menu", &[KeyCode::Char('a'), KeyCode::Char('.')]),
        ("agents-add", &[KeyCode::Char('a'), KeyCode::Char('N')]),
        ("hosts", &[KeyCode::Char('h')]),
        ("memory", &[KeyCode::Char('y')]),
        ("people", &[KeyCode::Char('m')]),
        ("people-menu", &[KeyCode::Char('m'), KeyCode::Down, KeyCode::Down, KeyCode::Char('.')]),
        ("notifications", &[KeyCode::Char('n')]),
        ("settings", &[KeyCode::Char('s')]),
        ("help-people", &[KeyCode::Char('m'), KeyCode::Char('?')]),
        ("messages-empty", &[KeyCode::Char('!')]),
    ];
    for (name, keys) in shots {
        let mut h = Harness::new(true).await;
        h.key(KeyCode::Esc).await; // the cursor starts in the composer
        for k in *keys {
            h.key(*k).await;
        }
        for (w, ht) in [(110, 32), (80, 24)] {
            write(dir, &format!("{name}-{w}x{ht}"), &format!("lt: {name} ({w}×{ht})"), &buffer(&h, w, ht, &t));
        }
    }
}

/// One input of a scripted shot.
enum In {
    K(KeyCode),
    T(&'static str),
}

/// `--setup DIR`: the guided setup flows (the list, an agent's steps, a
/// tool with its token masked, a failed check, the person's hand-offs),
/// at two sizes, into `DIR`.
async fn setup_gallery(dir: &Path) {
    use In::{K, T};
    use KeyCode::{Down, Enter, Tab};
    std::fs::create_dir_all(dir).unwrap();
    let t = Theme::DARK;
    let u = K(KeyCode::Char('u'));
    let shots: Vec<(&str, Vec<In>)> = vec![
        ("01-setup-list", vec![u]),
        ("02-agent-harness", vec![K(KeyCode::Char('u')), K(Enter)]),
        ("03-agent-name", vec![K(KeyCode::Char('u')), K(Enter), K(Enter), T("helper")]),
        ("04-agent-model", vec![K(KeyCode::Char('u')), K(Enter), K(Enter), T("helper"), K(Enter), K(Down)]),
        (
            "05-agent-review",
            vec![K(KeyCode::Char('u')), K(Enter), K(Enter), T("helper"), K(Enter), K(Down), K(Enter), K(KeyCode::Char(' ')), K(Enter)],
        ),
        (
            "06-agent-done",
            vec![
                K(KeyCode::Char('u')),
                K(Enter),
                K(Enter),
                T("helper"),
                K(Enter),
                K(Down),
                K(Enter),
                K(KeyCode::Char(' ')),
                K(Enter),
                K(Enter),
            ],
        ),
        (
            "07-tool-server",
            vec![K(KeyCode::Char('u')), K(Down), K(Enter), T("issues"), K(Tab), K(Down), K(Tab), T("https://mcp.example.com/v1")],
        ),
        (
            "08-tool-token-masked",
            vec![
                K(KeyCode::Char('u')),
                K(Down),
                K(Enter),
                T("issues"),
                K(Tab),
                K(Down),
                K(Tab),
                T("https://mcp.example.com/v1"),
                K(Enter),
                T("example-token-123"),
            ],
        ),
        (
            "09-tool-review",
            vec![
                K(KeyCode::Char('u')),
                K(Down),
                K(Enter),
                T("issues"),
                K(Tab),
                K(Down),
                K(Tab),
                T("https://mcp.example.com/v1"),
                K(Enter),
                T("example-token-123"),
                K(Enter),
                K(Enter),
            ],
        ),
        (
            "10-agent-failed-check",
            vec![K(KeyCode::Char('u')), K(Enter), K(Down), K(Enter), T("coder2"), K(Enter), K(Enter), K(Enter), K(Enter)],
        ),
        ("11-invite-handoff", vec![K(KeyCode::Char('u')), K(Down), K(Down), K(Down), K(Down), K(Down), K(Enter), K(Enter), K(Enter)]),
        ("12-phone-handoff", vec![K(KeyCode::Char('u')), K(Down), K(Down), K(Down), K(Down), K(Enter), K(Enter), K(Enter)]),
        ("13-box-from-here", vec![K(KeyCode::Char('u')), K(Down), K(Down), K(Down), K(Enter)]),
        ("14-folder", vec![K(KeyCode::Char('u')), K(Down), K(Down), K(Enter), T("~/code/northwind")]),
    ];
    for (name, ins) in shots {
        let mut h = Harness::new(true).await;
        h.key(KeyCode::Esc).await; // the cursor starts in the composer
        for i in ins {
            match i {
                In::K(k) => h.key(k).await,
                In::T(s) => h.chars(s).await,
            }
        }
        for (w, ht) in [(110, 32), (80, 24)] {
            write(dir, &format!("{name}-{w}x{ht}"), &format!("lt: {name} ({w}×{ht})"), &buffer(&h, w, ht, &t));
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, dir] = args.as_slice()
        && flag == "--gallery"
    {
        return gallery(Path::new(dir)).await;
    }
    if let [_, flag, dir] = args.as_slice()
        && flag == "--setup"
    {
        return setup_gallery(Path::new(dir)).await;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/screens");
    std::fs::create_dir_all(&dir).unwrap();
    let t = Theme::DARK;

    // A workflow run waiting on its sign-off gate.
    let mut h = Harness::new(true).await;
    h.key(KeyCode::Esc).await; // the cursor starts in the composer
    h.key(KeyCode::Char('f')).await;
    h.key(KeyCode::Down).await;
    h.key(KeyCode::Down).await;
    write(&dir, "workflow-run", "lt: a workflow run waiting on its sign-off gate", &buffer(&h, W, H, &t));

    // Agents: what each is doing and which host it runs on; runtimes.
    let mut h = Harness::new(true).await;
    h.key(KeyCode::Esc).await; // the cursor starts in the composer
    h.key(KeyCode::Char('a')).await;
    write(&dir, "agents", "lt: agents, the hosts they run on, hand-offs and runtimes", &buffer(&h, W, 30, &t));

    // A proposal with its diff.
    let mut h = Harness::new(true).await;
    h.key(KeyCode::Esc).await; // the cursor starts in the composer
    h.key(KeyCode::Char('p')).await;
    h.key(KeyCode::Enter).await;
    write(&dir, "proposals", "lt: the proposal queue with a diff", &buffer(&h, W, H, &t));

    // An agent streaming its reply, with a tool call.
    let mut h = Harness::new(true).await;
    stream_reply(&mut h).await;
    write(&dir, "chat", "lt: a chat with an agent streaming its reply", &buffer(&h, W, H, &t));

    // Help: every keybinding.
    let mut h = Harness::new(true).await;
    h.key(KeyCode::Esc).await; // the cursor starts in the composer
    h.key(KeyCode::Char('?')).await;
    write(&dir, "help", "lt: the keybindings", &buffer(&h, W, 42, &t));

    // First run, with the mark (for the social preview).
    let h = Harness::new(false).await;
    write(&dir, "onboarding", "lt: first run", &buffer(&h, W, H, &t));
}

async fn stream_reply(h: &mut Harness) {
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
    for (name, preview) in [("read_file", "src/checkout.rs"), ("edit_file", "src/checkout.rs")] {
        h.send(Msg::Event(Event::ToolActivity {
            workspace: ws.clone(),
            chat: chat.clone(),
            message: MessageId::new("s1"),
            call: ToolCall { name: name.into(), preview: preview.into(), ok: true, ..ToolCall::default() },
        }))
        .await;
    }
    for d in "The session check now reads the injected clock: thirty seconds matches the provider's timeout, so ".split_inclusive(' ') {
        h.send(Msg::Event(Event::MessageDelta {
            workspace: ws.clone(),
            chat: chat.clone(),
            message: MessageId::new("s1"),
            delta: d.into(),
        }))
        .await;
    }
}
