//! `lt` documents itself: every flag, plain command, record, key and
//! environment variable it reads appears in its help, so a person or an
//! agent never needs the docs site.

use std::path::{Path, PathBuf};
use std::process::Command;

use ligus_term::help::{COMMANDS, ENV, RECORDS, TOPICS, keys_text, record_json};
use ligus_term::keymap::{BINDINGS, Context};
use ligus_term::{cli, plain};

/// The binary with a clean env and a throwaway HOME, never a real device.
fn lt(args: &[&str]) -> (i32, String, String, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_lt"))
        .env_clear()
        .env("HOME", home.path())
        .env("LT_DATA_DIR", home.path().join("lt"))
        .env("LIGUS_SECRET_BACKEND", "file")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into(), home)
}

#[test]
fn every_flag_is_in_the_long_help_and_has_its_own_page() {
    let (code, help, _, _h) = lt(&["--help"]);
    assert_eq!(code, 0);
    let flags = cli::flags();
    assert!(flags.len() >= 10, "{flags:?}");
    for (long, _, text) in &flags {
        assert!(help.contains(&format!("--{long}")), "--{long} missing from lt --help");
        assert!(!text.trim().is_empty(), "--{long} has no help");
        let (code, page, err, _h) = lt(&["help", &format!("--{long}")]);
        assert_eq!(code, 0, "lt help --{long}: {err}");
        assert!(page.starts_with(&format!("--{long}")), "{page}");
    }
}

#[test]
fn every_plain_command_is_documented() {
    let (_, long, _, _h) = lt(&["--help"]);
    for name in plain::COMMANDS {
        let doc = COMMANDS.iter().find(|c| c.name == *name).unwrap_or_else(|| panic!("plain command {name} has no help::COMMANDS entry"));
        assert!(doc.usage.starts_with(name), "{name}: usage {:?}", doc.usage);
        assert!(!doc.about.is_empty() && !doc.example.is_empty(), "{name}");
        assert!(plain::HELP.contains(name), "{name} missing from lt --plain help");
        assert!(long.contains(name), "{name} missing from lt --help");
        let (code, page, err, _h) = lt(&["help", name]);
        assert_eq!(code, 0, "lt help {name}: {err}");
        assert!(page.contains(doc.usage), "lt help {name}:\n{page}");
        for kind in doc.prints.split([',', ' ']).filter(|k| k.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !k.is_empty()) {
            if kind != "or" && kind != "-" {
                assert!(RECORDS.iter().any(|(k, _)| *k == kind), "{name} prints {kind}, which RECORDS doesn't name");
            }
        }
    }
    // No help for a command plain mode doesn't have (wait and quit are
    // interactive-only, handled before the command table).
    for c in COMMANDS {
        assert!(plain::COMMANDS.contains(&c.name) || ["wait", "quit"].contains(&c.name), "help documents unknown command {}", c.name);
    }
}

#[test]
fn every_topic_works_and_unknown_ones_exit_2() {
    for (t, _) in TOPICS {
        let (code, out, err, _h) = lt(&["help", t]);
        assert_eq!(code, 0, "lt help {t}: {err}");
        assert!(out.lines().count() > 5, "lt help {t}: {out}");
    }
    let (code, _, err, _h) = lt(&["help", "frobnicate"]);
    assert_eq!(code, 2);
    assert!(err.contains("quickstart") && err.contains("troubleshooting"), "{err}");
    // `lt help` is `lt --help`, and needs no device.
    let (code, out, _, home) = lt(&["help"]);
    assert_eq!(code, 0);
    assert_eq!(out, lt(&["--help"]).1);
    assert!(!home.path().join("lt").exists(), "lt help opened a data dir");
    // The plain-mode form prints the same text as help records.
    let (code, out, _, _h) = lt(&["--demo", "--plain", "help", "run"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("help\trun W [INPUT]\n"), "{out}");
}

/// The key table is built from keymap.rs's binding table: every binding is
/// one row with its keys, config name and description, and nothing else is.
#[test]
fn the_keys_topic_is_generated_from_the_keymap() {
    let text = keys_text();
    let lines: Vec<&str> = text.lines().collect();
    for ctx in Context::ALL {
        let header = format!("{} [keys.{}]", ctx.title(), ctx.name());
        let at = lines.iter().position(|l| *l == header).unwrap_or_else(|| panic!("no section {header}"));
        let shown: Vec<&str> = lines[at + 1..].iter().take_while(|l| !l.is_empty()).copied().collect();
        let want: Vec<String> = BINDINGS
            .iter()
            .filter(|b| b.context == ctx)
            .map(|b| format!("  {:<24} {:<20} {}", b.keys.join(" / "), b.name, b.help))
            .collect();
        assert_eq!(shown, want, "section {header}");
    }
    assert_eq!(lt(&["help", "keys"]).1, text);
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Names in `CALL("NAME"` for each env-reading call in the source.
fn env_reads(src: &str) -> Vec<String> {
    let mut names = vec![];
    for call in ["env(\"", "var(\"", "var_os(\"", "env_on(\""] {
        for (i, _) in src.match_indices(call) {
            let rest = &src[i + call.len()..];
            let name: String = rest.chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').collect();
            if !name.is_empty() && rest[name.len()..].starts_with('"') {
                names.push(name);
            }
        }
    }
    names
}

/// Every environment variable the source reads is in lt --help. (The
/// detection lists are checked in `help.rs`'s unit tests; the Ligus
/// crates' variables lt depends on are listed by hand in `help::ENV`.)
#[test]
fn every_env_var_read_is_documented() {
    let (_, long, _, _h) = lt(&["--help"]);
    let mut files = vec![];
    rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut seen = std::collections::BTreeSet::new();
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        // Only the code: a test module may set variables it never reads.
        let code = src.split("#[cfg(test)]").next().unwrap();
        for name in env_reads(code) {
            assert!(ENV.iter().any(|v| v.name == name), "{} reads {name}, which help::ENV doesn't document", f.display());
            seen.insert(name);
        }
    }
    for must in ["LT_DATA_DIR", "LT_CONFIG", "LT_RELAY_URL", "LT_AGENT_TOOLS", "LT_RUN_COMMAND", "NO_COLOR"] {
        assert!(seen.contains(must), "the scan missed {must}: {seen:?}");
    }
    for v in ENV {
        assert!(long.contains(v.name), "{} missing from lt --help", v.name);
    }
}

/// Every record keyword plain mode prints has named fields for --json.
#[test]
fn every_record_has_json_field_names() {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/plain.rs")).unwrap();
    let code = src.split("#[cfg(test)]").next().unwrap();
    let mut kinds = std::collections::BTreeSet::new();
    for (i, _) in code.match_indices("\\t") {
        let before = &code[..i];
        let start = before.rfind('"').map(|s| s + 1).unwrap_or(i);
        let word = &code[start..i];
        if !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
            kinds.insert(word.to_string());
        }
    }
    assert!(kinds.len() > 30, "{kinds:?}");
    for k in &kinds {
        assert!(RECORDS.iter().any(|(r, _)| r == k), "plain mode prints {k:?} records, which help::RECORDS doesn't name");
    }
}

#[test]
fn json_output_from_the_binary() {
    for args in [
        &["--demo", "--plain", "--json", "workspaces"][..],
        &["--demo", "--plain", "workspaces", "--json"],
        &["--demo", "--json", "workspaces"],
    ] {
        let (code, out, err, _h) = lt(args);
        assert_eq!(code, 0, "{args:?}: {err}");
        let first = out.lines().next().unwrap_or_default();
        assert!(
            first.starts_with(r#"{"record":"workspace","n":1,"id":"ws-apiary","name":"Apiary Tracker","role":"owner""#),
            "{args:?}: {out}"
        );
        for l in out.lines() {
            assert!(l.starts_with("{\"record\":") && l.ends_with('}'), "{l}");
        }
    }
    let (code, out, _, _h) = lt(&["--demo", "--plain", "--json", "frobnicate"]);
    assert_eq!(code, 2);
    assert!(out.starts_with(r#"{"record":"error","message":"unknown command frobnicate"#), "{out}");
    assert_eq!(record_json("setting\tname\tAda"), r#"{"record":"setting","key":"name","value":"Ada"}"#);
}
