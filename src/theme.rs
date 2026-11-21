//! Colors for dark and light terminals, and a monochrome theme for
//! `NO_COLOR`. The background is always the terminal's own.

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

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
    pub mention: Color,
    pub agent: Color,
    /// Monochrome: lean on bold/reverse instead of color.
    pub mono: bool,
}

impl Theme {
    /// Amber on the terminal's dark background. The inks are the Ligus
    /// brand tokens (`ligus-brand-tokens.json`): paper for text, soft for
    /// dim, honeyLight for the accent, activeLight for mentions, tileTopDark
    /// for the selection.
    pub const DARK: Theme = Theme {
        name: "dark",
        text: Color::Rgb(0xf4, 0xf1, 0xea),
        dim: Color::Rgb(0x8a, 0x80, 0x73),
        accent: Color::Rgb(0xf0, 0xc2, 0x5a),
        border: Color::Rgb(0x4a, 0x45, 0x3c),
        focus: Color::Rgb(0xf0, 0xc2, 0x5a),
        selected_bg: Color::Rgb(0x3a, 0x28, 0x12),
        good: Color::Rgb(0x8c, 0xc8, 0x6b),
        warn: Color::Rgb(0xf0, 0xc2, 0x5a),
        bad: Color::Rgb(0xef, 0x6b, 0x5b),
        added: Color::Rgb(0x8c, 0xc8, 0x6b),
        removed: Color::Rgb(0xef, 0x6b, 0x5b),
        hunk: Color::Rgb(0x7f, 0xb4, 0xe0),
        mention: Color::Rgb(0xf0, 0xb8, 0x78),
        agent: Color::Rgb(0xb7, 0x9c, 0xe8),
        mono: false,
    };

    /// Darker inks for light backgrounds: the brand's ink for text,
    /// honeyDeep for the accent and cellMid for the selection. Dim is a
    /// shade darker than the brand's soft, which is too faint on white.
    pub const LIGHT: Theme = Theme {
        name: "light",
        text: Color::Rgb(0x22, 0x1f, 0x1a),
        dim: Color::Rgb(0x6d, 0x66, 0x5a),
        accent: Color::Rgb(0x8a, 0x5a, 0x14),
        border: Color::Rgb(0xb9, 0xb0, 0x9f),
        focus: Color::Rgb(0x8a, 0x5a, 0x14),
        selected_bg: Color::Rgb(0xf0, 0xe0, 0xb8),
        good: Color::Rgb(0x2f, 0x6f, 0x1f),
        warn: Color::Rgb(0x8a, 0x55, 0x00),
        bad: Color::Rgb(0xb0, 0x26, 0x1a),
        added: Color::Rgb(0x2f, 0x6f, 0x1f),
        removed: Color::Rgb(0xb0, 0x26, 0x1a),
        hunk: Color::Rgb(0x1f, 0x5a, 0x8f),
        mention: Color::Rgb(0x8a, 0x55, 0x00),
        agent: Color::Rgb(0x5b, 0x3d, 0x9e),
        mono: false,
    };

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
        mention: Color::Reset,
        agent: Color::Reset,
        mono: true,
    };

    /// Pick a theme: an explicit choice wins, `NO_COLOR` means mono, and
    /// `auto` reads `COLORFGBG` ("fg;bg", bg 7 or 15 is light), else dark.
    pub fn resolve(choice: ThemeChoice, env: impl Fn(&str) -> Option<String>) -> Theme {
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

impl Theme {
    /// This theme in the colours the terminal has.
    pub fn with_depth(self, depth: ColorDepth) -> Theme {
        let map = |c: Color| match (c, depth) {
            (Color::Rgb(r, g, b), ColorDepth::Ansi256) => Color::Indexed(nearest_256(r, g, b)),
            (Color::Rgb(r, g, b), ColorDepth::Ansi16) => nearest_16(r, g, b),
            (c, _) => c,
        };
        let mut t = Theme {
            text: map(self.text),
            dim: map(self.dim),
            accent: map(self.accent),
            border: map(self.border),
            focus: map(self.focus),
            selected_bg: map(self.selected_bg),
            good: map(self.good),
            warn: map(self.warn),
            bad: map(self.bad),
            added: map(self.added),
            removed: map(self.removed),
            hunk: map(self.hunk),
            mention: map(self.mention),
            agent: map(self.agent),
            ..self
        };
        // With 16 colours the selection's dark tint rounds to the
        // background's own colour: use the palette's gray.
        if depth == ColorDepth::Ansi16 && !self.mono {
            t.selected_bg = if self.name == "light" { Color::Gray } else { Color::DarkGray };
        }
        t
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
