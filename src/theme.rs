//! Colours: the shared Ligus palettes (Pollen, Studio, Harbor, Meadow,
//! Slate, Obsidian), each for a light and a dark terminal, custom themes
//! from theme files, and a monochrome theme for `NO_COLOR`. The background
//! is always the terminal's own.

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::theme_file::ThemeFile;
use crate::theme_tokens::{self as tokens, PALETTES, SchemeTokens, TokenColor};

/// Light or dark: `auto` follows the terminal's background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Auto,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::Auto, Appearance::Light, Appearance::Dark];

    pub fn name(self) -> &'static str {
        match self {
            Appearance::Auto => "auto",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    /// The next one round: auto, light, dark.
    pub fn next(self) -> Appearance {
        match self {
            Appearance::Auto => Appearance::Light,
            Appearance::Light => Appearance::Dark,
            Appearance::Dark => Appearance::Auto,
        }
    }
}

impl std::str::FromStr for Appearance {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Appearance::Auto),
            "light" => Ok(Appearance::Light),
            "dark" => Ok(Appearance::Dark),
            _ => Err(format!("unknown appearance “{s}” (auto, light, dark)")),
        }
    }
}

/// One layer of what was asked for (the config file, then the flags):
/// `theme` names a palette, a custom theme, `mono`, or one of the old
/// values `auto`, `dark` and `light` (Pollen, with that appearance).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeSpec {
    pub theme: Option<String>,
    pub appearance: Option<Appearance>,
}

impl ThemeSpec {
    /// The old values as what they mean: `auto`, `dark` and `light` set
    /// the appearance (unless this same layer sets one) and leave the
    /// palette to the default (`auto`) or name Pollen.
    fn expanded(&self) -> (Option<Option<String>>, Option<Appearance>) {
        let (theme, implied) = match self.theme.as_deref().map(str::trim) {
            None => (None, None),
            Some("auto") => (Some(None), Some(Appearance::Auto)),
            Some("dark") => (Some(Some(PALETTES[0].name.to_string())), Some(Appearance::Dark)),
            Some("light") => (Some(Some(PALETTES[0].name.to_string())), Some(Appearance::Light)),
            Some(t) => (Some(Some(t.to_string())), None),
        };
        (theme, self.appearance.or(implied))
    }

    /// `over` (the flags) on top of `self` (the config file).
    pub fn layered(&self, over: &ThemeSpec) -> Resolved {
        let (t0, a0) = self.expanded();
        let (t1, a1) = over.expanded();
        Resolved { theme: t1.or(t0).flatten(), appearance: a1.or(a0).unwrap_or(Appearance::Auto) }
    }
}

/// What to draw with, once the layers are merged: a theme by name (`None`
/// when none was named: Pollen, or mono under `NO_COLOR`) and the
/// appearance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub theme: Option<String>,
    pub appearance: Appearance,
}

/// A theme change asked for in the app (Settings, the palette), for the
/// runtime to make; this session only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeRequest {
    /// Keep this theme and/or appearance.
    Set { theme: Option<String>, appearance: Option<Appearance> },
    /// Show it while the palette's selection is on it.
    Preview { theme: Option<String>, appearance: Option<Appearance> },
    /// Back to the one kept.
    Revert,
}

/// The full-screen app's theme for this run: what is kept, and what it
/// needs to switch (Settings, the palette) without asking the terminal
/// again.
#[derive(Debug, Clone)]
pub struct Session {
    pub set: ThemeSet,
    pub depth: ColorDepth,
    pub ascii: bool,
    /// What `appearance = auto` found at start.
    pub auto_light: bool,
    /// The theme kept, by name, and the appearance.
    pub theme: String,
    pub appearance: Appearance,
}

impl Session {
    /// `id` with `appearance`, in the terminal's colours.
    pub fn build(&self, id: &str, appearance: Appearance) -> Theme {
        let light = match appearance {
            Appearance::Light => true,
            Appearance::Dark => false,
            Appearance::Auto => self.auto_light,
        };
        Theme { ascii: self.ascii, ..self.set.theme(id, light).unwrap_or(Theme::DARK).with_depth(self.depth) }
    }

    /// The kept theme.
    pub fn current(&self) -> Theme {
        self.build(&self.theme, self.appearance)
    }

    /// How Settings shows a theme: "Pollen (24-bit colour)", "Mono".
    pub fn theme_label(&self, t: &Theme) -> String {
        if t.mono { t.label.to_string() } else { format!("{} ({})", t.label, self.depth.label()) }
    }

    /// How Settings shows the appearance: "auto (light)", "dark".
    pub fn appearance_label(&self) -> String {
        match self.appearance {
            Appearance::Auto => format!("auto ({})", if self.auto_light { "light" } else { "dark" }),
            a => a.name().to_string(),
        }
    }

    /// Every theme, (id, label), in the order Settings cycles them.
    pub fn list(&self) -> Vec<(String, String)> {
        self.set.families().iter().map(|f| (f.id.to_string(), f.label.to_string())).chain([("mono".into(), "Mono".into())]).collect()
    }
}

/// A theme lt can draw: a built-in palette or a custom one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Family {
    pub id: &'static str,
    pub label: &'static str,
    pub light: &'static SchemeTokens,
    pub dark: &'static SchemeTokens,
    pub custom: bool,
}

/// The built-in palettes' names as Settings shows them.
const LABELS: [(&str, &str); 6] =
    [("pollen", "Pollen"), ("studio", "Studio"), ("harbor", "Harbor"), ("meadow", "Meadow"), ("slate", "Slate"), ("obsidian", "Obsidian")];

fn label_of(id: &'static str) -> &'static str {
    LABELS.iter().find(|(i, _)| *i == id).map(|(_, l)| *l).unwrap_or(id)
}

/// Every theme this run knows: the palettes, then the custom themes (from
/// `themes/` next to the config file), then mono.
#[derive(Debug, Clone, Default)]
pub struct ThemeSet {
    pub custom: Vec<Family>,
}

impl ThemeSet {
    /// The built-in palettes only.
    pub fn builtin() -> ThemeSet {
        ThemeSet::default()
    }

    /// The palettes and these custom themes. Their colours live for the
    /// rest of the run (lt reads its theme files once, at start).
    pub fn with_custom(files: Vec<ThemeFile>) -> ThemeSet {
        let custom = files
            .into_iter()
            .map(|f| Family {
                id: Box::leak(f.id.into_boxed_str()),
                label: Box::leak(f.label.into_boxed_str()),
                light: Box::leak(Box::new(f.light)),
                dark: Box::leak(Box::new(f.dark)),
                custom: true,
            })
            .collect();
        ThemeSet { custom }
    }

    /// Every family, palettes first.
    pub fn families(&self) -> Vec<Family> {
        PALETTES
            .iter()
            .map(|p| Family { id: p.name, label: label_of(p.name), light: &p.light, dark: &p.dark, custom: false })
            .chain(self.custom.iter().copied())
            .collect()
    }

    pub fn find(&self, id: &str) -> Option<Family> {
        self.families().into_iter().find(|f| f.id == id)
    }

    /// Every name `theme =` takes, in the order Settings cycles them.
    pub fn ids(&self) -> Vec<&'static str> {
        self.families().iter().map(|f| f.id).chain(["mono"]).collect()
    }

    /// The theme after `id`, round again.
    pub fn next(&self, id: &str) -> &'static str {
        let ids = self.ids();
        let i = ids.iter().position(|x| *x == id).map(|i| i + 1).unwrap_or(0);
        ids[i % ids.len()]
    }

    /// `id` in `light` or dark, or mono; `None` for a name lt doesn't know.
    pub fn theme(&self, id: &str, light: bool) -> Option<Theme> {
        if id == "mono" {
            return Some(Theme::MONO);
        }
        let f = self.find(id)?;
        Some(Theme::from_tokens(f.id, f.label, if light { f.light } else { f.dark }))
    }

    /// The names lt knows, for an error message.
    pub fn known(&self) -> String {
        self.ids().join(", ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// What `theme =` names: a palette, a custom theme's file name, or mono.
    pub name: &'static str,
    /// What Settings shows.
    pub label: &'static str,
    /// Drawn for a light background.
    pub light: bool,
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
    /// The tokens this theme was built from (none for mono), for fewer
    /// colours.
    pub tokens: Option<&'static SchemeTokens>,
}

const fn rgb(c: TokenColor) -> Color {
    Color::Rgb(c.rgb.0, c.rgb.1, c.rgb.2)
}

impl Theme {
    /// A theme from a palette's tokens (`theme_tokens.rs`, generated from
    /// `assets/branding/ligus-brand-tokens.json` in the ligus repo, or a
    /// custom theme file), in 24-bit colour. Focus is the accent; hunk
    /// headers are info.
    pub const fn from_tokens(name: &'static str, label: &'static str, k: &'static SchemeTokens) -> Theme {
        Theme {
            name,
            label,
            light: k.light,
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
            agent: rgb(k.agent),
            mono: false,
            ascii: false,
            tokens: Some(k),
        }
    }

    /// Pollen for a dark background. Warn is an amber-orange off the honey
    /// accent, so an open proposal or a warning never reads as the accent
    /// or focus.
    pub const DARK: Theme = Theme::from_tokens("pollen", "Pollen", &tokens::DARK);

    /// Pollen for a light background.
    pub const LIGHT: Theme = Theme::from_tokens("pollen", "Pollen", &tokens::LIGHT);

    pub const MONO: Theme = Theme {
        name: "mono",
        label: "Mono",
        light: false,
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
        tokens: None,
    };

    /// Pick a theme: a named theme wins; none named means Pollen, or mono
    /// under `NO_COLOR`. `auto` asks `probe` (the terminal's own answer,
    /// OSC 11) first, then reads `COLORFGBG`, else dark. An unknown name
    /// is an error naming the ones lt knows.
    pub fn pick(
        want: &Resolved,
        set: &ThemeSet,
        env: impl Fn(&str) -> Option<String>,
        probe: impl FnOnce() -> Option<bool>,
    ) -> Result<Theme, String> {
        let no_color = env("NO_COLOR").is_some_and(|v| !v.is_empty());
        let id = match want.theme.as_deref() {
            Some(id) => id,
            None if no_color => return Ok(Theme::MONO),
            None => PALETTES[0].name,
        };
        if id == "mono" {
            return Ok(Theme::MONO);
        }
        if set.find(id).is_none() {
            return Err(format!("unknown theme “{id}” ({}, or a file in themes/: lt themes list)", set.known()));
        }
        let light = match want.appearance {
            Appearance::Light => true,
            Appearance::Dark => false,
            Appearance::Auto => background_is_light(&env, probe),
        };
        Ok(set.theme(id, light).unwrap_or(Theme::DARK))
    }

    /// The old entry point: `auto`, `dark`, `light` or `mono`, with the
    /// terminal never asked.
    pub fn resolve(choice: &str, env: impl Fn(&str) -> Option<String>) -> Theme {
        let want = ThemeSpec::default().layered(&ThemeSpec { theme: Some(choice.into()), appearance: None });
        Theme::pick(&want, &ThemeSet::builtin(), env, || None).unwrap_or(Theme::DARK)
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

/// Whether the terminal's background is light, for `appearance = auto`:
/// the terminal's own answer (`probe`, OSC 11) first, since it is always
/// current; then `COLORFGBG` ("fg;bg" or "fg;default;bg": 7 and 9-15 are
/// light), which a shell can inherit from another terminal; else dark.
pub fn background_is_light(env: impl Fn(&str) -> Option<String>, probe: impl FnOnce() -> Option<bool>) -> bool {
    if let Some(light) = probe() {
        return light;
    }
    let bg = env("COLORFGBG").and_then(|v| v.rsplit(';').next().and_then(|b| b.trim().parse::<u8>().ok()));
    matches!(bg, Some(7 | 9..=15))
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
        let Some(k) = self.tokens else { return self };
        let pick = |c: TokenColor, fallback: Color| match depth {
            ColorDepth::Ansi256 => Color::Indexed(c.xterm256),
            ColorDepth::Ansi16 => c.ansi16.map(ansi).unwrap_or(fallback),
            ColorDepth::TrueColor => rgb(c),
        };
        let selection = if self.light { Color::Gray } else { Color::DarkGray };
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
            agent: pick(k.agent, Color::Magenta),
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

    fn spec(theme: Option<&str>, appearance: Option<Appearance>) -> ThemeSpec {
        ThemeSpec { theme: theme.map(String::from), appearance }
    }

    fn pick(config: ThemeSpec, flags: ThemeSpec, e: impl Fn(&str) -> Option<String>, probe: Option<bool>) -> Result<Theme, String> {
        Theme::pick(&config.layered(&flags), &ThemeSet::builtin(), e, || probe)
    }

    /// The old values still work: auto, dark and light are Pollen with
    /// that appearance; mono is colourless; NO_COLOR gives mono unless a
    /// theme is named.
    #[test]
    fn old_values_are_aliases() {
        let t = |name: &str, e| Theme::resolve(name, e);
        assert_eq!((t("auto", env(&[])).name, t("auto", env(&[])).light), ("pollen", false));
        assert!(t("auto", env(&[("COLORFGBG", "0;15")])).light);
        assert!(!t("auto", env(&[("COLORFGBG", "15;0")])).light);
        assert_eq!(t("auto", env(&[("NO_COLOR", "1")])).name, "mono");
        assert_eq!(t("dark", env(&[])), Theme::DARK);
        assert_eq!(t("light", env(&[])), Theme::LIGHT);
        assert_eq!(t("light", env(&[("NO_COLOR", "1")])), Theme::LIGHT, "a named theme wins over NO_COLOR");
        assert_eq!(t("mono", env(&[])), Theme::MONO);
        assert_eq!(t("auto", env(&[("NO_COLOR", "")])).name, "pollen", "an empty NO_COLOR is unset");
    }

    #[test]
    fn palettes_and_appearance() {
        let light = Some(Appearance::Light);
        let h = pick(spec(Some("harbor"), light), ThemeSpec::default(), env(&[]), None).unwrap();
        assert_eq!((h.name, h.label, h.light), ("harbor", "Harbor", true));
        assert_eq!(h.text, rgb(tokens::PALETTES[2].light.text));
        // A named palette ignores NO_COLOR; an unnamed one doesn't.
        assert_eq!(pick(spec(Some("meadow"), None), ThemeSpec::default(), env(&[("NO_COLOR", "1")]), None).unwrap().name, "meadow");
        assert_eq!(pick(spec(None, light), ThemeSpec::default(), env(&[("NO_COLOR", "1")]), None).unwrap().name, "mono");
        // The flags win over the config file; an old alias in the flags
        // sets the appearance there too.
        let t = pick(spec(Some("slate"), light), spec(Some("dark"), None), env(&[]), None).unwrap();
        assert_eq!((t.name, t.light), ("pollen", false));
        let t = pick(spec(Some("slate"), None), spec(None, light), env(&[]), None).unwrap();
        assert_eq!((t.name, t.light), ("slate", true));
        // In one layer, an explicit appearance beats the alias's.
        let t = pick(spec(Some("dark"), light), ThemeSpec::default(), env(&[]), None).unwrap();
        assert_eq!((t.name, t.light), ("pollen", true));
        let e = pick(spec(Some("heather"), None), ThemeSpec::default(), env(&[]), None).unwrap_err();
        assert!(e.contains("heather") && e.contains("obsidian") && e.contains("lt themes list"), "{e}");
    }

    /// The terminal's own answer (OSC 11) decides `auto` first: COLORFGBG
    /// can be inherited from another terminal (over ssh, in tmux). It is
    /// never asked when the appearance is set, or for mono.
    #[test]
    fn auto_asks_the_terminal_first() {
        let auto = |e, probe| pick(spec(None, None), ThemeSpec::default(), e, probe).unwrap().light;
        assert!(auto(env(&[]), Some(true)));
        assert!(!auto(env(&[("COLORFGBG", "0;15")]), Some(false)), "the terminal's answer beats a stale COLORFGBG");
        assert!(auto(env(&[("COLORFGBG", "0;15")]), None));
        assert!(auto(env(&[("COLORFGBG", "0;default;15")]), None));
        assert!(auto(env(&[("COLORFGBG", "11;15")]), None), "Solarized Light in iTerm2");
        assert!(!auto(env(&[("COLORFGBG", "12;8")]), None), "Solarized Dark in iTerm2");
        assert!(!auto(env(&[("COLORFGBG", "15;default")]), None));
        assert!(!auto(env(&[]), None));
        let never = || -> Option<bool> { panic!("asked the terminal") };
        let set = ThemeSet::builtin();
        for want in [spec(Some("dark"), None), spec(None, Some(Appearance::Light)), spec(Some("mono"), None)] {
            Theme::pick(&want.layered(&ThemeSpec::default()), &set, env(&[]), never).unwrap();
        }
        Theme::pick(&spec(None, None).layered(&ThemeSpec::default()), &set, env(&[("NO_COLOR", "1")]), never).unwrap();
    }

    #[test]
    fn cycles_every_theme() {
        let set = ThemeSet::builtin();
        assert_eq!(set.ids(), vec!["pollen", "studio", "harbor", "slate", "meadow", "obsidian", "mono"]);
        assert_eq!(set.next("pollen"), "studio");
        assert_eq!(set.next("mono"), "pollen");
        assert_eq!(set.next("gone"), "pollen");
        assert_eq!(Appearance::Auto.next().next().next(), Appearance::Auto);
        let custom = crate::theme_file::parse("heather", "[dark]\nink = \"#ffffff\"\n", false).theme.unwrap();
        let set = ThemeSet::with_custom(vec![custom]);
        assert_eq!(set.next("obsidian"), "heather");
        assert_eq!(set.next("heather"), "mono");
        let t = set.theme("heather", false).unwrap();
        assert_eq!((t.label, t.text), ("heather", Color::Rgb(255, 255, 255)));
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
        for t in all_themes() {
            assert_ne!(t.warn, t.accent, "{}", t.name);
            assert_ne!(t.warn, t.focus, "{}", t.name);
            let (_, warn) = oklch(t.warn);
            let (_, accent) = oklch(t.accent);
            let Color::Rgb(r, g, b) = t.accent else { unreachable!() };
            let (_, a, bb) = crate::theme_file::oklab((r, g, b));
            if a.hypot(bb) < 0.04 {
                // A grey accent (Studio, Obsidian) has no hue to clash.
                continue;
            }
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

    fn all_themes() -> Vec<Theme> {
        let set = ThemeSet::builtin();
        set.ids().iter().filter(|i| **i != "mono").flat_map(|i| [set.theme(i, true).unwrap(), set.theme(i, false).unwrap()]).collect()
    }

    /// The themes are the shared tokens, colour for colour: what the
    /// desktop and the phone draw with (the parity test).
    #[test]
    fn themes_are_the_shared_tokens() {
        for t in all_themes() {
            let k = t.tokens.unwrap();
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
