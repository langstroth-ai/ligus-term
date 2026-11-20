//! Helpers for the real-backend tests: file secrets, the in-process relay,
//! a fake model, a software passkey playing the browser, and `lt` devices
//! with temporary data dirs. Nothing here touches the login keychain.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use axum::Router;
use axum::routing::post;
use futures::StreamExt;
use ligus_core::presence::soft::SoftAuthenticator;
use ligus_core::presence::unb64url;
use ligus_term::backend::model::*;
use ligus_term::backend::presence::{Opener, urlencode};
use ligus_term::backend::real::{RealBackend, RealConfig};
use ligus_term::backend::{Backend, Event};

static ENV: Once = Once::new();

/// File secrets for every device in this process: never the login keychain.
pub fn file_secrets() {
    ENV.call_once(|| unsafe { std::env::set_var("LIGUS_SECRET_BACKEND", "file") });
}

pub async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

/// An OpenAI-compatible model that streams one reply, word by word.
pub async fn fake_model() -> String {
    fake_model_counted().await.0
}

/// [`fake_model`], and how many requests it has had.
pub async fn fake_model_counted() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = calls.clone();
    let words = ["Hello ", "from ", "the ", "hive."];
    let body: String = words
        .iter()
        .map(|w| format!("data: {}\n\n", serde_json::json!({"choices": [{"index": 0, "delta": {"content": w}}]})))
        .chain(std::iter::once("data: [DONE]\n\n".to_string()))
        .collect();
    let router = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let body = body.clone();
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                axum::response::Response::builder().header("content-type", "text/event-stream").body(axum::body::Body::from(body)).unwrap()
            }
        }),
    );
    (format!("{}/v1/chat/completions", serve(router).await), calls)
}

/// Plays the system browser with a software passkey: reads the hand-off
/// page, runs `create` or `get` over its challenge and posts the result
/// back to the loopback, as the hand-off page does.
pub struct SoftBrowser(pub Arc<SoftAuthenticator>);

impl Opener for SoftBrowser {
    fn open(&self, page: &Path) -> std::io::Result<()> {
        let html = std::fs::read_to_string(page)?;
        let start = html.find("location.replace(\"").unwrap() + "location.replace(\"".len();
        let url = html[start..].split('"').next().unwrap().to_string();
        let frag = url.split_once('#').unwrap().1.to_string();
        let get = |k: &str| frag.split('&').find_map(|p| p.strip_prefix(&format!("{k}="))).unwrap_or_default().to_string();
        let (port, nonce, op) = (get("p"), get("n"), get("op"));
        let challenge = unb64url(&get("c")).unwrap();
        let soft = self.0.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let a = if op == "create" { soft.create(&challenge) } else { soft.get(&challenge) };
            let field = if op == "create" { "attestationObject" } else { "authenticatorData" };
            let body = format!(
                "credentialId={}&{field}={}&clientDataJSON={}&signature={}",
                urlencode(&a.credential_id),
                urlencode(&a.authenticator_data),
                urlencode(&a.client_data_json),
                urlencode(&a.signature)
            );
            let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            write!(
                s,
                "POST /{nonce} HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            let mut resp = String::new();
            let _ = s.read_to_string(&mut resp);
        });
        Ok(())
    }
}

/// One `lt` device and everything it emitted.
pub struct Device {
    pub b: RealBackend,
    pub events: Arc<Mutex<Vec<Event>>>,
    /// A recovery-words file to answer `joinApproved` prompts with.
    pub words_file: Arc<Mutex<Option<PathBuf>>>,
}

pub fn device(dir: &Path, name: &str, relay: &str) -> Device {
    device_with(dir, name, relay, |_| {})
}

/// A device, with `tune` adjusting its config first.
pub fn device_with(dir: &Path, name: &str, relay: &str, tune: impl FnOnce(&mut RealConfig)) -> Device {
    file_secrets();
    let mut cfg = RealConfig::new(dir.to_path_buf());
    cfg.device_name = name.into();
    cfg.relay_url = Some(relay.into());
    cfg.sync_every = Duration::from_millis(150);
    cfg.opener = Arc::new(SoftBrowser(Arc::new(SoftAuthenticator::es256())));
    cfg.presence_wait = Duration::from_secs(20);
    cfg.cli_fallback = false;
    cfg.beat_every = Duration::from_millis(300);
    tune(&mut cfg);
    let b = RealBackend::open(cfg).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let words_file: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let mut stream = b.events();
    let (b2, ev, wf) = (b.clone(), events.clone(), words_file.clone());
    tokio::spawn(async move {
        while let Some(e) = stream.next().await {
            if let Event::PresencePrompt(p) = &e {
                // The invite is confirmed with the recovery words; every
                // other prompt with the (software) passkey in the browser.
                let answer = match (p.kind.as_str(), wf.lock().unwrap().clone()) {
                    ("joinApproved", Some(path)) => PresenceAnswer::RecoveryWordsFile(path.display().to_string()),
                    _ => PresenceAnswer::Browser,
                };
                let (b3, id) = (b2.clone(), p.id);
                tokio::spawn(async move { b3.answer_presence(id, answer).await.unwrap() });
            }
            ev.lock().unwrap().push(e);
        }
    });
    Device { b, events, words_file }
}

impl Device {
    pub fn saw(&self, f: impl Fn(&Event) -> bool) -> bool {
        self.events.lock().unwrap().iter().any(f)
    }
}

/// Poll `f` until it returns `Some`, up to 30 s.
pub async fn until<T, F, Fut>(what: &str, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    for _ in 0..300 {
        if let Some(v) = f().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
