//! `lt link-request`, `lt link-redeem` and `lt link-status` (with or
//! without `--plain`): linking this computer to a person's devices from a
//! shell, as the CLI conventions want it. Data goes to stdout (the request
//! code alone, so `lt --plain link-request | pbcopy` copies exactly the
//! code); the fingerprint, the instructions and the errors go to stderr.
//!
//! Redeeming never needs the desktop's code on the command line: it is read
//! from stdin (`-`), or, at a terminal with no code given, from a hidden
//! paste or the clipboard. A code is never echoed, logged or printed. At a
//! terminal `--approver-fingerprint` may be left out: lt shows who approved
//! the code and asks; without a terminal the flag is required (exit 4).

use std::io::{self, Write};

use crate::backend::Backend;
use crate::backend::model::{LinkPending, LinkStatusView};
use crate::link_code::{MAX_PASTE, Pasted, grant_in, hh_mm, not_a_grant};

/// The commands this module runs.
pub const COMMANDS: &[&str] = &["link-request", "link-redeem", "link-status"];

/// How long the desktop may approve a request (ligus `REQUEST_TTL_MS`).
pub const REQUEST_TTL_MS: i64 = 15 * 60 * 1000;

/// Whether `cmd` is one of [`COMMANDS`].
pub fn handles(cmd: &str) -> bool {
    COMMANDS.contains(&cmd)
}

/// The person's terminal, the clipboard and stdin, behind a trait so the
/// tests drive every path.
pub trait Terminal {
    /// A person is at a terminal (stderr is one, and so is stdin or
    /// `/dev/tty`): lt may ask.
    fn interactive(&self) -> bool;
    /// All of stdin (`-`), at most [`MAX_PASTE`] bytes.
    fn read_stdin(&mut self) -> io::Result<String>;
    /// The clipboard's text, read only when the person asked for it.
    fn clipboard(&mut self) -> Option<String>;
    /// Copy `text`: how, or `None` when no clipboard was reachable.
    fn copy(&mut self, text: &str) -> Option<String>;
    /// Ask on the terminal and read a line without echoing it; `None` when
    /// the person cancelled (Esc, ctrl-c).
    fn ask_hidden(&mut self, prompt: &str) -> io::Result<Option<String>>;
    /// Ask a yes/no question; only `y` or `yes` is yes.
    fn confirm(&mut self, prompt: &str) -> io::Result<bool>;
}

/// Where the output goes, and the clock.
pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    pub term: &'a mut dyn Terminal,
    /// One JSON object on stdout instead of text.
    pub json: bool,
    /// Now (Unix ms) and the person's offset from UTC (seconds).
    pub now_ms: i64,
    pub utc_offset: i32,
}

struct Failure {
    message: String,
    exit: i32,
}

fn fail<T>(message: impl Into<String>, exit: i32) -> Result<T, Failure> {
    Err(Failure { message: message.into(), exit })
}

fn backend_fail<T>(message: String) -> Result<T, Failure> {
    let exit = crate::plain::error_code(&message).1;
    Err(Failure { message, exit })
}

fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// A name from a code, on one line.
fn one_line(s: &str) -> String {
    crate::plain::clean(s).replace('\t', " ")
}

/// Run `args` (`args[0]` is the command); the exit code.
pub async fn run(b: &dyn Backend, args: &[String], io: &mut Io<'_>) -> i32 {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    let r = match cmd {
        "link-request" => request(b, rest, io).await,
        "link-redeem" => redeem(b, rest, io).await,
        "link-status" => status(b, rest, io).await,
        other => fail(format!("unknown command {other}: lt help lists them"), 2),
    };
    match r {
        Ok(()) => 0,
        Err(f) => {
            let _ = writeln!(io.err, "lt: {}", one_line(&f.message));
            if io.json {
                let _ = writeln!(io.out, "{}", crate::help::record_json(&format!("error\t{}", one_line(&f.message))));
            }
            f.exit
        }
    }
}

fn unknown_flags(rest: &[String], known: &[&str], usage: &str) -> Result<(), Failure> {
    match rest.iter().find(|a| a.starts_with("--") && !known.contains(&a.as_str())) {
        Some(f) => fail(format!("unknown option {f}: {usage}"), 2),
        None => Ok(()),
    }
}

async fn request(b: &dyn Backend, rest: &[String], io: &mut Io<'_>) -> Result<(), Failure> {
    let usage = "usage: link-request [--copy] [--json]";
    unknown_flags(rest, &["--copy"], usage)?;
    if let Some(a) = rest.iter().find(|a| !a.starts_with("--")) {
        return fail(format!("link-request takes no argument ({a}): {usage}"), 2);
    }
    let copy = rest.iter().any(|a| a == "--copy");
    let r = match b.link_request().await {
        Ok(r) => r,
        Err(e) => return backend_fail(e.0),
    };
    let approve_by = io.now_ms + REQUEST_TTL_MS;
    let copied = if copy { io.term.copy(&r.code) } else { None };
    if io.json {
        let _ = writeln!(
            io.out,
            "{{\"record\":\"link-request\",\"code\":{},\"fingerprint\":{},\"label\":{},\"expiresAt\":{approve_by},\"newDevice\":{},\"copied\":{}}}",
            q(&r.code),
            q(&r.fingerprint),
            q(&r.label),
            r.new_device,
            copied.is_some()
        );
    } else if copied.is_none() {
        // The data, alone: `| pbcopy` copies exactly the code.
        let _ = writeln!(io.out, "{}", r.code);
    }
    let e = &mut io.err;
    if copy {
        match &copied {
            Some(how) => {
                let _ = writeln!(e, "Copied this computer's request ({how}).");
            }
            None if io.json => {
                let _ = writeln!(e, "lt: couldn't reach the clipboard: the request is in the JSON (code).");
            }
            None => {
                let _ = writeln!(e, "lt: couldn't reach the clipboard: the request is printed above.");
            }
        }
    }
    let _ = writeln!(e, "Fingerprint: {}  ({})", r.fingerprint, one_line(&r.label));
    let _ = writeln!(
        e,
        "On your desktop: Settings > Profile > My devices > Add a computer. Paste the request, check the fingerprint \
         matches, and confirm with your passkey (by {}).",
        hh_mm(approve_by, io.utc_offset)
    );
    let _ = writeln!(e, "Then here: lt --plain link-redeem   (it reads the desktop's code from the clipboard, or paste it at the prompt)");
    if r.new_device {
        let _ = writeln!(e, "This install was already in use, so it asks with a new device key (used only once you redeem).");
    }
    Ok(())
}

/// The desktop's code: from stdin (`-`), the argument, or (at a terminal)
/// a hidden paste or the clipboard.
fn read_grant(code: Option<&str>, io: &mut Io<'_>) -> Result<String, Failure> {
    let usage = "usage: link-redeem [CODE | -] [--approver-fingerprint FP]";
    let text = match code {
        Some("-") => match io.term.read_stdin() {
            Ok(t) => zeroize::Zeroizing::new(t),
            Err(e) => return fail(format!("couldn't read the code from stdin: {e}"), 1),
        },
        Some(c) => zeroize::Zeroizing::new(c.to_string()),
        None if !io.term.interactive() => {
            return fail(format!("{usage} (no terminal to paste at: pipe the code in with -)"), 2);
        }
        None => return ask_for_grant(io),
    };
    match grant_in(&text) {
        Pasted::Grant(g) => Ok(g),
        other => fail(not_a_grant(&other, false), 1),
    }
}

/// At a terminal: a hidden paste, or the clipboard on a bare Enter. Asked
/// again (up to three times) when what came isn't a desktop's code: the
/// clipboard may still hold the command the desktop copied.
fn ask_for_grant(io: &mut Io<'_>) -> Result<String, Failure> {
    let mut prompt = "Paste the desktop's code (hidden) and press Enter, or just press Enter to read it from the clipboard: ";
    let mut last = String::new();
    for _ in 0..3 {
        let (text, from_clipboard) = match io.term.ask_hidden(prompt) {
            Err(e) => return fail(format!("couldn't read the terminal: {e}"), 1),
            Ok(None) => return fail("cancelled: nothing was linked", 1),
            Ok(Some(t)) if !t.trim().is_empty() => (zeroize::Zeroizing::new(t), false),
            Ok(Some(_)) => match io.term.clipboard() {
                Some(t) => (zeroize::Zeroizing::new(t), true),
                None => {
                    return fail(
                        "couldn't read the clipboard (no pbpaste, wl-paste, xclip, xsel or powershell here): paste the \
                         code at the prompt, or pipe it in: lt --plain link-redeem - < FILE",
                        3,
                    );
                }
            },
        };
        match grant_in(&text) {
            Pasted::Grant(g) => return Ok(g),
            other => last = not_a_grant(&other, from_clipboard),
        }
        let _ = writeln!(io.err, "lt: {last}");
        prompt = "Try again: paste the code, or press Enter to read the clipboard (Esc cancels): ";
    }
    fail(last, 1)
}

async fn redeem(b: &dyn Backend, rest: &[String], io: &mut Io<'_>) -> Result<(), Failure> {
    let usage = "usage: link-redeem [CODE | -] [--approver-fingerprint FP]";
    unknown_flags(rest, &["--approver-fingerprint"], usage)?;
    let mut fp = None;
    let mut code: Vec<&str> = vec![];
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--approver-fingerprint" {
            let Some(v) = rest.get(i + 1) else {
                return fail(format!("--approver-fingerprint needs the desktop's fingerprint: {usage}"), 2);
            };
            let mut f = v.trim().to_string();
            i += 2;
            // A fingerprint copied with spaces and not quoted: its groups.
            let group = |a: &str| a.len() == 4 && a.chars().all(|c| c.is_ascii_hexdigit());
            while rest.get(i).is_some_and(|a| group(a)) {
                f.push('-');
                f.push_str(&rest[i]);
                i += 1;
            }
            fp = Some(f);
        } else {
            code.push(&rest[i]);
            i += 1;
        }
    }
    // A code split by the shell (it had spaces in it) is one code.
    let joined = (!code.is_empty()).then(|| code.concat());
    let grant = zeroize::Zeroizing::new(read_grant(joined.as_deref(), io)?);
    let check = match b.inspect_link_grant(&grant).await {
        Ok(c) => c,
        Err(e) => return backend_fail(e.0),
    };
    let (person, ws, approver) = (one_line(&check.person_name), one_line(&check.workspace_name), one_line(&check.approver_fingerprint));
    let who = format!("Approved by {person}'s desktop · fingerprint {approver} · for {ws}");
    match fp {
        Some(f) if crate::link_code::same_fingerprint(&f, &check.approver_fingerprint) => {
            let _ = writeln!(io.err, "{who}.");
        }
        Some(f) => {
            return fail(format!("approved by desktop {approver}, not {}: don't use this code", one_line(&f)), 1);
        }
        None if io.term.interactive() => match io.term.confirm(&format!("{who}. Is that your desktop?")) {
            Ok(true) => {}
            Ok(false) => return fail("not linked: you didn't confirm the desktop", 1),
            Err(e) => return fail(format!("couldn't read the terminal: {e}"), 1),
        },
        None => {
            return fail(
                format!(
                    "confirm who approved this ({person}'s desktop {approver} for {ws}): add --approver-fingerprint FP as \
                     your desktop shows it, or run this at a terminal"
                ),
                4,
            );
        }
    }
    match b.redeem_link(&grant).await {
        Ok(w) => {
            if io.json {
                let _ = writeln!(
                    io.out,
                    "{{\"record\":\"linked\",\"id\":{},\"name\":{},\"person\":{},\"approver\":{}}}",
                    q(&w.id.0),
                    q(&w.name),
                    q(&check.person_name),
                    q(&check.approver_fingerprint)
                );
            }
            let _ = writeln!(
                io.err,
                "Linked: this computer is one of {person}'s devices in {}. It acts as them; votes ask for their passkey.",
                one_line(&w.name)
            );
            Ok(())
        }
        Err(e) => backend_fail(e.0),
    }
}

/// The `link-status` line.
pub fn status_line(v: &LinkStatusView, now_ms: i64, utc_offset: i32) -> String {
    match (&v.linked_to, &v.pending) {
        (Some(p), _) => format!("Linked: one of {}'s devices", one_line(p)),
        (None, Some(p)) => pending_line(p, now_ms, utc_offset),
        (None, None) => "Not linked, and no link waiting (lt --plain link-request starts one)".into(),
    }
}

/// "Linking waiting for your desktop's code (started HH:MM; expires HH:MM)".
pub fn pending_line(p: &LinkPending, now_ms: i64, utc_offset: i32) -> String {
    let mut s = p.line(now_ms, |ms| hh_mm(ms, utc_offset));
    if p.count > 1 {
        s.push_str(&format!(" · {} requests waiting", p.count));
    }
    s
}

async fn status(b: &dyn Backend, rest: &[String], io: &mut Io<'_>) -> Result<(), Failure> {
    if let Some(a) = rest.first() {
        return fail(format!("link-status takes no argument ({a}): usage: link-status [--json]"), 2);
    }
    let v = match b.link_status().await {
        Ok(v) => v,
        Err(e) => return backend_fail(e.0),
    };
    if io.json {
        let pending = match &v.pending {
            Some(p) => format!(
                "{{\"startedAt\":{},\"approveBy\":{},\"expiresAt\":{},\"count\":{}}}",
                p.started_ms, p.approve_by_ms, p.expires_ms, p.count
            ),
            None => "null".into(),
        };
        let person = v.linked_to.as_deref().map(q).unwrap_or_else(|| "null".into());
        let _ = writeln!(
            io.out,
            "{{\"record\":\"link-status\",\"linked\":{},\"person\":{person},\"pending\":{pending},\"text\":{}}}",
            v.linked_to.is_some(),
            q(&status_line(&v, io.now_ms, io.utc_offset))
        );
    } else {
        let _ = writeln!(io.out, "{}", status_line(&v, io.now_ms, io.utc_offset));
    }
    Ok(())
}

/// The real terminal: stdin, `/dev/tty`, the platform's clipboard tools.
pub struct RealTerminal;

impl RealTerminal {
    fn tty_available() -> bool {
        use std::io::IsTerminal;
        io::stdin().is_terminal() || (cfg!(unix) && std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").is_ok())
    }
}

/// Raw mode (and bracketed paste) for as long as it lives.
struct Raw;

impl Raw {
    fn on() -> io::Result<Raw> {
        crossterm::terminal::enable_raw_mode()?;
        let _ = crossterm::execute!(io::stderr(), crossterm::event::EnableBracketedPaste);
        Ok(Raw)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = crossterm::execute!(io::stderr(), crossterm::event::DisableBracketedPaste);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Read a line in raw mode; `echo` writes typed characters back (never for
/// a code). `None` on Esc or ctrl-c.
fn read_raw_line(echo: bool, limit: usize) -> io::Result<Option<String>> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, read};
    let raw = Raw::on()?;
    let mut s = String::new();
    let mut err = io::stderr();
    loop {
        match read()? {
            Event::Paste(p) => {
                s.push_str(&p);
                if echo {
                    let _ = write!(err, "{}", crate::plain::clean(&p));
                }
            }
            Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Enter => break,
                KeyCode::Esc => {
                    drop(raw);
                    let _ = writeln!(err);
                    return Ok(None);
                }
                KeyCode::Char('c' | 'd') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    drop(raw);
                    let _ = writeln!(err);
                    return Ok(None);
                }
                KeyCode::Backspace => {
                    if s.pop().is_some() && echo {
                        let _ = write!(err, "\x08 \x08");
                    }
                }
                KeyCode::Char(c) => {
                    s.push(c);
                    if echo {
                        let _ = write!(err, "{c}");
                    }
                }
                _ => {}
            },
            _ => {}
        }
        let _ = err.flush();
        if s.len() > limit {
            break;
        }
    }
    drop(raw);
    let _ = writeln!(err);
    Ok(Some(s))
}

impl Terminal for RealTerminal {
    fn interactive(&self) -> bool {
        use std::io::IsTerminal;
        io::stderr().is_terminal() && Self::tty_available()
    }

    fn read_stdin(&mut self) -> io::Result<String> {
        use std::io::{IsTerminal, Read};
        if io::stdin().is_terminal() {
            return Err(io::Error::other(
                "stdin is a terminal (it would echo the code): pipe it in, or run lt --plain link-redeem with no code to paste it hidden",
            ));
        }
        let mut s = String::new();
        io::stdin().take(MAX_PASTE as u64).read_to_string(&mut s)?;
        Ok(s)
    }

    fn clipboard(&mut self) -> Option<String> {
        crate::clipboard::paste_native(MAX_PASTE)
    }

    fn copy(&mut self, text: &str) -> Option<String> {
        use std::io::IsTerminal;
        if let Some(tool) = crate::clipboard::copy_native(text) {
            return Some(format!("with {tool}"));
        }
        // The terminal's own clipboard (OSC 52), through stderr.
        if io::stderr().is_terminal() {
            let mut e = io::stderr();
            if e.write_all(&crate::clipboard::osc52(text, std::env::var_os("TMUX").is_some())).is_ok() && e.flush().is_ok() {
                return Some("through the terminal".into());
            }
        }
        None
    }

    fn ask_hidden(&mut self, prompt: &str) -> io::Result<Option<String>> {
        let mut err = io::stderr();
        write!(err, "{prompt}")?;
        err.flush()?;
        let got = read_raw_line(false, MAX_PASTE)?;
        if let Some(s) = &got
            && !s.trim().is_empty()
        {
            let _ = writeln!(err, "(read {} characters)", s.trim().chars().count());
        }
        Ok(got)
    }

    fn confirm(&mut self, prompt: &str) -> io::Result<bool> {
        let mut err = io::stderr();
        write!(err, "{prompt} [y/N] ")?;
        err.flush()?;
        let line = read_raw_line(true, 16)?.unwrap_or_default();
        Ok(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
    }
}
