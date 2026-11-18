//! The `lt` binary end to end: flags, plain mode on a pipe, and refusals.

use std::io::Write;
use std::process::{Command, Stdio};

/// The binary with a clean env and a throwaway HOME (kept alive by the
/// returned guard).
fn lt() -> (Command, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_lt"));
    c.env_clear().env("HOME", home.path()).env("PATH", std::env::var("PATH").unwrap_or_default());
    (c, home)
}

fn run(mut c: Command, stdin: &str) -> (i32, String, String) {
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

#[test]
fn version() {
    let (mut c, _home) = lt();
    c.arg("--version");
    let (code, out, _) = run(c, "");
    assert_eq!(code, 0);
    assert!(out.starts_with(&format!("lt {}", env!("CARGO_PKG_VERSION"))), "{out}");
    assert!(out.contains("ligustica"));
}

#[test]
fn piped_stdout_uses_plain_mode() {
    let (mut c, _home) = lt();
    c.arg("--demo");
    let (code, out, _) = run(c, "workspaces\nmembers\nquit\n");
    assert_eq!(code, 0);
    assert!(out.contains("workspace\t1\tws-apiary\tApiary Tracker\towner"), "{out}");
    assert!(out.contains("member\t2\tm-ada\tAda\tadmin"), "{out}");
    assert!(!out.contains('\u{1b}'), "no escape codes in plain mode");
}

#[test]
fn print_config_is_valid() {
    let (mut c, _home) = lt();
    c.arg("--print-config");
    let (code, out, _) = run(c, "");
    assert_eq!(code, 0);
    assert!(ligus_term::config::Config::parse(&out).is_ok());
}

#[test]
fn refuses_the_desktop_data_dir_and_bad_config() {
    let (mut c, _home) = lt();
    c.args(["--plain", "--data-dir", "/tmp/x/Library/Application Support/ai.ligus.desktop"]).env("HOME", "/tmp/x");
    let (code, _, err) = run(c, "");
    assert_eq!(code, 2);
    assert!(err.contains("desktop app's data"), "{err}");

    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("c.toml");
    std::fs::write(&cfg, "[keys.global]\nhelp = [\"q\"]\n").unwrap();
    let (mut c, _home) = lt();
    c.arg("--plain").arg("--config").arg(&cfg);
    let (code, _, err) = run(c, "");
    assert_eq!(code, 2);
    assert!(err.contains("bound to both"), "{err}");
}

#[test]
fn does_not_create_its_data_dir_yet() {
    let home = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_lt"));
    c.env_clear().env("HOME", home.path()).arg("--plain");
    let (code, _, _) = run(c, "quit\n");
    assert_eq!(code, 0);
    assert!(!home.path().join(".ligus-term").exists(), "the demo backend keeps nothing on disk");
}
