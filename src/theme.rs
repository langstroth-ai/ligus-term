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
    /// Amber on the terminal's dark background.
    pub const DARK: Theme = Theme {
        name: "dark",
        text: Color::Rgb(0xe6, 0xe1, 0xd6),
        dim: Color::Rgb(0x8f, 0x89, 0x7c),
        accent: Color::Rgb(0xf2, 0xb1, 0x34),
        border: Color::Rgb(0x4a, 0x45, 0x3c),
        focus: Color::Rgb(0xf2, 0xb1, 0x34),
        selected_bg: Color::Rgb(0x3a, 0x32, 0x22),
        good: Color::Rgb(0x8c, 0xc8, 0x6b),
        warn: Color::Rgb(0xf2, 0xb1, 0x34),
        bad: Color::Rgb(0xef, 0x6b, 0x5b),
        added: Color::Rgb(0x8c, 0xc8, 0x6b),
        removed: Color::Rgb(0xef, 0x6b, 0x5b),
        hunk: Color::Rgb(0x7f, 0xb4, 0xe0),
        mention: Color::Rgb(0xf7, 0xc8, 0x5f),
        agent: Color::Rgb(0xb7, 0x9c, 0xe8),
        mono: false,
    };

    /// Darker inks for light backgrounds.
    pub const LIGHT: Theme = Theme {
        name: "light",
        text: Color::Rgb(0x2a, 0x25, 0x1d),
        dim: Color::Rgb(0x6d, 0x66, 0x5a),
        accent: Color::Rgb(0x9a, 0x5b, 0x00),
        border: Color::Rgb(0xb9, 0xb0, 0x9f),
        focus: Color::Rgb(0x9a, 0x5b, 0x00),
        selected_bg: Color::Rgb(0xf3, 0xe3, 0xbf),
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
