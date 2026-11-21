//! What this device can run agents on, found the way the desktop finds it at
//! onboarding: the CLI agents on `PATH` (Claude Code, Codex, and through
//! `ligus_runtime::cli_detect` the Antigravity and Copilot CLIs), a local
//! Ollama server, and hosted-API keys in the environment.
//!
//! Rules:
//! * Nothing here sends a prompt or spends quota: a CLI is only asked for
//!   `--version` (bounded, stdin closed), Ollama only for its model list.
//! * Nothing opens a keychain. A sign-in is "signed in" from a file's
//!   existence or an environment variable's presence; where the sign-in
//!   lives only in the OS keychain, it is "unknown" (which still counts as
//!   ready: the turn says so if it isn't).
//! * No secret's value is kept: the environment is reduced to "set"
//!   markers before anything reads it.
//!
//! TODO(ligus 0.1.1): the Claude Code, Codex and Ollama checks repeat the
//! desktop's `detect_environment` / `detect_providers` and `ligus setup`
//! (both outside the library). Move them into `ligus_runtime::cli_detect`
//! next to the Antigravity and Copilot checks and call that from here.

use std::collections::HashMap;
#[cfg(test)]
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ligus_core::ModelProviderKind;
use ligus_runtime::cli_detect::{self, DetectEnv, LoginState};

/// Where the desktop looks for Ollama.
pub const OLLAMA_DEFAULT: &str = "http://127.0.0.1:11434";
/// The prefix of a device-local runtime id (`local:codex`,
/// `local:ollama/qwen3:8b`).
pub const LOCAL_PREFIX: &str = "local:";

/// Environment variables whose presence (never value) detection reads.
pub(crate) const PRESENCE_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    // Copilot CLI tokens (`cli_detect::copilot_login`).
    "COPILOT_GITHUB_TOKEN",
    "GH_TOKEN",
    "GITHUB_TOKEN",
];
/// Directories the CLIs keep their config in (paths, not secrets).
pub(crate) const DIR_VARS: &[&str] = &["COPILOT_HOME", "CODEX_HOME", "CLAUDE_CONFIG_DIR"];

/// Hosted APIs found by their key in the environment: (variable, provider,
/// default model, suggested agent name). The models are `ligus setup`'s.
const API_KEYS: &[(&str, ModelProviderKind, &str, &str)] = &[
    ("ANTHROPIC_API_KEY", ModelProviderKind::Anthropic, "claude-sonnet-4-6", "anthropic"),
    ("OPENAI_API_KEY", ModelProviderKind::OpenAI, "gpt-4o", "gpt"),
    ("OPENROUTER_API_KEY", ModelProviderKind::OpenRouter, "openai/gpt-4o-mini", "openrouter"),
];

/// One thing found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub provider: ModelProviderKind,
    pub model: Option<String>,
    pub version: Option<String>,
    pub ready: bool,
    pub detail: String,
    pub unsupported: Option<String>,
    pub suggested: String,
}

/// This process's environment as detection sees it: `PATH`, the home dir,
/// the CLIs' config dirs, and a marker for each credential variable that
/// is set. A classic GitHub token is marked as such (Copilot refuses it);
/// no value is copied.
pub fn process_env() -> DetectEnv {
    let mut vars = HashMap::new();
    for k in PRESENCE_VARS {
        if let Some(v) = std::env::var_os(k).filter(|v| !v.is_empty()) {
            let classic = v.to_string_lossy().trim().starts_with("ghp_");
            vars.insert((*k).to_string(), if classic { "ghp_ (classic token)" } else { "set" }.to_string());
        }
    }
    for k in DIR_VARS {
        if let Some(v) = std::env::var(k).ok().filter(|v| !v.is_empty()) {
            vars.insert((*k).to_string(), v);
        }
    }
    DetectEnv { path: std::env::var_os("PATH").unwrap_or_default(), home: dirs::home_dir().unwrap_or_default(), vars }
}

fn set(env: &DetectEnv, k: &str) -> bool {
    env.vars.get(k).is_some_and(|v| !v.trim().is_empty())
}

fn dir_var(env: &DetectEnv, k: &str) -> Option<PathBuf> {
    env.vars.get(k).filter(|v| !v.trim().is_empty()).map(PathBuf::from)
}

/// `local:<provider>[/<model>]`.
pub fn local_id(provider: ModelProviderKind, model: Option<&str>) -> String {
    match model.filter(|m| !m.is_empty()) {
        Some(m) => format!("{LOCAL_PREFIX}{}/{m}", slug(provider)),
        None => format!("{LOCAL_PREFIX}{}", slug(provider)),
    }
}

/// The provider and model a device-local runtime id names.
pub fn parse_local_id(id: &str) -> Option<(ModelProviderKind, String)> {
    let rest = id.strip_prefix(LOCAL_PREFIX)?;
    let (p, model) = rest.split_once('/').unwrap_or((rest, ""));
    let provider = match p {
        "claude-code" => ModelProviderKind::ClaudeCode,
        "codex" => ModelProviderKind::Codex,
        "antigravity" => ModelProviderKind::Antigravity,
        "copilot" => ModelProviderKind::Copilot,
        "ollama" => ModelProviderKind::Ollama,
        "anthropic" => ModelProviderKind::Anthropic,
        "openai" => ModelProviderKind::OpenAI,
        "openrouter" => ModelProviderKind::OpenRouter,
        _ => return None,
    };
    Some((provider, model.to_string()))
}

fn slug(p: ModelProviderKind) -> &'static str {
    match p {
        ModelProviderKind::ClaudeCode => "claude-code",
        ModelProviderKind::Codex => "codex",
        ModelProviderKind::Antigravity => "antigravity",
        ModelProviderKind::Copilot => "copilot",
        ModelProviderKind::Ollama => "ollama",
        ModelProviderKind::Anthropic => "anthropic",
        ModelProviderKind::OpenAI => "openai",
        ModelProviderKind::OpenRouter => "openrouter",
        _ => "other",
    }
}

/// The program a CLI runtime runs (the desktop's defaults).
pub fn program(p: ModelProviderKind) -> &'static str {
    match p {
        ModelProviderKind::ClaudeCode => "claude",
        ModelProviderKind::Codex => "codex",
        ModelProviderKind::Antigravity => "agy",
        ModelProviderKind::Copilot => "copilot",
        _ => "",
    }
}

/// The environment variable a hosted API's key is read from at turn time.
pub fn key_var(p: ModelProviderKind) -> Option<&'static str> {
    API_KEYS.iter().find(|(_, k, _, _)| *k == p).map(|(v, ..)| *v)
}

/// The chat endpoint a hosted API runtime talks to (`ligus setup`'s).
pub fn api_endpoint(p: ModelProviderKind) -> &'static str {
    match p {
        ModelProviderKind::OpenAI => "https://api.openai.com/v1/chat/completions",
        ModelProviderKind::OpenRouter => "https://openrouter.ai/api/v1/chat/completions",
        _ => "",
    }
}

/// Run `program --version` with stdin closed and a 5 s limit; the first
/// version-looking word of its output.
fn version_of(program: &Path, env: &DetectEnv) -> Option<String> {
    let mut child = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PATH", &env.path)
        .env("NO_COLOR", "1")
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.success(),
            Ok(None) if start.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let text = reader.join().unwrap_or_default();
    if ok { cli_detect::parse_version(&text) } else { None }
}

/// Readiness from a sign-in state: signed in, or unknown (counts as ready),
/// or not signed in (with what to run).
fn readiness(login: LoginState, detail: &str, login_command: &str) -> (bool, String) {
    match login {
        LoginState::LoggedIn => (true, detail.to_string()),
        LoginState::Unknown if !detail.is_empty() => (true, format!("{detail}; if a turn fails, run `{login_command}` in a terminal")),
        LoginState::Unknown => (true, format!("sign-in not checked; if a turn fails, run `{login_command}` in a terminal")),
        LoginState::NotLoggedIn => (false, format!("{detail}: run `{login_command}` in a terminal")),
    }
}

/// Claude Code's sign-in without opening the keychain: a key or token in
/// the environment, or its credentials file (Linux, Windows). On macOS the
/// sign-in lives in the keychain, so it is "unknown" there.
fn claude_login(env: &DetectEnv) -> (LoginState, String) {
    if set(env, "CLAUDE_CODE_OAUTH_TOKEN") {
        return (LoginState::LoggedIn, "token in CLAUDE_CODE_OAUTH_TOKEN".into());
    }
    if set(env, "ANTHROPIC_API_KEY") {
        return (LoginState::LoggedIn, "ANTHROPIC_API_KEY is set (billed per token)".into());
    }
    let dir = dir_var(env, "CLAUDE_CONFIG_DIR").unwrap_or_else(|| env.home.join(".claude"));
    if dir.join(".credentials.json").is_file() {
        return (LoginState::LoggedIn, "signed in (claude login)".into());
    }
    if cfg!(target_os = "macos") {
        (LoginState::Unknown, "its sign-in is in the macOS keychain, which lt doesn't open".into())
    } else {
        (LoginState::NotLoggedIn, "no Claude Code sign-in found".into())
    }
}

/// Codex's sign-in: `OPENAI_API_KEY`, or its auth file exists (never read).
fn codex_login(env: &DetectEnv) -> (LoginState, String) {
    if set(env, "OPENAI_API_KEY") {
        return (LoginState::LoggedIn, "OPENAI_API_KEY is set (billed per token)".into());
    }
    let dir = dir_var(env, "CODEX_HOME").unwrap_or_else(|| env.home.join(".codex"));
    if dir.join("auth.json").is_file() {
        return (LoginState::LoggedIn, "signed in (codex login)".into());
    }
    (LoginState::NotLoggedIn, "no Codex sign-in found".into())
}

/// The CLI agents on `PATH`. Blocking (runs `--version`s).
pub fn clis(env: &DetectEnv) -> Vec<Found> {
    let mut out = Vec::new();
    for (provider, login, login_command, name) in [
        (ModelProviderKind::ClaudeCode, claude_login as fn(&DetectEnv) -> (LoginState, String), "claude", "claude"),
        (ModelProviderKind::Codex, codex_login, "codex login", "codex"),
    ] {
        let Some(path) = cli_detect::find_on_path(program(provider), &env.path) else { continue };
        let (state, detail) = login(env);
        let (ready, detail) = readiness(state, &detail, login_command);
        out.push(Found {
            id: local_id(provider, None),
            provider,
            model: None,
            version: version_of(&path, env),
            ready,
            detail,
            unsupported: None,
            suggested: name.into(),
        });
    }
    // Antigravity, Copilot and (to explain it) Gemini CLI: the desktop's
    // Settings → Models check. Passive: `agy models` is never run.
    for c in cli_detect::detect_cli_agents(env, false) {
        if !c.installed {
            continue;
        }
        let provider = cli_detect::provider_for(&c.id);
        let (ready, detail) = if c.supported { readiness(c.login, &c.login_detail, &c.login_command) } else { (false, String::new()) };
        out.push(Found {
            id: provider.map(|p| local_id(p, None)).unwrap_or_else(|| format!("{LOCAL_PREFIX}{}", c.id)),
            provider: provider.unwrap_or(ModelProviderKind::Custom),
            model: None,
            version: c.version,
            ready: ready && c.supported,
            detail,
            unsupported: (!c.supported || provider.is_none()).then(|| c.note.clone()),
            suggested: match c.id.as_str() {
                "antigravity" => "agy".into(),
                other => other.to_string(),
            },
        });
    }
    out
}

/// Hosted APIs whose key is in the environment (presence only).
pub fn api_keys(env: &DetectEnv) -> Vec<Found> {
    API_KEYS
        .iter()
        .filter(|(var, ..)| set(env, var))
        .map(|(var, provider, model, name)| Found {
            id: local_id(*provider, Some(model)),
            provider: *provider,
            model: Some((*model).to_string()),
            version: None,
            ready: true,
            detail: format!("{var} is set (billed per token)"),
            unsupported: None,
            suggested: (*name).to_string(),
        })
        .collect()
}

/// The models on an Ollama server, if one answers at `base` (a 300 ms
/// connect probe first, as the desktop does).
pub async fn ollama(base: &str) -> Vec<Found> {
    let base = ligus_runtime::provider::ollama::base_url(base);
    let hostport = base.split("://").nth(1).unwrap_or(&base).split('/').next().unwrap_or_default().to_string();
    let up = matches!(tokio::time::timeout(Duration::from_millis(300), tokio::net::TcpStream::connect(hostport.as_str())).await, Ok(Ok(_)));
    if !up {
        return Vec::new();
    }
    let client = ligus_runtime::provider::OllamaClient::new(&base).with_connect_retries(0);
    let models = match tokio::time::timeout(Duration::from_secs(3), client.list_models()).await {
        Ok(Ok(m)) => m,
        _ => Vec::new(),
    };
    if models.is_empty() {
        return vec![Found {
            id: local_id(ModelProviderKind::Ollama, None),
            provider: ModelProviderKind::Ollama,
            model: None,
            version: None,
            ready: false,
            detail: format!("Ollama answers at {hostport} but has no models: run `ollama pull <model>`"),
            unsupported: None,
            suggested: "ollama".into(),
        }];
    }
    models
        .into_iter()
        .take(8)
        .map(|m| Found {
            id: local_id(ModelProviderKind::Ollama, Some(&m.name)),
            provider: ModelProviderKind::Ollama,
            suggested: model_name(&m.name),
            model: Some(m.name),
            version: None,
            ready: true,
            detail: format!("Ollama at {hostport}"),
            unsupported: None,
        })
        .collect()
}

/// An agent name from a model id: `qwen3-coder:30b` → `qwen3-coder`.
fn model_name(model: &str) -> String {
    let base = model.rsplit('/').next().unwrap_or(model);
    let base = base.split(':').next().unwrap_or(base);
    let s: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "ollama".into() } else { s }
}

/// Reserved mention words an agent may not be named (the desktop's list).
pub const RESERVED: &[&str] = &["primary", "ligus", "you", "all", "everyone", "owners", "admins", "contributors", "viewers"];

/// Why `name` can't name a new agent, given the names already taken.
pub fn name_refusal(name: &str, taken: &[String]) -> Option<String> {
    if name.is_empty() {
        return Some("an agent needs a name".into());
    }
    if name.chars().any(|c| c.is_whitespace() || c == '@') {
        return Some(format!("\u{201c}{name}\u{201d} can't be mentioned: use letters, digits and dashes"));
    }
    if RESERVED.contains(&name.to_lowercase().as_str()) {
        return Some(format!("\u{201c}{name}\u{201d} is a reserved mention name: pick another"));
    }
    if taken.iter().any(|t| t.eq_ignore_ascii_case(name)) {
        return Some(format!("\u{201c}{name}\u{201d} is already taken by another agent or member: pick another"));
    }
    None
}

/// `want`, or `want-2`, `want-3`, ... the first one not taken.
pub fn unique_name(want: &str, taken: &[String]) -> String {
    let free = |n: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(n)) && !RESERVED.contains(&n);
    if free(want) {
        return want.to_string();
    }
    (2..).map(|i| format!("{want}-{i}")).find(|n| free(n)).unwrap_or_else(|| want.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty environment over `path` and `home`.
    fn env_for(path: OsString, home: PathBuf) -> DetectEnv {
        DetectEnv { path, home, vars: HashMap::new() }
    }

    #[test]
    fn local_ids_round_trip() {
        for (p, m) in [
            (ModelProviderKind::Codex, ""),
            (ModelProviderKind::Ollama, "qwen3-coder:30b"),
            (ModelProviderKind::OpenRouter, "openai/gpt-4o-mini"),
        ] {
            let id = local_id(p, Some(m));
            assert_eq!(parse_local_id(&id), Some((p, m.to_string())), "{id}");
        }
        assert_eq!(local_id(ModelProviderKind::ClaudeCode, None), "local:claude-code");
        assert_eq!(parse_local_id("codex"), None);
        assert_eq!(parse_local_id("local:nope"), None);
    }

    #[test]
    fn names() {
        assert_eq!(model_name("qwen3-coder:30b"), "qwen3-coder");
        assert_eq!(model_name("hf.co/org/Llama_3.1:Q4"), "llama-3-1");
        let taken = vec!["codex".to_string(), "Codex-2".to_string()];
        assert_eq!(unique_name("codex", &taken), "codex-3");
        assert_eq!(unique_name("claude", &taken), "claude");
        assert_eq!(unique_name("primary", &taken), "primary-2");
        assert!(name_refusal("codex", &taken).unwrap().contains("taken"));
        assert!(name_refusal("all", &taken).unwrap().contains("reserved"));
        assert!(name_refusal("two words", &taken).is_some());
        assert!(name_refusal("", &taken).is_some());
        assert_eq!(name_refusal("reviewer", &taken), None);
    }

    #[test]
    fn presence_markers_never_copy_a_value() {
        // Only "set" markers reach the detection environment.
        let env = process_env();
        for k in PRESENCE_VARS {
            if let Some(v) = env.vars.get(*k) {
                assert!(v == "set" || v.starts_with("ghp_ (classic"), "{k}");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn stub_clis_are_found_with_versions_and_sign_in() {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        for (name, out) in [("claude", "2.1.3 (Claude Code)"), ("codex", "codex-cli 0.98.0")] {
            let p = bin.path().join(name);
            // Answers only `--version`; anything else is a failure.
            std::fs::write(&p, format!("#!/bin/sh\n[ \"$1\" = --version ] && {{ echo '{out}'; exit 0; }}\nexit 9\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut env = env_for(bin.path().as_os_str().to_owned(), home.path().to_path_buf());
        let found = clis(&env);
        let codex = found.iter().find(|f| f.provider == ModelProviderKind::Codex).unwrap();
        assert_eq!(codex.version.as_deref(), Some("0.98.0"));
        assert!(!codex.ready && codex.detail.contains("codex login"), "{codex:?}");
        let claude = found.iter().find(|f| f.provider == ModelProviderKind::ClaudeCode).unwrap();
        assert_eq!(claude.version.as_deref(), Some("2.1.3"));
        assert_eq!(found.len(), 2, "no agy or copilot on this PATH: {found:?}");

        // Signed in: a credentials file exists (never read), or a key is set.
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex/auth.json"), "{}").unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(home.path().join(".claude/.credentials.json"), "{}").unwrap();
        let found = clis(&env);
        assert!(found.iter().all(|f| f.ready), "{found:?}");
        env.vars.insert("OPENROUTER_API_KEY".into(), "set".into());
        let keys = api_keys(&env);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].id, "local:openrouter/openai/gpt-4o-mini");
        assert!(keys[0].detail.contains("OPENROUTER_API_KEY") && !keys[0].detail.contains("set\""));
    }
}
