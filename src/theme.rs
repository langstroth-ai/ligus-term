//! Colors for dark and light terminals, and a monochrome theme for
//! `NO_COLOR`. The background is always the terminal's own.

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::theme_tokens::{self as tokens, SchemeTokens, TokenColor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    Auto,
    Dark,
    Light,
    Mono,
}

impl std::str::FromStr for ThemeChoice {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(ThemeChoice::Auto),
            "dark" => Ok(ThemeChoice::Dark),
            "light" => Ok(ThemeChoice::Light),
            "mono" => Ok(ThemeChoice::Mono),
            _ => Err(format!("unknown theme “{s}” (auto, dark, light, mono)")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub text: Color,
    pub dim: Color,
    pub accent: Color,
    pub border: Color,
    pub focus: Color,
    pub selected_bg: Color,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
    pub added: Color,
    pub removed: Color,
    pub hunk: Color,
    /// Running and working (a blue-teal, the desktop's info).
    pub info: Color,
    pub mention: Color,
    pub agent: Color,
    /// Monochrome: lean on bold/reverse instead of color.
    pub mono: bool,
    /// ASCII only (`--ascii`): no box drawing or symbols, for limited fonts
    /// and braille displays.
    pub ascii: bool,
}

/// lt's own colour for agents' names: not a shared role (the tokens'
/// presence colours are for people), a lavender on dark and a violet on
/// light, kept from earlier releases.
const AGENT_DARK: Color = Color::Rgb(0xb7, 0x9c, 0xe8);
const AGENT_LIGHT: Color = Color::Rgb(0x5b, 0x3d, 0x9e);

const fn rgb(c: TokenColor) -> Color {
    Color::Rgb(c.rgb.0, c.rgb.1, c.rgb.2)
}

impl Theme {
    /// A theme from the shared Ligus tokens (`theme_tokens.rs`, generated
    /// from `assets/branding/ligus-brand-tokens.json` in the ligus repo), in
    /// 24-bit colour. Focus is the accent; hunk headers are info.
    const fn from_tokens(name: &'static str, k: &SchemeTokens, agent: Color) -> Theme {
        Theme {
            name,
            text: rgb(k.text),
            dim: rgb(k.dim),
            accent: rgb(k.accent),
            border: rgb(k.border),
            focus: rgb(k.accent),
            selected_bg: rgb(k.selection),
            good: rgb(k.ok),
            warn: rgb(k.warn),
            bad: rgb(k.danger),
            added: rgb(k.diff_add),
            removed: rgb(k.diff_del),
            hunk: rgb(k.info),
            info: rgb(k.info),
            mention: rgb(k.mention),
            agent,
            mono: false,
            ascii: false,
        }
    }

    /// The tokens a theme was built from (none for mono).
    fn tokens(&self) -> Option<&'static SchemeTokens> {
        match self.name {
            "dark" => Some(&tokens::DARK),
            "light" => Some(&tokens::LIGHT),
            _ => None,
        }
    }

    /// For a dark background: the shared tokens' dark scheme (the Pollen
    /// palette's inks). Warn is an amber-orange off the honey accent, so
    /// an open proposal or a warning never reads as the accent or focus.
    pub const DARK: Theme = Theme::from_tokens("dark", &tokens::DARK, AGENT_DARK);

    /// For a light background: the shared tokens' light scheme.
    pub const LIGHT: Theme = Theme::from_tokens("light", &tokens::LIGHT, AGENT_LIGHT);

    pub const MONO: Theme = Theme {
        name: "mono",
        text: Color::Reset,
        dim: Color::Reset,
        accent: Color::Reset,
        border: Color::Reset,
        focus: Color::Reset,
        selected_bg: Color::Reset,
        good: Color::Reset,
        warn: Color::Reset,
        bad: Color::Reset,
        added: Color::Reset,
        removed: Color::Reset,
        hunk: Color::Reset,
        info: Color::Reset,
        mention: Color::Reset,
        agent: Color::Reset,
        mono: true,
        ascii: false,
    };

    /// Pick a theme: an explicit choice wins, `NO_COLOR` means mono, and
    /// `auto` reads `COLORFGBG` ("fg;bg", bg 7 or 15 is light), else dark.
    pub fn resolve(choice: ThemeChoice, env: impl Fn(&str) -> Option<String>) -> Theme {
        Theme::resolve_with(choice, env, || None)
    }

    /// [`Theme::resolve`], asking `probe` (the terminal's own answer, OSC
    /// 11) when `auto` has no `COLORFGBG` to go on.
    pub fn resolve_with(choice: ThemeChoice, env: impl Fn(&str) -> Option<String>, probe: impl FnOnce() -> Option<bool>) -> Theme {
        if matches!(choice, ThemeChoice::Auto)
            && env("NO_COLOR").is_none_or(|v| v.is_empty())
            && env("COLORFGBG").is_none()
            && let Some(light) = probe()
        {
            return if light { Theme::LIGHT } else { Theme::DARK };
        }
        if env("NO_COLOR").is_some_and(|v| !v.is_empty()) && matches!(choice, ThemeChoice::Auto) {
            return Theme::MONO;
        }
        match choice {
            ThemeChoice::Dark => Theme::DARK,
            ThemeChoice::Light => Theme::LIGHT,
            ThemeChoice::Mono => Theme::MONO,
            ThemeChoice::Auto => {
                let bg = env("COLORFGBG").and_then(|v| v.rsplit(';').next().and_then(|b| b.parse::<u8>().ok()));
                match bg {
                    Some(7) | Some(15) => Theme::LIGHT,
                    _ => Theme::DARK,
                }
            }
        }
    }

    pub fn text(&self) -> Style {
        Style::default().fg(self.text)
    }
    pub fn dim(&self) -> Style {
        if self.mono { Style::default().add_modifier(Modifier::DIM) } else { Style::default().fg(self.dim) }
    }
    pub fn accent(&self) -> Style {
        Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
    }
    pub fn fg(&self, c: Color) -> Style {
        Style::default().fg(c)
    }
    pub fn border(&self, focused: bool) -> Style {
        if focused { Style::default().fg(self.focus).add_modifier(Modifier::BOLD) } else { Style::default().fg(self.border) }
    }
    pub fn selected(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default().bg(self.selected_bg).add_modifier(Modifier::BOLD)
        }
    }
    /// A refused (disabled) control.
    pub fn disabled(&self) -> Style {
        self.dim().add_modifier(Modifier::CROSSED_OUT)
    }
    /// A session state's glyph colour: info for working, the warm accent
    /// (a person is involved) for needs you, warn for queued, then ok,
    /// danger and neutral. Always next to the state's glyph.
    pub fn state(&self, st: crate::backend::model::SessionState) -> Style {
        use crate::backend::model::SessionState::*;
        if self.mono {
            return match st {
                NeedsYou | Failed => Style::default().add_modifier(Modifier::BOLD),
                _ => Style::default(),
            };
        }
        match st {
            Idle => self.dim(),
            Working => self.fg(self.info),
            NeedsYou => self.fg(self.mention).add_modifier(Modifier::BOLD),
            Queued => self.fg(self.warn),
            Done => self.fg(self.good),
            Failed => self.fg(self.bad).add_modifier(Modifier::BOLD),
            Paused => self.dim(),
        }
    }
    pub fn banner(&self, c: Color) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default().fg(c).add_modifier(Modifier::BOLD)
        }
    }
}

/// How many colours the terminal shows. The themes are written in 24-bit
/// colour; a terminal without it gets the nearest of its 256 or 16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
    Ansi16,
}

impl ColorDepth {
    /// `LT_COLOR_DEPTH` (truecolor, 256, 16) when set; else `COLORTERM`
    /// truecolor or 24bit means 24-bit colour; else `TERM` says 16 colours
    /// (linux, vt100, xterm, screen and the like without "256") or 256.
    pub fn detect(env: impl Fn(&str) -> Option<String>) -> ColorDepth {
        let forced = env("LT_COLOR_DEPTH").map(|v| v.trim().to_ascii_lowercase());
        match forced.as_deref() {
            Some("truecolor" | "24bit" | "24") => return ColorDepth::TrueColor,
            Some("256") => return ColorDepth::Ansi256,
            Some("16" | "8") => return ColorDepth::Ansi16,
            _ => {}
        }
        if env("COLORTERM").is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "truecolor" | "24bit")) {
            return ColorDepth::TrueColor;
        }
        let term = env("TERM").unwrap_or_default().to_ascii_lowercase();
        let basic = ["linux", "vt100", "vt102", "vt220", "xterm", "screen", "tmux", "ansi", "cons25", "rxvt"];
        if term.ends_with("-16color") || term.ends_with("-8color") || basic.contains(&term.as_str()) {
            ColorDepth::Ansi16
        } else {
            ColorDepth::Ansi256
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorDepth::TrueColor => "24-bit colour",
            ColorDepth::Ansi256 => "256 colours",
            ColorDepth::Ansi16 => "16 colours",
        }
    }
}

/// The xterm-256 index nearest an RGB colour (the 6×6×6 cube or the grays).
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let idx = |v: u8| LEVELS.iter().enumerate().min_by_key(|(_, l)| (i32::from(**l) - i32::from(v)).abs()).map(|(i, _)| i).unwrap_or(0);
    let (ri, gi, bi) = (idx(r), idx(g), idx(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let gray_i = ((avg.saturating_sub(8)) / 10).min(23) as u8;
    let gray = 8 + 10 * gray_i;
    let d = |c: (u8, u8, u8)| dist((r, g, b), c);
    if d((gray, gray, gray)) < d(cube) { 232 + gray_i } else { 16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8 }
}

fn dist(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let sq = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2) as u32;
    sq(a.0, b.0) + sq(a.1, b.1) + sq(a.2, b.2)
}

/// The ANSI colour for an RGB colour: grays by lightness, others by hue
/// (so red stays red), bright for light colours. The terminal's own palette
/// then applies.
pub fn nearest_16(r: u8, g: u8, b: u8) -> Color {
    let (rf, gf, bf) = (f32::from(r), f32::from(g), f32::from(b));
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let light = (max + min) / 2.0;
    if max - min < 40.0 {
        return match light {
            l if l < 50.0 => Color::Black,
            l if l < 150.0 => Color::DarkGray,
            l if l < 215.0 => Color::Gray,
            _ => Color::White,
        };
    }
    let d = max - min;
    let hue = if max == rf {
        60.0 * (((gf - bf) / d).rem_euclid(6.0))
    } else if max == gf {
        60.0 * ((bf - rf) / d + 2.0)
    } else {
        60.0 * ((rf - gf) / d + 4.0)
    };
    let bright = light >= 128.0;
    let pick = |normal, brighter| if bright { brighter } else { normal };
    match hue {
        h if !(30.0..330.0).contains(&h) => pick(Color::Red, Color::LightRed),
        h if h < 90.0 => pick(Color::Yellow, Color::LightYellow),
        h if h < 150.0 => pick(Color::Green, Color::LightGreen),
        h if h < 210.0 => pick(Color::Cyan, Color::LightCyan),
        h if h < 270.0 => pick(Color::Blue, Color::LightBlue),
        _ => pick(Color::Magenta, Color::LightMagenta),
    }
}

/// A 16-colour slot (0..15) as the terminal's own named colour.
fn ansi(slot: u8) -> Color {
    match slot {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        _ => Color::White,
    }
}

impl Theme {
    /// This theme in the colours the terminal has: the tokens' own
    /// 256-colour index and 16-colour slot. A token with no 16-colour hue
    /// (the text, the accent and focus, mentions) takes the terminal's own
    /// foreground, and the accent relies on bold and reverse, so yellow
    /// stays warn's.
    pub fn with_depth(self, depth: ColorDepth) -> Theme {
        if self.mono || depth == ColorDepth::TrueColor {
            return self;
        }
        let Some(k) = self.tokens() else { return self };
        let pick = |c: TokenColor, fallback: Color| match depth {
            ColorDepth::Ansi256 => Color::Indexed(c.xterm256),
            ColorDepth::Ansi16 => c.ansi16.map(ansi).unwrap_or(fallback),
            ColorDepth::TrueColor => rgb(c),
        };
        let agent = match (self.agent, depth) {
            (Color::Rgb(r, g, b), ColorDepth::Ansi256) => Color::Indexed(nearest_256(r, g, b)),
            (Color::Rgb(r, g, b), ColorDepth::Ansi16) => nearest_16(r, g, b),
            (c, _) => c,
        };
        let selection = if self.name == "light" { Color::Gray } else { Color::DarkGray };
        Theme {
            text: pick(k.text, Color::Reset),
            dim: pick(k.dim, Color::DarkGray),
            accent: pick(k.accent, Color::Reset),
            border: pick(k.border, Color::DarkGray),
            focus: pick(k.accent, Color::Reset),
            selected_bg: pick(k.selection, selection),
            good: pick(k.ok, Color::Green),
            warn: pick(k.warn, Color::Yellow),
            bad: pick(k.danger, Color::Red),
            added: pick(k.diff_add, Color::Green),
            removed: pick(k.diff_del, Color::Red),
            hunk: pick(k.info, Color::Cyan),
            info: pick(k.info, Color::Cyan),
            mention: pick(k.mention, Color::Reset),
            agent,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn resolves() {
        assert_eq!(Theme::resolve(ThemeChoice::Auto, env(&[])).name, "dark");
        assert_eq!(Theme::resolve(ThemeChoice::Auto, env(&[("COLORFGBG", "0;15")])).name, "light");
        assert_eq!(Theme::resolve(ThemeChoice::Auto, env(&[("COLORFGBG", "15;0")])).name, "dark");
        assert_eq!(Theme::resolve(ThemeChoice::Auto, env(&[("NO_COLOR", "1")])).name, "mono");
        assert_eq!(Theme::resolve(ThemeChoice::Light, env(&[("NO_COLOR", "1")])).name, "light");
    }

    /// The terminal's own answer (OSC 11) decides `auto` when COLORFGBG
    /// doesn't; it is never asked when the choice is made otherwise.
    #[test]
    fn auto_asks_the_terminal_last() {
        assert_eq!(Theme::resolve_with(ThemeChoice::Auto, env(&[]), || Some(true)).name, "light");
        assert_eq!(Theme::resolve_with(ThemeChoice::Auto, env(&[]), || Some(false)).name, "dark");
        assert_eq!(Theme::resolve_with(ThemeChoice::Auto, env(&[]), || None).name, "dark");
        let never = || -> Option<bool> { panic!("asked the terminal") };
        assert_eq!(Theme::resolve_with(ThemeChoice::Auto, env(&[("COLORFGBG", "0;15")]), never).name, "light");
        assert_eq!(Theme::resolve_with(ThemeChoice::Auto, env(&[("NO_COLOR", "1")]), never).name, "mono");
        assert_eq!(Theme::resolve_with(ThemeChoice::Dark, env(&[]), never).name, "dark");
    }
}

#[cfg(test)]
mod hue_tests {
    use super::*;

    /// OKLCH lightness and hue (degrees) of an sRGB colour.
    fn oklch(c: Color) -> (f64, f64) {
        let Color::Rgb(r, g, b) = c else { panic!("{c:?} isn't 24-bit") };
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        let (r, g, b) = (lin(r), lin(g), lin(b));
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        let ll = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
        let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
        let bb = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
        (ll, bb.atan2(a).to_degrees().rem_euclid(360.0))
    }

    fn gap(a: f64, b: f64) -> f64 {
        let d = (a - b).abs();
        d.min(360.0 - d)
    }

    /// Warn, the accent and the focused border are three different colours,
    /// and warn's hue sits well away from the honey accent and from danger's
    /// red, in both themes and in 256 colours.
    #[test]
    fn warn_is_neither_the_accent_nor_focus() {
        for t in [Theme::DARK, Theme::LIGHT] {
            assert_ne!(t.warn, t.accent, "{}", t.name);
            assert_ne!(t.warn, t.focus, "{}", t.name);
            let (_, warn) = oklch(t.warn);
            let (_, accent) = oklch(t.accent);
            let (_, focus) = oklch(t.focus);
            let (_, bad) = oklch(t.bad);
            assert!(gap(warn, accent) >= 25.0, "{}: warn {warn:.0}° vs accent {accent:.0}°", t.name);
            assert!(gap(warn, focus) >= 25.0, "{}: warn {warn:.0}° vs focus {focus:.0}°", t.name);
            // Hue alone never separates warn from danger: their glyphs do,
            // and the tokens' validator checks the pair under colour-blind
            // simulation. They still differ.
            assert!(gap(warn, bad) >= 10.0, "{}: warn {warn:.0}° vs danger {bad:.0}°", t.name);
            let t256 = t.with_depth(ColorDepth::Ansi256);
            assert_ne!(t256.warn, t256.accent, "{} in 256 colours", t.name);
            assert_ne!(t256.warn, t256.bad, "{} in 256 colours", t.name);
        }
        // In 16 colours warn is the terminal's yellow, never danger's red.
        let dark16 = Theme::DARK.with_depth(ColorDepth::Ansi16);
        assert_eq!(dark16.warn, Color::Yellow);
        assert_ne!(dark16.warn, dark16.accent);
        assert_ne!(dark16.warn, dark16.bad);
    }

    /// The themes are the shared tokens, colour for colour: what the
    /// desktop and the phone draw with (the parity test).
    #[test]
    fn themes_are_the_shared_tokens() {
        for (t, k) in [(Theme::DARK, &tokens::DARK), (Theme::LIGHT, &tokens::LIGHT)] {
            let pairs = [
                (t.text, k.text),
                (t.dim, k.dim),
                (t.accent, k.accent),
                (t.focus, k.accent),
                (t.border, k.border),
                (t.selected_bg, k.selection),
                (t.good, k.ok),
                (t.warn, k.warn),
                (t.bad, k.danger),
                (t.info, k.info),
                (t.mention, k.mention),
                (t.added, k.diff_add),
                (t.removed, k.diff_del),
            ];
            for (c, tok) in pairs {
                assert_eq!(c, rgb(tok), "{}", t.name);
            }
            let t256 = t.with_depth(ColorDepth::Ansi256);
            assert_eq!(t256.warn, Color::Indexed(k.warn.xterm256));
        }
        // An amber-orange: off the honey accent's hue on both backgrounds.
        let (_, h) = oklch(Theme::DARK.warn);
        assert!((40.0..60.0).contains(&h), "h {h:.1}");
        // The accent has no 16-colour hue; it leans on bold.
        assert_eq!(Theme::DARK.with_depth(ColorDepth::Ansi16).accent, Color::Reset);
    }
}

#[cfg(test)]
mod depth_tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn detects_colour_depth() {
        assert_eq!(ColorDepth::detect(env(&[("COLORTERM", "truecolor"), ("TERM", "xterm-256color")])), ColorDepth::TrueColor);
        assert_eq!(ColorDepth::detect(env(&[("TERM", "xterm-256color")])), ColorDepth::Ansi256);
        assert_eq!(ColorDepth::detect(env(&[("TERM", "linux")])), ColorDepth::Ansi16);
        assert_eq!(ColorDepth::detect(env(&[("TERM", "screen")])), ColorDepth::Ansi16);
        assert_eq!(ColorDepth::detect(env(&[("TERM", "linux"), ("LT_COLOR_DEPTH", "truecolor")])), ColorDepth::TrueColor);
        assert_eq!(ColorDepth::detect(env(&[("COLORTERM", "truecolor"), ("LT_COLOR_DEPTH", "16")])), ColorDepth::Ansi16);
    }

    #[test]
    fn maps_to_the_nearest_colour() {
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(0x8a, 0x80, 0x73), 101);
        assert_eq!(nearest_256(0x80, 0x80, 0x80), 244);
        assert_eq!(nearest_16(0xef, 0x6b, 0x5b), Color::LightRed);
        assert_eq!(nearest_16(0x8c, 0xc8, 0x6b), Color::LightGreen);
        assert_eq!(nearest_16(0xb0, 0x26, 0x1a), Color::Red);
        assert_eq!(nearest_16(0xf0, 0xc2, 0x5a), Color::LightYellow);
        assert_eq!(nearest_16(0x8a, 0x80, 0x73), Color::DarkGray);
        // Every theme colour leaves 24-bit behind; mono stays as it is.
        for (theme, depth) in [(Theme::DARK, ColorDepth::Ansi256), (Theme::LIGHT, ColorDepth::Ansi16)] {
            let t = theme.with_depth(depth);
            for c in [t.text, t.dim, t.accent, t.border, t.focus, t.selected_bg, t.good, t.warn, t.bad, t.hunk, t.mention, t.agent] {
                assert!(!matches!(c, Color::Rgb(..)), "{c:?}");
            }
        }
        assert_eq!(Theme::MONO.with_depth(ColorDepth::Ansi16), Theme::MONO);
        assert_eq!(Theme::DARK.with_depth(ColorDepth::TrueColor), Theme::DARK);
    }
}
