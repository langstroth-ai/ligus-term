//! Dev-only: render lt's screens (the demo backend, through ratatui's
//! `TestBackend`) as SVG terminal images for the README, into
//! `assets/screens/`. Not part of the `lt` binary.
//!
//! ```sh
//! cargo run --example screenshots
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

/// The window's colours: the brand's dark tile by default; `--themes`
/// draws on a plain light or dark terminal instead.
static WINDOW: std::sync::Mutex<Option<(&'static str, &'static str, &'static str)>> = std::sync::Mutex::new(None);

fn window() -> (&'static str, &'static str, &'static str) {
    WINDOW.lock().unwrap().unwrap_or((TERM_BG, BAR_BG, SOFT))
}

const CW: f32 = 8.4; // cell width (0.6em of 14px)
const CH: f32 = 18.0; // cell height: two half-block pixels make a square
const FONT: f32 = 14.0;
const PAD: f32 = 14.0;
const BAR: f32 = 30.0;

fn hex(c: Color, theme: &Theme, fg: bool) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset if fg => hex(theme.text, theme, true),
        Color::Reset => window().0.into(),
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
            if bg != window().0 {
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
        r#"<rect width="{W}" height="{H}" rx="10" fill="{TB}"/><path d="M10 0H{R}a10 10 0 0 1 10 10V{BAR}H0V10A10 10 0 0 1 10 0z" fill="{BB}"/>"#,
        TB = window().0,
        BB = window().1,
        W = n(width),
        H = n(height),
        R = n(width - 10.0),
    );
    for (i, c) in ["#f0e0b8", "#f0c25a", "#d98a44"].iter().enumerate() {
        let _ = write!(out, r#"<circle cx="{}" cy="15" r="5.5" fill="{c}" opacity=".85"/>"#, 20 + i * 18);
    }
    let _ = write!(
        out,
        r#"<text x="{}" y="20" text-anchor="middle" fill="{}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,'DejaVu Sans Mono','Liberation Mono',monospace" font-size="12">lt --demo</text>"#,
        n(width / 2.0),
        window().2
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
    write_in(dir, name, title, buf, &Theme::DARK);
}

fn write_in(dir: &Path, name: &str, title: &str, buf: &Buffer, theme: &Theme) {
    let path = dir.join(format!("{name}.svg"));
    std::fs::write(&path, to_svg(buf, theme, title)).unwrap();
    println!("{}", path.display());
}

/// `--themes DIR`: two views in every palette, light (on white) and dark
/// (on a plain dark grey), plus the first-run mark on each background, for
/// reviewing a colour change side by side. Nothing in the repo uses them.
async fn themes(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let set = ligus_term::theme::ThemeSet::builtin();
    let views: &[(&str, &[KeyCode], bool)] =
        &[("chats", &[], true), ("proposals", &[KeyCode::Char('p'), KeyCode::Enter], true), ("first-run", &[], false)];
    for (view, keys, demo) in views {
        let mut h = Harness::new(*demo).await;
        if *demo {
            h.key(KeyCode::Esc).await;
        }
        for k in *keys {
            h.key(*k).await;
        }
        for id in set.ids().into_iter().filter(|i| *i != "mono") {
            for light in [true, false] {
                if *view == "first-run" && id != "pollen" {
                    continue;
                }
                let t = set.theme(id, light).unwrap();
                *WINDOW.lock().unwrap() = Some(if light { ("#ffffff", "#e8e8e8", "#6b6b6b") } else { ("#1e1e1e", "#2d2d2d", "#9a9a9a") });
                let mode = if light { "light" } else { "dark" };
                write_in(dir, &format!("{view}-{id}-{mode}"), &format!("lt: {view}, {} ({mode})", t.label), &buffer(&h, W, H, &t), &t);
            }
        }
    }
    *WINDOW.lock().unwrap() = None;
}

const W: u16 = 104;
const H: u16 = 26;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, dir] = args.as_slice()
        && flag == "--themes"
    {
        return themes(Path::new(dir)).await;
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
