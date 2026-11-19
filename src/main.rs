use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use ligus_term::app::State;
use ligus_term::backend::Backend;
use ligus_term::backend::fake::{FakeBackend, FakeConfig};
use ligus_term::backend::real::{RealBackend, RealConfig};
use ligus_term::config::{Config, default_config_text};
use ligus_term::theme::{Theme, ThemeChoice};
use ligus_term::{app, paths, plain};

const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nbackend: the Ligus crates (--demo: an in-memory sample)",
    "\nLigus is short for ligustica, the Italian honeybee."
);

/// lt: a full Ligus client in the terminal.
#[derive(Parser, Debug)]
#[command(name = "lt", version, long_version = LONG_VERSION, about, long_about = None)]
struct Args {
    /// Line-oriented output for screen readers and scripts (one record per
    /// line; commands on stdin). Also used when stdout isn't a terminal.
    #[arg(long)]
    plain: bool,
    /// The in-memory demo backend with sample workspaces (keeps nothing on
    /// disk) instead of this device's real data.
    #[arg(long)]
    demo: bool,
    /// This device's data dir (default: $LT_DATA_DIR, else ~/.ligus-term).
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,
    /// Config file (default: $LT_CONFIG, else <config dir>/ligus-term/config.toml).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// auto, dark, light or mono (overrides the config file).
    #[arg(long, value_name = "THEME")]
    theme: Option<ThemeChoice>,
    /// Print a config file with every default and exit.
    #[arg(long)]
    print_config: bool,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("lt: {msg}");
    std::process::exit(2);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.print_config {
        print!("{}", default_config_text());
        return Ok(());
    }
    let data = paths::data_dir(args.data_dir.as_deref(), env).unwrap_or_else(|e| fail(e));
    if let Some(w) = &data.warning {
        eprintln!("lt: warning: {w}");
    }
    let config_path = paths::config_file(args.config.as_deref(), env);
    let config = Config::load(config_path.as_deref()).unwrap_or_else(|e| fail(e));
    let keymap = config.keymap().unwrap_or_else(|e| fail(format!("config: {e}")));
    let choice = args.theme.or(config.theme).unwrap_or(ThemeChoice::Auto);
    let theme = Theme::resolve(choice, env);

    let backend: Arc<dyn Backend> = if args.demo {
        let cfg = FakeConfig { data_dir: "(in memory: the demo keeps nothing)".into(), ..FakeConfig::default() };
        Arc::new(FakeBackend::demo(cfg))
    } else {
        let mut cfg = RealConfig::new(data.path.clone());
        cfg.relay_url = env("LT_RELAY_URL").map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        Arc::new(RealBackend::open(cfg).unwrap_or_else(|e| fail(e)))
    };

    if args.plain || !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        let stdin = tokio::io::BufReader::new(tokio::io::stdin());
        return plain::run(backend, stdin, Box::new(tokio::io::stdout())).await;
    }

    let mut state = State::new(backend.device(), keymap, app::runtime::now_ms());
    state.config_path = config_path.map(|p| p.display().to_string());
    state.theme_name = theme.name.to_string();
    app::runtime::run_tui(backend, state, theme).await
}
