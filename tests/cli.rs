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
    assert!(!out.contains(['▀', '▄', '█']), "no splash art in plain mode: {out}");
}

/// Plain mode on a first run (no workspace yet, where the TUI shows the
/// mark) prints records only.
#[test]
fn plain_first_run_has_no_splash() {
    let home = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_lt"));
    c.env_clear().env("HOME", home.path()).env("LIGUS_SECRET_BACKEND", "file").arg("--plain");
    let (code, out, err) = run(c, "help\nworkspaces\nquit\n");
    assert_eq!(code, 0, "{out}{err}");
    assert!(!out.contains(['▀', '▄', '█']) && !out.contains('\u{1b}'), "{out}");
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
fn the_demo_keeps_nothing_on_disk() {
    let home = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_lt"));
    c.env_clear().env("HOME", home.path()).args(["--plain", "--demo"]);
    let (code, _, _) = run(c, "quit\n");
    assert_eq!(code, 0);
    assert!(!home.path().join(".ligus-term").exists(), "the demo backend keeps nothing on disk");
}

/// Without `--demo`, lt opens its own device dir (owner-only), and makes no
/// identity until the person creates or joins a workspace. File secrets: a
/// test never touches the login keychain.
#[test]
fn first_run_opens_its_own_device_dir() {
    let home = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_lt"));
    c.env_clear().env("HOME", home.path()).env("LIGUS_SECRET_BACKEND", "file").arg("--plain");
    let (code, out, err) = run(c, "workspaces\nquit\n");
    assert_eq!(code, 0, "{out}{err}");
    let dir = home.path().join(".ligus-term");
    assert!(dir.is_dir(), "{err}");
    assert!(!dir.join("identity.json").exists(), "no identity before create or join");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o077, 0, "the data dir is owner-only");
    }
}

/// One-shot plain mode: one command, its records, and an exit code (0 ok,
/// 1 an error, 2 an unknown command); a command needs --plain.
#[test]
fn one_shot_commands_exit_with_a_status() {
    let (mut c, _home) = lt();
    c.args(["--demo", "--plain", "runs"]);
    let (code, out, _) = run(c, "");
    assert_eq!(code, 0);
    assert!(out.starts_with("run\t1\trun-7\tawaiting-gate\t"), "{out}");
    assert!(!out.contains("ready\t"), "one-shot prints only the command's records: {out}");

    let (mut c, _home) = lt();
    c.args(["--demo", "--plain", "--workspace", "queen-rearing", "workflows"]);
    let (code, out, _) = run(c, "");
    assert_eq!((code, out.as_str()), (0, ""));

    let (mut c, _home) = lt();
    c.args(["--demo", "--plain", "--workspace", "nowhere", "runs"]);
    assert_eq!(run(c, "").0, 1);

    let (mut c, _home) = lt();
    c.args(["--demo", "--plain", "gate", "2", "approve"]);
    let (code, out, _) = run(c, "");
    assert_eq!(code, 1, "{out}");

    let (mut c, _home) = lt();
    c.args(["--demo", "--plain", "frobnicate"]);
    assert_eq!(run(c, "").0, 2);

    // A command runs without --plain too; an unknown one is refused before
    // any data dir is opened.
    let (mut c, _home) = lt();
    c.args(["--demo", "runs"]);
    let (code, out, _) = run(c, "");
    assert_eq!(code, 0);
    assert!(out.starts_with("run\t1\t"), "{out}");
    let (mut c, home) = lt();
    c.args(["frobnicate"]);
    let (code, _, err) = run(c, "");
    assert_eq!(code, 2);
    assert!(err.contains("unknown command frobnicate") && err.contains("lt help"), "{err}");
    assert!(!home.path().join(".ligus-term").exists(), "nothing opened");

    // --json numbers are numbers.
    let (mut c, _home) = lt();
    c.args(["--demo", "runs", "--json"]);
    let (code, out, _) = run(c, "");
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(v["n"], serde_json::json!(1), "{out}");
}

/// A one-shot create on a new device takes its name from --name.
#[test]
fn one_shot_create_takes_a_name() {
    let (mut c, home) = lt();
    let data = home.path().join("lt");
    c.env("LT_DATA_DIR", &data).env("LIGUS_SECRET_BACKEND", "file").args(["create", "Apiary", "--out", "w.txt"]);
    let (_, out, _) = run(c, "");
    assert!(out.contains("tell us your name first"), "{out}");
}
