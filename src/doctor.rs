//! `lt doctor`: what this machine and this device's setup look like to lt,
//! as the shared `ligus.checks.v1` report (`--json`) or its text form.
//!
//! It never opens the device: no database, no secret store (so no keychain
//! prompt) and no network. It reads the data dir's workspace list as a
//! plain file, the config file and the environment, and looks for agent
//! runtimes on `PATH` the way the Agents view does (version checks only).
//! The relay is checked where it runs: the report names the command for
//! each relay in `next`.

use std::path::{Path, PathBuf};

use serde_json::json;

/// A check's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Skip => "skip",
        }
    }
}

/// One check of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Stable and dotted: `data_dir.usable`.
    pub id: &'static str,
    pub title: &'static str,
    pub status: Status,
    /// One line of what was found.
    pub detail: String,
    /// One line of how to fix it, ideally a command.
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub checks: Vec<Check>,
    /// Commands to run next, in order.
    pub next: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        !self.checks.iter().any(|c| c.status == Status::Fail)
    }

    /// 0 when nothing failed (warnings allowed), else 1.
    pub fn exit_code(&self) -> i32 {
        if self.ok() { 0 } else { 1 }
    }

    /// The `ligus.checks.v1` document.
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "schema": "ligus.checks.v1",
            "tool": "lt",
            "version": env!("CARGO_PKG_VERSION"),
            "command": "doctor",
            "ok": self.ok(),
            "exitCode": self.exit_code(),
            "checks": self.checks.iter().map(|c| {
                let mut o = json!({"id": c.id, "title": c.title, "status": c.status.word(), "detail": c.detail});
                if let Some(f) = &c.fix {
                    o["fix"] = json!(f);
                }
                o
            }).collect::<Vec<_>>(),
            "next": self.next,
        })
    }

    /// The text form every Ligus doctor prints: a header, one line per
    /// check with an indented `fix:`, then a summary with what to run next.
    pub fn to_text(&self) -> String {
        let mut s = format!("lt doctor (lt {})\n", env!("CARGO_PKG_VERSION"));
        let w = self.checks.iter().map(|c| c.title.len()).max().unwrap_or(0);
        for c in &self.checks {
            s.push_str(&format!("{:<5} {:<w$}  {}\n", c.status.word(), c.title, c.detail));
            if let Some(f) = &c.fix {
                s.push_str(&format!("      fix: {f}\n"));
            }
        }
        let count = |st| self.checks.iter().filter(|c| c.status == st).count();
        let mut parts = vec![];
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        if count(Status::Fail) > 0 {
            parts.push(plural(count(Status::Fail), "failure", "failures"));
        }
        if count(Status::Warn) > 0 {
            parts.push(plural(count(Status::Warn), "warning", "warnings"));
        }
        if parts.is_empty() {
            parts.push("all good".into());
        }
        let next = if self.next.is_empty() { "nothing to do".to_string() } else { self.next.join(" ; ") };
        s.push_str(&format!("{} · next: {next}\n", parts.join(", ")));
        s
    }
}

/// What the doctor looks at (the parts that aren't the environment).
pub struct Inputs<'a> {
    pub data_dir_flag: Option<&'a Path>,
    pub config_flag: Option<&'a Path>,
    pub find_runtimes: bool,
}

/// Run every check.
pub fn run(inputs: Inputs<'_>, env: impl Fn(&str) -> Option<String> + Copy) -> Report {
    let mut r = Report::default();
    let data = crate::paths::data_dir(inputs.data_dir_flag, env);
    match &data {
        Ok(d) => {
            let (status, detail, fix) = data_dir_state(&d.path);
            r.checks.push(Check { id: "data_dir.usable", title: "Data dir", status, detail, fix });
            if let Some(w) = &d.warning {
                r.checks.push(Check { id: "data_dir.warning", title: "Data dir", status: Status::Warn, detail: w.clone(), fix: None });
            }
        }
        Err(e) => r.checks.push(Check {
            id: "data_dir.usable",
            title: "Data dir",
            status: Status::Fail,
            detail: e.clone(),
            fix: Some("give lt a data dir of its own: --data-dir PATH or LT_DATA_DIR".into()),
        }),
    }

    let config_path = crate::paths::config_file(inputs.config_flag, env);
    // Theme files: whether each loads, and whether the theme named exists.
    let reports = crate::theme_file::themes_dir(config_path.as_deref()).map(|d| crate::theme_file::read_dir(&d)).unwrap_or_default();
    if !reports.is_empty() || crate::config::Config::load(config_path.as_deref()).is_ok_and(|c| c.theme.is_some()) {
        let broken: Vec<&str> = reports.iter().filter(|r| !r.ok()).map(|r| r.id.as_str()).collect();
        let set = crate::theme::ThemeSet::with_custom(reports.iter().filter_map(|r| r.theme.clone()).collect());
        let named =
            crate::config::Config::load(config_path.as_deref()).ok().and_then(|c| c.theme_spec().layered(&Default::default()).theme);
        let unknown = named.as_deref().filter(|n| *n != "mono" && set.find(n).is_none());
        let ok = reports.len() - broken.len();
        r.checks.push(Check {
            id: "themes.valid",
            title: "Themes",
            status: if broken.is_empty() && unknown.is_none() { Status::Ok } else { Status::Warn },
            detail: match unknown {
                Some(n) => format!("theme = \"{n}\" isn't a palette or a theme file that loads; lt uses Pollen"),
                None if broken.is_empty() => format!("{ok} theme file{} load", if ok == 1 { "" } else { "s" }),
                None => format!("not loaded: {}", broken.join(", ")),
            },
            fix: (!broken.is_empty() || unknown.is_some())
                .then(|| "lt themes list; lt themes check NAME shows each error and its line".into()),
        });
    }
    match crate::config::Config::load(config_path.as_deref()) {
        Ok(c) => match c.keymap() {
            Ok(_) => r.checks.push(Check {
                id: "config.valid",
                title: "Config",
                status: Status::Ok,
                detail: match &config_path {
                    Some(p) if p.exists() => p.display().to_string(),
                    _ => "no config file: the defaults".into(),
                },
                fix: None,
            }),
            Err(e) => r.checks.push(Check {
                id: "config.valid",
                title: "Config",
                status: Status::Fail,
                detail: e,
                fix: Some("lt --print-config prints every key and its default".into()),
            }),
        },
        Err(e) => r.checks.push(Check {
            id: "config.valid",
            title: "Config",
            status: Status::Fail,
            detail: e,
            fix: Some("lt --print-config prints a valid file with every default".into()),
        }),
    }

    let store = env("LIGUS_SECRET_BACKEND").unwrap_or_default();
    r.checks.push(Check {
        id: "secret_store.kind",
        title: "Secret store",
        status: Status::Ok,
        detail: if store.trim() == "file" {
            "owner-only files (LIGUS_SECRET_BACKEND=file)".into()
        } else if cfg!(target_os = "macos") {
            "the macOS keychain".into()
        } else if cfg!(target_os = "linux") {
            "the Secret Service (GNOME Keyring, KWallet)".into()
        } else {
            "the OS secret store".into()
        },
        fix: None,
    });

    let relay_env = env("LT_RELAY_URL").map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    let workspaces = data.as_ref().map(|d| saved_workspaces(&d.path)).unwrap_or_default();
    let mut relays: Vec<String> = workspaces.iter().filter_map(|(_, relay)| relay.clone()).collect();
    if let Some(u) = &relay_env {
        let usable = u.starts_with("https://") || u.starts_with("http://");
        r.checks.push(Check {
            id: "relay.url",
            title: "Relay URL",
            status: if usable { Status::Ok } else { Status::Fail },
            detail: format!("LT_RELAY_URL={u}"),
            fix: (!usable).then(|| "LT_RELAY_URL takes an http:// or https:// URL".into()),
        });
        if usable {
            relays.push(u.clone());
        }
    }
    let local_only = workspaces.iter().filter(|(_, relay)| relay.is_none()).count();
    r.checks.push(Check {
        id: "workspaces.list",
        title: "Workspaces",
        status: Status::Ok,
        detail: match (workspaces.len(), local_only) {
            (0, _) => "none yet: lt creates or joins one on its first run".into(),
            (n, 0) => format!("{n}, each through a relay"),
            (n, l) if l == n => format!("{n} on this device only (invites need a relay)"),
            (n, l) => format!("{n}, {l} on this device only"),
        },
        fix: None,
    });
    relays.sort();
    relays.dedup();
    if relays.is_empty() {
        r.checks.push(Check {
            id: "relay.reachable",
            title: "Relay",
            status: Status::Skip,
            detail: "no relay configured".into(),
            fix: None,
        });
    } else {
        r.checks.push(Check {
            id: "relay.reachable",
            title: "Relay",
            status: Status::Skip,
            detail: format!("{}: lt doctor makes no network calls; check it on the relay host", relays.join(", ")),
            fix: None,
        });
        for u in &relays {
            r.next.push(format!("langstroth-relay check --url {u}"));
        }
    }

    if inputs.find_runtimes {
        let denv = crate::backend::real::detect::process_env();
        let found: Vec<_> =
            crate::backend::real::detect::clis(&denv).into_iter().chain(crate::backend::real::detect::api_keys(&denv)).collect();
        let usable: Vec<String> = found
            .iter()
            .filter(|f| f.unsupported.is_none())
            .map(|f| format!("{}{}", f.suggested, if f.ready { "" } else { " (not signed in)" }))
            .collect();
        r.checks.push(if usable.is_empty() {
            Check {
                id: "runtimes.found",
                title: "Agent runtimes",
                status: Status::Warn,
                detail: "none on PATH and no API key set".into(),
                fix: Some("install Claude Code or Codex, run Ollama, or set ANTHROPIC_API_KEY; lt help agents".into()),
            }
        } else {
            Check { id: "runtimes.found", title: "Agent runtimes", status: Status::Ok, detail: usable.join(", "), fix: None }
        });
    }

    let term = env("TERM").unwrap_or_default();
    let depth = crate::theme::ColorDepth::detect(env);
    let mono = env("NO_COLOR").is_some_and(|v| !v.is_empty());
    r.checks.push(Check {
        id: "terminal.capabilities",
        title: "Terminal",
        status: if term == "dumb" { Status::Warn } else { Status::Ok },
        detail: format!(
            "TERM={}{}",
            if term.is_empty() { "unset" } else { &term },
            if mono { ", NO_COLOR: mono".to_string() } else { format!(", {}", depth.label()) }
        ),
        fix: (term == "dumb").then(|| "the full-screen app needs a real terminal; lt --plain works anywhere".into()),
    });

    // The next doctor down: the ligus CLI, when it's here.
    if env("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("ligus").is_file())) {
        r.next.push("ligus doctor".into());
    }
    r
}

/// The data dir's state: there and owner-only, there and readable by
/// others, or not made yet.
fn data_dir_state(path: &Path) -> (Status, String, Option<String>) {
    let shown = path.display().to_string();
    let Ok(meta) = std::fs::metadata(path) else {
        return (Status::Ok, format!("{shown} (made on the first run)"), None);
    };
    if !meta.is_dir() {
        return (Status::Fail, format!("{shown} isn't a directory"), Some("give lt a directory: --data-dir PATH".into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return (Status::Warn, format!("{shown} is readable by others"), Some(format!("chmod 700 {shown}")));
        }
    }
    (Status::Ok, format!("{shown} (owner-only)"), None)
}

/// The data dir's workspaces: name and relay, from the plain list file.
fn saved_workspaces(dir: &Path) -> Vec<(String, Option<String>)> {
    #[derive(serde::Deserialize)]
    struct List {
        #[serde(default)]
        workspaces: Vec<Saved>,
    }
    #[derive(serde::Deserialize)]
    struct Saved {
        #[serde(default)]
        name: String,
        #[serde(default)]
        relay_url: String,
    }
    let file: PathBuf = dir.join("workspaces.json");
    let Ok(bytes) = std::fs::read(file) else { return vec![] };
    let Ok(list) = serde_json::from_slice::<List>(&bytes) else { return vec![] };
    list.workspaces
        .into_iter()
        .map(|w| {
            let r = w.relay_url.trim().to_string();
            let usable = r.starts_with("https://") || r.starts_with("http://");
            (w.name, usable.then_some(r))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> + Copy {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn report_shape_and_text() {
        let r = Report {
            checks: vec![
                Check { id: "a.ok", title: "A", status: Status::Ok, detail: "fine".into(), fix: None },
                Check { id: "b.warn", title: "Bee", status: Status::Warn, detail: "hmm".into(), fix: Some("do x".into()) },
            ],
            next: vec!["langstroth-relay check --url https://r".into()],
        };
        let j = r.to_json();
        assert_eq!(j["schema"], "ligus.checks.v1");
        assert_eq!(j["tool"], "lt");
        assert_eq!(j["command"], "doctor");
        assert_eq!(j["ok"], true);
        assert_eq!(j["exitCode"], 0);
        assert_eq!(j["checks"][1]["status"], "warn");
        assert_eq!(j["checks"][1]["fix"], "do x");
        assert!(j["checks"][0].get("fix").is_none());
        let t = r.to_text();
        assert!(t.contains("ok    A    fine\nwarn  Bee  hmm\n      fix: do x\n"), "{t}");
        assert!(t.ends_with("1 warning · next: langstroth-relay check --url https://r\n"), "{t}");
        let mut bad = r.clone();
        bad.checks[0].status = Status::Fail;
        assert_eq!(bad.exit_code(), 1);
        assert_eq!(bad.to_json()["ok"], false);
    }

    #[test]
    fn checks_a_data_dir_and_its_relays_without_opening_anything() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("dev");
        std::fs::create_dir(&data).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(
            data.join("workspaces.json"),
            r#"{"workspaces":[{"name":"Northwind","room":"r","relay_url":"https://relay.example.com"},{"name":"Solo","room":"s"}]}"#,
        )
        .unwrap();
        // The env is Copy, so it borrows a 'static copy of the home path.
        let home_static: &'static str = Box::leak(dir.path().to_string_lossy().to_string().into_boxed_str());
        let env = move |k: &str| match k {
            "HOME" => Some(home_static.to_string()),
            "TERM" => Some("xterm-256color".into()),
            _ => None,
        };
        let r = run(Inputs { data_dir_flag: Some(&data), config_flag: Some(&dir.path().join("none.toml")), find_runtimes: false }, env);
        let by = |id: &str| r.checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id} in {r:?}"));
        assert_eq!(by("data_dir.usable").status, Status::Ok, "{r:?}");
        assert_eq!(by("config.valid").status, Status::Ok);
        assert!(by("workspaces.list").detail.contains("2, 1 on this device only"), "{r:?}");
        assert_eq!(by("relay.reachable").status, Status::Skip);
        assert_eq!(r.next, vec!["langstroth-relay check --url https://relay.example.com".to_string()]);
        assert_eq!(std::fs::read_dir(&data).unwrap().count(), 1, "nothing written");
        assert!(r.ok());
        // A bad config fails, with the fix.
        let cfg = dir.path().join("bad.toml");
        std::fs::write(&cfg, "theme = 3").unwrap();
        let r = run(Inputs { data_dir_flag: Some(&data), config_flag: Some(&cfg), find_runtimes: false }, env);
        assert_eq!(r.exit_code(), 1);
        assert!(r.checks.iter().any(|c| c.id == "config.valid" && c.status == Status::Fail && c.fix.is_some()));
        // A theme that doesn't load is a warning naming it.
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "theme = \"heather\"").unwrap();
        std::fs::create_dir(dir.path().join("themes")).unwrap();
        std::fs::write(dir.path().join("themes/heather.toml"), "[light]\nink = \"pink\"\n").unwrap();
        let r = run(Inputs { data_dir_flag: Some(&data), config_flag: Some(&cfg), find_runtimes: false }, env);
        let t = r.checks.iter().find(|c| c.id == "themes.valid").unwrap();
        assert_eq!(t.status, Status::Warn, "{t:?}");
        assert!(t.detail.contains("heather") && t.fix.as_deref().unwrap().contains("lt themes check"), "{t:?}");
        std::fs::write(dir.path().join("themes/heather.toml"), "[light]\nink = \"#222\"\n").unwrap();
        let r = run(Inputs { data_dir_flag: Some(&data), config_flag: Some(&cfg), find_runtimes: false }, env);
        assert_eq!(r.checks.iter().find(|c| c.id == "themes.valid").unwrap().status, Status::Ok);
        // A bad LT_RELAY_URL fails too.
        let r = run(
            Inputs { data_dir_flag: Some(&data), config_flag: None, find_runtimes: false },
            env_of(&[("LT_RELAY_URL", "relay.example")]),
        );
        assert!(r.checks.iter().any(|c| c.id == "relay.url" && c.status == Status::Fail));
    }
}
