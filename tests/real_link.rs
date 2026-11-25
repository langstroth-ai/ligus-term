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
    until("the link to be redeemed", || async {
        match b.b.redeem_link(grant).await {
            Ok(w) => Some(w),
            Err(e) if e.0.contains("hasn't reached this computer") => None,
            Err(e) => panic!("redeem failed: {e}"),
        }
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fresh_install_links_as_the_persons_device() {
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

/// Today's two-person state: Ana invited herself into lt as "Ana (lt)". Linking
/// the existing install asks with a new device key and proves it was that
/// member; afterwards it acts as Ana and the desktop can remove the duplicate.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_existing_install_moves_from_its_own_member_to_the_person() {
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
        assert_eq!(listed[0].checking.as_deref(), Some("Ana's desktop"), "{listed:?}");

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
        assert_eq!(joined.id, ws);
        assert_eq!(joined.your_role, Role::Owner, "it carries Ana's role");
        assert_eq!(laptop.b.linked_person().as_deref(), Some("Ana"));
        assert!(!tmp.path().join("laptop").join(ligus_runtime::approval_mailbox::PENDING_FILE).exists(), "the request was settled");
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
        let (_ws, wsu, conn) = founded(&a, tmp.path()).await;
        let laptop = device(&tmp.path().join("laptop"), "lt on laptop", &relay);

        let bare = serve(axum::Router::new()).await;
        let c = decode_workspace_invite(&conn).unwrap();
        let no_routes = ApprovalAddress::derive(&bare, wsu, c.fk.as_deref().unwrap(), 0, &[7; 32], 0).encode();
        let start = laptop.b.link_live(&no_routes).await.unwrap();
        assert!(matches!(start, LiveLinkStart::NoMailbox { ref request } if request.code.starts_with("liguslink1:")), "{start:?}");

        let address = desk(&a, &tmp.path().join("ana"), &relay, wsu, &conn).await.address;
        for _ in 0..3 {
            let LiveLinkStart::Posted { rid, .. } = laptop.b.link_live(&address).await.unwrap() else { panic!() };
            laptop.b.link_live_cancel(&rid).await.unwrap();
            update_where(&laptop, &rid, "cancelled", |u| *u == LiveLinkUpdate::Ended(LiveLinkEnd::Cancelled)).await;
        }
        let e = laptop.b.link_live(&address).await.unwrap_err();
        assert_eq!(e.0, TOO_MANY_DEAD);
        // An address that isn't one.
        assert!(laptop.b.link_live("ligusaddr1:nope").await.unwrap_err().0.contains("approval address"));
    }
}
