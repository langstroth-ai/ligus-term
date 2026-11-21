//! `lt`'s command line: the flags, and what `lt --help` prints after them.
//!
//! The flags live here (not in `main.rs`) so the help tests can walk them
//! and check each one is documented.

use std::path::PathBuf;

use clap::Parser;

use crate::theme::ThemeChoice;

pub const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nbackend: the Ligus crates (--demo: an in-memory sample)",
    "\nLigus is short for ligustica, the Italian honeybee."
);

/// lt: a full Ligus client in the terminal.
///
/// With no command, lt opens the full-screen app. `lt --plain` is a line
/// mode for screen readers and scripts, and `lt --plain COMMAND` runs one
/// command and exits. `lt help TOPIC` explains the rest.
#[derive(Parser, Debug)]
#[command(
    name = "lt",
    version,
    long_version = LONG_VERSION,
    about,
    after_help = "More: lt --help (every flag, variable and exit code), lt help TOPIC, lt help COMMAND.",
    after_long_help = crate::help::after_long_help(),
)]
pub struct Args {
    /// Line-oriented output for screen readers and scripts: one record per
    /// line (a keyword, then tab-separated fields), commands on stdin. Also
    /// used when stdin or stdout isn't a terminal. `lt help plain` has the
    /// records.
    #[arg(long)]
    pub plain: bool,
    /// With plain mode: write each record as one JSON object per line
    /// (JSON Lines) instead of tab-separated fields. Implies --plain. A
    /// one-shot listing command also takes it at the end:
    /// lt --plain agents --json.
    #[arg(long)]
    pub json: bool,
    /// The in-memory demo backend with sample workspaces (keeps nothing on
    /// disk) instead of this device's real data.
    #[arg(long)]
    pub demo: bool,
    /// This device's data dir: its identity, database and secret store
    /// (default: $LT_DATA_DIR, else ~/.ligus-term). It must not overlap the
    /// desktop app's or the ligus CLI's.
    #[arg(long, value_name = "PATH")]
    pub data_dir: Option<PathBuf>,
    /// Config file (default: $LT_CONFIG, else <config dir>/ligus-term/config.toml).
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// auto, dark, light or mono (overrides the config file).
    #[arg(long, value_name = "THEME")]
    pub theme: Option<ThemeChoice>,
    /// Print a config file with every default and exit.
    #[arg(long)]
    pub print_config: bool,
    /// The workspace plain mode uses (a name, an id or a number; default the
    /// first).
    #[arg(long, value_name = "WORKSPACE")]
    pub workspace: Option<String>,
    /// Answer passkey confirmations in plain mode with this recovery-words
    /// file (owner-only, chmod 600), instead of the browser. A new passkey
    /// (create, connect) always needs the browser.
    #[arg(long, value_name = "PATH")]
    pub recovery_words_file: Option<PathBuf>,
    /// With --plain: run this one command and exit (0 ok, 1 an error or an
    /// unfinished run, 2 an unknown command), e.g. lt --plain run 1 "fix the
    /// tests". Without --plain, only `help [TOPIC|COMMAND|FLAG]`.
    #[arg(trailing_var_arg = true, value_name = "COMMAND")]
    pub command: Vec<String>,
}

/// Each flag: its long name, value name and help (for `lt help --FLAG`).
pub fn flags() -> Vec<(String, Option<String>, String)> {
    use clap::CommandFactory;
    let mut cmd = Args::command();
    cmd.build();
    cmd.get_arguments()
        .filter_map(|a| {
            let long = a.get_long()?.to_string();
            let value = a.get_action().takes_values().then(|| a.get_value_names().and_then(|v| v.first()).map(|v| v.to_string())).flatten();
            let help = a.get_long_help().or(a.get_help()).map(|h| h.to_string()).unwrap_or_default();
            Some((long, value, help))
        })
        .collect()
}

/// `lt --help` as text.
pub fn long_help() -> String {
    use clap::CommandFactory;
    Args::command().render_long_help().to_string()
}

/// What `lt help [NAME]` prints: the long help, or a topic, a plain
/// command or a flag; an error (exit 2) for anything else.
pub fn help_text(rest: &[String]) -> Result<String, String> {
    let Some(name) = rest.first() else { return Ok(long_help()) };
    crate::help::topic(name, &flags()).ok_or_else(|| crate::help::unknown(name))
}

/// One-shot commands whose last argument is free text, where a trailing
/// `--json` is part of the text.
const FREE_TEXT: &[&str] = &["say", "new", "name", "run", "start", "create"];

/// Drop a trailing `--json` from a one-shot command (`lt --plain agents
/// --json`); whether there was one.
pub fn take_trailing_json(command: &mut Vec<String>) -> bool {
    let free = command.first().is_some_and(|c| FREE_TEXT.contains(&c.as_str()));
    if command.len() > 1 && !free && command.last().is_some_and(|l| l == "--json") {
        command.pop();
        return true;
    }
    false
}
