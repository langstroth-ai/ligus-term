//! Every view, in every palette, on common terminal backgrounds: each
//! cell's colour against what is behind it (its own background, else the
//! terminal's). Text needs WCAG AA's 4.5:1, box drawing 3:1. The table of
//! the palettes' ratios is a snapshot, so a token change shows its effect.

mod common;

use common::Harness;
use crossterm::event::KeyCode;
use ligus_term::theme::{Theme, ThemeSet};
use ligus_term::theme_file::contrast;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

/// White (macOS Terminal Basic, GitHub Light), Solarized Light; black, a
/// common grey, Solarized Dark, GitHub Dark.
const LIGHT_BGS: [(&str, (u8, u8, u8)); 2] = [("white", (255, 255, 255)), ("solarized-light", (253, 246, 227))];
const DARK_BGS: [(&str, (u8, u8, u8)); 4] =
    [("black", (0, 0, 0)), ("grey", (30, 30, 30)), ("solarized-dark", (0, 43, 54)), ("github-dark", (13, 17, 23))];

fn rgb(c: Color) -> Option<(u8, u8, u8)> {
    match c {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

fn is_line_art(sym: &str) -> bool {
    sym.chars().all(|c| ('\u{2500}'..='\u{257f}').contains(&c) || matches!(c, '▏' | '▍' | '▌' | '│'))
}

/// The mark is a logo (exempt from contrast rules) drawn in its own colours.
fn is_block_art(sym: &str) -> bool {
    sym.chars().all(|c| ('\u{2580}'..='\u{259f}').contains(&c))
}

const VIEWS: &[(&str, &[KeyCode])] = &[
    ("chats", &[]),
    ("workflows", &[KeyCode::Char('f')]),
    ("proposals", &[KeyCode::Char('p')]),
    ("agents", &[KeyCode::Char('a')]),
    ("hosts", &[KeyCode::Char('h')]),
    ("memory", &[KeyCode::Char('y')]),
    ("people", &[KeyCode::Char('m')]),
    ("notifications", &[KeyCode::Char('n')]),
    ("settings", &[KeyCode::Char('s')]),
    ("help", &[KeyCode::Char('?')]),
    ("palette", &[KeyCode::Char(':')]),
];

#[tokio::test]
async fn every_view_reads_in_every_palette() {
    let set = ThemeSet::builtin();
    let mut failures = vec![];
    for (view, keys) in VIEWS {
        let mut h = Harness::new(true).await;
        h.key(KeyCode::Esc).await;
        for k in *keys {
            h.key(*k).await;
        }
        for id in set.ids().into_iter().filter(|i| *i != "mono") {
            for light in [true, false] {
                let t = set.theme(id, light).unwrap();
                let mut term = Terminal::new(TestBackend::new(110, 32)).unwrap();
                term.draw(|f| ligus_term::ui::draw(f, &h.state, &t)).unwrap();
                let buf = term.backend().buffer().clone();
                let bgs: &[(&str, (u8, u8, u8))] = if light { &LIGHT_BGS } else { &DARK_BGS };
                for (bg_name, term_bg) in bgs {
                    let mut worst: Option<(f64, String)> = None;
                    for cell in buf.content() {
                        let sym = cell.symbol();
                        if sym.trim().is_empty() || is_block_art(sym) {
                            continue;
                        }
                        let (mut fg, mut bg) = (rgb(cell.fg).unwrap_or(rgb(t.text).unwrap()), rgb(cell.bg).unwrap_or(*term_bg));
                        if cell.modifier.contains(Modifier::REVERSED) {
                            std::mem::swap(&mut fg, &mut bg);
                        }
                        let need = if is_line_art(sym) { 3.0 } else { 4.5 };
                        let c = contrast(fg, bg);
                        if c < need && worst.as_ref().is_none_or(|(w, _)| c < *w) {
                            worst = Some((
                                c,
                                format!("“{sym}” #{:02x}{:02x}{:02x} on #{:02x}{:02x}{:02x}", fg.0, fg.1, fg.2, bg.0, bg.1, bg.2),
                            ));
                        }
                    }
                    if let Some((c, what)) = worst {
                        failures.push(format!("{view} {id}/{} on {bg_name}: {c:.2}:1 {what}", if light { "light" } else { "dark" }));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} low-contrast cells:\n{}", failures.len(), failures.join("\n"));
}

/// Every palette's ratios, as a table to review: each token on the
/// palette's canvas and on the worst common terminal background.
#[test]
fn contrast_table() {
    let set = ThemeSet::builtin();
    let mut out = String::from("palette/appearance  token         canvas  worst-terminal\n");
    for id in set.ids().into_iter().filter(|i| *i != "mono") {
        for light in [true, false] {
            let t: Theme = set.theme(id, light).unwrap();
            let k = t.tokens.unwrap();
            let bgs: Vec<(u8, u8, u8)> =
                if light { LIGHT_BGS.iter().map(|b| b.1).collect() } else { DARK_BGS.iter().map(|b| b.1).collect() };
            let rows = [
                ("text", k.text, 4.5),
                ("dim", k.dim, 4.5),
                ("accent", k.accent, 4.5),
                ("mention", k.mention, 4.5),
                ("agent", k.agent, 4.5),
                ("ok", k.ok, 4.5),
                ("info", k.info, 4.5),
                ("warn", k.warn, 4.5),
                ("danger", k.danger, 4.5),
                ("neutral", k.neutral, 4.5),
                ("diff_add", k.diff_add, 4.5),
                ("diff_del", k.diff_del, 4.5),
                ("diff_comment", k.diff_comment, 3.0),
                ("border", k.border, 3.0),
            ];
            for (name, c, need) in rows {
                let on_canvas = contrast(c.rgb, k.canvas.rgb);
                let worst = bgs.iter().map(|b| contrast(c.rgb, *b)).fold(f64::INFINITY, f64::min);
                assert!(on_canvas >= need && worst >= need, "{id} {name}: {on_canvas:.2} / {worst:.2} (needs {need})");
                out.push_str(&format!(
                    "{:<19} {name:<13} {on_canvas:>6.2}  {worst:>6.2}\n",
                    format!("{id}/{}", if light { "light" } else { "dark" })
                ));
            }
            let sel = contrast(k.text.rgb, k.selection.rgb);
            assert!(sel >= 4.5, "{id}: text on the selection {sel:.2}");
            out.push_str(&format!("{:<19} {:<13} {sel:>6.2}\n", format!("{id}/{}", if light { "light" } else { "dark" }), "text/selected"));
        }
    }
    insta::assert_snapshot!("contrast_table", out);
}

/// The table lt draws with is the shared tokens' (generated in the ligus
/// repo from assets/branding/ligus-brand-tokens.json): the copy here is
/// the file it was made from, byte for byte, and the colours lt uses as
/// they are (the ink, the canvas, the diff inks) are that file's.
#[test]
fn the_colour_table_matches_the_brand_tokens() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/branding/ligus-brand-tokens.json")).unwrap();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in &bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    assert_eq!(
        h,
        ligus_term::theme_tokens::SOURCE_FNV1A64,
        "src/theme_tokens.rs wasn't generated from assets/branding/ligus-brand-tokens.json: run scripts/sync-theme-tokens.sh"
    );
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let names: Vec<&str> = json["themes"].as_object().unwrap().keys().map(String::as_str).collect();
    let mut ours: Vec<&str> = ligus_term::theme_tokens::PALETTES.iter().map(|p| p.name).collect();
    let mut theirs = names.clone();
    ours.sort();
    theirs.sort();
    assert_eq!(ours, theirs);
    let parse = |v: &serde_json::Value| ligus_term::theme_file::parse_color(v.as_str().unwrap()).unwrap().rgb();
    for p in ligus_term::theme_tokens::PALETTES {
        for (variant, k) in [("light", &p.light), ("dark", &p.dark)] {
            let src = &json["themes"][p.name][variant];
            if src["scheme"] == variant {
                assert_eq!(k.text.rgb, parse(&src["ink"]), "{} {variant} ink", p.name);
                assert_eq!(k.canvas.rgb, parse(&src["canvas"]), "{} {variant} canvas", p.name);
            }
            assert_eq!(k.diff_add.rgb, parse(&json["schemes"][variant]["diff"]["addInk"]), "{} {variant}", p.name);
            assert_eq!(k.diff_del.rgb, parse(&json["schemes"][variant]["diff"]["delInk"]), "{} {variant}", p.name);
        }
    }
}

/// The theme file's tokens are the shared schema's (assets/themes, a copy
/// of the ligus repo's docs-site/docs/schemas/ligus-theme.v1.json).
#[test]
fn theme_tokens_are_the_schemas() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/themes/ligus-theme.v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut props: Vec<String> = schema["$defs"]["scheme"]["properties"].as_object().unwrap().keys().cloned().collect();
    let mut ours: Vec<String> = ligus_term::theme_file::LT_TOKENS
        .iter()
        .map(|(t, _)| t.to_string())
        .chain(ligus_term::theme_file::DESKTOP_TOKENS.iter().map(|t| t.to_string()))
        .collect();
    props.sort();
    ours.sort();
    assert_eq!(props, ours);
    let palettes: Vec<&str> = schema["properties"]["extends"]["enum"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    for p in ligus_term::theme_tokens::PALETTES {
        assert!(palettes.contains(&p.name), "{}", p.name);
    }
}
