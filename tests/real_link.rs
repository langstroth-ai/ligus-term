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
                Ok(w) => Some(w),
                Err(e) if e.0.contains("hasn't reached this computer") => {
                    eprintln!("redeem try {tries} at {:?}: {e}", start.elapsed());
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

/// The owner's 0.3.0 report: lt joined by invite as its own member (an
/// Owner), linked as the person's device with a new device key, then the
/// desktop removed the old member. lt must post as the person, in a chat
/// the desktop made and in one lt made as its old member.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_linked_install_posts_after_its_old_member_is_removed() {
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

    let zaz = a.b.members(&ws).await.unwrap().into_iter().find(|m| m.name == "zaz").unwrap();
    a.b.remove_member(&ws, &zaz.id).await.unwrap();
    until("lt to see zaz removed", || async { (!tui.b.members(&ws).await.unwrap().iter().any(|m| m.name == "zaz")).then_some(()) }).await;

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
    let zaz = a.b.members(&ws).await.unwrap().into_iter().find(|m| m.name == "zaz").unwrap();
    a.b.remove_member(&ws, &zaz.id).await.unwrap();
    until("lt to see zaz removed", || async { (!lt.b.members(&ws).await.unwrap().iter().any(|m| m.name == "zaz")).then_some(()) }).await;
    lt.b.shutdown();
    drop(lt);

    // The link record goes missing: lt would be its removed member again.
    // It refuses to sign, and says how to fix it.
    std::fs::remove_file(lt_dir.join(ligus_runtime::computer_link::STATE_FILE)).unwrap();
    let lt = device(&lt_dir, "lt", &relay);
    lt.b.startup().await.unwrap();
    let e = lt.b.send_message(&ws, &chat.id, "as nobody").await.unwrap_err();
    assert!(e.0.contains("one of Ana's devices") && e.0.contains("re-link this computer"), "{e}");

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
    assert!(e.0.contains("zaz, who isn't a member of this workspace any more") && e.0.contains("re-link this computer"), "{e}");
    assert!(!e.0.contains("cannot perform"), "{e}");
}
