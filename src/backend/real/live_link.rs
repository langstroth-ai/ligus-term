//! Add to my devices, live: code-match approval through the workspace's
//! approval address (`ligus_runtime::approval_mailbox`; the ligus repo's
//! `docs/design/code-match-approval.md`, 6.3 to 6.5 and 7.5).
//!
//! 1. [`Inner::link_live`]: a fresh link request, posted sealed to the
//!    address's mailbox. Its paste text and fingerprint stay on the screen,
//!    so an older desktop can still approve it the old way.
//! 2. The desktop opens it: both screens show the same code, and the person
//!    says whether they match. No ends the request (nothing is granted).
//! 3. The desktop's offer is acknowledged only after the person said Yes.
//! 4. The result: checks 1 to 4 on the grant ([`Requester::check_grant`]),
//!    the offline checks the paste path makes, then the connection is saved
//!    and pulled and check 5 runs on the log (retried for up to 2 minutes
//!    while the approval syncs).
//! 5. Check 6: "Link this computer as Ana (verified on the workspace
//!    log)?" Yes records the link exactly as the paste path does; No puts
//!    everything back. Either way the request is deleted from the relay.
//!
//! The desktop's short code (`K7Q-4MX`, typed instead of the address) is
//! resolved to the address first ([`Inner::address_of_code`]). If the relay
//! restarts and forgets the request, it is posted again (up to three times)
//! and the screen says so.
//!
//! One live request at a time; a new one ends the last.

use std::sync::Arc;
use std::time::Duration;

use ligus_core::match_code::MatchKind;
use ligus_runtime::approval_mailbox::MoreGrantCheck;
use ligus_runtime::approval_mailbox::{
    AbortReason, ApprovalAddress, CheckedGrant, ClaimView, GrantError, GrantTarget, LOG_CHECK_RETRY_MS, LogApprovalCheck, LogVerdict,
    MailboxClient, MailboxError, Outcome, RequestSpec, Requester, RequesterError, RequesterEvent, ResultBody, Signer, claim_view, hint,
    list_live, retry_log_check,
};
use ligus_runtime::computer_link::LinkRequest;
use ligus_runtime::display::shown;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use super::link::PreparedLink;
use super::{Inner, lock, one_line};
use crate::backend::model::{
    ApprovalKind, ApprovalRequest, LinkCodeState, LinkCodeView, Linked, LiveLinkEnd, LiveLinkStart, LiveLinkUpdate, MatchCodeView,
};
use ligus_runtime::approval_mailbox::{CodeState, LinkCodeOffer};
use crate::backend::{BackendError, Event, Result, WorkspaceId, err};

/// What the person said, sent to the request's task.
pub(crate) enum Cmd {
    /// The codes match (`true`) or don't.
    Code(bool),
    /// Link as the verified person (`true`) or link nothing.
    Person(bool, oneshot::Sender<Result<Option<Linked>>>),
    Cancel,
}

/// The live request in flight.
pub(crate) struct LiveHandle {
    rid: String,
    cmd: mpsc::UnboundedSender<Cmd>,
}

/// How long the verified person may wait for an answer (the grant lives 30).
const CONFIRM_WAIT: Duration = Duration::from_secs(10 * 60);
/// The long poll for the desktop's messages; commands are read between polls.
const POLL_SECS: u64 = 2;
/// The longest pause between tries while the relay doesn't answer.
const RETRY_MAX_SECS: u64 = 15;
/// How long a request is tried at all: the relay ends it after 15 minutes
/// (its own `RequestExpired`); a relay that can't be reached never says so,
/// so this side stops then too, with a little slack for clocks.
const REQUEST_LIFE: Duration = Duration::from_millis(ligus_runtime::computer_link::REQUEST_TTL_MS as u64 + 30_000);

/// Where the poll loop stands while the relay doesn't answer.
#[derive(Debug, Default)]
pub(crate) struct Retry {
    backoff: u64,
    /// The last error, while retrying (shown once per new error).
    shown: Option<String>,
}

/// What the loop does after a poll.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RetryStep {
    /// Answered: say so if the screen showed a retry.
    Answered { was_retrying: bool },
    /// Didn't answer: wait this long (say `why` if it is new), then try again.
    Wait { secs: u64, say: Option<String> },
    /// Didn't answer and the request's life is over.
    GiveUp(String),
}

impl Retry {
    pub(crate) fn new() -> Self {
        Self { backoff: 1, shown: None }
    }

    /// The poll answered.
    pub(crate) fn answered(&mut self) -> RetryStep {
        self.backoff = 1;
        RetryStep::Answered { was_retrying: self.shown.take().is_some() }
    }

    /// The poll failed with a retryable `why`, `elapsed` into the request.
    pub(crate) fn failed(&mut self, why: String, elapsed: Duration, life: Duration) -> RetryStep {
        if elapsed >= life {
            return RetryStep::GiveUp(why);
        }
        let secs = self.backoff;
        self.backoff = (self.backoff * 2).min(RETRY_MAX_SECS);
        let say = (self.shown.as_deref() != Some(why.as_str())).then(|| why.clone());
        self.shown = Some(why);
        RetryStep::Wait { secs, say }
    }
}

/// Whether `relay` is an `http://` address on this machine (a local or a
/// test relay): the only plain-text relay an address may name.
fn loopback_http(relay: &str) -> bool {
    let Some(rest) = relay.strip_prefix("http://") else { return false };
    let host = rest.split('/').next().unwrap_or("");
    let host = if host.starts_with('[') {
        host.split(']').next().map(|h| format!("{h}]")).unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or("").to_string()
    };
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]")
}

/// Said when check 3 refuses a grant that names another relay than the
/// address.
const WRONG_RELAY: &str = "your desktop's approval points at another relay than the approval address you pasted, so it may \
                           come from a copy of the workspace, not the real one; check the address on your desktop";

/// An operator's relay override (`LIGUS_RELAY_URL`, as the ligus CLI reads
/// it): a grant may then name another relay, and the connection uses this
/// one.
fn relay_override() -> Option<String> {
    std::env::var("LIGUS_RELAY_URL")
        .ok()
        .map(|r| r.trim().trim_end_matches('/').to_string())
        .filter(|r| ligus_runtime::workspace_invite::usable_relay_url(r))
}

/// Read a pasted approval address (`ligusaddr1:`): an `https://` relay, or
/// `http://` on this machine only.
pub(crate) fn parse_address(text: &str) -> Result<ApprovalAddress> {
    let a = ApprovalAddress::decode_with(text, true).map_err(|e| BackendError(e.to_string()))?;
    if !a.relay.starts_with("https://") && !loopback_http(&a.relay) {
        return Err(BackendError(ligus_runtime::approval_mailbox::AddressError::InsecureRelay.to_string()));
    }
    Ok(a)
}

/// One line for a failed post.
fn post_error(e: RequesterError) -> BackendError {
    match e {
        RequesterError::TooManyDead => BackendError(crate::backend::model::TOO_MANY_DEAD.into()),
        RequesterError::Mailbox(MailboxError::Unreachable(why)) => {
            BackendError(format!("the relay couldn't be reached ({why}): check the network and try again"))
        }
        other => BackendError(other.to_string()),
    }
}

/// Times a request is posted again after relay restarts.
const MAX_REPOSTS: usize = 3;

/// A short code that found no address, in one line.
fn code_error(e: MailboxError, relay: &str) -> BackendError {
    let host = relay.split("://").nth(1).unwrap_or(relay).trim_end_matches('/').to_string();
    BackendError(match e {
        MailboxError::NoRoutes => {
            format!("{host} has no short codes yet: paste the approval address your desktop shows instead (Add a computer > Other ways to link)")
        }
        MailboxError::CodeUnknown => format!(
            "no device is showing that code on {host}: check it, or get a new one on your desktop (with your own relay, type its address after the code)"
        ),
        MailboxError::Malformed => "the relay's answer for that code didn't check out, so nothing was sent: get a new code".into(),
        MailboxError::Unreachable(why) => format!("the relay couldn't be reached ({why}): check the network and try again"),
        other => other.to_string(),
    })
}

impl Inner {
    /// The approval address behind a short code (`K7Q-4MX`, optionally
    /// followed by the relay it is on), or the pasted address itself.
    pub(crate) async fn address_of_code(&self, typed: &str) -> Result<ApprovalAddress> {
        let Some((code, relay)) = crate::link_code::short_code_in(typed) else { return parse_address(typed) };
        let code = ligus_core::link_code::ShortCode::parse(&code).ok_or_else(|| BackendError("that isn't a short code".into()))?;
        let relay = relay
            .or_else(|| self.cfg.relay_url.clone().filter(|r| ligus_runtime::workspace_invite::usable_relay_url(r)))
            .unwrap_or_else(|| ligus_core::COMMUNITY_RELAY_URL.to_string());
        let client = MailboxClient::new(&relay);
        let a = ligus_runtime::approval_mailbox::resolve_link_code(&client, &code, true).await.map_err(|e| code_error(e, &relay))?;
        // The same checks as a pasted address.
        parse_address(&a.encode())
    }

    /// Post this computer's link request to `address` (or the address
    /// behind the short code typed there) and drive it in the background;
    /// progress is [`Event::LiveLink`].
    pub(crate) async fn link_live(self: &Arc<Self>, address: &str) -> Result<LiveLinkStart> {
        let address = self.address_of_code(address).await?;
        if let Some(h) = lock(&self.live).take() {
            let _ = h.cmd.send(Cmd::Cancel);
        }
        let me = self.clone();
        let (view, request, device) = tokio::task::spawn_blocking(move || me.make_link_request()).await.map_err(one_line)??;
        let ids = self.identity_store();
        let signer = Signer::Ed25519(ids.device_keypair(device).map_err(one_line)?);
        let ka = ids.key_agreement(device).map_err(one_line)?;
        let spec = RequestSpec { kind: MatchKind::Computer, code: view.code.clone(), signer, ka: Some(ka) };
        let client = MailboxClient::new(&address.relay);
        let req = match Requester::post(client, &address, spec, Some(&self.cfg.data_dir)).await {
            Ok(r) => r,
            // No approval mailbox on this relay: the paste path, as before.
            Err(RequesterError::Mailbox(MailboxError::NoRoutes)) => return Ok(LiveLinkStart::NoMailbox { request: view }),
            Err(e) => return Err(post_error(e)),
        };
        let mut req = req;
        req.allow_relay_override(relay_override().is_some());
        let rid = req.rid_hex();
        let (tx, rx) = mpsc::unbounded_channel();
        *lock(&self.live) = Some(LiveHandle { rid: rid.clone(), cmd: tx });
        let me = self.clone();
        let handle = rid.clone();
        tokio::spawn(async move { Drive { inner: me, handle, req, request, device, rx }.run().await });
        Ok(LiveLinkStart::Posted { rid, request: view })
    }

    /// Hand `cmd` to the live request `rid`.
    fn live_cmd(&self, rid: &str, cmd: Cmd) -> Result<()> {
        let live = lock(&self.live);
        match live.as_ref() {
            Some(h) if h.rid == rid => h.cmd.send(cmd).map_err(|_| BackendError("that request has ended: start again".into())),
            _ => err("that request has ended: start again"),
        }
    }

    pub(crate) fn link_live_code(&self, rid: &str, matches: bool) -> Result<()> {
        self.live_cmd(rid, Cmd::Code(matches))
    }

    pub(crate) async fn link_live_person(&self, rid: &str, link: bool) -> Result<Option<Linked>> {
        let (tx, rx) = oneshot::channel();
        self.live_cmd(rid, Cmd::Person(link, tx))?;
        rx.await.map_err(|_| BackendError("that request has ended: start again".into()))?
    }

    pub(crate) fn link_live_cancel(&self, rid: &str) -> Result<()> {
        match self.live_cmd(rid, Cmd::Cancel) {
            Ok(()) => Ok(()),
            // Already over: nothing to cancel.
            Err(_) => Ok(()),
        }
    }

    /// The live requests in `ws`'s approval mailboxes (the newest three key
    /// epochs this device holds), read only, each claim classified against
    /// the folded workspace (design note 6.4: a claim by a device that can't
    /// approve that kind, or an open without a claim, is contested).
    pub(crate) async fn approvals(self: &Arc<Self>, ws: Uuid) -> Result<Vec<ApprovalRequest>> {
        let saved = self.saved(ws)?;
        let Some(relay) = saved.relay().map(str::to_string) else { return Ok(Vec::new()) };
        let src = self.key_source(&saved)?;
        let epochs = self.read(ws, move |svc| svc.approval_epoch_keys(ws, &src))?;
        let classifier = self.read(ws, |svc| svc.approval_classifier(ws))?;
        let me = self.core()?.identity.device.id;
        let client = MailboxClient::new(&relay).with_auth(saved.secrets.token.clone());
        let listing = match list_live(&client, &epochs).await {
            Ok(l) => l,
            // A relay without an approval mailbox holds no requests.
            Err(MailboxError::NoRoutes) => return Ok(Vec::new()),
            Err(e) => return Err(BackendError(e.to_string())),
        };
        Ok(listing
            .live
            .iter()
            .filter(|l| l.ended.is_none())
            .map(|l| {
                let (checking, mine, contested) = match claim_view(l, me, &classifier) {
                    ClaimView::Unclaimed => (None, false, false),
                    ClaimView::Mine => (None, true, false),
                    ClaimView::ClaimedBy { name, .. } => (Some(hint(&name)), false, false),
                    ClaimView::Contested { by } => (by.map(|(n, _)| hint(&n)), false, true),
                };
                ApprovalRequest {
                    kind: match l.kind() {
                        MatchKind::Box => ApprovalKind::Box,
                        MatchKind::Computer => ApprovalKind::Computer,
                        MatchKind::Phone => ApprovalKind::Phone,
                        MatchKind::Presence => ApprovalKind::Presence,
                    },
                    label: hint(&l.label()),
                    at: l.put_at,
                    checking,
                    mine,
                    contested,
                }
            })
            .collect())
    }
}

/// The short code this computer shows (Link a device): which showing it
/// is, and what takes it back.
pub(crate) struct ShownCode {
    showing: u64,
    offer: Arc<tokio::sync::Mutex<LinkCodeOffer>>,
    client: MailboxClient,
    used: bool,
}

/// How often a shown code is checked.
const CODE_CHECK_SECS: u64 = 3;

fn code_view(offer: &LinkCodeOffer, state: LinkCodeState, notice: Option<String>) -> LinkCodeView {
    let community = ligus_core::link_code::relay_salt(offer.relay()) == ligus_core::link_code::relay_salt(ligus_core::COMMUNITY_RELAY_URL);
    LinkCodeView {
        code: offer.code().shown(),
        relay: (!community).then(|| offer.relay().to_string()),
        expires_at: offer.expires_at(),
        state,
        notice,
    }
}

impl Inner {
    /// Show a short code for `ws`'s approval address and keep it posted
    /// until it is used, expires or the screen closes.
    pub(crate) async fn link_code_show(self: &Arc<Self>, ws: Uuid) -> Result<LinkCodeView> {
        self.link_code_close().await;
        let saved = self.saved(ws)?;
        let Some(relay) = saved.relay().map(str::to_string) else { return err("this workspace has no relay: a new device can't reach it") };
        let src = self.key_source(&saved)?;
        let epochs = self.read(ws, move |svc| svc.approval_epoch_keys(ws, &src))?;
        let pin = self.read(ws, move |svc| svc.founder_pin_hex(ws))?.ok_or_else(|| BackendError("this workspace has no founder pin here yet".into()))?;
        let client = MailboxClient::new(&relay).with_auth(saved.secrets.token.clone());
        let offer = match LinkCodeOffer::post(&client, ws, &pin, &epochs).await {
            Ok(o) => o,
            Err(MailboxError::NoRoutes) => {
                let address = ligus_runtime::approval_mailbox::current_address(&client, ws, &pin, &epochs).await.ok().map(|a| a.encode());
                return Ok(LinkCodeView { code: String::new(), relay: None, expires_at: 0, state: LinkCodeState::NoRoutes { address }, notice: None });
            }
            Err(e) => return Err(BackendError(e.to_string())),
        };
        let view = code_view(&offer, LinkCodeState::Live, None);
        let offer = Arc::new(tokio::sync::Mutex::new(offer));
        let showing = {
            let mut shown = lock(&self.shown_code);
            let showing = shown.as_ref().map_or(1, |s| s.showing + 1);
            *shown = Some(ShownCode { showing, offer: offer.clone(), client: client.clone(), used: false });
            showing
        };
        let me = self.clone();
        tokio::spawn(async move { me.keep_code(showing, offer, client, pin, epochs).await });
        Ok(view)
    }

    async fn keep_code(
        self: Arc<Self>,
        showing: u64,
        offer: Arc<tokio::sync::Mutex<LinkCodeOffer>>,
        client: MailboxClient,
        pin: String,
        epochs: Vec<ligus_runtime::approval_mailbox::EpochKey>,
    ) {
        let current = |me: &Inner| lock(&me.shown_code).as_ref().is_some_and(|s| s.showing == showing);
        let mut trouble = false;
        loop {
            tokio::time::sleep(Duration::from_secs(CODE_CHECK_SECS)).await;
            if !current(&self) {
                return;
            }
            let mut o = offer.lock().await;
            let (state, notice, done) = match o.check(&client, &pin, &epochs).await {
                Ok(CodeState::Live) if !trouble => continue,
                Ok(CodeState::Live) => (LinkCodeState::Live, None, false),
                Ok(CodeState::Reposted) => (LinkCodeState::Live, Some("The relay restarted; the code works again.".to_string()), false),
                Ok(CodeState::Used) => (LinkCodeState::Used, None, true),
                Ok(CodeState::Expired) => (LinkCodeState::Expired, None, true),
                Err(e) if e.retryable() => (LinkCodeState::Live, Some(format!("Can't reach the relay right now ({e}). Still trying…")), false),
                Err(e) => (LinkCodeState::Failed(e.to_string()), None, true),
            };
            trouble = notice.as_deref().is_some_and(|n| n.starts_with("Can't reach"));
            if !current(&self) {
                return;
            }
            if matches!(state, LinkCodeState::Used) {
                if let Some(s) = lock(&self.shown_code).as_mut() {
                    s.used = true;
                }
            }
            self.emit(Event::LinkCode(code_view(&o, state, notice)));
            if done {
                return;
            }
        }
    }

    /// Take a shown code nobody typed back.
    pub(crate) async fn link_code_close(&self) {
        let Some(s) = lock(&self.shown_code).take() else { return };
        if !s.used {
            let _ = s.offer.lock().await.withdraw(&s.client).await;
        }
    }
}

/// One live request's task.
struct Drive {
    inner: Arc<Inner>,
    /// The first request's rid: the screen and the commands keep using it
    /// when the request is posted again after a relay restart.
    handle: String,
    req: Requester,
    request: LinkRequest,
    device: Uuid,
    rx: mpsc::UnboundedReceiver<Cmd>,
}

impl Drive {
    fn emit(&self, update: LiveLinkUpdate) {
        self.inner.emit(Event::LiveLink { rid: self.handle.clone(), update });
    }

    /// The request is over: delete it from the relay, say why, let go.
    async fn end(&mut self, why: Option<LiveLinkEnd>) {
        let _ = self.req.settle().await;
        if let Some(w) = why {
            self.emit(LiveLinkUpdate::Ended(w));
        }
        let rid = self.handle.clone();
        let mut live = lock(&self.inner.live);
        if live.as_ref().is_some_and(|h| h.rid == rid) {
            *live = None;
        }
    }

    async fn run(mut self) {
        let mut said_yes = false;
        let mut offered = false;
        let mut acked = false;
        let mut retry = Retry::new();
        let started = tokio::time::Instant::now();
        let mut restarts = 0;
        loop {
            // Never silently forever: past the request's life it is over,
            // even when the relay never said so.
            if started.elapsed() >= REQUEST_LIFE {
                return self.end(Some(LiveLinkEnd::Expired)).await;
            }
            while let Ok(cmd) = self.rx.try_recv() {
                match cmd {
                    Cmd::Code(true) => said_yes = true,
                    Cmd::Code(false) => {
                        let _ = self.req.abort(AbortReason::Mismatch).await;
                        return self.end(Some(LiveLinkEnd::Mismatch)).await;
                    }
                    Cmd::Cancel => {
                        let _ = self.req.abort(AbortReason::Cancelled).await;
                        return self.end(Some(LiveLinkEnd::Cancelled)).await;
                    }
                    Cmd::Person(_, reply) => {
                        let _ = reply.send(err("nothing to confirm yet"));
                    }
                }
            }
            // The offer is acknowledged only once the person said the codes
            // match on this computer.
            if said_yes && offered && !acked {
                // All the workspaces the desktop offers come along.
                if let Err(e) = self.req.ack_all().await {
                    return self.end(Some(LiveLinkEnd::Refused(e.to_string()))).await;
                }
                acked = true;
                self.emit(LiveLinkUpdate::Acknowledged);
            }
            let events = match self.req.poll(POLL_SECS).await {
                Ok(ev) => {
                    if retry.answered() == (RetryStep::Answered { was_retrying: true }) {
                        self.emit(LiveLinkUpdate::Retrying(None));
                    }
                    ev
                }
                Err(RequesterError::Mailbox(e)) if e.retryable() => {
                    match retry.failed(e.to_string(), started.elapsed(), REQUEST_LIFE) {
                        RetryStep::GiveUp(why) => return self.end(Some(LiveLinkEnd::Unreachable(why))).await,
                        RetryStep::Wait { secs, say } => {
                            if let Some(why) = say {
                                self.emit(LiveLinkUpdate::Retrying(Some(why)));
                            }
                            tokio::time::sleep(Duration::from_secs(secs)).await;
                        }
                        RetryStep::Answered { .. } => {}
                    }
                    continue;
                }
                // The relay forgot the request (it restarted): post it again.
                Err(RequesterError::Mailbox(e)) if ligus_runtime::approval_mailbox::relay_forgot(&e) && restarts < MAX_REPOSTS => {
                    restarts += 1;
                    self.emit(LiveLinkUpdate::Reposted);
                    match self.repost(started).await {
                        Ok(r) => {
                            let mut old = std::mem::replace(&mut self.req, r);
                            let _ = old.settle().await;
                            (said_yes, offered, acked) = (false, false, false);
                            continue;
                        }
                        Err(e) => return self.end(Some(LiveLinkEnd::Refused(e.0))).await,
                    }
                }
                Err(RequesterError::Mailbox(MailboxError::RequestUnknown)) => return self.end(Some(LiveLinkEnd::Expired)).await,
                Err(e) => return self.end(Some(LiveLinkEnd::Refused(e.to_string()))).await,
            };
            for ev in events {
                match ev {
                    RequesterEvent::Opened { code, claimed } => self.emit(LiveLinkUpdate::Opened(MatchCodeView {
                        shown: code.shown,
                        spelled: code.spelled,
                        claimed: format!("{}'s desktop", claimed.name),
                        expires_at: code.expires_at,
                    })),
                    RequesterEvent::Offered { .. } => offered = true,
                    RequesterEvent::Result(r) => return self.finish(r).await,
                    RequesterEvent::Aborted { .. } => return self.end(Some(LiveLinkEnd::EndedByDesktop)).await,
                    RequesterEvent::CodeExpired | RequesterEvent::RequestExpired => return self.end(Some(LiveLinkEnd::Expired)).await,
                }
            }
        }
    }

    /// Post the same link request again to the same address, waiting while
    /// the relay's mailbox comes back (a member's device registers it again).
    async fn repost(&mut self, started: tokio::time::Instant) -> Result<Requester> {
        let address = self.req.address().clone();
        loop {
            if let Ok(Cmd::Cancel) = self.rx.try_recv() {
                return err("cancelled");
            }
            let ids = self.inner.identity_store();
            let signer = Signer::Ed25519(ids.device_keypair(self.device).map_err(one_line)?);
            let ka = ids.key_agreement(self.device).map_err(one_line)?;
            let spec = RequestSpec { kind: MatchKind::Computer, code: self.request.encode(), signer, ka: Some(ka) };
            match Requester::post(MailboxClient::new(&address.relay), &address, spec, Some(&self.inner.cfg.data_dir)).await {
                Ok(mut r) => {
                    r.allow_relay_override(relay_override().is_some());
                    return Ok(r);
                }
                Err(RequesterError::Mailbox(e))
                    if (ligus_runtime::approval_mailbox::relay_forgot(&e) || e.retryable()) && started.elapsed() < REQUEST_LIFE =>
                {
                    tokio::time::sleep(Duration::from_secs(POLL_SECS)).await;
                }
                Err(e) => return Err(post_error(e)),
            }
        }
    }

    /// The desktop's result: checks 1 to 6, then link or not.
    async fn finish(mut self, r: ResultBody) {
        let ids = self.inner.identity_store();
        let (ka, key) = match (ids.key_agreement(self.device), ids.device_keypair(self.device)) {
            (Ok(ka), Ok(k)) => (ka, k.public_key_bytes()),
            _ => return self.end(Some(LiveLinkEnd::Refused("this computer's request keys are gone".into()))).await,
        };
        let now = ligus_core::flow::now_ms();
        let target = GrantTarget::Computer { request: &self.request, ka: &ka, device_key: &key };
        let checked = match self.req.check_grant(&r, target, now) {
            Ok(c) => c,
            Err(GrantError::NotApproved(Outcome::Declined)) => return self.end(Some(LiveLinkEnd::Declined)).await,
            Err(GrantError::NotApproved(Outcome::Expired)) => return self.end(Some(LiveLinkEnd::Expired)).await,
            Err(GrantError::NotApproved(_)) => return self.end(Some(LiveLinkEnd::EndedByDesktop)).await,
            Err(GrantError::WrongRelay) => return self.end(Some(LiveLinkEnd::Refused(WRONG_RELAY.into()))).await,
            Err(e) => return self.end(Some(LiveLinkEnd::Refused(e.to_string()))).await,
        };
        let more = self.req.check_more_grants(&checked, GrantTarget::Computer { request: &self.request, ka: &ka, device_key: &key }, now);
        let CheckedGrant::Computer { grant, .. } = &checked else {
            return self.end(Some(LiveLinkEnd::Refused("the answer isn't for a computer".into()))).await;
        };
        let mut grant = (**grant).clone();
        // An operator's relay override: the connection syncs (and check 5
        // reads the log) there, never at the grant's relay.
        if let Some(r) = relay_override() {
            grant.conn.relay_url = r;
        }
        let person = match self.inner.check_opened(&grant, self.device) {
            Ok(p) => p,
            Err(e) => return self.end(Some(LiveLinkEnd::Refused(e.0))).await,
        };
        self.emit(LiveLinkUpdate::Checking);
        let ws = grant.workspace_id;
        let workspace = grant.conn.display_name();
        let prepared = match self.inner.prepare_link(grant, self.device).await {
            Ok(p) => p,
            Err(e) => return self.end(Some(LiveLinkEnd::Refused(e.0))).await,
        };
        // Check 5: the desktop's certificate for this computer, signed by
        // the device that showed the code, on the workspace log.
        let verdict = {
            let Some(la) = self.req.log_approval(&checked, &key, now) else {
                self.inner.rollback_link(prepared);
                return self.end(Some(LiveLinkEnd::Refused("no desktop opened this request".into()))).await;
            };
            let (inner, la) = (&self.inner, &la);
            retry_log_check(
                move || async move {
                    let _ = inner.pull_for_link(ws).await;
                    inner.read(ws, |svc| Ok(svc.check_approval_on_log(la))).unwrap_or_else(|e| LogVerdict::NotYet(e.0))
                },
                Duration::from_millis(LOG_CHECK_RETRY_MS as u64),
                Duration::from_secs(3),
            )
            .await
        };
        let name = match verdict {
            LogVerdict::Verified { name, .. } => hint(&name),
            LogVerdict::NotYet(_) => {
                self.inner.rollback_link(prepared);
                return self.end(Some(LiveLinkEnd::Refused("the approval wasn't found on the workspace log".into()))).await;
            }
            LogVerdict::Refused(why) => {
                self.inner.rollback_link(prepared);
                return self.end(Some(LiveLinkEnd::Refused(why))).await;
            }
        };
        // The person's other workspaces the desktop added this computer to:
        // each checked on its own log, as the same person.
        let (extras, not_linked) = self.more(more, &person).await;
        let workspace = names_line(&workspace, &extras);
        // Check 6: the person confirms who the log says approved it.
        self.emit(LiveLinkUpdate::Verified { person: name.clone(), workspace });
        let answer = tokio::time::timeout(CONFIRM_WAIT, async {
            loop {
                match self.rx.recv().await {
                    Some(Cmd::Person(yes, reply)) => return Some((yes, reply)),
                    Some(Cmd::Cancel) | None => return None,
                    Some(Cmd::Code(_)) => {}
                }
            }
        })
        .await
        .ok()
        .flatten();
        match answer {
            Some((true, reply)) => {
                let linked = self.inner.commit_link(prepared, &person).await;
                if linked.is_ok() {
                    for (p, wsn) in extras {
                        let ws = p.g.workspace_id;
                        let status = match self.inner.commit_more_link(p, &person).await {
                            Ok(()) => {
                                format!("“{wsn}” linked: this computer is one of {}'s devices there too", shown(&person.display_name))
                            }
                            Err(e) => format!("“{wsn}” not linked ({}): {LINK_LATER}", e.0),
                        };
                        self.inner.emit(Event::Sync { workspace: WorkspaceId::new(ws.to_string()), status });
                    }
                    for (ws, wsn, why) in &not_linked {
                        let status = format!("“{wsn}” not linked ({why}): {LINK_LATER}");
                        self.inner.emit(Event::Sync { workspace: WorkspaceId::new(ws.to_string()), status });
                    }
                } else {
                    for (p, _) in extras.into_iter().rev() {
                        self.inner.rollback_link(p);
                    }
                }
                // Settled (deleted from the relay) before the answer.
                self.end(None).await;
                let _ = reply.send(linked.map(Some));
            }
            Some((false, reply)) => {
                for (p, _) in extras.into_iter().rev() {
                    self.inner.rollback_link(p);
                }
                self.inner.rollback_link(prepared);
                self.end(Some(LiveLinkEnd::NotLinked { person: name })).await;
                let _ = reply.send(Ok(None));
            }
            None => {
                for (p, _) in extras.into_iter().rev() {
                    self.inner.rollback_link(p);
                }
                self.inner.rollback_link(prepared);
                self.end(Some(LiveLinkEnd::Cancelled)).await;
            }
        }
    }

    /// The more workspaces of a link (`ligus_runtime::approval_mailbox`
    /// "more workspaces"): each one's connection saved and pulled, and the
    /// desktop's certificate for this computer found on its own log (retried
    /// while it syncs), as `person`. The ones that check out, ready to
    /// commit; the others with why.
    #[allow(clippy::type_complexity)]
    async fn more(
        &self,
        more: Vec<MoreGrantCheck>,
        person: &ligus_core::ActorIdentity,
    ) -> (Vec<(PreparedLink, String)>, Vec<(Uuid, String, String)>) {
        let mut ready = Vec::new();
        let mut not_linked = Vec::new();
        for m in more {
            let mut g = match m.outcome {
                Ok(CheckedGrant::Computer { grant, .. }) => *grant,
                Ok(_) => continue,
                Err(GrantError::NotSent) => {
                    not_linked.push((m.workspace_id, hint(&m.name), "your desktop didn't add this computer there".into()));
                    continue;
                }
                Err(e) => {
                    not_linked.push((m.workspace_id, hint(&m.name), e.to_string()));
                    continue;
                }
            };
            if let Some(r) = relay_override() {
                g.conn.relay_url = r;
            }
            let wsn = g.conn.display_name();
            let ws = g.workspace_id;
            let p = match self.inner.prepare_link(g, self.device).await {
                Ok(p) => p,
                Err(e) => {
                    not_linked.push((ws, wsn, e.0));
                    continue;
                }
            };
            let deadline = tokio::time::Instant::now() + Duration::from_millis(LOG_CHECK_RETRY_MS as u64);
            let checked = loop {
                let _ = self.inner.pull_for_link(ws).await;
                match self.inner.check_link_on_log(&p.g) {
                    Ok(on_log) if on_log.account_id == person.account_id && on_log.id == person.id => break Ok(()),
                    Ok(_) => break Err("the workspace names another person".to_string()),
                    Err(e) if waiting(&e.0) && tokio::time::Instant::now() < deadline => tokio::time::sleep(Duration::from_secs(3)).await,
                    Err(e) => break Err(e.0),
                }
            };
            match checked {
                Ok(()) => ready.push((p, wsn)),
                Err(why) => {
                    self.inner.rollback_link(p);
                    not_linked.push((ws, wsn, why));
                }
            }
        }
        (ready, not_linked)
    }
}

/// What a workspace that didn't come along says to do.
const LINK_LATER: &str = "link it later: palette > Link this computer to my devices, with that workspace's approval address";

/// Whether check 5 is still waiting for the desktop's certificate to sync.
fn waiting(why: &str) -> bool {
    why.contains(ligus_runtime::computer_link::NOT_ON_RELAY_YET) || why.contains("hasn't synced")
}

/// "Acme Web", or "Acme Web, Beta and Gamma" when more come along.
fn names_line(first: &str, extras: &[(PreparedLink, String)]) -> String {
    let mut names: Vec<&str> = vec![first];
    names.extend(extras.iter().map(|(_, n)| n.as_str()));
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => first.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_are_shown_once_per_reason_and_end_with_the_request() {
        let life = Duration::from_secs(15 * 60);
        let mut r = Retry::new();
        let at = |s| Duration::from_secs(s);
        assert_eq!(r.failed("timed out".into(), at(1), life), RetryStep::Wait { secs: 1, say: Some("timed out".into()) });
        assert_eq!(r.failed("timed out".into(), at(2), life), RetryStep::Wait { secs: 2, say: None });
        assert_eq!(r.failed("502".into(), at(4), life), RetryStep::Wait { secs: 4, say: Some("502".into()) });
        for _ in 0..5 {
            let _ = r.failed("502".into(), at(10), life);
        }
        assert_eq!(r.failed("502".into(), at(60), life), RetryStep::Wait { secs: RETRY_MAX_SECS, say: None });
        // The relay answers: the screen hears so once.
        assert_eq!(r.answered(), RetryStep::Answered { was_retrying: true });
        assert_eq!(r.answered(), RetryStep::Answered { was_retrying: false });
        assert_eq!(r.failed("timed out".into(), at(70), life), RetryStep::Wait { secs: 1, say: Some("timed out".into()) });
        // Never past the request's life.
        assert_eq!(r.failed("timed out".into(), life, life), RetryStep::GiveUp("timed out".into()));
    }
}
