//! Passkey presence from a terminal.
//!
//! The desktop asks for a passkey through the system browser: it listens on
//! `127.0.0.1:<random port>` with a one-time nonce, writes a private redirect
//! page under the data dir (so the nonce never appears in a command line any
//! local process can list), and opens that page, which forwards to
//! `<workspace origin>/presence#op=get|create&rp=…&c=…&p=<port>&n=<nonce>`
//! (`https://ligus.ai/presence` by default). The page runs the WebAuthn
//! ceremony and submits the result as a top-level form `POST /<nonce>` to
//! the loopback.
//!
//! `lt` does exactly the same here, so the hand-off page needs no change.
//! What differs is the confirmation sheet: the terminal shows the kind,
//! summary and digest short id, and offers the recovery-words file (the
//! CLI's `--recovery-words-file`) when no browser is at hand (SSH, a
//! headless box).
//!
//! This module is transport only: it returns the browser's fields. The
//! backend checks the returned challenge is its own, and the fold verifies
//! the assertion again on every peer.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// How long `lt` waits for the browser (the desktop's value).
pub const WAIT: Duration = Duration::from_secs(180);

/// The default hand-off page (`PresencePolicy::ligus().page_url()`).
pub const DEFAULT_PAGE_URL: &str = "https://ligus.ai/presence";

/// Caps on one loopback request: a line, the header count, the header
/// block and the form body (the desktop's B-12 limits).
const MAX_LINE: u64 = 4 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_HEAD: u64 = 16 * 1024;
const MAX_BODY: usize = 64 * 1024;
/// Loopback connections read at once; more are dropped unread.
const MAX_CONNECTIONS: usize = 16;

/// What the hand-off page submits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HandOff {
    pub credential_id: String,
    pub authenticator_data: String,
    pub client_data_json: String,
    pub signature: String,
    pub attestation_object: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandOffError {
    Cancelled,
    TimedOut,
    Browser(String),
    Io(String),
}

impl std::fmt::Display for HandOffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandOffError::Cancelled => f.write_str("cancelled"),
            HandOffError::TimedOut => f.write_str("timed out waiting for the passkey"),
            HandOffError::Browser(e) => write!(f, "couldn't open the browser: {e}"),
            HandOffError::Io(e) => f.write_str(e),
        }
    }
}

/// Opens a local file in the system browser. A trait so tests (and SSH
/// sessions, which print the path instead) can stand in.
pub trait Opener: Send + Sync {
    fn open(&self, page: &Path) -> io::Result<()>;
    /// Open the page whose address is `url`. The default opens the private
    /// redirect page; an opener that shows the address for a browser on
    /// another device (no browser here) uses `url`.
    fn open_url(&self, page: &Path, _url: &str) -> io::Result<()> {
        self.open(page)
    }
}

/// `open` on macOS, `xdg-open` elsewhere. The argument is the redirect
/// page's path, never the URL with the nonce.
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, page: &Path) -> io::Result<()> {
        let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let status = std::process::Command::new(cmd)
            .arg(page)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        if status.success() { Ok(()) } else { Err(io::Error::other(format!("{cmd} exited with {status}"))) }
    }
}

/// Percent-encode a fragment value (RFC 3986 unreserved kept).
pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Decode a form value (`+` is a space).
pub fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => {
                    out.push(v);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The fragment for a confirm (`navigator.credentials.get`), as the desktop
/// builds it. `challenge_b64` is the base64url digest.
pub fn get_fragment(rp_id: &str, challenge_b64: &str, credential_ids: &[String], kind: &str, summary: &str) -> String {
    format!(
        "op=get&rp={}&c={}&cred={}&k={}&s={}",
        urlencode(rp_id),
        challenge_b64,
        urlencode(&credential_ids.join(",")),
        urlencode(kind),
        urlencode(summary)
    )
}

/// 16 random bytes from the OS, base64url (no padding).
fn nonce() -> io::Result<String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b64url(&b))
}

fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, &v)| acc | (v as u32) << (16 - 8 * i));
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// The private redirect page (0600, in a 0700 dir under the data dir). It
/// forwards to `url`, fragment and all, and is deleted when the wait ends.
pub fn redirect_page(dir: &Path, url: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let js = url.replace('\\', "\\\\").replace('"', "\\\"").replace('<', "\\u003c");
    let attr = url.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
    let page = format!(
        "<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer>\
         <meta http-equiv=refresh content=\"0;url={attr}\"><title>Ligus</title>\
         <script>location.replace(\"{js}\")</script>"
    );
    let path = dir.join(format!("presence-{}.html", nonce()?.replace('-', "a").replace('_', "b")));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    opts.open(&path)?.write_all(page.as_bytes())?;
    Ok(path)
}

/// Run one ceremony: listen on `127.0.0.1:port` (0: any free port), open
/// the browser, wait for `POST /<nonce>`. `cancel` resolving stops the wait
/// (the sheet's Cancel, or the person choosing the recovery-words file
/// instead).
pub async fn hand_off(
    presence_dir: &Path,
    page_url: &str,
    fragment: &str,
    opener: &dyn Opener,
    port: u16,
    cancel: impl std::future::Future<Output = ()>,
    wait: Duration,
) -> Result<HandOff, HandOffError> {
    let io = |e: io::Error| HandOffError::Io(e.to_string());
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| HandOffError::Io(format!("couldn't listen on 127.0.0.1:{port} for the passkey page: {e}")))?;
    let port = listener.local_addr().map_err(io)?.port();
    let nonce = nonce().map_err(io)?;
    let url = format!("{page_url}#{fragment}&p={port}&n={nonce}");
    let page = redirect_page(presence_dir, &url).map_err(|e| HandOffError::Io(format!("couldn't prepare the passkey page: {e}")))?;
    if let Err(e) = opener.open_url(&page, &url) {
        let _ = std::fs::remove_file(&page);
        return Err(HandOffError::Browser(e.to_string()));
    }
    // Each connection is read on its own task (each capped at 5 s), so a
    // local process holding idle connections open can't keep the browser's
    // submission waiting; at most MAX_CONNECTIONS are read at once.
    let accept = async {
        let nonce: std::sync::Arc<str> = nonce.as_str().into();
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<HandOff>(1);
        loop {
            tokio::select! {
                Some(r) = rx.recv() => return Ok(r),
                accepted = listener.accept() => {
                    let (stream, _) = accepted.map_err(io)?;
                    let Ok(slot) = slots.clone().try_acquire_owned() else { continue };
                    let (tx, nonce) = (tx.clone(), nonce.clone());
                    tokio::spawn(async move {
                        if let Some(r) = read_submission(stream, &nonce).await {
                            let _ = tx.send(r).await;
                        }
                        drop(slot);
                    });
                }
            }
        }
    };
    let result = tokio::select! {
        r = accept => r,
        _ = cancel => Err(HandOffError::Cancelled),
        _ = tokio::time::sleep(wait) => Err(HandOffError::TimedOut),
    };
    let _ = std::fs::remove_file(&page);
    result
}

/// One loopback request. Anything but `POST /<nonce>` with a form body is
/// answered 404 and ignored.
async fn read_submission(mut stream: TcpStream, nonce: &str) -> Option<HandOff> {
    let body = tokio::time::timeout(Duration::from_secs(5), parse_submission(&mut stream, nonce)).await.ok().and_then(|r| r.ok());
    let Some(body) = body else {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        return None;
    };
    let page = "<!doctype html><meta charset=utf-8><title>Ligus</title><p style=\"font-family:system-ui\">Done. You can close this tab and return to your terminal.</p>";
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        page.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(page.as_bytes()).await;
    Some(parse_form(&body))
}

/// The form fields the page submits.
pub fn parse_form(body: &[u8]) -> HandOff {
    let mut r = HandOff::default();
    for pair in String::from_utf8_lossy(body).split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = urldecode(v);
        match k {
            "credentialId" => r.credential_id = v,
            "authenticatorData" => r.authenticator_data = v,
            "clientDataJSON" => r.client_data_json = v,
            "signature" => r.signature = v,
            "attestationObject" => r.attestation_object = v,
            _ => {}
        }
    }
    r
}

async fn capped_line<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<String, Rejected> {
    let mut buf = Vec::new();
    let mut limited = reader.take(MAX_LINE);
    limited.read_until(b'\n', &mut buf).await.map_err(|_| Rejected)?;
    if buf.last() != Some(&b'\n') {
        return Err(Rejected);
    }
    String::from_utf8(buf).map_err(|_| Rejected)
}

/// A loopback request that isn't the hand-off's answer (wrong path or
/// method, too large, malformed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejected;

/// The body of a `POST /<nonce>`, reading at most the capped head + body.
pub async fn parse_submission<S: tokio::io::AsyncRead + Unpin>(stream: S, nonce: &str) -> Result<Vec<u8>, Rejected> {
    let mut reader = BufReader::new(stream.take(MAX_HEAD + MAX_BODY as u64));
    let line = capped_line(&mut reader).await?;
    let ok_path = format!("POST /{nonce} ");
    let mut len = 0usize;
    let mut head = line.len() as u64;
    let mut ended = false;
    for _ in 0..MAX_HEADERS {
        let h = capped_line(&mut reader).await?;
        head += h.len() as u64;
        if head > MAX_HEAD {
            return Err(Rejected);
        }
        if h == "\r\n" || h == "\n" {
            ended = true;
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    if !ended || !line.starts_with(&ok_path) || len == 0 || len > MAX_BODY {
        return Err(Rejected);
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await.map_err(|_| Rejected)?;
    Ok(body)
}

/// Read a secret from a file: a regular file (never a FIFO or a device,
/// which could block or stream forever), owner-only on Unix, and at most
/// `max` bytes (never read into memory whole when it is bigger). A link is
/// followed (a mounted secret is often one); the file opened is the one
/// checked.
pub fn read_owner_only(path: &Path, max: u64) -> Result<String, String> {
    use std::io::Read;
    let shown = path.display();
    // Checked before opening: opening a FIFO for reading blocks.
    let before = std::fs::metadata(path).map_err(|e| format!("reading {shown}: {e}"))?;
    if !before.is_file() {
        return Err(format!("{shown} isn't a regular file"));
    }
    let f = std::fs::File::open(path).map_err(|e| format!("reading {shown}: {e}"))?;
    let meta = f.metadata().map_err(|e| format!("reading {shown}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{shown} isn't a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(format!("others can read {shown}: a secret file must be owner-only (chmod 600 it)"));
        }
    }
    if meta.len() > max {
        return Err(format!("{shown} is too big for a secret"));
    }
    let mut text = String::new();
    f.take(max + 1).read_to_string(&mut text).map_err(|e| format!("reading {shown}: {e}"))?;
    if text.len() as u64 > max {
        return Err(format!("{shown} is too big for a secret"));
    }
    Ok(text)
}

/// Read a recovery-words file (the CLI's `--recovery-words-file`): 24
/// whitespace-separated words. The real backend then checks the BIP-39
/// checksum (`RecoveryKey::from_words`); this only checks the shape and
/// that the file isn't readable by others.
pub fn read_recovery_words_file(path: &Path) -> Result<Vec<String>, String> {
    let text = read_owner_only(path, 4 * 1024)?;
    let words: Vec<String> = text.split_whitespace().map(|w| w.to_lowercase()).collect();
    if words.len() != 24 {
        return Err(format!("a recovery key is 24 words; {} has {}", path.display(), words.len()));
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Plays the browser: reads the redirect page, finds the port and nonce,
    /// and posts a form back, as the hand-off page would.
    struct FakeBrowser {
        seen_url: Mutex<Option<String>>,
        wrong_nonce_first: bool,
    }

    impl Opener for FakeBrowser {
        fn open(&self, page: &Path) -> io::Result<()> {
            let html = std::fs::read_to_string(page)?;
            let start = html.find("location.replace(\"").unwrap() + "location.replace(\"".len();
            let url = html[start..].split('"').next().unwrap().to_string();
            *self.seen_url.lock().unwrap() = Some(url.clone());
            let frag = url.split_once('#').unwrap().1.to_string();
            let get = |k: &str| frag.split('&').find_map(|p| p.strip_prefix(&format!("{k}="))).unwrap().to_string();
            let (port, nonce) = (get("p"), get("n"));
            let wrong = self.wrong_nonce_first;
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                let post = |n: &str| {
                    let body = "credentialId=abc&clientDataJSON=e30&signature=c2ln&authenticatorData=YXV0aA";
                    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
                    write!(s, "POST /{n} HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
                    let mut resp = String::new();
                    let _ = s.read_to_string(&mut resp);
                    resp
                };
                if wrong {
                    assert!(post("not-the-nonce").starts_with("HTTP/1.1 404"));
                }
                assert!(post(&nonce).starts_with("HTTP/1.1 200"));
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn hand_off_round_trip_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let pdir = dir.path().join("presence");
        let browser = FakeBrowser { seen_url: Mutex::new(None), wrong_nonce_first: true };
        let frag = get_fragment("ligus.ai", "Y2hhbA", &["cred1".into()], "memberRemoved", "Remove Ada");
        let r = hand_off(&pdir, DEFAULT_PAGE_URL, &frag, &browser, 0, std::future::pending(), Duration::from_secs(10)).await.unwrap();
        assert_eq!(r.credential_id, "abc");
        assert_eq!(r.signature, "c2ln");
        let url = browser.seen_url.lock().unwrap().clone().unwrap();
        assert!(url.starts_with("https://ligus.ai/presence#op=get&rp=ligus.ai&c=Y2hhbA"), "{url}");
        assert!(url.contains("&s=Remove%20Ada&p="));
        // The redirect page is gone; the dir stays private.
        assert_eq!(std::fs::read_dir(&pdir).unwrap().count(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&pdir).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    struct NoBrowser;
    impl Opener for NoBrowser {
        fn open(&self, _: &Path) -> io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancel_and_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let r = hand_off(dir.path(), DEFAULT_PAGE_URL, "op=get", &NoBrowser, 0, async {}, Duration::from_secs(10)).await;
        assert_eq!(r, Err(HandOffError::Cancelled));
        let r = hand_off(dir.path(), DEFAULT_PAGE_URL, "op=get", &NoBrowser, 0, std::future::pending(), Duration::from_millis(20)).await;
        assert_eq!(r, Err(HandOffError::TimedOut));
    }

    /// A local process that connects and sends nothing must not hold up the
    /// browser's submission (each stalled connection used to cost the whole
    /// wait 5 s, one after another).
    #[tokio::test]
    async fn a_stalled_connection_doesnt_block_the_submission() {
        struct Staller(FakeBrowser);
        impl Opener for Staller {
            fn open(&self, page: &Path) -> io::Result<()> {
                let html = std::fs::read_to_string(page)?;
                let port = html.split("&p=").nth(1).unwrap().split('&').next().unwrap().to_string();
                // Three idle connections first, held open.
                let idle: Vec<_> = (0..3).map(|_| std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap()).collect();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(8));
                    drop(idle);
                });
                std::thread::sleep(Duration::from_millis(100));
                self.0.open(page)
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let browser = Staller(FakeBrowser { seen_url: Mutex::new(None), wrong_nonce_first: false });
        let started = std::time::Instant::now();
        let r = hand_off(dir.path(), DEFAULT_PAGE_URL, "op=get", &browser, 0, std::future::pending(), Duration::from_secs(4)).await;
        assert_eq!(r.map(|h| h.credential_id), Ok("abc".to_string()), "after {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn submission_caps_and_path() {
        let ok = b"POST /n1 HTTP/1.1\r\nContent-Length: 3\r\n\r\na=b".to_vec();
        assert_eq!(parse_submission(&ok[..], "n1").await, Ok(b"a=b".to_vec()));
        assert!(parse_submission(&ok[..], "other").await.is_err());
        let get = b"GET /n1 HTTP/1.1\r\nContent-Length: 3\r\n\r\na=b".to_vec();
        assert!(parse_submission(&get[..], "n1").await.is_err());
        let long = format!("POST /n1 HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(5000));
        assert!(parse_submission(long.as_bytes(), "n1").await.is_err());
        let huge = format!("POST /n1 HTTP/1.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1);
        assert!(parse_submission(huge.as_bytes(), "n1").await.is_err());
    }

    #[test]
    fn url_codec() {
        assert_eq!(urlencode("a b&c"), "a%20b%26c");
        assert_eq!(urldecode("a+b%26c%2"), "a b&c%2");
        assert_eq!(b64url(b"hi?"), "aGk_");
        assert_eq!(b64url(b"h"), "aA");
    }

    #[test]
    fn recovery_words_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("words");
        std::fs::write(&p, "abandon ".repeat(24)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(read_recovery_words_file(&p).unwrap_err().contains("chmod 600"));
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert_eq!(read_recovery_words_file(&p).unwrap().len(), 24);
        std::fs::write(&p, "one two").unwrap();
        assert!(read_recovery_words_file(&p).unwrap_err().contains("24 words"));
    }

    /// A secret file is read bounded, and only when it is a regular file: a
    /// huge file isn't read into memory, a FIFO or a device never blocks or
    /// streams forever.
    #[cfg(unix)]
    #[test]
    fn secret_files_are_regular_owner_only_and_small() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big");
        std::fs::File::create(&big).unwrap().set_len(256 * 1024 * 1024).unwrap();
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_recovery_words_file(&big).unwrap_err().contains("too big"));
        assert!(crate::plain::read_secret_file(big.to_str().unwrap()).unwrap_err().contains("too big"));
        let fifo = dir.path().join("fifo");
        assert!(std::process::Command::new("mkfifo").arg("-m").arg("600").arg(&fifo).status().unwrap().success());
        let (tx, rx) = std::sync::mpsc::channel();
        let (f1, f2) = (fifo.clone(), fifo.clone());
        std::thread::spawn(move || {
            let a = read_recovery_words_file(&f1);
            let b = crate::plain::read_secret_file(f2.to_str().unwrap());
            let _ = tx.send((a, b));
        });
        let (a, b) = rx.recv_timeout(std::time::Duration::from_secs(3)).expect("reading a FIFO blocked");
        assert!(a.unwrap_err().contains("regular file"));
        assert!(b.unwrap_err().contains("regular file"));
    }
}
