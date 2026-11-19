//! Where `lt` keeps its device and its config.
//!
//! `lt` is its own Ligus device, so it needs its own data dir: its identity,
//! database and secret-store keychain live there. It follows the `ligus`
//! CLI's convention (an env var, else a dot-dir in `$HOME`, else a local
//! dir) under its own names, so it never shares a device with the desktop
//! app or with a `ligus` CLI on the same machine:
//!
//! | | `ligus` CLI | `lt` |
//! |---|---|---|
//! | flag | — | `--data-dir` |
//! | env | `LIGUS_DATA_DIR` | `LT_DATA_DIR` |
//! | default | `$HOME/.ligus` | `$HOME/.ligus-term` |
//! | no `$HOME` | `./ligus-data` | `./ligus-term-data` |
//!
//! `LIGUS_DATA_DIR` is deliberately not read: a shell that exports it for
//! the CLI would otherwise make `lt` the same device as the CLI. A data dir
//! that overlaps the desktop app's (`<app data>/ai.ligus.desktop`) or the
//! CLI's (`$LIGUS_DATA_DIR`, `$HOME/.ligus`) is refused outright: two
//! programs on one device identity would sign from the same per-device
//! chain and equivocate.

use std::path::{Component, Path, PathBuf};

/// The desktop app's bundle id, which names its data dir.
pub const DESKTOP_APP_ID: &str = "ai.ligus.desktop";
/// `lt`'s default data dir under `$HOME`.
pub const HOME_DATA_DIR: &str = ".ligus-term";
/// The CLI's default under `$HOME` (only to warn about sharing it).
pub const CLI_HOME_DATA_DIR: &str = ".ligus";
/// The config file under the platform config dir.
pub const CONFIG_DIR_NAME: &str = "ligus-term";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    pub path: PathBuf,
    /// Where it came from ("--data-dir", "LT_DATA_DIR", "default").
    pub source: &'static str,
    /// A warning to show (none today: an overlap with another program's
    /// data dir is refused instead).
    pub warning: Option<String>,
}

/// Resolve the data dir. `env` looks up an environment variable.
pub fn data_dir(flag: Option<&Path>, env: impl Fn(&str) -> Option<String>) -> Result<DataDir, String> {
    let home = env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    let (path, source) = if let Some(p) = flag {
        (p.to_path_buf(), "--data-dir")
    } else if let Some(p) = env("LT_DATA_DIR").filter(|p| !p.is_empty()) {
        (PathBuf::from(p), "LT_DATA_DIR")
    } else if let Some(h) = &home {
        (h.join(HOME_DATA_DIR), "default")
    } else {
        (PathBuf::from("./ligus-term-data"), "default")
    };
    let norm = normalize(&path);
    if let Some(h) = &home {
        for base in ["Library/Application Support", ".local/share", ".config"] {
            let desktop = normalize(&h.join(base).join(DESKTOP_APP_ID));
            if norm.starts_with(&desktop) || desktop.starts_with(&norm) {
                return Err(format!(
                    "{} overlaps the desktop app's data at {}: lt is its own device, so give it its own data dir (--data-dir or LT_DATA_DIR)",
                    path.display(),
                    desktop.display()
                ));
            }
        }
    }
    let mut cli_dirs: Vec<PathBuf> = env("LIGUS_DATA_DIR").filter(|p| !p.is_empty()).map(PathBuf::from).into_iter().collect();
    if let Some(h) = &home {
        cli_dirs.push(h.join(CLI_HOME_DATA_DIR));
    }
    if let Some(d) = cli_dirs.iter().map(|d| normalize(d)).find(|d| norm.starts_with(d) || d.starts_with(&norm)) {
        return Err(format!(
            "{} overlaps the ligus CLI's data at {}: lt is its own device, so give it its own data dir (--data-dir or LT_DATA_DIR)",
            path.display(),
            d.display()
        ));
    }
    Ok(DataDir { path, source, warning: None })
}

/// The config file: `--config`, else `LT_CONFIG`, else
/// `<platform config dir>/ligus-term/config.toml`.
pub fn config_file(flag: Option<&Path>, env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(p) = flag {
        return Some(p.to_path_buf());
    }
    if let Some(p) = env("LT_CONFIG").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    dirs::config_dir().map(|d| d.join(CONFIG_DIR_NAME).join("config.toml"))
}

/// Lexical normalization (no filesystem access): drops `.`, folds `..`.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn defaults_to_its_own_dot_dir() {
        let d = data_dir(None, env(&[("HOME", "/home/a")])).unwrap();
        assert_eq!(d.path, PathBuf::from("/home/a/.ligus-term"));
        assert_eq!(d.source, "default");
        assert!(d.warning.is_none());
    }

    #[test]
    fn ignores_the_cli_env_var() {
        let d = data_dir(None, env(&[("HOME", "/home/a"), ("LIGUS_DATA_DIR", "/srv/ligus")])).unwrap();
        assert_eq!(d.path, PathBuf::from("/home/a/.ligus-term"));
    }

    #[test]
    fn flag_beats_env_beats_default() {
        let e = env(&[("HOME", "/home/a"), ("LT_DATA_DIR", "/x/lt")]);
        assert_eq!(data_dir(None, &e).unwrap().path, PathBuf::from("/x/lt"));
        assert_eq!(data_dir(Some(Path::new("/y")), &e).unwrap().source, "--data-dir");
        assert_eq!(data_dir(None, env(&[])).unwrap().path, PathBuf::from("./ligus-term-data"));
    }

    #[test]
    fn refuses_the_desktop_dir() {
        let e = env(&[("HOME", "/Users/a"), ("LIGUS_DATA_DIR", "/srv/ligus")]);
        let r = data_dir(Some(Path::new("/Users/a/Library/Application Support/ai.ligus.desktop/sub")), &e);
        assert!(r.unwrap_err().contains("desktop app's data"));
        let r = data_dir(Some(Path::new("/Users/a/.local/share")), &e);
        assert!(r.is_err(), "a parent of the desktop dir overlaps it too");
    }

    /// Two programs on one device identity sign from the same per-device
    /// chain and equivocate; lt never shares the CLI's data dir (exact or
    /// nested either way), as it never shares the desktop's.
    #[test]
    fn refuses_the_cli_dir_and_anything_overlapping_it() {
        let e = env(&[("HOME", "/Users/a"), ("LIGUS_DATA_DIR", "/srv/ligus")]);
        for p in ["/Users/a/.ligus", "/Users/a/.ligus/lt", "/srv/./ligus", "/srv/ligus/x", "/srv"] {
            let r = data_dir(Some(Path::new(p)), &e);
            assert!(r.as_ref().is_err_and(|m| m.contains("ligus CLI")), "{p}: {r:?}");
        }
        assert!(data_dir(Some(Path::new("/Users/a/.ligus-term")), &e).is_ok());
    }

    #[test]
    fn config_file_precedence() {
        assert_eq!(config_file(Some(Path::new("/c.toml")), env(&[])), Some(PathBuf::from("/c.toml")));
        assert_eq!(config_file(None, env(&[("LT_CONFIG", "/e.toml")])), Some(PathBuf::from("/e.toml")));
        assert!(config_file(None, env(&[])).unwrap().ends_with("ligus-term/config.toml"));
    }
}
