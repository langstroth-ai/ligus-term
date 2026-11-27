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
//!
//! During a setup run ([`TermPresence::offer_handoff`]) a write the
//! workspace's rules hold for the person's passkey goes to their own
//! desktop or phone first (the passkey hand-off): the drafted change is
//! posted to the workspace's approval mailbox, the sheet shows its code,
//! and the answer comes back as the passkey's assertion over the draft's
//! challenge ([`PresenceProvider::assert_draft`]). Only answers signed by
//! the person's own devices count. Esc sends the cancel abort; the browser
//! and the recovery-words file stay one key away (they end the hand-off
//! first). A relay without the mailbox, or a change too large for it,
//! keeps today's sheet. A hand-off left waiting when lt quits stays in
//! `presence-pending.json` and is offered on the next start
//! ([`TermPresence::wait_resumed`]).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ligus_core::presence::{PresenceAssertion, RecoveryKey, b64url, unb64url};
use ligus_core::{SessionEvent, SessionEventEnvelope};
use ligus_runtime::approval_mailbox::{
    self as am, MailboxClient, MailboxError, PresenceAsk, PresenceOutcome, PresenceRequester, RequesterError, Signer,
};
use ligus_runtime::chat_service::{PresenceProvider, PresenceRequest};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::backend::Event;
use crate::backend::model::{HandoffUpdate, PresenceAnswer, PresenceId, PresenceOp, PresencePrompt};
use crate::backend::presence::{self, HandOff, HandOffError, Opener};

/// How long lt waits for the person's desktop or phone by default (the
/// CLI's; the request itself lives 15 minutes).
pub const HANDOFF_WAIT: Duration = Duration::from_secs(10 * 60);

/// What a declined hand-off says (Not me, or ended on the other device).
pub const DECLINED: &str = "Declined on your other device: nothing was written";

/// What the hand-off needs, read before a setup run starts (the provider
/// runs while the run holds the write service, so it can't read it then).
pub(crate) struct HandoffCtx {
    pub data_dir: PathBuf,
    pub ws: Uuid,
    pub relay: String,
    pub token: Option<String>,
    pub founder_pin: String,
    /// The workspace key epochs this device holds (secrets).
    pub epochs: Vec<am::EpochKey>,
    /// This account's own devices in the workspace: the only ones whose
    /// answer counts.
    pub mine: HashSet<Uuid>,
    /// This device's signing key seed (the draft's signer).
    pub device_seed: Zeroizing<[u8; 32]>,
    /// This computer's name, as a hint for the approver.
    pub label: String,
    /// Tests: the mailbox's clock.
    pub clock: Option<am::Clock>,
}

impl HandoffCtx {
    fn client(&self) -> MailboxClient {
        mailbox_client(&self.relay, self.token.clone(), self.clock.clone())
    }

    fn signer(&self) -> Signer {
        device_signer(&self.device_seed)
    }
}

pub(crate) fn mailbox_client(relay: &str, token: Option<String>, clock: Option<am::Clock>) -> MailboxClient {
    let c = MailboxClient::new(relay).with_auth(token);
    match clock {
        Some(clock) => c.with_clock(clock),
        None => c,
    }
}

pub(crate) fn device_signer(seed: &[u8; 32]) -> Signer {
    Signer::Ed25519(ligus_core::crypto::SigningKeypair::from_seed(seed).expect("a 32-byte seed"))
}

/// The draft a ceremony may send to the person's other device.
struct Elsewhere<'a> {
    ctx: &'a HandoffCtx,
    draft: &'a SessionEventEnvelope,
}

/// What a ceremony does once a hand-off ended.
enum After {
    Answer(Option<PresenceAnswer>),
    Wait,
    Done(Result<PresenceAssertion, String>),
}

/// Why a hand-off couldn't be posted.
enum PostError {
    /// Not here (no mailbox routes, the change too large to send): today's
    /// sheet stands, unchanged.
    Unavailable,
    /// Something the person should know (the relay unreachable, too many
    /// unanswered requests lately): today's sheet, with this line.
    Problem(String),
}

/// How waiting for the other device ended.
enum Step {
    /// The person answered here (Esc, the browser, the words file), or the
    /// prompt went away.
    Here(Option<PresenceAnswer>),
    Approved(PresenceAssertion),
    Declined,
    /// No answer: why; `cancel` when the request should be ended (it is
    /// still live on the relay).
    Stopped {
        why: String,
        cancel: bool,
    },
}

pub struct TermPresence {
    tx: broadcast::Sender<Event>,
    pending: Mutex<HashMap<PresenceId, mpsc::UnboundedSender<PresenceAnswer>>>,
    next: AtomicU64,
    rt: tokio::runtime::Handle,
    presence_dir: PathBuf,
    opener: Arc<dyn Opener>,
    wait: Duration,
    /// The loopback port the hand-off listens on (0: any free one).
    port: u16,
    /// Show the hand-off page's address instead of opening a browser here
    /// (for a browser on another device that reaches this loopback).
    no_browser: bool,
    /// The passkey hand-off a setup run offers, for the thread it runs on.
    handoff: Mutex<Option<(std::thread::ThreadId, Arc<HandoffCtx>)>>,
    /// How long a hand-off waits for the other device.
    handoff_wait: Duration,
    /// The hand-offs this process is waiting on (by request id): not
    /// offered again as left waiting.
    live: Mutex<HashSet<String>>,
}

/// While held, writes on this thread that need a passkey go to the
/// person's desktop or phone first. Dropped: today's sheet again.
pub(crate) struct HandoffOffer<'a>(&'a TermPresence);

impl Drop for HandoffOffer<'_> {
    fn drop(&mut self) {
        *self.0.handoff.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
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
        Self {
            tx,
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            rt,
            presence_dir,
            opener,
            wait,
            port: 0,
            no_browser: false,
            handoff: Mutex::new(None),
            handoff_wait: HANDOFF_WAIT,
            live: Mutex::new(HashSet::new()),
        }
    }

    /// How long a passkey hand-off waits for the other device.
    pub fn handoff_wait(mut self, wait: Duration) -> Self {
        self.handoff_wait = wait;
        self
    }

    /// Offer the passkey hand-off to writes on this thread until the
    /// returned guard drops.
    pub(crate) fn offer_handoff(&self, ctx: HandoffCtx) -> HandoffOffer<'_> {
        *self.handoff.lock().unwrap_or_else(|e| e.into_inner()) = Some((std::thread::current().id(), Arc::new(ctx)));
        HandoffOffer(self)
    }

    fn handoff_here(&self) -> Option<Arc<HandoffCtx>> {
        let g = self.handoff.lock().unwrap_or_else(|e| e.into_inner());
        g.as_ref().filter(|(t, _)| *t == std::thread::current().id()).map(|(_, c)| c.clone())
    }

    /// Whether this process waits on hand-off `rid` now.
    pub(crate) fn is_live(&self, rid: &str) -> bool {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).contains(rid)
    }

    fn set_live(&self, rid: &str, on: bool) {
        let mut l = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if on {
            l.insert(rid.to_string());
        } else {
            l.remove(rid);
        }
    }

    /// Listen on `port` (0: any) and, with `no_browser`, show the page's
    /// address in the prompt instead of opening a browser on this device.
    pub fn hand_off_on(mut self, port: u16, no_browser: bool) -> Self {
        self.port = port;
        self.no_browser = no_browser;
        self
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
        self.ceremony(req, Ask::Get, None)
    }

    /// Confirm the drafted `draft` (whose challenge `req` carries): on the
    /// person's desktop or phone when a setup run offers the hand-off, else
    /// as [`TermPresence::confirm`]. Blocking.
    pub fn confirm_draft(&self, req: &PresenceRequest, draft: &SessionEventEnvelope) -> Result<PresenceAssertion, String> {
        match self.handoff_here() {
            // A passkey's registration is confirmed where it is made.
            Some(ctx) if !matches!(draft.payload, SessionEvent::PasskeyRegistered { .. }) => {
                self.ceremony(req, Ask::Get, Some(Elsewhere { ctx: &ctx, draft }))
            }
            _ => self.confirm(req),
        }
    }

    /// Create a passkey over `req.challenge` (`navigator.credentials.create`).
    /// The attestation object rides in `authenticator_data`. Blocking.
    pub fn create(&self, req: &PresenceRequest, user_name: &str) -> Result<PresenceAssertion, String> {
        self.ceremony(req, Ask::Create { user_name }, None)
    }

    /// Wait again for a hand-off left waiting (`h`, resumed from the data
    /// dir), on a sheet that offers only waiting and cancelling. Blocking.
    pub(crate) fn wait_resumed(
        &self,
        req: &PresenceRequest,
        mut h: PresenceRequester,
        mine: &HashSet<Uuid>,
    ) -> Result<PresenceAssertion, String> {
        let (id, mut rx) = self.open_prompt();
        let rid = h.rid_hex();
        if self.is_live(&rid) {
            self.close_prompt(id);
            return Err("lt is already waiting for that one".into());
        }
        self.set_live(&rid, true);
        self.emit(Event::PresencePrompt(PresencePrompt {
            id,
            kind: req.kind.clone(),
            summary: req.summary.clone(),
            digest_short: req.digest_short.clone(),
            op: PresenceOp::Get,
            handoff: Some(h.code().shown.clone()),
            resumed: true,
        }));
        let outcome = self.rt.block_on(async {
            loop {
                match self.wait_handoff(&mut h, mine, &mut rx).await {
                    Step::Here(None | Some(PresenceAnswer::Cancel)) => {
                        end_handoff(&mut h, true).await;
                        return Err("cancelled".to_string());
                    }
                    // Nothing else to confirm it with here.
                    Step::Here(Some(_)) => {}
                    Step::Approved(a) => {
                        end_handoff(&mut h, false).await;
                        return Ok(a);
                    }
                    Step::Declined => {
                        end_handoff(&mut h, false).await;
                        return Err(DECLINED.to_string());
                    }
                    Step::Stopped { why, cancel } => {
                        end_handoff(&mut h, cancel).await;
                        return Err(format!("{why}: nothing was written"));
                    }
                }
            }
        });
        self.set_live(&rid, false);
        self.close_prompt(id);
        self.emit(Event::PresenceDone { id, ok: outcome.is_ok(), error: outcome.as_ref().err().cloned() });
        outcome
    }

    fn open_prompt(&self) -> (PresenceId, mpsc::UnboundedReceiver<PresenceAnswer>) {
        let id = PresenceId(self.next.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = mpsc::unbounded_channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        (id, rx)
    }

    fn close_prompt(&self, id: PresenceId) {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    }

    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
    }

    fn ceremony(&self, req: &PresenceRequest, ask: Ask<'_>, elsewhere: Option<Elsewhere<'_>>) -> Result<PresenceAssertion, String> {
        let (id, mut rx) = self.open_prompt();
        let op = match ask {
            Ask::Get => PresenceOp::Get,
            Ask::Create { .. } => PresenceOp::Create,
        };
        // Ask the person's other device first; when it can't be asked here,
        // today's sheet (with the reason, when there is one to tell).
        let mut hand = None;
        let mut note = None;
        if let Some(e) = &elsewhere {
            match self.rt.block_on(post(e.ctx, e.draft, &req.summary)) {
                Ok(h) => {
                    self.set_live(&h.rid_hex(), true);
                    hand = Some(h);
                }
                Err(PostError::Unavailable) => {}
                Err(PostError::Problem(p)) => note = Some(p),
            }
        }
        self.emit(Event::PresencePrompt(PresencePrompt {
            id,
            kind: req.kind.clone(),
            summary: req.summary.clone(),
            digest_short: req.digest_short.clone(),
            op,
            handoff: hand.as_ref().map(|h| h.code().shown.clone()),
            resumed: false,
        }));
        if let Some(n) = note {
            self.emit(Event::PresenceProblem { id, error: n });
        }
        let outcome = self.rt.block_on(self.run(id, req, &ask, &mut rx, hand, elsewhere.as_ref()));
        self.close_prompt(id);
        self.emit(Event::PresenceDone { id, ok: outcome.is_ok(), error: outcome.as_ref().err().cloned() });
        outcome
    }

    /// Wait on hand-off `h` until its answer, an answer here, or the wait's
    /// end. Only results and aborts signed by `mine` count (the requester
    /// checks the assertion names the draft's challenge).
    async fn wait_handoff(
        &self,
        h: &mut PresenceRequester,
        mine: &HashSet<Uuid>,
        rx: &mut mpsc::UnboundedReceiver<PresenceAnswer>,
    ) -> Step {
        let deadline = Instant::now() + self.handoff_wait;
        let is_mine = |d: Uuid| mine.contains(&d);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Step::Stopped { why: "No one confirmed it on your desktop or phone in time".into(), cancel: true };
            }
            tokio::select! {
                a = rx.recv() => return Step::Here(a),
                r = h.poll(left.as_secs().clamp(1, am::MAX_WAIT_SECS), &is_mine) => match r {
                    Ok(None) => {}
                    Ok(Some(PresenceOutcome::Approved { assertion, .. })) => return Step::Approved(assertion),
                    Ok(Some(PresenceOutcome::Declined { .. } | PresenceOutcome::Aborted { .. })) => return Step::Declined,
                    Ok(Some(PresenceOutcome::Expired)) => return Step::Stopped { why: "The request expired (15 minutes)".into(), cancel: false },
                    Err(RequesterError::Mailbox(m)) if m.retryable() => {
                        tokio::select! {
                            a = rx.recv() => return Step::Here(a),
                            _ = tokio::time::sleep(Duration::from_secs(3).min(left)) => {}
                        }
                    }
                    Err(e) => return Step::Stopped { why: format!("Couldn't reach your desktop or phone: {}", one_line(&e.to_string())), cancel: true },
                },
            }
        }
    }

    async fn run(
        &self,
        id: PresenceId,
        req: &PresenceRequest,
        ask: &Ask<'_>,
        rx: &mut mpsc::UnboundedReceiver<PresenceAnswer>,
        mut hand: Option<PresenceRequester>,
        elsewhere: Option<&Elsewhere<'_>>,
    ) -> Result<PresenceAssertion, String> {
        // An answer to act on before waiting for the next one.
        let mut next: Option<Option<PresenceAnswer>> = None;
        loop {
            let answer = match next.take() {
                Some(a) => a,
                None => match (hand.as_mut(), elsewhere) {
                    (Some(h), Some(e)) => {
                        let step = self.wait_handoff(h, &e.ctx.mine, rx).await;
                        let rid = h.rid_hex();
                        let after = match step {
                            // Confirming here (or cancelling) ends the hand-off first.
                            Step::Here(a) => {
                                end_handoff(h, true).await;
                                After::Answer(a)
                            }
                            Step::Approved(a) => {
                                end_handoff(h, false).await;
                                After::Done(Ok(a))
                            }
                            Step::Declined => {
                                end_handoff(h, false).await;
                                After::Done(Err(DECLINED.to_string()))
                            }
                            Step::Stopped { why, cancel } => {
                                end_handoff(h, cancel).await;
                                self.emit(Event::PresenceHandoff { id, update: HandoffUpdate::Stopped { why } });
                                After::Wait
                            }
                        };
                        self.set_live(&rid, false);
                        hand = None;
                        match after {
                            After::Answer(a) => a,
                            After::Wait => rx.recv().await,
                            After::Done(done) => return done,
                        }
                    }
                    _ => rx.recv().await,
                },
            };
            match answer {
                None | Some(PresenceAnswer::Cancel) => return Err("cancelled".into()),
                Some(PresenceAnswer::Handoff) => match elsewhere {
                    Some(e) => match post(e.ctx, e.draft, &req.summary).await {
                        Ok(h) => {
                            self.set_live(&h.rid_hex(), true);
                            self.emit(Event::PresenceHandoff { id, update: HandoffUpdate::Waiting { code: h.code().shown.clone() } });
                            hand = Some(h);
                        }
                        Err(PostError::Unavailable) => self.emit(Event::PresenceHandoff {
                            id,
                            update: HandoffUpdate::Stopped { why: "Your desktop or phone can't be asked through this relay".into() },
                        }),
                        Err(PostError::Problem(p)) => self.emit(Event::PresenceHandoff { id, update: HandoffUpdate::Stopped { why: p } }),
                    },
                    None => self.emit(Event::PresenceProblem { id, error: "this one can't be sent to your desktop or phone".into() }),
                },
                Some(PresenceAnswer::RecoveryWordsFile(path)) => match self.recovery(req, ask, &path) {
                    Ok(a) => return Ok(a),
                    Err(Fatal(e)) => return Err(e),
                    // Stay on the sheet so the person can fix it.
                    Err(Fixable(e)) => self.emit(Event::PresenceProblem { id, error: e }),
                },
                Some(PresenceAnswer::Browser) => {
                    let mut interrupt = None;
                    let show = ShowUrl { tx: self.tx.clone(), id, port: self.port };
                    let opener: &dyn Opener = if self.no_browser { &show } else { self.opener.as_ref() };
                    let r = presence::hand_off(
                        &self.presence_dir,
                        &req.policy.page_url(),
                        &fragment(req, ask),
                        opener,
                        self.port,
                        async { interrupt = rx.recv().await },
                        self.wait,
                    )
                    .await;
                    match r {
                        Ok(h) => return assertion(req, ask, h),
                        Err(HandOffError::Cancelled) => next = Some(interrupt),
                        // The person can retry, or use the recovery words.
                        Err(e) => self.emit(Event::PresenceProblem { id, error: e.to_string() }),
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

/// End a hand-off: `cancel` posts the cancel abort first (the other device
/// drops it); then the request is deleted and its pending entry removed.
async fn end_handoff(h: &mut PresenceRequester, cancel: bool) {
    if cancel {
        let _ = h.cancel().await;
    }
    let _ = h.settle().await;
}

fn one_line(s: &str) -> String {
    s.lines().next().unwrap_or_default().to_string()
}

/// Post the drafted change to the person's own devices through the
/// workspace's approval mailbox.
async fn post(ctx: &HandoffCtx, draft: &SessionEventEnvelope, summary: &str) -> Result<PresenceRequester, PostError> {
    let client = ctx.client();
    let address = match am::current_address(&client, ctx.ws, &ctx.founder_pin, &ctx.epochs).await {
        Ok(a) => a,
        Err(MailboxError::NoRoutes) => return Err(PostError::Unavailable),
        Err(e) => {
            return Err(PostError::Problem(format!(
                "Couldn't reach your desktop or phone through the relay ({})",
                one_line(&e.to_string())
            )));
        }
    };
    let signer = ctx.signer();
    // Too large for the mailbox, or a change this build can't send.
    let Ok(ask) = PresenceAsk::new(&ctx.label, &signer.public_key(), draft) else { return Err(PostError::Unavailable) };
    match PresenceRequester::post(client, &address, signer, ask, summary, Some(&ctx.data_dir)).await {
        Ok(r) => Ok(r),
        Err(RequesterError::Mailbox(MailboxError::NoRoutes | MailboxError::TooLarge)) => Err(PostError::Unavailable),
        Err(RequesterError::TooManyDead) => Err(PostError::Problem(
            "3 requests to your desktop or phone ended unanswered in the last 15 minutes, so lt isn't sending more for now: confirm here instead"
                .into(),
        )),
        Err(e) => Err(PostError::Problem(format!("Couldn't ask your desktop or phone ({})", one_line(&e.to_string())))),
    }
}

/// No browser here: the prompt shows the hand-off page's address, to open
/// in a browser on another device whose `127.0.0.1:<port>` reaches this
/// one's (an SSH port forward). The address carries the prompt's one-time
/// nonce and challenge (enough to answer a passkey creation), so it goes to
/// the person's terminal only: the full-screen sheet, or plain mode's
/// controlling terminal; never stdout, a file or a command line.
struct ShowUrl {
    tx: broadcast::Sender<Event>,
    id: PresenceId,
    port: u16,
}

impl Opener for ShowUrl {
    fn open(&self, _: &std::path::Path) -> std::io::Result<()> {
        Err(std::io::Error::other("no browser on this device"))
    }
    fn open_url(&self, _: &std::path::Path, url: &str) -> std::io::Result<()> {
        // The port actually listened on, from the address itself.
        let port = url.rsplit("&p=").next().and_then(|r| r.split('&').next()).and_then(|p| p.parse().ok()).unwrap_or(self.port);
        let _ = self.tx.send(Event::PresenceUrl { id: self.id, url: url.to_string(), port });
        Ok(())
    }
    fn uses_page(&self) -> bool {
        false
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

    fn assert_draft(&self, req: &PresenceRequest, draft: &SessionEventEnvelope) -> Result<PresenceAssertion, String> {
        self.0.confirm_draft(req, draft)
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

    /// No browser here: the prompt shows the page's address (on the fixed
    /// port, never opening a browser), and a browser elsewhere that reaches
    /// that port (here: this test) answers the very challenge.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_browser_shows_the_address_and_takes_the_answer_on_the_fixed_port() {
        let tmp = tempfile::tempdir().unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (tx, mut rx) = broadcast::channel(64);
        let p = Arc::new(
            TermPresence::new(
                tx,
                tokio::runtime::Handle::current(),
                tmp.path().join("presence"),
                Arc::new(NoBrowser),
                Duration::from_secs(5),
            )
            .hand_off_on(port, true),
        );
        let p2 = p.clone();
        let asked = tokio::task::spawn_blocking(move || p2.confirm(&request()));
        let mut shown = None;
        while let Ok(e) = rx.recv().await {
            match e {
                Event::PresencePrompt(pr) => p.answer(pr.id, PresenceAnswer::Browser).unwrap(),
                Event::PresenceUrl { url, port: shown_port, .. } => {
                    assert_eq!(shown_port, port);
                    assert!(url.starts_with("https://ligus.ai/presence#op=get") && url.contains(&format!("&p={port}&n=")), "{url}");
                    let nonce = url.rsplit("&n=").next().unwrap().to_string();
                    let client = b64url(format!("{{\"challenge\":\"{}\"}}", b64url(&[7u8; 32])).as_bytes());
                    shown = Some(url);
                    std::thread::spawn(move || {
                        use std::io::{Read, Write};
                        let body = format!("credentialId=abc&clientDataJSON={client}&signature=c2ln&authenticatorData=YXV0aA");
                        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                        write!(s, "POST /{nonce} HTTP/1.1\r\nHost: x\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
                        let mut resp = String::new();
                        let _ = s.read_to_string(&mut resp);
                    });
                }
                Event::PresenceProblem { error, .. } => panic!("{error}"),
                Event::PresenceDone { ok, error, .. } => {
                    assert!(ok, "{error:?}");
                    break;
                }
                _ => {}
            }
        }
        assert!(shown.is_some());
        assert_eq!(asked.await.unwrap().unwrap().credential_id, "abc");
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
        let bad = words_file(tmp.path(), &"word ".repeat(24));
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
