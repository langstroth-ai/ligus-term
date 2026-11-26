//! `lt link-request`, `lt link-redeem`, `lt link-status`: data on stdout,
//! everything else on stderr; the desktop's code from stdin, a hidden
//! paste or the clipboard; the desktop confirmed at a terminal, or by flag
//! (exit 4 without one).

use std::io;

use ligus_term::backend::Backend as _;
use ligus_term::backend::fake::{DEMO_APPROVER, DEMO_LINK_REQUEST, FakeBackend, FakeConfig};
use ligus_term::link_cli::{Io, Terminal, run};

const GRANT: &str = "ligusgrant1:eyJ2IjoxLCJkZW1vIjp0cnVlfQ";
const NOW: i64 = 1_795_615_350_000; // 14:02:30 UTC

#[derive(Default)]
struct FakeTerm {
    interactive: bool,
    stdin: Option<String>,
    clipboard: Option<String>,
    /// What the clipboard holds on each read before `clipboard`.
    clip_queue: std::collections::VecDeque<String>,
    hidden: Option<String>,
    answer: bool,
    copied: Vec<String>,
    asked: Vec<String>,
    copy_works: bool,
}

impl Terminal for FakeTerm {
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn read_stdin(&mut self) -> io::Result<String> {
        self.stdin.take().ok_or_else(|| io::Error::other("no stdin"))
    }
    fn clipboard(&mut self) -> Option<String> {
        self.clip_queue.pop_front().or_else(|| self.clipboard.clone())
    }
    fn copy(&mut self, text: &str) -> Option<String> {
        self.copied.push(text.into());
        self.copy_works.then(|| "with pbcopy".into())
    }
    fn ask_hidden(&mut self, prompt: &str) -> io::Result<Option<String>> {
        self.asked.push(prompt.into());
        Ok(self.hidden.clone())
    }
    fn confirm(&mut self, prompt: &str) -> io::Result<bool> {
        self.asked.push(prompt.into());
        Ok(self.answer)
    }
}

struct Ran {
    code: i32,
    out: String,
    err: String,
}

async fn lt(b: &FakeBackend, args: &[&str], json: bool, term: &mut FakeTerm) -> Ran {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut io = Io { out: &mut out, err: &mut err, term, json, now_ms: NOW, utc_offset: 0 };
    let code = run(b, &args, &mut io).await;
    Ran { code, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap() }
}

fn backend() -> FakeBackend {
    FakeBackend::new(FakeConfig::instant())
}

#[tokio::test]
async fn link_request_prints_only_the_code_on_stdout() {
    let b = backend();
    let r = lt(&b, &["link-request"], false, &mut FakeTerm::default()).await;
    assert_eq!(r.code, 0);
    assert_eq!(r.out, format!("{DEMO_LINK_REQUEST}\n"), "| pbcopy copies exactly the code");
    assert!(r.err.contains("Fingerprint: 7F3A-91C0-77DE-0B14-5E62"), "{}", r.err);
    assert!(r.err.contains("My devices > Add a computer"));
    assert!(r.err.contains("(by 14:17)"));
    assert!(r.err.contains("lt --plain link-redeem"));
}

#[tokio::test]
async fn link_request_json_and_copy() {
    let b = backend();
    let r = lt(&b, &["link-request"], true, &mut FakeTerm::default()).await;
    assert_eq!(r.code, 0);
    let v: serde_json::Value = serde_json::from_str(r.out.trim()).unwrap();
    assert_eq!(v["code"], DEMO_LINK_REQUEST);
    assert_eq!(v["fingerprint"], "7F3A-91C0-77DE-0B14-5E62");
    assert_eq!(v["expiresAt"], NOW + 15 * 60_000);
    assert_eq!(r.out.lines().count(), 1, "one object");

    let mut t = FakeTerm { copy_works: true, ..Default::default() };
    let r = lt(&b, &["link-request", "--copy"], false, &mut t).await;
    assert_eq!(r.code, 0);
    assert_eq!(t.copied, vec![DEMO_LINK_REQUEST.to_string()]);
    assert_eq!(r.out, "", "copied, so nothing on stdout");
    assert!(r.err.contains("Copied") && r.err.contains("Fingerprint:"), "{}", r.err);

    // No clipboard: the code is printed after all.
    let r = lt(&b, &["link-request", "--copy"], false, &mut FakeTerm::default()).await;
    assert_eq!(r.out, format!("{DEMO_LINK_REQUEST}\n"));
    assert!(r.err.contains("couldn't reach the clipboard"));

    let r = lt(&b, &["link-request", "--nope"], false, &mut FakeTerm::default()).await;
    assert_eq!(r.code, 2);
}

#[tokio::test]
async fn redeem_from_stdin_with_the_flag() {
    let b = backend();
    lt(&b, &["link-request"], false, &mut FakeTerm::default()).await;
    let mut t = FakeTerm { stdin: Some(format!("{GRANT}\n")), ..Default::default() };
    let r = lt(&b, &["link-redeem", "-", "--approver-fingerprint", DEMO_APPROVER], false, &mut t).await;
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(r.err.contains("Approved by Alex's desktop · fingerprint 2C41-88E0-1B9D-04AF-6E73 · for Northwind API"));
    assert!(r.err.contains("Linked"));
    assert!(!r.err.contains(GRANT) && !r.out.contains(GRANT), "the code is never printed");
}

#[tokio::test]
async fn without_a_terminal_the_flag_is_required() {
    let b = backend();
    let mut t = FakeTerm { stdin: Some(GRANT.into()), ..Default::default() };
    let r = lt(&b, &["link-redeem", "-"], false, &mut t).await;
    assert_eq!(r.code, 4, "a person must confirm the desktop");
    assert!(r.err.contains("--approver-fingerprint"));
    assert!(b.linked_person().is_none());
    // No code and no terminal: a usage error, nothing asked.
    let mut t = FakeTerm::default();
    let r = lt(&b, &["link-redeem"], false, &mut t).await;
    assert_eq!(r.code, 2);
    assert!(t.asked.is_empty());
    // The wrong desktop.
    let mut t = FakeTerm { stdin: Some(GRANT.into()), ..Default::default() };
    let r = lt(&b, &["link-redeem", "-", "--approver-fingerprint", "AAAA-BBBB"], false, &mut t).await;
    assert_eq!(r.code, 1);
    assert!(r.err.contains("don't use this code"));
}

#[tokio::test]
async fn at_a_terminal_it_reads_the_clipboard_and_asks() {
    let b = backend();
    // Enter at the hidden prompt: the clipboard, with what a copy adds.
    let mut t = FakeTerm { interactive: true, hidden: Some(String::new()), clipboard: Some(format!("  {GRANT}\n")), ..Default::default() };
    let r = lt(&b, &["link-redeem"], false, &mut t).await;
    assert_eq!(r.code, 1, "answered no");
    assert!(t.asked[0].contains("hidden"));
    assert!(t.asked[1].contains("Approved by Alex's desktop · fingerprint 2C41-88E0-1B9D-04AF-6E73 · for Northwind API"));
    assert!(t.asked[1].ends_with("Is that your desktop?"));
    assert!(b.linked_person().is_none(), "no: nothing linked");

    let mut t =
        FakeTerm { interactive: true, hidden: Some(String::new()), clipboard: Some(GRANT.into()), answer: true, ..Default::default() };
    let r = lt(&b, &["link-redeem"], false, &mut t).await;
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(b.linked_person().as_deref(), Some("Alex"));
    assert!(!r.err.contains(GRANT));
    let st = b.link_status().await.unwrap();
    assert!(st.pending.is_none());
}

#[tokio::test]
async fn a_hidden_paste_or_the_whole_command_works_too() {
    let b = backend();
    let pasted = format!("lt --plain link-redeem '{GRANT}' --approver-fingerprint {DEMO_APPROVER}");
    let mut t = FakeTerm { interactive: true, hidden: Some(pasted), answer: true, ..Default::default() };
    let r = lt(&b, &["link-redeem"], false, &mut t).await;
    assert_eq!(r.code, 0, "{}", r.err);
}

#[tokio::test]
async fn a_clipboard_without_a_code_says_what_to_copy() {
    let b = backend();
    for (clip, says) in [
        ("lt --plain link-redeem --approver-fingerprint 2C41", "Copy for lt"),
        ("hello", "doesn't hold a desktop code"),
        (DEMO_LINK_REQUEST, "this computer's request"),
    ] {
        let mut t = FakeTerm { interactive: true, hidden: Some(String::new()), clipboard: Some(clip.into()), ..Default::default() };
        let r = lt(&b, &["link-redeem"], false, &mut t).await;
        assert_eq!(r.code, 1);
        assert!(r.err.contains(says), "{clip}: {}", r.err);
    }
    // Cancelled at the prompt.
    let mut t = FakeTerm { interactive: true, hidden: None, ..Default::default() };
    let r = lt(&b, &["link-redeem"], false, &mut t).await;
    assert_eq!(r.code, 1);
    assert!(r.err.contains("cancelled"));
}

#[tokio::test]
async fn link_status_says_what_waits() {
    let b = backend();
    let r = lt(&b, &["link-status"], false, &mut FakeTerm::default()).await;
    assert_eq!(r.code, 0);
    assert!(r.out.starts_with("Not linked"));
    lt(&b, &["link-request"], false, &mut FakeTerm::default()).await;
    let r = lt(&b, &["link-status"], true, &mut FakeTerm::default()).await;
    let v: serde_json::Value = serde_json::from_str(r.out.trim()).unwrap();
    assert_eq!(v["linked"], false);
    assert_eq!(v["pending"]["count"], 1);
    assert!(v["text"].as_str().unwrap().starts_with("Linking waiting"));
}

/// The desktop's Copy command, then Copy for lt: the first Enter finds the
/// command on the clipboard and asks again; the second finds the code.
#[tokio::test]
async fn the_copied_command_then_the_code() {
    let b = backend();
    let mut t = FakeTerm {
        interactive: true,
        hidden: Some(String::new()),
        clip_queue: [format!("lt --plain link-redeem --approver-fingerprint {DEMO_APPROVER}")].into(),
        clipboard: Some(GRANT.into()),
        ..Default::default()
    };
    let r = lt(&b, &["link-redeem", "--approver-fingerprint", DEMO_APPROVER], false, &mut t).await;
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.err.contains("Copy for lt"));
    assert_eq!(t.asked.len(), 2, "asked twice, no y/N with the flag: {:?}", t.asked);
    assert!(t.asked[1].starts_with("Try again"));
}

/// The fingerprint as the desktop's copy button gives it, quoted or not.
#[tokio::test]
async fn a_fingerprint_with_spaces_matches() {
    for fp in [vec!["2C41 88E0 1B9D 04AF 6E73"], vec!["2c41", "88e0", "1b9d", "04af", "6e73"]] {
        let b = backend();
        let mut t = FakeTerm { stdin: Some(GRANT.into()), ..Default::default() };
        let mut args = vec!["link-redeem", "-", "--approver-fingerprint"];
        args.extend(fp);
        let r = lt(&b, &args, false, &mut t).await;
        assert_eq!(r.code, 0, "{}", r.err);
    }
}
