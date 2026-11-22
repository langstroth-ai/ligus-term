use std::io::IsTerminal;
use std::sync::Arc;

use clap::Parser;
use ligus_term::app::State;
use ligus_term::backend::Backend;
use ligus_term::backend::fake::{FakeBackend, FakeConfig};
use ligus_term::backend::real::{RealBackend, RealConfig};
use ligus_term::cli::Args;
use ligus_term::config::{Config, default_config_text};
use ligus_term::theme::{ColorDepth, Theme, ThemeChoice};
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
    // Completions and the man page need no device either (a package
    // manager runs them at install time).
    if !args.plain && !args.json {
        match args.command.first().map(String::as_str) {
            Some("completions") => {
                let shell = args.command.get(1).map(String::as_str).unwrap_or("");
                match ligus_term::completions::script(shell) {
                    Some(s) => print!("{s}"),
                    None => fail(format!("completions: give a shell, one of {}", ligus_term::completions::SHELLS.join(", "))),
                }
                return Ok(());
            }
            Some("man") => {
                print!("{}", ligus_term::completions::man_page());
                return Ok(());
            }
            Some("__complete") => {
                let data = paths::data_dir(args.data_dir.as_deref(), env).ok().map(|d| d.path);
                for line in ligus_term::completions::complete(args.command.get(1).map(String::as_str).unwrap_or(""), data.as_deref()) {
                    println!("{line}");
                }
                return Ok(());
            }
            _ => {}
        }
    }
    // `lt --plain agents --json`: a trailing --json on a one-shot command
    // that takes no free text.
    if ligus_term::cli::take_trailing_json(&mut args.command) {
        args.json = true;
    }
    // A command runs one-shot in plain mode, with or without --plain.
    // Without --plain, an unknown command (a typo) is refused on stderr
    // before any data dir is opened; with it, as an error record.
    if let Some(name) = args.command.first()
        && !args.plain
        && !args.json
        && !plain::COMMANDS.contains(&name.as_str())
    {
        fail(format!("unknown command {name}: lt help lists the commands, lt help COMMAND explains one"));
    }
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
    let depth = ColorDepth::detect(env);
    // The full-screen app may ask the terminal for its background (OSC 11);
    // plain mode never writes anything but records.
    let plain_mode = args.plain || args.json || !args.command.is_empty();
    let tui = !plain_mode && std::io::stdout().is_terminal() && std::io::stdin().is_terminal();
    let probe = || if tui { ligus_term::termprobe::background_is_light(ligus_term::termprobe::PROBE_TIMEOUT) } else { None };
    let ascii =
        args.ascii || env("LT_ASCII").is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"));
    let theme = Theme { ascii, ..Theme::resolve_with(choice, env, probe).with_depth(depth) };

    let mut real = None;
    let backend: Arc<dyn Backend> = if args.demo {
        let cfg = FakeConfig { data_dir: "(in memory: the demo keeps nothing)".into(), ..FakeConfig::default() };
        Arc::new(FakeBackend::demo(cfg))
    } else {
        let mut cfg = RealConfig::new(data.path.clone());
        cfg.relay_url = env("LT_RELAY_URL").map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        cfg.presence_port = match env("LT_PRESENCE_PORT").map(|p| p.trim().to_string()).filter(|p| !p.is_empty()) {
            None => 0,
            Some(p) => match p.parse::<u16>() {
                Ok(n) if n > 0 => n,
                _ => fail(format!("LT_PRESENCE_PORT={p} isn't a port number (1-65535)")),
            },
        };
        cfg.no_browser = match env("LT_NO_BROWSER").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("" | "0" | "false" | "off" | "no") => false,
            Some("1" | "true" | "on" | "yes") => true,
            Some(v) => fail(format!("LT_NO_BROWSER={v}: use 1 or 0")),
        };
        let b = RealBackend::open(cfg).unwrap_or_else(|e| fail(e));
        real = Some(b.clone());
        Arc::new(b)
    };

    if plain_mode || !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        let opts = plain::PlainOpts {
            one_shot: (!args.command.is_empty()).then(|| args.command.join(" ")),
            workspace: args.workspace.clone(),
            json: args.json,
            name: args.name.clone(),
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
    state.theme_name = app::runtime::theme_label(&theme, depth);
    state.mouse = config.mouse.unwrap_or(true);
    let mouse = state.mouse;
    let state = app::runtime::run_tui(backend, state, theme, depth, mouse).await?;
    if let Some(real) = &real {
        real.shutdown();
    }
    if state.exec_demo {
        // Look around first: the same binary, in the demo.
        let exe = std::env::current_exe()?;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--demo");
        if let Some(t) = args.theme {
            cmd.arg("--theme").arg(format!("{t:?}").to_lowercase());
        }
        if args.ascii {
            cmd.arg("--ascii");
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            return Err(cmd.exec().into());
        }
        #[cfg(not(unix))]
        {
            let status = cmd.status()?;
            std::process::exit(status.code().unwrap_or(1));
        }
    }
    Ok(())
}
