use std::io::IsTerminal;
use std::sync::Arc;

use clap::Parser;
use ligus_term::app::State;
use ligus_term::backend::Backend;
use ligus_term::backend::fake::{FakeBackend, FakeConfig};
use ligus_term::backend::real::{RealBackend, RealConfig};
use ligus_term::cli::Args;
use ligus_term::config::{Config, default_config_text};
use ligus_term::theme::{Theme, ThemeChoice};
use ligus_term::{app, paths, plain};

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("lt: {msg}");
    std::process::exit(2);
}

fn main() -> anyhow::Result<()> {
    // The agent sandbox on Linux re-runs this binary as its pre-exec helper
    // (inside bubblewrap): that run must stop here, before anything else.
    #[cfg(target_os = "linux")]
    ligus_runtime::executor::linux::maybe_run_helper();
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let mut args = Args::parse();
    // `lt help ...` needs no device: answer it before opening anything.
    if !args.plain && !args.json && args.command.first().is_some_and(|c| c == "help") {
        match ligus_term::cli::help_text(&args.command[1..]) {
            Ok(text) => {
                print!("{text}");
                return Ok(());
            }
            Err(e) => fail(e),
        }
    }
    // `lt --plain agents --json`: a trailing --json on a one-shot command
    // that takes no free text.
    if ligus_term::cli::take_trailing_json(&mut args.command) {
        args.json = true;
    }
    let plain_mode = args.plain || args.json;
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

    let mut real = None;
    let backend: Arc<dyn Backend> = if args.demo {
        let cfg = FakeConfig { data_dir: "(in memory: the demo keeps nothing)".into(), ..FakeConfig::default() };
        Arc::new(FakeBackend::demo(cfg))
    } else {
        let mut cfg = RealConfig::new(data.path.clone());
        cfg.relay_url = env("LT_RELAY_URL").map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        let b = RealBackend::open(cfg).unwrap_or_else(|e| fail(e));
        real = Some(b.clone());
        Arc::new(b)
    };

    if !args.command.is_empty() && !plain_mode {
        fail("a command runs in plain mode: lt --plain COMMAND (lt help COMMAND explains it)");
    }
    if plain_mode || !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        let opts = plain::PlainOpts {
            one_shot: (!args.command.is_empty()).then(|| args.command.join(" ")),
            workspace: args.workspace.clone(),
            json: args.json,
            presence: args
                .recovery_words_file
                .as_ref()
                .map(|p| ligus_term::backend::model::PresenceAnswer::RecoveryWordsFile(p.display().to_string())),
        };
        let stdin = tokio::io::BufReader::new(tokio::io::stdin());
        let code = plain::run_with(backend.clone(), stdin, Box::new(tokio::io::stdout()), opts).await?;
        if let Some(real) = &real {
            real.shutdown();
        }
        if code != 0 {
            std::process::exit(code);
        }
        return Ok(());
    }

    let mut state = State::new(backend.device(), keymap, app::runtime::now_ms());
    state.config_path = config_path.map(|p| p.display().to_string());
    state.theme_name = theme.name.to_string();
    app::runtime::run_tui(backend, state, theme).await
}
