//! "Add to my devices" end to end with the real backend: a throwaway
//! workspace founded by one `lt` device (Ana's "desktop", with her software
//! passkey), and a second computer that links as one of her devices instead
//! of joining as a new member. Temporary data dirs, file secrets, the
//! in-process test relay.

mod support;

use ligus_runtime::computer_link::LinkRequest;
use ligus_runtime::workspace_invite::{decode_workspace_invite, encode_workspace_invite};
use ligus_term::backend::Backend;
use ligus_term::backend::model::*;
use support::*;
use uuid::Uuid;

/// Ana's workspace and the connection her "desktop" seals into a grant (no
/// invite in it).
async fn founded(a: &Device, tmp: &std::path::Path) -> (WorkspaceId, Uuid, String) {
    let created = a.b.create_workspace("Ana", "Acme Web").await.unwrap();
    let ws = created.workspace.id.clone();
    let words = tmp.join("words.txt");
    std::fs::write(&words, created.recovery_words.join(" ")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    *a.words_file.lock().unwrap() = Some(words);
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    let mut conn = decode_workspace_invite(&invite).unwrap();
    conn.inv = None;
    conn.sig = None;
    (ws.clone(), Uuid::parse_str(ws.as_str()).unwrap(), encode_workspace_invite(&conn))
}

/// Ana's desktop approves `request` (her passkey, in the soft browser).
async fn approve(a: &Device, ws: Uuid, request: &str, conn: &str) -> String {
    let svc = a.b.service().unwrap();
    let (request, conn) = (LinkRequest::decode(request).unwrap(), conn.to_string());
    tokio::task::spawn_blocking(move || svc.lock().unwrap().issue_computer_link(ws, &request, &conn, ligus_core::flow::now_ms()))
        .await
        .unwrap()
        .unwrap()
}

/// Redeem, retrying while Ana's certificate is still on its way to the relay.
async fn redeem(b: &Device, grant: &str) -> WorkspaceSummary {
    let start = std::time::Instant::now();
    let mut tries = 0;
    until("the link to be redeemed", || {
        tries += 1;
        async move {
            match b.b.redeem_link(grant).await {
                Ok(l) => {
                    assert_eq!(l.unconfirmed, None, "confirmed here: {l:?}");
                    Some(l.workspace)
                }
                Err(e) if e.0.contains("hasn't reached this computer") => {
                    trace(&format!("redeem try {tries} at {:?}: {e}", start.elapsed()));
                    None
                }
                Err(e) => panic!("redeem failed: {e}"),
            }
        }
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fresh_install_links_as_the_persons_device() {
    let _watchdog = watchdog("a_fresh_install_links_as_the_persons_device", 420);
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
    let (ws, wsu, conn) = founded(&a, tmp.path()).await;
    let chat = a.b.create_chat(&ws, "Plans").await.unwrap();

    let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);
    assert!(laptop.b.linked_person().is_none());
    let req = laptop.b.link_request().await.unwrap();
    assert!(req.code.starts_with("liguslink1:") && !req.new_device, "{req:?}");
    assert_eq!(req.fingerprint.len(), 24);
    let grant = approve(&a, wsu, &req.code, &conn).await;

    // Who approved it, before anything is saved.
    let check = laptop.b.inspect_link_grant(&grant).await.unwrap();
    assert_eq!(check.person_name, "Ana");
    assert_eq!(check.workspace_name, "Acme Web");
    assert!(laptop.b.workspaces().await.unwrap().is_empty(), "nothing saved by inspecting");
    let joined = redeem(&laptop, &grant).await;
    assert_eq!(joined.id, ws);
    assert_eq!(joined.your_role, Role::Owner, "it carries Ana's role");
    assert_eq!(laptop.b.linked_person().as_deref(), Some("Ana"));

    // It posts as Ana; there is one Ana, not two.
    until("the laptop to see the chat", || async { laptop.b.chats(&ws).await.unwrap().iter().any(|c| c.id == chat.id).then_some(()) })
        .await;
    laptop.b.send_message(&ws, &chat.id, "from the laptop").await.unwrap();
    until("Ana's desktop to see it as Ana's", || async {
        a.b.transcript(&ws, &chat.id)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "from the laptop" && matches!(&m.author, Author::Member { name, .. } if name == "Ana"))
            .then_some(())
    })
    .await;
    let people = a.b.members(&ws).await.unwrap();
    assert_eq!(people.iter().filter(|m| m.name == "Ana").count(), 1, "{people:?}");

    // Single use; and a linked computer founds and joins nothing.
    assert!(laptop.b.redeem_link(&grant).await.unwrap_err().0.contains("doesn't answer a request"));
    let e = laptop.b.create_workspace("Ana", "Side project").await.unwrap_err();
    assert!(e.0.contains("This computer is Ana's device"), "{e}");
    assert!(laptop.b.join_workspace("Ana", &conn).await.is_err());

    // Reopened from disk, it is still Ana's device.
    laptop.b.shutdown();
    drop(laptop);
    let again = device(&tmp.path().join("laptop"), "lt on laptop", &relay);
    assert_eq!(again.b.startup().await.unwrap().your_name.as_deref(), Some("Ana"));
    assert_eq!(again.b.linked_person().as_deref(), Some("Ana"));
}

/// The desktop's approval is slow to reach this computer: redeeming waits
/// and says so, stops cleanly when asked (nothing half done), and links
/// once the approval arrives.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redeem_waits_for_a_slow_approval_and_can_stop() {
    let _watchdog = watchdog("redeem_waits_for_a_slow_approval_and_can_stop", 420);
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
    let (ws, wsu, conn) = founded(&a, tmp.path()).await;
    a.b.publish_now(wsu).await.unwrap();
    let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);
    let req = laptop.b.link_request().await.unwrap();
    // The desktop goes quiet: its approval stays on it for now.
    a.b.shutdown();
    let grant = approve(&a, wsu, &req.code, &conn).await;

    let waiting = |d: &Device| d.events.lock().unwrap().iter().any(|e| matches!(e, ligus_term::backend::Event::LinkWaiting { .. }));
    let (l2, g2) = (laptop.b.clone(), grant.clone());
    let task = tokio::spawn(async move { l2.redeem_link(&g2).await });
    until("lt to say it is waiting", || async { waiting(&laptop).then_some(()) }).await;
    laptop.b.cancel_link_redeem();
    let e = task.await.unwrap().unwrap_err();
    assert!(e.0.contains("stopped waiting"), "{e}");
    assert!(laptop.b.linked_person().is_none(), "nothing linked");
    assert!(laptop.b.workspaces().await.unwrap().is_empty(), "nothing half done");

    // Again; the approval arrives while it waits.
    let (l2, g2) = (laptop.b.clone(), grant.clone());
    let task = tokio::spawn(async move { l2.redeem_link(&g2).await });
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    a.b.publish_now(wsu).await.unwrap();
    let linked = task.await.unwrap().unwrap();
    assert_eq!(linked.workspace.id, ws);
    assert_eq!(linked.unconfirmed, None, "{linked:?}");
    assert_eq!(laptop.b.linked_person().as_deref(), Some("Ana"));
}

/// Today's two-person state: Ana invited herself into lt as "Ana (lt)". Linking
/// the existing install asks with a new device key and proves it was that
/// member; afterwards it acts as Ana and the desktop can remove the duplicate.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_existing_install_moves_from_its_own_member_to_the_person() {
    let _watchdog = watchdog("an_existing_install_moves_from_its_own_member_to_the_person", 420);
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
    let (ws, wsu, conn) = founded(&a, tmp.path()).await;
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    let lt = device(&tmp.path().join("lt"), "lt", &relay);
    lt.b.join_workspace("Ana (lt)", &invite).await.unwrap();
    until("Ana to see the second Ana", || async { (a.b.members(&ws).await.unwrap().len() == 2).then_some(()) }).await;
    let old_device = lt.b.device_id().unwrap();

    let req = lt.b.link_request().await.unwrap();
    assert!(req.new_device, "an install in use asks with a new device key");
    let decoded = LinkRequest::decode(&req.code).unwrap();
    assert_ne!(decoded.device_id, old_device);
    let svc = a.b.service().unwrap();
    let preview = {
        let d = decoded.clone();
        tokio::task::spawn_blocking(move || svc.lock().unwrap().preview_computer_link(wsu, &d)).await.unwrap().unwrap()
    };
    assert_eq!(preview.previous_member.map(|m| m.actor.display_name), Some("Ana (lt)".into()));
    let grant = approve(&a, wsu, &req.code, &conn).await;
    redeem(&lt, &grant).await;
    assert_ne!(lt.b.device_id().unwrap(), old_device, "switched to the new device key");
    assert_eq!(lt.b.linked_person().as_deref(), Some("Ana"));
    let me = lt.b.members(&ws).await.unwrap().into_iter().find(|m| m.you).unwrap();
    assert_eq!(me.name, "Ana");
}

// ── Live: code match through the approval address ──────────────────────

mod live {
    use super::*;
    use ligus_runtime::IdentityStore;
    use ligus_runtime::approval_mailbox::{
        ApprovalAddress, ApproverIdentity, ApproverPerson, ApproverSession, EpochKey, MailboxClient, RequestSubject, SeenSet, Signer,
        list_live,
    };
    use ligus_runtime::secret_store::StoreKeyVault;
    use ligus_term::backend::Event;

    /// Ana's desktop side of the mailbox: its epoch key, the address it
    /// shows, and who it approves as (her own device key).
    struct Desk {
        client: MailboxClient,
        ek: EpochKey,
        address: String,
        me: ApproverIdentity,
    }

    async fn desk(a: &Device, dir: &std::path::Path, relay: &str, wsu: Uuid, conn: &str) -> Desk {
        let c = decode_workspace_invite(conn).unwrap();
        let ek = match c.ek.iter().filter_map(|d| d.key().map(|k| (d.v, k))).max_by_key(|(v, _)| *v) {
            Some((epoch, key)) => EpochKey { epoch, key },
            None => EpochKey { epoch: 0, key: ligus_core::derive_workspace_key(c.key.as_deref().unwrap()) },
        };
        let client = MailboxClient::new(relay);
        let n = client.ensure_registered(&ek.key).await.unwrap();
        let address = ApprovalAddress::derive(relay, wsu, c.fk.as_deref().unwrap(), ek.epoch, &ek.key, n).encode();
        let ids = IdentityStore::new(dir, StoreKeyVault::for_data_dir(dir));
        let stored = ids.load().unwrap().unwrap();
        assert_eq!(Some(stored.device.id), a.b.device_id());
        let me = ApproverIdentity {
            signer: Signer::Ed25519(ids.device_keypair(stored.device.id).unwrap()),
            device_name: "Ana's desktop".into(),
            person: ApproverPerson { account_id: stored.account.id, name: "Ana".into() },
        };
        Desk { client, ek, address, me }
    }

    /// Wait for lt's acknowledgement of the desktop's offer.
    async fn acked(session: &mut ApproverSession) {
        for _ in 0..30 {
            if session.await_ack(1).await.unwrap() {
                return;
            }
        }
        panic!("no ack");
    }

    /// The live updates `d` emitted for `rid`, in order.
    fn updates(d: &Device, rid: &str) -> Vec<LiveLinkUpdate> {
        d.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::LiveLink { rid: r, update } if r == rid => Some(update.clone()),
                _ => None,
            })
            .collect()
    }

    async fn update_where(d: &Device, rid: &str, what: &str, f: impl Fn(&LiveLinkUpdate) -> bool) -> LiveLinkUpdate {
        until(what, || async { updates(d, rid).into_iter().find(|u| f(u)) }).await
    }

    /// Ana's desktop opens the request and both sides see the same code;
    /// the desktop's session, the code it shows and the request.
    async fn open(desk: Desk, laptop: &Device, rid: &str) -> (ApproverSession, String, LinkRequest) {
        let l = until("the request in the mailbox", || async {
            list_live(&desk.client, std::slice::from_ref(&desk.ek)).await.unwrap().live.into_iter().find(|l| l.rid_hex() == rid)
        })
        .await;
        let mut session =
            ApproverSession::begin(desk.client.clone(), &desk.ek, &l.request, desk.me, &mut SeenSet::in_memory()).await.unwrap();
        let LiveLinkUpdate::Opened(code) = update_where(laptop, rid, "the code on lt", |u| matches!(u, LiveLinkUpdate::Opened(_))).await
        else {
            unreachable!()
        };
        assert_eq!(code.claimed, "Ana's desktop");
        let mut desk_code = None;
        for _ in 0..50 {
            if let Some(c) = session.await_reveal(1).await.unwrap() {
                desk_code = Some(c);
                break;
            }
        }
        let desk_code = desk_code.expect("the reveal");
        assert_eq!(desk_code.shown, code.shown, "both screens show the same code");
        let RequestSubject::Computer(req) = session.request().subject.clone() else { panic!() };
        (session, code.shown, req)
    }

    /// The codes match, Ana confirms on both screens, her desktop offers,
    /// lt acknowledges, the passkey publishes the certificate, lt checks it
    /// on the log and asks to link as the verified person: Yes links,
    /// exactly as the paste path does.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_fresh_install_links_through_the_approval_address() {
        let tmp = tempfile::tempdir().unwrap();
        let relay = serve(ligus_runtime::test_relay::router()).await;
        let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
        let (ws, wsu, conn) = founded(&a, tmp.path()).await;
        let d = desk(&a, &tmp.path().join("ana"), &relay, wsu, &conn).await;
        let address = d.address.clone();

        let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);
        let LiveLinkStart::Posted { rid, request } = laptop.b.link_live(&address).await.unwrap() else { panic!("no mailbox?") };
        assert!(request.code.starts_with("liguslink1:"), "the paste text for an older desktop too");
        // Any member's lt lists it, read only.
        let listed = until("the request in Ana's approvals", || async { a.b.approvals(&ws).await.unwrap().into_iter().next() }).await;
        assert_eq!(
            (listed.kind, listed.label.as_str(), listed.checking.as_deref(), listed.contested),
            (ApprovalKind::Computer, "lt on laptop", None, false)
        );
        let (mut session, _, req) = open(d, &laptop, &rid).await;
        let listed = a.b.approvals(&ws).await.unwrap();
        assert!(listed[0].mine && !listed[0].contested, "{listed:?}");

        // Nothing is acknowledged before the person says the codes match.
        let svc = a.b.service().unwrap();
        let grant = {
            let (svc, req, conn) = (svc.clone(), req.clone(), conn.clone());
            tokio::task::spawn_blocking(move || svc.lock().unwrap().prepare_computer_link(wsu, &req, &conn, ligus_core::flow::now_ms()))
                .await
                .unwrap()
                .unwrap()
        };
        session.offer(grant).await.unwrap();
        assert!(!session.await_ack(2).await.unwrap(), "no ack before Yes");
        laptop.b.link_live_code(&rid, true).await.unwrap();
        acked(&mut session).await;
        update_where(&laptop, &rid, "Acknowledged", |u| *u == LiveLinkUpdate::Acknowledged).await;
        // Only now: Ana's passkey and the certificate.
        {
            let (svc, req, conn) = (svc.clone(), req.clone(), conn.clone());
            tokio::task::spawn_blocking(move || svc.lock().unwrap().publish_computer_link(wsu, &req, &conn, ligus_core::flow::now_ms()))
                .await
                .unwrap()
                .unwrap();
        }
        session.finish_approved(None).await.unwrap();
        let v =
            update_where(&laptop, &rid, "the verified person", |u| matches!(u, LiveLinkUpdate::Verified { .. } | LiveLinkUpdate::Ended(_)))
                .await;
        assert_eq!(v, LiveLinkUpdate::Verified { person: "Ana".into(), workspace: "Acme Web".into() });
        assert!(laptop.b.linked_person().is_none(), "not linked before Yes, link");
        let joined = laptop.b.link_live_person(&rid, true).await.unwrap().expect("linked");
        assert_eq!(joined.unconfirmed, None, "confirmed here: {joined:?}");
        let joined = joined.workspace;
        assert_eq!(joined.id, ws);
        assert_eq!(joined.your_role, Role::Owner, "it carries Ana's role");
        assert_eq!(laptop.b.linked_person().as_deref(), Some("Ana"));
        assert!(!tmp.path().join("laptop").join(ligus_runtime::approval_mailbox::PENDING_FILE).exists(), "the request was settled");
    }

    /// Check 3 pins the relay: an approval whose connection names another
    /// relay than the address (here the same server under another name) is
    /// refused, and nothing is linked.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_grant_naming_another_relay_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let relay = serve(ligus_runtime::test_relay::router()).await;
        let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
        let (_ws, wsu, conn) = founded(&a, tmp.path()).await;
        let elsewhere = relay.replace("127.0.0.1", "localhost");
        let d = desk(&a, &tmp.path().join("ana"), &elsewhere, wsu, &conn).await;
        let address = d.address.clone();
        let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);
        let LiveLinkStart::Posted { rid, .. } = laptop.b.link_live(&address).await.unwrap() else { panic!() };
        let (mut session, _, req) = open(d, &laptop, &rid).await;
        laptop.b.link_live_code(&rid, true).await.unwrap();
        let svc = a.b.service().unwrap();
        let grant = {
            let (svc, req, conn) = (svc.clone(), req.clone(), conn.clone());
            tokio::task::spawn_blocking(move || svc.lock().unwrap().prepare_computer_link(wsu, &req, &conn, ligus_core::flow::now_ms()))
                .await
                .unwrap()
                .unwrap()
        };
        session.offer(grant).await.unwrap();
        acked(&mut session).await;
        {
            let (svc, req, conn) = (svc.clone(), req.clone(), conn.clone());
            tokio::task::spawn_blocking(move || svc.lock().unwrap().publish_computer_link(wsu, &req, &conn, ligus_core::flow::now_ms()))
                .await
                .unwrap()
                .unwrap();
        }
        session.finish_approved(None).await.unwrap();
        let end = update_where(&laptop, &rid, "the end", |u| matches!(u, LiveLinkUpdate::Ended(_) | LiveLinkUpdate::Verified { .. })).await;
        let LiveLinkUpdate::Ended(LiveLinkEnd::Refused(why)) = end else { panic!("{end:?}") };
        assert!(why.contains("another relay"), "{why}");
        assert!(laptop.b.linked_person().is_none());
        assert!(laptop.b.workspaces().await.unwrap().is_empty());
    }

    /// The codes don't match: No ends the request and nothing is granted.
    /// After check 5, No on the verified person links nothing and puts the
    /// install back as it was.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn no_on_the_code_or_the_person_links_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let relay = serve(ligus_runtime::test_relay::router()).await;
        let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
        let (_ws, wsu, conn) = founded(&a, tmp.path()).await;
        let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);

        // No on the code.
        let d = desk(&a, &tmp.path().join("ana"), &relay, wsu, &conn).await;
        let address = d.address.clone();
        let LiveLinkStart::Posted { rid, .. } = laptop.b.link_live(&address).await.unwrap() else { panic!() };
        let (mut session, _, _) = open(d, &laptop, &rid).await;
        laptop.b.link_live_code(&rid, false).await.unwrap();
        let end = update_where(&laptop, &rid, "the end", |u| matches!(u, LiveLinkUpdate::Ended(_))).await;
        assert_eq!(end, LiveLinkUpdate::Ended(LiveLinkEnd::Mismatch));
        // The desktop's side ends too.
        let mut ended = false;
        for _ in 0..30 {
            if session.await_ack(1).await.is_err() {
                ended = true;
                break;
            }
        }
        assert!(ended, "the desktop sees the abort");

        // No on the verified person.
        let d = desk(&a, &tmp.path().join("ana"), &relay, wsu, &conn).await;
        let LiveLinkStart::Posted { rid, .. } = laptop.b.link_live(&address).await.unwrap() else { panic!() };
        let (mut session, _, req) = open(d, &laptop, &rid).await;
        laptop.b.link_live_code(&rid, true).await.unwrap();
        let svc = a.b.service().unwrap();
        let grant = {
            let (svc, req, conn) = (svc.clone(), req.clone(), conn.clone());
            tokio::task::spawn_blocking(move || svc.lock().unwrap().prepare_computer_link(wsu, &req, &conn, ligus_core::flow::now_ms()))
                .await
                .unwrap()
                .unwrap()
        };
        session.offer(grant).await.unwrap();
        acked(&mut session).await;
        {
            let (svc, req, conn) = (svc.clone(), req.clone(), conn.clone());
            tokio::task::spawn_blocking(move || svc.lock().unwrap().publish_computer_link(wsu, &req, &conn, ligus_core::flow::now_ms()))
                .await
                .unwrap()
                .unwrap();
        }
        session.finish_approved(None).await.unwrap();
        update_where(&laptop, &rid, "the verified person", |u| matches!(u, LiveLinkUpdate::Verified { .. })).await;
        assert_eq!(laptop.b.link_live_person(&rid, false).await.unwrap(), None);
        let end = update_where(&laptop, &rid, "the end", |u| matches!(u, LiveLinkUpdate::Ended(_))).await;
        assert_eq!(end, LiveLinkUpdate::Ended(LiveLinkEnd::NotLinked { person: "Ana".into() }));
        assert!(laptop.b.linked_person().is_none());
        assert!(laptop.b.workspaces().await.unwrap().is_empty(), "the connection was taken back");
    }

    /// A relay without an approval mailbox: the paste path (no error). Three
    /// requests that end without an approval stop the requester.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn no_mailbox_falls_back_and_three_dead_requests_stop() {
        let tmp = tempfile::tempdir().unwrap();
        let relay = serve(ligus_runtime::test_relay::router()).await;
        let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
        let (ws, wsu, conn) = founded(&a, tmp.path()).await;
        let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);

        let bare = serve(axum::Router::new()).await;
        let c = decode_workspace_invite(&conn).unwrap();
        let no_routes = ApprovalAddress::derive(&bare, wsu, c.fk.as_deref().unwrap(), 0, &[7; 32], 0).encode();
        let start = laptop.b.link_live(&no_routes).await.unwrap();
        assert!(matches!(start, LiveLinkStart::NoMailbox { ref request } if request.code.starts_with("liguslink1:")), "{start:?}");

        let d = desk(&a, &tmp.path().join("ana"), &relay, wsu, &conn).await;
        let address = d.address.clone();
        for i in 0..3 {
            let LiveLinkStart::Posted { rid, .. } = laptop.b.link_live(&address).await.unwrap() else { panic!() };
            if i == 0 {
                // A device that isn't in the workspace races a claim and an
                // open: Ana's lt lists it as contested.
                let l = until("the request in the mailbox", || async {
                    list_live(&d.client, std::slice::from_ref(&d.ek)).await.unwrap().live.into_iter().find(|l| l.rid_hex() == rid)
                })
                .await;
                let stranger = ApproverIdentity {
                    signer: Signer::Ed25519(ligus_core::crypto::SigningKeypair::generate().unwrap()),
                    device_name: "Ben's desktop".into(),
                    person: ApproverPerson { account_id: Uuid::new_v4(), name: "Ben".into() },
                };
                ApproverSession::begin(d.client.clone(), &d.ek, &l.request, stranger, &mut SeenSet::in_memory()).await.unwrap();
                let listed =
                    until("the contested request", || async { a.b.approvals(&ws).await.unwrap().into_iter().find(|r| r.contested) }).await;
                assert_eq!(listed.checking.as_deref(), Some("Ben's desktop"));
            }
            laptop.b.link_live_cancel(&rid).await.unwrap();
            update_where(&laptop, &rid, "cancelled", |u| *u == LiveLinkUpdate::Ended(LiveLinkEnd::Cancelled)).await;
        }
        let e = laptop.b.link_live(&address).await.unwrap_err();
        assert_eq!(e.0, TOO_MANY_DEAD);
        // An address that isn't one.
        assert!(laptop.b.link_live("ligusaddr1:nope").await.unwrap_err().0.contains("approval address"));
    }

    /// One link brings Ana's other workspace too: her desktop offers it,
    /// lt takes it, both certificates are on their logs, and lt links into
    /// both as Ana (the first one opens).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn one_link_brings_the_persons_other_workspaces() {
        let tmp = tempfile::tempdir().unwrap();
        let relay = serve(ligus_runtime::test_relay::router()).await;
        let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
        let (ws, wsu, conn) = founded(&a, tmp.path()).await;
        let created = a.b.create_workspace("Ana", "Beta").await.unwrap();
        let beta = created.workspace.id.clone();
        let betau = Uuid::parse_str(beta.as_str()).unwrap();
        // Beta's invite is confirmed with Beta's own recovery words.
        let words = tmp.path().join("beta-words.txt");
        std::fs::write(&words, created.recovery_words.join(" ")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        *a.words_file.lock().unwrap() = Some(words);
        let beta_conn = {
            let mut c = decode_workspace_invite(&a.b.create_invite(&beta, Role::Contributor).await.unwrap()).unwrap();
            c.inv = None;
            c.sig = None;
            encode_workspace_invite(&c)
        };
        let d = desk(&a, &tmp.path().join("ana"), &relay, wsu, &conn).await;
        let address = d.address.clone();
        let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);
        let LiveLinkStart::Posted { rid, .. } = laptop.b.link_live(&address).await.unwrap() else { panic!() };
        let (mut session, _, req) = open(d, &laptop, &rid).await;

        let svc = a.b.service().unwrap();
        let (grant, more) = {
            let (svc, req, conn, beta_conn) = (svc.clone(), req.clone(), conn.clone(), beta_conn.clone());
            tokio::task::spawn_blocking(move || {
                let mut svc = svc.lock().unwrap();
                let now = ligus_core::flow::now_ms();
                let grant = svc.prepare_computer_link(wsu, &req, &conn, now).unwrap();
                let more = svc.prepare_more_computer_links(&req, &[(betau, "Beta".into(), beta_conn)], now);
                (grant, more.into_iter().map(|(_, g)| g.unwrap()).collect::<Vec<_>>())
            })
            .await
            .unwrap()
        };
        assert!(session.offer_with(grant, more).await.unwrap().is_empty());
        laptop.b.link_live_code(&rid, true).await.unwrap();
        acked(&mut session).await;
        assert_eq!(session.acked_more().len(), 1, "lt takes the other workspace");
        {
            let (svc, req, conn, beta_conn) = (svc.clone(), req.clone(), conn.clone(), beta_conn.clone());
            tokio::task::spawn_blocking(move || {
                let mut svc = svc.lock().unwrap();
                let now = ligus_core::flow::now_ms();
                svc.publish_computer_link(wsu, &req, &conn, now).unwrap();
                svc.publish_computer_link_in(betau, &req, &beta_conn, now).unwrap();
            })
            .await
            .unwrap();
        }
        session.finish_approved_with(&[betau]).await.unwrap();
        let v =
            update_where(&laptop, &rid, "the verified person", |u| matches!(u, LiveLinkUpdate::Verified { .. } | LiveLinkUpdate::Ended(_)))
                .await;
        assert_eq!(v, LiveLinkUpdate::Verified { person: "Ana".into(), workspace: "Acme Web and Beta".into() });
        let joined = laptop.b.link_live_person(&rid, true).await.unwrap().expect("linked");
        assert_eq!(joined.workspace.id, ws, "the first workspace opens");
        assert_eq!(laptop.b.linked_person().as_deref(), Some("Ana"));
        let held: Vec<WorkspaceId> = laptop.b.workspaces().await.unwrap().into_iter().map(|w| w.id).collect();
        assert!(held.contains(&ws) && held.contains(&beta), "{held:?}");
        let state = ligus_runtime::computer_link::LinkState::load(&tmp.path().join("laptop")).unwrap();
        let ana = a.b.service().unwrap().lock().unwrap().author().account_id;
        assert_eq!((state.account_in(wsu), state.account_in(betau)), (ana, ana), "the same person in both");
        // On Ana's desktop: still one member in Beta, and lt is one of her devices there.
        until("lt among Ana's devices in Beta", || async {
            let svc = a.b.service().unwrap();
            let device = laptop.b.device_id().unwrap();
            let mine = svc.lock().unwrap().own_devices(betau).unwrap();
            mine.iter().any(|d| d.device_id == device && !d.revoked).then_some(())
        })
        .await;
        assert_eq!(a.b.members(&beta).await.unwrap().iter().filter(|m| !m.service).count(), 1, "never a second member");
    }
}

/// The owner's 0.3.0 report: lt joined by invite as its own member (an
/// Owner), linked as the person's device with a new device key, then the
/// desktop removed the old member. lt must post as the person, in a chat
/// the desktop made and in one lt made as its old member.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_linked_install_posts_after_its_old_member_is_removed() {
    let _watchdog = watchdog("a_linked_install_posts_after_its_old_member_is_removed", 420);
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
    let (ws, wsu, conn) = founded(&a, tmp.path()).await;
    let desk_chat = a.b.create_chat(&ws, "Plans").await.unwrap();
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    let lt = device(&tmp.path().join("lt"), "lt", &relay);
    lt.b.join_workspace("zaz", &invite).await.unwrap();
    until("Ana to see zaz", || async { (a.b.members(&ws).await.unwrap().len() == 2).then_some(()) }).await;
    let zaz = a.b.members(&ws).await.unwrap().into_iter().find(|m| m.name == "zaz").unwrap();
    a.b.set_role(&ws, &zaz.id, Role::Owner).await.unwrap();
    until("lt to be an Owner", || async {
        lt.b.workspaces().await.unwrap().iter().any(|w| w.id == ws && w.your_role == Role::Owner).then_some(())
    })
    .await;
    let lt_chat = lt.b.create_chat(&ws, "From lt").await.unwrap();
    lt.b.send_message(&ws, &lt_chat.id, "as zaz").await.unwrap();

    let req = lt.b.link_request().await.unwrap();
    assert!(req.new_device);
    let grant = approve(&a, wsu, &req.code, &conn).await;
    redeem(&lt, &grant).await;
    assert_eq!(lt.b.linked_person().as_deref(), Some("Ana"));
    assert_eq!(lt.b.settings().await.unwrap().linked_to.as_deref(), Some("Ana"));

    // The desktop removes the old member.
    let zaz = a.b.members(&ws).await.unwrap().into_iter().find(|m| m.name == "zaz").unwrap();
    a.b.remove_member(&ws, &zaz.id).await.unwrap();
    until("lt to see zaz removed", || async { (!lt.b.members(&ws).await.unwrap().iter().any(|m| m.name == "zaz")).then_some(()) }).await;

    until("lt to see the desktop's chat", || async { lt.b.chats(&ws).await.unwrap().iter().any(|c| c.id == desk_chat.id).then_some(()) })
        .await;
    for (chat, text) in [(&desk_chat.id, "linked, desktop chat"), (&lt_chat.id, "linked, lt chat")] {
        lt.b.send_message(&ws, chat, text).await.unwrap_or_else(|e| panic!("{text}: {e}"));
        until("Ana to see it as hers", || async {
            a.b.transcript(&ws, chat)
                .await
                .ok()?
                .iter()
                .any(|m| m.text == text && matches!(&m.author, Author::Member { name, .. } if name == "Ana"))
                .then_some(())
        })
        .await;
    }

    // Reopened from disk: still Ana, still posting.
    lt.b.shutdown();
    drop(lt);
    let again = device(&tmp.path().join("lt"), "lt", &relay);
    again.b.startup().await.unwrap();
    again.b.send_message(&ws, &desk_chat.id, "after restart").await.unwrap();
}

/// The same, with lt open while another lt on the same data dir (`lt --plain
/// link-redeem`) links it: the open one must not keep signing as the old
/// member.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_open_lt_follows_a_link_redeemed_by_another_lt() {
    let _watchdog = watchdog("an_open_lt_follows_a_link_redeemed_by_another_lt", 420);
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
    let (ws, wsu, conn) = founded(&a, tmp.path()).await;
    let desk_chat = a.b.create_chat(&ws, "Plans").await.unwrap();
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    let tui = device(&tmp.path().join("lt"), "lt", &relay);
    tui.b.join_workspace("zaz", &invite).await.unwrap();
    until("Ana to see zaz", || async { (a.b.members(&ws).await.unwrap().len() == 2).then_some(()) }).await;
    until("lt to see the desktop's chat", || async { tui.b.chats(&ws).await.unwrap().iter().any(|c| c.id == desk_chat.id).then_some(()) })
        .await;

    // A second lt on the same data dir links it (the plain commands).
    let plain = device(&tmp.path().join("lt"), "lt", &relay);
    plain.b.startup().await.unwrap();
    let req = plain.b.link_request().await.unwrap();
    let grant = approve(&a, wsu, &req.code, &conn).await;
    redeem(&plain, &grant).await;
    plain.b.shutdown();
    drop(plain);

    // The redeem retired zaz into Ana (rules 3, identity model v2 8.3);
    // where it couldn't (an older workspace), the desktop removes it.
    if let Some(zaz) = a.b.members(&ws).await.unwrap().into_iter().find(|m| m.name == "zaz") {
        let _ = a.b.remove_member(&ws, &zaz.id).await;
    }
    until("lt to see zaz gone", || async { (!tui.b.members(&ws).await.unwrap().iter().any(|m| m.name == "zaz")).then_some(()) }).await;

    // The open lt posts as Ana now.
    tui.b.send_message(&ws, &desk_chat.id, "from the open lt").await.unwrap();
    assert_eq!(tui.b.linked_person().as_deref(), Some("Ana"));
    let st = tui.b.settings().await.unwrap();
    assert_eq!((st.linked_to.as_deref(), st.your_name.as_str()), (Some("Ana"), "Ana"), "Settings shows Device of Ana, never zaz");
    until("Ana to see it as hers", || async {
        a.b.transcript(&ws, &desk_chat.id)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text == "from the open lt" && matches!(&m.author, Author::Member { name, .. } if name == "Ana"))
            .then_some(())
    })
    .await;
}

/// Recovery and refusals around a link: a removed member is told what
/// happened and how to fix it (never "role Viewer cannot perform"); an
/// install whose link record went missing signs nothing as its old member;
/// and linking again (a new request, a new grant) heals it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_broken_link_says_so_and_linking_again_heals_it() {
    let _watchdog = watchdog("a_broken_link_says_so_and_linking_again_heals_it", 420);
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
    let (ws, wsu, conn) = founded(&a, tmp.path()).await;
    let chat = a.b.create_chat(&ws, "Plans").await.unwrap();
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    let lt_dir = tmp.path().join("lt");
    let lt = device(&lt_dir, "lt", &relay);
    lt.b.join_workspace("zaz", &invite).await.unwrap();
    until("lt to see the chat", || async { lt.b.chats(&ws).await.unwrap().iter().any(|c| c.id == chat.id).then_some(()) }).await;
    let req = lt.b.link_request().await.unwrap();
    let grant = approve(&a, wsu, &req.code, &conn).await;
    redeem(&lt, &grant).await;
    if let Some(zaz) = a.b.members(&ws).await.unwrap().into_iter().find(|m| m.name == "zaz") {
        let _ = a.b.remove_member(&ws, &zaz.id).await;
    }
    until("lt to see zaz gone", || async { (!lt.b.members(&ws).await.unwrap().iter().any(|m| m.name == "zaz")).then_some(()) }).await;
    lt.b.shutdown();
    drop(lt);

    // The link record goes missing: lt would be its removed member again.
    // It refuses to sign, and says how to fix it.
    std::fs::remove_file(lt_dir.join(ligus_runtime::computer_link::STATE_FILE)).unwrap();
    let lt = device(&lt_dir, "lt", &relay);
    lt.b.startup().await.unwrap();
    let e = lt.b.send_message(&ws, &chat.id, "as nobody").await.unwrap_err();
    // Either lt's words or the runtime's own (ligus 0.3.1 explains a role
    // refusal itself): who it signs as, and how to link it again.
    assert!(
        (e.0.contains("one of Ana's devices") && e.0.contains("re-link this computer"))
            || (e.0.contains("signs as zaz") && e.0.contains("link it again")),
        "{e}"
    );

    // Linking again heals it: a new request, approved on the desktop.
    let req = lt.b.link_request().await.unwrap();
    assert!(req.new_device);
    let grant = approve(&a, wsu, &req.code, &conn).await;
    redeem(&lt, &grant).await;
    assert_eq!(lt.b.linked_person().as_deref(), Some("Ana"));
    lt.b.send_message(&ws, &chat.id, "healed").await.unwrap();
    until("Ana to see it as hers", || async {
        a.b.transcript(&ws, &chat.id)
            .await
            .ok()?
            .iter()
            .any(|m| m.text == "healed" && matches!(&m.author, Author::Member { name, .. } if name == "Ana"))
            .then_some(())
    })
    .await;
}

/// A member removed by an Admin (no link at all) is told what happened and
/// what to do, not "role Viewer cannot perform messageAppended".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_removed_member_is_told_why_and_what_to_do() {
    let _watchdog = watchdog("a_removed_member_is_told_why_and_what_to_do", 420);
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
    let (ws, _, _) = founded(&a, tmp.path()).await;
    let chat = a.b.create_chat(&ws, "Plans").await.unwrap();
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    let lt = device(&tmp.path().join("lt"), "lt", &relay);
    lt.b.join_workspace("zaz", &invite).await.unwrap();
    until("lt to see the chat", || async { lt.b.chats(&ws).await.unwrap().iter().any(|c| c.id == chat.id).then_some(()) }).await;
    let zaz = a.b.members(&ws).await.unwrap().into_iter().find(|m| m.name == "zaz").unwrap();
    a.b.remove_member(&ws, &zaz.id).await.unwrap();
    until("lt to see itself removed", || async { (!lt.b.members(&ws).await.unwrap().iter().any(|m| m.name == "zaz")).then_some(()) }).await;
    let e = lt.b.send_message(&ws, &chat.id, "still here?").await.unwrap_err();
    assert!(
        (e.0.contains("zaz, who isn't a member of this workspace any more") && e.0.contains("re-link this computer"))
            || (e.0.contains("zaz, who isn't a member of this workspace") && e.0.contains("link it again")),
        "{e}"
    );
    assert!(!e.0.contains("cannot perform"), "{e}");
}

/// A request that waits shows as waiting (Settings, `link-status`); one no
/// grant can answer any more is dropped from the data dir on its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_waiting_link_shows_and_an_expired_one_is_cleaned_up() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);
    assert_eq!(laptop.b.link_status().await.unwrap(), LinkStatusView::default());
    laptop.b.link_request().await.unwrap();
    laptop.b.link_request().await.unwrap();
    let st = laptop.b.link_status().await.unwrap();
    let p = st.pending.expect("waiting");
    assert_eq!(p.count, 2);
    assert_eq!(p.approve_by_ms - p.started_ms, 15 * 60_000);
    assert_eq!(p.expires_ms - p.started_ms, 45 * 60_000);
    assert_eq!(laptop.b.settings().await.unwrap().link_pending, Some(p));

    // An hour later: both are past any grant's life.
    let dir = tmp.path().join("laptop");
    let path = ligus_runtime::computer_link::LinkState::path(&dir);
    let mut state = ligus_runtime::computer_link::LinkState::load(&dir).unwrap();
    for q in &mut state.pending {
        q.created_at -= 60 * 60_000;
    }
    state.save(&dir).unwrap();
    assert!(laptop.b.link_status().await.unwrap().pending.is_none());
    let left = ligus_runtime::computer_link::LinkState::load(&dir).unwrap();
    assert!(left.pending.is_empty(), "dropped from {}", path.display());
}

/// Workspace model v3, 10.1 and 10.4: Ana renames her linked laptop on her
/// desktop and the laptop's lt takes that name live (kept in its data dir,
/// a toast), while its own rename stays its own. And a linked computer can't
/// make Ana leave: lt says why and nothing is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_linked_computer_follows_a_rename_and_cant_leave_for_its_person() {
    let _watchdog = watchdog("the_linked_computer_follows_a_rename_and_cant_leave_for_its_person", 420);
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "Ana's desktop", &relay);
    let (ws, wsu, conn) = founded(&a, tmp.path()).await;
    let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);
    let req = laptop.b.link_request().await.unwrap();
    let grant = approve(&a, wsu, &req.code, &conn).await;
    redeem(&laptop, &grant).await;
    assert_eq!(laptop.b.device().device_name, "lt on laptop");

    // Ana's desktop renames it.
    let lsvc = laptop.b.service().unwrap();
    let laptop_id = lsvc.lock().unwrap().device_id();
    until("the desktop to know the laptop", || async {
        let svc = a.b.service().unwrap();
        tokio::task::spawn_blocking(move || {
            let mut svc = svc.lock().unwrap();
            svc.set_device_label(wsu, laptop_id, "Ana's work laptop").ok()
        })
        .await
        .unwrap()
    })
    .await;
    a.b.publish_now(wsu).await.unwrap();
    until("the laptop to take the name", || async { (laptop.b.device().device_name == "Ana's work laptop").then_some(()) }).await;
    let kept = tmp.path().join("laptop").join(ligus_term::backend::real::label::LABEL_FILE);
    assert_eq!(std::fs::read_to_string(&kept).unwrap().trim(), "Ana's work laptop");
    assert!(laptop.saw(|e| matches!(e, ligus_term::backend::Event::DeviceRenamed { name } if name == "Ana's work laptop")));

    // Its own rename afterwards is newer: it is not taken back.
    assert_eq!(laptop.b.rename_this_device("Laptop").await.unwrap(), "Laptop");
    laptop.b.publish_now(wsu).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert_eq!(laptop.b.device().device_name, "Laptop");

    // Leaving from the linked laptop is refused, before anything is written.
    let check = laptop.b.leave_check(&ws).await.unwrap();
    assert_eq!(check.state, LeaveState::Refused, "{check:?}");
    assert!(check.refusal.as_deref().unwrap_or_default().contains("linked device"), "{check:?}");
    assert!(laptop.b.leave_workspace(&ws).await.is_err());
    assert!(laptop.b.workspaces().await.unwrap().iter().any(|w| w.id == ws), "still here");
}
