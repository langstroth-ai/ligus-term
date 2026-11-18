//! `config.toml`: the theme and key overrides. Every field is optional; a
//! missing file means the defaults.
//!
//! ```toml
//! theme = "auto"        # auto | dark | light | mono
//! [keys.global]
//! help = ["?", "f1"]
//! ```

use std::path::Path;

use serde::Deserialize;

use crate::keymap::{Keymap, Overrides};
use crate::theme::ThemeChoice;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub theme: Option<ThemeChoice>,
    #[serde(default)]
    pub keys: Overrides,
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Read the file; a missing file is the defaults.
    pub fn load(path: Option<&Path>) -> Result<Config, String> {
        let Some(path) = path else { return Ok(Config::default()) };
        match std::fs::read_to_string(path) {
            Ok(t) => Config::parse(&t).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn keymap(&self) -> Result<Keymap, String> {
        Keymap::with_overrides(&self.keys)
    }
}

/// A commented config with every default, for `lt --print-config`.
pub fn default_config_text() -> String {
    use crate::keymap::{BINDINGS, Context};
    let mut out = String::from(
        "# lt config. Every setting is optional; delete what you don't change.\n\
         # theme: auto (from COLORFGBG), dark, light, or mono (also set by NO_COLOR)\n\
         theme = \"auto\"\n",
    );
    for ctx in Context::ALL {
        out.push_str(&format!("\n[keys.{}]\n", ctx.name()));
        for b in BINDINGS.iter().filter(|b| b.context == ctx) {
            let keys: Vec<String> = b.keys.iter().map(|k| format!("{k:?}")).collect();
            out.push_str(&format!("{} = [{}]  # {}\n", b.name, keys.join(", "), b.help));
        }
    }
    out
}
