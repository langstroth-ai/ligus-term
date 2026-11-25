//! Keybinding tests: defaults, contexts, typing, overrides from config.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ligus_term::config::{Config, default_config_text};
use ligus_term::keymap::{Action, BINDINGS, Chord, Context, Keymap};

fn ev(code: KeyCode, m: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, m)
}

fn plain(c: char) -> KeyEvent {
    ev(KeyCode::Char(c), KeyModifiers::NONE)
}

#[test]
fn defaults_resolve_in_context_order() {
    let k = Keymap::default();
    use Context::*;
    assert_eq!(k.resolve(&[Chat, Nav, Global], &plain('q'), false), Some(Action::Quit));
    assert_eq!(k.resolve(&[Chat, Nav, Global], &plain('j'), false), Some(Action::Down));
    assert_eq!(k.resolve(&[Chat, Nav, Global], &ev(KeyCode::Enter, KeyModifiers::NONE), false), Some(Action::Select));
    // The same key means different things in different places.
    assert_eq!(k.resolve(&[People, Nav, Global], &plain('r'), false), Some(Action::ChangeRole));
    assert_eq!(k.resolve(&[Agents, Nav, Global], &plain('r'), false), Some(Action::ChangeRuntime));
    assert_eq!(k.resolve(&[Review, Nav, Global], &plain('R'), false), Some(Action::Reject));
    assert_eq!(k.resolve(&[Diff, Review, Nav, Global], &plain(']'), false), Some(Action::NextHunk));
    // n and N keep their global meaning in the diff viewer.
    assert_eq!(k.resolve(&[Diff, Review, Nav, Global], &plain('n'), false), Some(Action::GoNotifications));
    assert_eq!(k.resolve(&[Review, Nav, Global], &plain('n'), false), Some(Action::GoNotifications));
    // R is reject wherever it means anything; mark-all-read is M.
    assert_eq!(k.resolve(&[Notifications, Nav, Global], &plain('R'), false), None);
    assert_eq!(k.resolve(&[Notifications, Nav, Global], &plain('M'), false), Some(Action::MarkAllRead));
    assert_eq!(k.resolve(&[Nav, Global], &plain('z'), false), None);
}

#[test]
fn typing_only_lets_command_chords_reach_global() {
    let k = Keymap::default();
    let ctx = [Context::Composer, Context::Global];
    assert_eq!(k.resolve(&ctx, &plain('q'), true), None, "q types");
    assert_eq!(k.resolve(&ctx, &plain('?'), true), None);
    assert_eq!(k.resolve(&ctx, &ev(KeyCode::Char('w'), KeyModifiers::CONTROL), true), Some(Action::SwitchWorkspace));
    assert_eq!(k.resolve(&ctx, &ev(KeyCode::F(1), KeyModifiers::NONE), true), Some(Action::Help));
    assert_eq!(k.resolve(&ctx, &ev(KeyCode::Enter, KeyModifiers::NONE), true), Some(Action::Send));
    for m in [KeyModifiers::ALT, KeyModifiers::SHIFT] {
        assert_eq!(k.resolve(&ctx, &ev(KeyCode::Enter, m), true), Some(Action::Newline), "{m:?}-enter");
    }
    assert_eq!(k.resolve(&ctx, &ev(KeyCode::Char('j'), KeyModifiers::CONTROL), true), Some(Action::Newline));
}

#[test]
fn shifted_letters_and_symbols_match_however_the_terminal_reports_them() {
    let k = Keymap::default();
    let ctx = [Context::Review, Context::Nav, Context::Global];
    // Some terminals send 'A' with SHIFT, some without.
    assert_eq!(k.resolve(&ctx, &ev(KeyCode::Char('A'), KeyModifiers::SHIFT), false), Some(Action::Approve));
    assert_eq!(k.resolve(&ctx, &plain('A'), false), Some(Action::Approve));
    assert_eq!(k.resolve(&ctx, &ev(KeyCode::Char('?'), KeyModifiers::SHIFT), false), Some(Action::Help));
    assert_eq!(k.resolve(&ctx, &ev(KeyCode::BackTab, KeyModifiers::SHIFT), false), Some(Action::FocusPrev));
    // Ctrl-letters may arrive uppercase.
    assert_eq!(k.resolve(&ctx, &ev(KeyCode::Char('C'), KeyModifiers::CONTROL), false), Some(Action::Quit));
}

#[test]
fn chords_parse_and_display() {
    for s in ["ctrl-c", "alt-enter", "shift-enter", "f1", "G", "?", "space", "pageup", "ctrl-alt-x", "backtab"] {
        let c = Chord::parse(s).unwrap();
        let again = Chord::parse(&c.to_string().replace("shift-tab", "backtab").replace('↑', "up")).unwrap();
        assert_eq!(c, again, "{s}");
    }
    assert_eq!(Chord::parse("shift-a").unwrap(), Chord::parse("A").unwrap());
    assert_eq!(Chord::parse("Ctrl-X").unwrap(), Chord::parse("ctrl-x").unwrap());
    assert!(Chord::parse("ctrl-").is_err());
    assert!(Chord::parse("hyper-q").is_err());
}

#[test]
fn every_default_parses_and_names_are_unique_per_context() {
    let mut seen = std::collections::HashSet::new();
    for b in BINDINGS {
        assert!(seen.insert((b.context, b.name)), "duplicate {}.{}", b.context.name(), b.name);
        for key in b.keys {
            Chord::parse(key).unwrap_or_else(|e| panic!("{}.{}: {e}", b.context.name(), b.name));
        }
    }
}

#[test]
fn config_overrides_replace_an_actions_keys() {
    let c = Config::parse(
        r#"
theme = "light"
[keys.global]
quit = ["ctrl-q"]
help = ["f1"]
[keys.composer]
send = ["ctrl-s"]
newline = ["enter"]
"#,
    )
    .unwrap();
    let k = c.keymap().unwrap();
    let g = [Context::Chat, Context::Nav, Context::Global];
    assert_eq!(k.resolve(&g, &plain('q'), false), None, "q no longer quits");
    assert_eq!(k.resolve(&g, &ev(KeyCode::Char('q'), KeyModifiers::CONTROL), false), Some(Action::Quit));
    assert_eq!(k.resolve(&g, &plain('?'), false), None);
    let comp = [Context::Composer, Context::Global];
    assert_eq!(k.resolve(&comp, &ev(KeyCode::Enter, KeyModifiers::NONE), true), Some(Action::Newline));
    assert_eq!(k.resolve(&comp, &ev(KeyCode::Char('s'), KeyModifiers::CONTROL), true), Some(Action::Send));
    assert_eq!(k.hint(Context::Global, Action::Quit), "ctrl-q");
}

#[test]
fn bad_config_is_reported_clearly() {
    let e = Config::parse("[keys.global]\nfly = [\"x\"]").unwrap().keymap().unwrap_err();
    assert!(e.contains("no such action"), "{e}");
    let e = Config::parse("[keys.nowhere]\nquit = [\"x\"]").unwrap().keymap().unwrap_err();
    assert!(e.contains("no such context"), "{e}");
    let e = Config::parse("[keys.global]\nhelp = [\"q\"]").unwrap().keymap().unwrap_err();
    assert!(e.contains("bound to both"), "{e}");
    let e = Config::parse("[keys.global]\nhelp = [\"meta-z\"]").unwrap().keymap().unwrap_err();
    assert!(e.contains("unknown key"), "{e}");
    assert!(Config::parse("colour = \"red\"").is_err(), "unknown fields are refused");
    // A theme name is checked at start (it may be a theme file's); its
    // type and the appearance are checked here.
    assert!(Config::parse("theme = 3").is_err());
    assert!(Config::parse("appearance = \"sepia\"").is_err());
    assert_eq!(Config::parse("theme = \"harbor\"\nappearance = \"light\"").unwrap().theme.as_deref(), Some("harbor"));
}

#[test]
fn printed_default_config_round_trips() {
    let text = default_config_text();
    let c = Config::parse(&text).unwrap();
    assert_eq!(c.mouse, Some(true), "the printed config has the mouse on");
    assert_eq!(Config::parse("mouse = false").unwrap().mouse, Some(false));
    assert_eq!(Config::parse("").unwrap().mouse, None, "unset is on (main's default)");
    let k = c.keymap().unwrap();
    let d = Keymap::default();
    for b in BINDINGS {
        assert_eq!(k.keys(b.context, b.action), d.keys(b.context, b.action), "{}", b.name);
    }
}

#[test]
fn missing_config_file_is_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let c = Config::load(Some(&dir.path().join("nope.toml"))).unwrap();
    assert!(c.theme.is_none() && c.keys.is_empty());
    std::fs::write(dir.path().join("bad.toml"), "theme = 3").unwrap();
    let e = Config::load(Some(&dir.path().join("bad.toml"))).unwrap_err();
    assert!(e.contains("bad.toml"), "{e}");
}

/// By default no view's keys hide a key that works anywhere (the composer,
/// where letters type, aside): a global key means the same in every view.
#[test]
fn no_default_shadows_a_global_key() {
    use ligus_term::keymap::{BINDINGS, Chord};
    let global: Vec<Chord> =
        BINDINGS.iter().filter(|b| b.context == Context::Global).flat_map(|b| b.keys.iter().map(|k| Chord::parse(k).unwrap())).collect();
    for b in BINDINGS.iter().filter(|b| !matches!(b.context, Context::Global | Context::Composer)) {
        for k in b.keys {
            let c = Chord::parse(k).unwrap();
            assert!(!global.contains(&c), "[keys.{}] {} = {k} hides a global key", b.context.name(), b.name);
        }
    }
}
