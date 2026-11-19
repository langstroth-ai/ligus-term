//! The passkey presence provider of the real backend.
//!
//! `ChatService` asks for presence from inside a write
//! ([`PresenceProvider::assert`]), on a blocking-pool thread. This provider
//! turns that into the terminal's prompt: it emits
//! [`Event::PresencePrompt`], waits for [`TermPresence::answer`], and then
//! either runs the browser hand-off on loopback ([`presence::hand_off`]) or
//! signs the challenge with the recovery key from a words file. A problem the
//! person can fix (an unreadable file, words that aren't a key) is reported
//! with [`Event::PresenceProblem`] and the prompt stays open.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ligus_core::presence::{PresenceAssertion, RecoveryKey, b64url, unb64url};
use ligus_runtime::chat_service::{PresenceProvider, PresenceRequest};
use tokio::sync::{broadcast, mpsc};

use crate::backend::Event;
use crate::backend::model::{PresenceAnswer, PresenceId, PresenceOp, PresencePrompt};
use crate::backend::presence::{self, HandOff, HandOffError, Opener};

pub struct TermPresence {
    tx: broadcast::Sender<Event>,
    pending: Mutex<HashMap<PresenceId, mpsc::UnboundedSender<PresenceAnswer>>>,
    next: AtomicU64,
    rt: tokio::runtime::Handle,
    presence_dir: PathBuf,
    opener: Arc<dyn Opener>,
    wait: Duration,
}

/// What a ceremony asks the browser for.
enum Ask<'a> {
    Get,
    Create { user_name: &'a str },
}

impl TermPresence {
    pub fn new(
        tx: broadcast::Sender<Event>,
        rt: tokio::runtime::Handle,
        presence_dir: PathBuf,
        opener: Arc<dyn Opener>,
        wait: Duration,
    ) -> Self {
        Self { tx, pending: Mutex::new(HashMap::new()), next: AtomicU64::new(1), rt, presence_dir, opener, wait }
    }

    /// Deliver the person's answer to prompt `id`.
    pub fn answer(&self, id: PresenceId, answer: PresenceAnswer) -> Result<(), String> {
        let tx = self.pending.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned();
        match tx {
            Some(tx) => tx.send(answer).map_err(|_| "that passkey prompt has ended".to_string()),
            None => Err("that passkey prompt has ended".into()),
        }
    }

    /// Confirm `req` (`navigator.credentials.get`, or the recovery key).
    /// Blocking: call it from a blocking-pool thread.
    pub fn confirm(&self, req: &PresenceRequest) -> Result<PresenceAssertion, String> {
        self.ceremony(req, Ask::Get)
    }

    /// Create a passkey over `req.challenge` (`navigator.credentials.create`).
    /// The attestation object rides in `authenticator_data`. Blocking.
    pub fn create(&self, req: &PresenceRequest, user_name: &str) -> Result<PresenceAssertion, String> {
        self.ceremony(req, Ask::Create { user_name })
    }

    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
    }

    fn ceremony(&self, req: &PresenceRequest, ask: Ask<'_>) -> Result<PresenceAssertion, String> {
        let id = PresenceId(self.next.fetch_add(1, Ordering::Relaxed));
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        let op = match ask {
            Ask::Get => PresenceOp::Get,
            Ask::Create { .. } => PresenceOp::Create,
        };
        self.emit(Event::PresencePrompt(PresencePrompt {
            id,
            kind: req.kind.clone(),
            summary: req.summary.clone(),
            digest_short: req.digest_short.clone(),
            op,
        }));
        let outcome = self.rt.block_on(self.run(id, req, &ask, &mut rx));
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        self.emit(Event::PresenceDone { id, ok: outcome.is_ok(), error: outcome.as_ref().err().cloned() });
        outcome
    }

    async fn run(
        &self,
        id: PresenceId,
        req: &PresenceRequest,
        ask: &Ask<'_>,
        rx: &mut mpsc::UnboundedReceiver<PresenceAnswer>,
    ) -> Result<PresenceAssertion, String> {
        let mut next = rx.recv().await;
        loop {
            match next.take() {
                None | Some(PresenceAnswer::Cancel) => return Err("cancelled".into()),
                Some(PresenceAnswer::RecoveryWordsFile(path)) => {
                    match self.recovery(req, ask, &path) {
                        Ok(a) => return Ok(a),
                        Err(Fatal(e)) => return Err(e),
                        // Stay on the sheet so the person can fix it.
                        Err(Fixable(e)) => self.emit(Event::PresenceProblem { id, error: e }),
                    }
                    next = rx.recv().await;
                }
                Some(PresenceAnswer::Browser) => {
                    let mut interrupt = None;
                    let r = presence::hand_off(
                        &self.presence_dir,
                        &req.policy.page_url(),
                        &fragment(req, ask),
                        self.opener.as_ref(),
                        async { interrupt = rx.recv().await },
                        self.wait,
                    )
                    .await;
                    match r {
                        Ok(h) => return assertion(req, ask, h),
                        Err(HandOffError::Cancelled) => next = interrupt,
                        Err(e) => {
                            // The person can retry, or use the recovery words.
                            self.emit(Event::PresenceProblem { id, error: e.to_string() });
                            next = rx.recv().await;
                        }
                    }
                }
            }
        }
    }

    fn recovery(&self, req: &PresenceRequest, ask: &Ask<'_>, path: &str) -> Result<PresenceAssertion, Problem> {
        if matches!(ask, Ask::Create { .. }) {
            return Err(Fatal("a recovery key doesn't create a passkey".into()));
        }
        let words = presence::read_recovery_words_file(std::path::Path::new(path)).map_err(Fixable)?;
        let key = RecoveryKey::from_words(&words.join(" ")).map_err(|e| Fixable(format!("those words aren't a recovery key: {}", e.0)))?;
        // The recovery key signs the very challenge requested.
        Ok(key.assert(&req.challenge))
    }
}

enum Problem {
    Fatal(String),
    Fixable(String),
}
use Problem::{Fatal, Fixable};

/// The hand-off page's fragment, as the desktop builds it.
fn fragment(req: &PresenceRequest, ask: &Ask<'_>) -> String {
    match ask {
        Ask::Get => presence::get_fragment(&req.policy.rp_id, &b64url(&req.challenge), &req.credential_ids, &req.kind, &req.summary),
        Ask::Create { user_name } => format!(
            "op=create&rp={}&c={}&u={}&un={}&x={}",
            presence::urlencode(&req.policy.rp_id),
            b64url(&req.challenge),
            b64url(req.account_id.as_bytes()),
            presence::urlencode(user_name),
            presence::urlencode(&req.credential_ids.join(",")),
        ),
    }
}

/// The challenge a returned `clientDataJSON` names.
fn returned_challenge(client_data_b64: &str) -> Option<Vec<u8>> {
    let raw = unb64url(client_data_b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    unb64url(v.get("challenge")?.as_str()?).ok()
}

/// Check the browser answered our challenge, and shape its fields.
fn assertion(req: &PresenceRequest, ask: &Ask<'_>, h: HandOff) -> Result<PresenceAssertion, String> {
    if returned_challenge(&h.client_data_json).as_deref() != Some(&req.challenge[..]) {
        return Err("the browser answered a different challenge".into());
    }
    Ok(match ask {
        Ask::Get => PresenceAssertion {
            credential_id: h.credential_id,
            authenticator_data: h.authenticator_data,
            client_data_json: h.client_data_json,
            signature: h.signature,
        },
        Ask::Create { .. } => PresenceAssertion {
            credential_id: h.credential_id,
            authenticator_data: h.attestation_object,
            client_data_json: h.client_data_json,
            signature: String::new(),
        },
    })
}

/// The handle `ChatService` holds.
pub struct Provider(pub Arc<TermPresence>);

impl PresenceProvider for Provider {
    fn assert(&self, req: &PresenceRequest) -> Result<PresenceAssertion, String> {
        self.0.confirm(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoBrowser;
    impl Opener for NoBrowser {
        fn open(&self, _: &std::path::Path) -> std::io::Result<()> {
            Err(std::io::Error::other("no browser here"))
        }
    }

    fn request() -> PresenceRequest {
        PresenceRequest {
            challenge: [7u8; 32],
            kind: "memberRemoved".into(),
            account_id: uuid::Uuid::new_v4(),
            credential_ids: vec![],
            digest_short: "0707070707070707".into(),
            summary: "Remove member x".into(),
            policy: ligus_core::presence::PresencePolicy::ligus(),
        }
    }

    fn words_file(dir: &std::path::Path, text: &str) -> String {
        let p = dir.join("words.txt");
        std::fs::write(&p, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        p.display().to_string()
    }

    /// Bad words and a missing browser are fixable problems: the prompt stays
    /// open until good words sign the very challenge, or the person cancels.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_recovery_words_sign_the_challenge_after_fixable_problems() {
        let tmp = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(64);
        let p = Arc::new(TermPresence::new(
            tx,
            tokio::runtime::Handle::current(),
            tmp.path().join("presence"),
            Arc::new(NoBrowser),
            Duration::from_secs(5),
        ));
        let key = RecoveryKey::generate().unwrap();
        let bad = words_file(tmp.path(), &"bee ".repeat(24));
        let p2 = p.clone();
        let asked = tokio::task::spawn_blocking(move || p2.confirm(&request()));
        let mut problems = 0;
        while let Ok(e) = rx.recv().await {
            match e {
                Event::PresencePrompt(pr) => {
                    assert_eq!(pr.op, PresenceOp::Get);
                    p.answer(pr.id, PresenceAnswer::Browser).unwrap();
                    p.answer(pr.id, PresenceAnswer::RecoveryWordsFile(bad.clone())).unwrap();
                }
                Event::PresenceProblem { id, .. } => {
                    problems += 1;
                    if problems == 2 {
                        let good = words_file(tmp.path(), &key.words());
                        p.answer(id, PresenceAnswer::RecoveryWordsFile(good)).unwrap();
                    }
                }
                Event::PresenceDone { ok, .. } => {
                    assert!(ok);
                    break;
                }
                _ => {}
            }
        }
        let a = asked.await.unwrap().unwrap();
        assert_eq!(a, key.assert(&[7u8; 32]));
        assert_eq!(problems, 2, "the missing browser and the bad words");

        // A recovery key never creates a passkey; cancel ends quietly.
        let p3 = p.clone();
        let asked = tokio::task::spawn_blocking(move || p3.create(&request(), "Ana"));
        while let Ok(e) = rx.recv().await {
            if let Event::PresencePrompt(pr) = e {
                p.answer(pr.id, PresenceAnswer::Cancel).unwrap();
                break;
            }
        }
        assert_eq!(asked.await.unwrap().unwrap_err(), "cancelled");
    }
}
