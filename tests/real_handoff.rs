//! The passkey hand-off end to end with the real backend: Ana's computer
//! running lt is linked as one of her devices; a setup step whose write the
//! workspace's rules hold for her passkey goes to her desktop (here, a
//! desktop-side approver stub on her founding device's own data dir and the
//! in-process test relay's approval mailbox), which confirms it with her
//! passkey (her recovery key stands in for the browser ceremony), says Not
//! me, or lets it expire. Temporary data dirs, file secrets.

mod support;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ligus_core::presence::RecoveryKey;
use ligus_core::{SessionEvent, workspace_config_session_id};
use ligus_runtime::IdentityStore;
use ligus_runtime::approval_mailbox::{
    self as am, ApproverIdentity, ApproverPerson, EpochKey, LiveRequest, MailboxClient, PresenceApproval, PresenceView, SeenSet, Signer,
};
use ligus_runtime::computer_link::LinkRequest;
use ligus_runtime::secret_store::StoreKeyVault;
use ligus_runtime::setup::FlowId;
use ligus_runtime::workspace_invite::{decode_workspace_invite, encode_workspace_invite};
use ligus_term::backend::model::*;
use ligus_term::backend::{Backend, Event};
use ligus_term::setup::{CancelFlag, LtFlow, SetupInputs, SetupOutcome, SetupSecrets};
use support::*;
use uuid::Uuid;

const MIN: i64 = 60 * 1000;

/// Ana's desktop (her founding device), her linked laptop running lt, and
/// the desktop's side of the approval mailbox.
struct World {
    tmp: tempfile::TempDir,
    relay: String,
    clock: ligus_runtime::test_relay::TestClock,
    /// The relay answers the mailbox routes with 404 (an older relay).
    no_mailbox: Arc<AtomicBool>,
    /// The relay refuses to delete a request (so it stays readable).
    no_delete: Arc<AtomicBool>,
    a: Device,
    laptop: Device,
    ws: WorkspaceId,
    wsu: Uuid,
    conn: String,
    ek: EpochKey,
    /// Ana's recovery key: her passkey on the desktop, for the stub.
    key: RecoveryKey,
    /// The requests the desktop stub has answered.
    handled: std::sync::Mutex<HashSet<String>>,
}

/// The test relay with switches for an older relay (no mailbox routes) and
/// a relay that keeps ended requests.
fn relay_router(no_mailbox: Arc<AtomicBool>, no_delete: Arc<AtomicBool>) -> (axum::Router, ligus_runtime::test_relay::TestClock) {
    use axum::response::IntoResponse;
    let (router, clock) = ligus_runtime::test_relay::router_with_clock();
    let router = router.layer(axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
        let (off, keep) = (no_mailbox.clone(), no_delete.clone());
        async move {
            let mailbox = req.uri().path().starts_with("/v1/mailboxes");
            if mailbox && off.load(Ordering::SeqCst) {
                return axum::http::StatusCode::NOT_FOUND.into_response();
            }
            if mailbox && req.method() == axum::http::Method::DELETE && keep.load(Ordering::SeqCst) {
                return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            next.run(req).await
        }
    }));
    (router, clock)
}

async fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let (no_mailbox, no_delete) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let (router, clock) = relay_router(no_mailbox.clone(), no_delete.clone());
    let relay = serve(router).await;
    let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
    let created = a.b.create_workspace("Ana", "Acme Web").await.unwrap();
    let ws = created.workspace.id.clone();
    let wsu = Uuid::parse_str(ws.as_str()).unwrap();
    let key = RecoveryKey::from_words(&created.recovery_words.join(" ")).unwrap();
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    let mut c = decode_workspace_invite(&invite).unwrap();
    c.inv = None;
    c.sig = None;
    let conn = encode_workspace_invite(&c);
    let ek = match c.ek.iter().filter_map(|d| d.key().map(|k| (d.v, k))).max_by_key(|(v, _)| *v) {
        Some((epoch, key)) => EpochKey { epoch, key },
        None => EpochKey { epoch: 0, key: ligus_core::derive_workspace_key(c.key.as_deref().unwrap()) },
    };
    // The desktop keeps the workspace's approval mailbox registered.
    MailboxClient::new(&relay).with_clock(clock.clock()).ensure_registered(&ek.key).await.unwrap();

    let laptop = device_with(&tmp.path().join("laptop"), "lt on laptop", &relay, |cfg| {
        cfg.mailbox_clock = Some(clock.clock());
    });
    laptop.manual.store(true, Ordering::SeqCst);
    let req = laptop.b.link_request().await.unwrap();
    let svc = a.b.service().unwrap();
    let grant = {
        let (request, conn) = (LinkRequest::decode(&req.code).unwrap(), conn.clone());
        tokio::task::spawn_blocking(move || svc.lock().unwrap().issue_computer_link(wsu, &request, &conn, ligus_core::flow::now_ms()))
            .await
            .unwrap()
            .unwrap()
    };
    until("the link to be redeemed", || async {
        match laptop.b.redeem_link(&grant).await {
            Ok(l) => Some(l),
            Err(e) if e.0.contains("hasn't reached this computer") => None,
            Err(e) => panic!("redeem failed: {e}"),
        }
    })
    .await;
    // The laptop knows the desktop as one of Ana's devices.
    let desk = a.b.device_id().unwrap();
    until("the laptop to see Ana's desktop as hers", || async {
        let svc = laptop.b.service().unwrap();
        let mine = tokio::task::spawn_blocking(move || svc.lock().unwrap().my_device_ids(wsu).unwrap()).await.unwrap();
        mine.contains(&desk).then_some(())
    })
    .await;
    World { tmp, relay, clock, no_mailbox, no_delete, a, laptop, ws, wsu, conn, ek, key, handled: Default::default() }
}

impl World {
    fn client(&self) -> MailboxClient {
        MailboxClient::new(&self.relay).with_clock(self.clock.clock())
    }

    /// Who an approver on `dir`'s device approves as.
    fn identity(&self, dir: &std::path::Path, name: &str) -> ApproverIdentity {
        let ids = IdentityStore::new(dir, StoreKeyVault::for_data_dir(dir));
        let stored = ids.load().unwrap().unwrap();
        ApproverIdentity {
            signer: Signer::Ed25519(ids.device_keypair(stored.device.id).unwrap()),
            device_name: name.into(),
            person: ApproverPerson { account_id: stored.account.id, name: name.into() },
        }
    }

    fn desk(&self) -> ApproverIdentity {
        self.identity(&self.tmp.path().join("ana"), "Ana's desktop")
    }

    /// The next hand-off the desktop hasn't answered, checked against the
    /// desktop's own folded workspace (its device and account) and rendered
    /// from the draft.
    async fn desk_sees(&self) -> (LiveRequest, PresenceView) {
        let svc = self.a.b.service().unwrap();
        let acct = svc.lock().unwrap().author().account_id.unwrap();
        until("a hand-off in the mailbox", || async {
            let l = am::list_live(&self.client(), std::slice::from_ref(&self.ek)).await.unwrap();
            let r = l.live.into_iter().find(|r| {
                r.kind() == ligus_core::match_code::MatchKind::Presence
                    && r.ended.is_none()
                    && !self.handled.lock().unwrap().contains(&r.rid_hex())
            })?;
            self.handled.lock().unwrap().insert(r.rid_hex());
            let wsu = self.wsu;
            let view = am::check_presence_ask(&r.request, wsu, acct, &|d| svc.lock().unwrap().is_my_device(wsu, d).unwrap())
                .expect("the desktop may confirm it");
            Some((r, view))
        })
        .await
    }

    /// The desktop opens the hand-off (its claim).
    async fn desk_opens(&self, live: &LiveRequest) -> PresenceApproval {
        PresenceApproval::begin(self.client(), &self.ek, &live.request, self.desk(), &mut SeenSet::in_memory()).await.unwrap()
    }

    /// Run the Folder flow on the laptop for a fresh git checkout.
    fn folder(&self, name: &str) -> tokio::task::JoinHandle<ligus_term::backend::Result<SetupOutcome>> {
        let repo = self.tmp.path().join(name);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-q", "--allow-empty", "-m", "root"]);
        git(&repo, &["remote", "add", "origin", &format!("https://github.com/acme/{name}")]);
        let inputs: SetupInputs = [("path".to_string(), repo.display().to_string())].into_iter().collect();
        let (b, ws) = (self.laptop.b.clone(), self.ws.clone());
        tokio::spawn(async move {
            b.setup_run(Some(&ws), LtFlow::Engine(FlowId::Folder), inputs, SetupSecrets::new(), false, CancelFlag::default()).await
        })
    }

    /// The laptop's hand-off prompts so far.
    fn prompts(&self) -> Vec<PresencePrompt> {
        self.laptop
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::PresencePrompt(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    async fn prompt_with(&self, code: &str) -> PresencePrompt {
        until("lt's hand-off sheet", || async { self.prompts().into_iter().find(|p| p.handoff.as_deref() == Some(code)) }).await
    }

    /// The workspace repositories as `d`'s log has them.
    async fn repos(&self, d: &Device) -> Vec<String> {
        let svc = d.b.service().unwrap();
        let wsu = self.wsu;
        tokio::task::spawn_blocking(move || {
            svc.lock()
                .unwrap()
                .store()
                .list(workspace_config_session_id(wsu))
                .unwrap()
                .into_iter()
                .filter_map(|e| match e.payload {
                    SessionEvent::WorkspaceRepoAdded { repo } => Some(repo.display_name),
                    _ => None,
                })
                .collect()
        })
        .await
        .unwrap()
    }

    /// The challenge of the laptop's stored `WorkspaceRepoAdded` for `name`.
    async fn stored_challenge(&self, name: &str) -> Option<[u8; 32]> {
        let svc = self.laptop.b.service().unwrap();
        let (wsu, name) = (self.wsu, name.to_string());
        tokio::task::spawn_blocking(move || {
            svc.lock()
                .unwrap()
                .store()
                .list(workspace_config_session_id(wsu))
                .unwrap()
                .into_iter()
                .find(|e| matches!(&e.payload, SessionEvent::WorkspaceRepoAdded { repo } if repo.display_name == name))
                .map(|e| {
                    assert!(e.actor_stamp.as_ref().and_then(|s| s.presence.as_ref()).is_some(), "stored with the passkey's assertion");
                    ligus_core::presence::challenge(&e)
                })
        })
        .await
        .unwrap()
    }

    fn dead(&self) -> usize {
        am::DeadLog::new(&self.tmp.path().join("laptop")).recent(self.clock.now()).unwrap()
    }
}

/// Confirm every hand-off the run raises on the desktop, until it ends.
async fn confirm_all(
    w: &World,
    run: tokio::task::JoinHandle<ligus_term::backend::Result<SetupOutcome>>,
) -> (SetupOutcome, Vec<PresenceView>) {
    let mut views = vec![];
    loop {
        if run.is_finished() {
            return (run.await.unwrap().unwrap(), views);
        }
        let l = am::list_live(&w.client(), std::slice::from_ref(&w.ek)).await.unwrap();
        let fresh = l.live.iter().any(|r| r.ended.is_none() && !w.handled.lock().unwrap().contains(&r.rid_hex()));
        if fresh {
            let (live, view) = w.desk_sees().await;
            let mut a = w.desk_opens(&live).await;
            a.approve(&w.key.assert(&view.challenge)).await.unwrap();
            views.push(view);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn says(o: &SetupOutcome, what: &str) -> bool {
    format!("{o:?}").contains(what)
}

/// Confirmed on the desktop: lt shows the code the desktop shows, a
/// stranger's answer (another member's device, with Ana's own valid
/// assertion) doesn't count, and the change is stored with Ana's
/// confirmation exactly as the desktop saw it. Not me writes nothing; Esc
/// sends the cancel abort.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_setup_step_is_confirmed_on_the_desktop() {
    let _watchdog = watchdog("a_setup_step_is_confirmed_on_the_desktop", 600);
    let w = world().await;

    // Ben, another member, holds the workspace key too.
    let invite = w.a.b.create_invite(&w.ws, Role::Contributor).await.unwrap();
    let ben = device(&w.tmp.path().join("ben"), "Ben's desktop", &w.relay);
    ben.b.join_workspace("Ben", &invite).await.unwrap();

    // 1. A stranger answers first; then Ana's desktop.
    let run = w.folder("web");
    let (live, view) = w.desk_sees().await;
    assert_eq!(view.kind, "workspaceRepoAdded");
    let p = w.prompt_with(&view.code.shown).await;
    assert_eq!(p.summary, view.summary, "lt and the desktop show the same change");
    assert!(!p.resumed);
    // The pending entry is owner-only while it waits.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let f = w.tmp.path().join("laptop").join(am::PRESENCE_PENDING_FILE);
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let mut stranger = PresenceApproval::begin(
        w.client(),
        &w.ek,
        &live.request,
        w.identity(&w.tmp.path().join("ben"), "Ben's desktop"),
        &mut SeenSet::in_memory(),
    )
    .await
    .unwrap();
    stranger.approve(&w.key.assert(&view.challenge)).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!run.is_finished(), "a stranger's answer doesn't count");
    let mut a = w.desk_opens(&live).await;
    a.approve(&w.key.assert(&view.challenge)).await.unwrap();
    let (o, more) = confirm_all(&w, run).await;
    assert_eq!(o.exit, 0, "{o:?}");
    assert_eq!(w.stored_challenge("web").await, Some(view.challenge), "stored as the desktop saw it");
    let _ = more;
    until("Ana's desktop to have it", || async { w.repos(&w.a).await.contains(&"web".to_string()).then_some(()) }).await;
    assert!(w.laptop.saw(|e| matches!(e, Event::PresenceDone { ok: true, .. })));
    assert!(w.laptop.b.pending_presence().await.unwrap().is_empty(), "nothing left waiting");

    // 2. Not me: nothing is written, and lt says so.
    let run = w.folder("nope");
    let (live, _) = w.desk_sees().await;
    w.desk_opens(&live).await.decline().await.unwrap();
    let o = run.await.unwrap().unwrap();
    assert_ne!(o.exit, 0);
    assert!(says(&o, "Declined on your other device: nothing was written"), "{o:?}");
    assert!(!w.repos(&w.laptop).await.contains(&"nope".to_string()));

    // 3. Esc: the cancel abort reaches the desktop (the relay keeps the
    //    request here, so the desktop reads it); nothing is written.
    w.no_delete.store(true, Ordering::SeqCst);
    let dead = w.dead();
    let run = w.folder("esc");
    let (live, view) = w.desk_sees().await;
    let mut a = w.desk_opens(&live).await;
    let p = w.prompt_with(&view.code.shown).await;
    w.laptop.b.answer_presence(p.id, PresenceAnswer::Cancel).await.unwrap();
    let o = run.await.unwrap().unwrap();
    assert_ne!(o.exit, 0, "{o:?}");
    assert_eq!(a.ready().await.unwrap_err(), am::ApproverError::Aborted(am::AbortReason::Cancelled));
    assert!(!w.repos(&w.laptop).await.contains(&"esc".to_string()));
    assert_eq!(w.dead(), dead + 1, "an ended hand-off counts toward the limit");
    w.no_delete.store(false, Ordering::SeqCst);

    // 4. A replayed approval (the first change's confirmation, for this
    //    one) isn't written: it doesn't confirm this change.
    let run = w.folder("replay");
    let (live, _) = w.desk_sees().await;
    w.desk_opens(&live).await.approve(&w.key.assert(&view.challenge)).await.unwrap();
    let o = run.await.unwrap().unwrap();
    assert_ne!(o.exit, 0, "{o:?}");
    assert!(!w.repos(&w.laptop).await.contains(&"replay".to_string()));
}

/// No answer for 15 minutes: lt says so and asks again on enter (a new
/// code), and the desktop's confirmation of the new request is written.
/// A hand-off left waiting when lt quit is offered and finished on the next
/// start; one asked by another device isn't. A relay without the mailbox
/// keeps today's sheet.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expiry_retry_resume_and_fallback() {
    let _watchdog = watchdog("expiry_retry_resume_and_fallback", 600);
    let w = world().await;

    // 1. Expiry, then ask again.
    let run = w.folder("late");
    let (_, first) = w.desk_sees().await;
    let p = w.prompt_with(&first.code.shown).await;
    w.clock.advance(16 * MIN);
    let why = until("lt to say it expired", || async {
        w.laptop.events.lock().unwrap().iter().find_map(|e| match e {
            Event::PresenceHandoff { id, update: HandoffUpdate::Stopped { why } } if *id == p.id => Some(why.clone()),
            _ => None,
        })
    })
    .await;
    assert!(why.contains("expired"), "{why}");
    w.laptop.b.answer_presence(p.id, PresenceAnswer::Handoff).await.unwrap();
    let code = until("a new code", || async {
        w.laptop.events.lock().unwrap().iter().find_map(|e| match e {
            Event::PresenceHandoff { id, update: HandoffUpdate::Waiting { code } } if *id == p.id => Some(code.clone()),
            _ => None,
        })
    })
    .await;
    assert_ne!(code, first.code.shown);
    let (live, view) = w.desk_sees().await;
    assert_eq!(view.code.shown, code);
    w.desk_opens(&live).await.approve(&w.key.assert(&view.challenge)).await.unwrap();
    let (o, _) = confirm_all(&w, run).await;
    assert_eq!(o.exit, 0, "{o:?}");
    assert_eq!(w.stored_challenge("late").await, Some(view.challenge));

    // 2. lt quit while it waited: the request stays in presence-pending.json
    //    (here posted as the hand-off posts it, and left).
    let rid = left_waiting(&w).await;
    let pending = w.laptop.b.pending_presence().await.unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].rid, rid);
    assert!(pending[0].summary.contains("coder"), "{pending:?}");
    // Not offered on another device's data dir (the desktop's).
    let theirs = w.tmp.path().join("ana").join(am::PRESENCE_PENDING_FILE);
    std::fs::copy(w.tmp.path().join("laptop").join(am::PRESENCE_PENDING_FILE), &theirs).unwrap();
    assert!(w.a.b.pending_presence().await.unwrap().is_empty(), "asked by another device");
    assert!(w.a.b.resume_presence(&rid).await.is_err());
    std::fs::remove_file(&theirs).unwrap();
    // Resumed: a sheet that offers waiting; confirmed, then written.
    let (b, r) = (w.laptop.b.clone(), rid.clone());
    let resumed = tokio::spawn(async move { b.resume_presence(&r).await });
    let (live, view) = w.desk_sees().await;
    let p = w.prompt_with(&view.code.shown).await;
    assert!(p.resumed, "{p:?}");
    w.desk_opens(&live).await.approve(&w.key.assert(&view.challenge)).await.unwrap();
    let summary = resumed.await.unwrap().unwrap();
    assert!(summary.contains("coder"), "{summary}");
    let svc = w.laptop.b.service().unwrap();
    let wsu = w.wsu;
    let names = tokio::task::spawn_blocking(move || svc.lock().unwrap().list_workspace_runtimes(wsu).unwrap()).await.unwrap();
    assert!(names.iter().any(|r| r.name == "coder"), "{names:?}");
    assert!(w.laptop.b.pending_presence().await.unwrap().is_empty());

    // 3. Left waiting again, then cancelled from the list: nothing written.
    let rid = left_waiting_named(&w, "other").await;
    w.laptop.b.cancel_pending_presence(&rid).await.unwrap();
    assert!(w.laptop.b.pending_presence().await.unwrap().is_empty());

    // 4. An older relay (no mailbox routes): today's sheet, unchanged.
    w.no_mailbox.store(true, Ordering::SeqCst);
    let before = w.prompts().len();
    let run = w.folder("old");
    let p = until("today's sheet", || async { w.prompts().get(before).cloned() }).await;
    assert_eq!(p.handoff, None, "{p:?}");
    w.laptop.b.answer_presence(p.id, PresenceAnswer::Cancel).await.unwrap();
    let o = run.await.unwrap().unwrap();
    assert_ne!(o.exit, 0, "{o:?}");
    assert!(!w.repos(&w.laptop).await.contains(&"old".to_string()));
}

async fn left_waiting(w: &World) -> String {
    left_waiting_named(w, "coder").await
}

/// What a hand-off leaves when lt quits while it waits: the laptop's draft
/// of a workspace runtime, posted to the mailbox with the pending entry,
/// and nobody waiting for it here.
async fn left_waiting_named(w: &World, name: &str) -> String {
    let svc = w.laptop.b.service().unwrap();
    let (wsu, name) = (w.wsu, name.to_string());
    let draft = tokio::task::spawn_blocking(move || {
        let mut s = svc.lock().unwrap();
        s.capture_presence(true);
        let rt = ligus_core::WorkspaceRuntime::new(format!("rt-{name}"), &name, ligus_core::ModelProviderKind::ClaudeCode, "sonnet");
        assert!(s.add_workspace_runtime(wsu, rt).is_err(), "captured, not written");
        s.take_presence_draft().expect("the draft")
    })
    .await
    .unwrap();
    let dir = w.tmp.path().join("laptop");
    let ids = IdentityStore::new(&dir, StoreKeyVault::for_data_dir(&dir));
    let device = ids.load().unwrap().unwrap().device.id;
    let signer = Signer::Ed25519(ids.device_keypair(device).unwrap());
    let c = decode_workspace_invite(&w.conn).unwrap();
    let address = am::current_address(&w.client(), w.wsu, c.fk.as_deref().unwrap(), std::slice::from_ref(&w.ek)).await.unwrap();
    let ask = am::PresenceAsk::new("lt on laptop", &signer.public_key(), &draft.env).unwrap();
    let req = am::PresenceRequester::post(w.client(), &address, signer, ask, &draft.request.summary, Some(&dir)).await.unwrap();
    req.rid_hex()
}
