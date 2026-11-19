//! The real backend end to end: two `lt` devices (temporary data dirs, file
//! secrets) syncing through the Ligus crates' in-process test relay, with a
//! fake OpenAI-compatible model server and a software passkey in place of
//! the browser.
//!
//! Covered for real: first run, creating a workspace (founder passkey and
//! recovery key), an invite (confirmed with the recovery-words file), joining
//! by invite, chats, a streamed agent reply answered by the agent owner's
//! device through the relay-sync gate, the local-send gate, proposals and a
//! vote, apply on host, people (roles and refusals), hosts and a lockdown.
//! Nothing here touches the login keychain, a real relay or a real model.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use axum::Router;
use axum::routing::post;
use futures::StreamExt;
use hive_core::presence::soft::SoftAuthenticator;
use hive_core::presence::unb64url;
use hive_core::{ModelProviderKind, WorkspaceAgent, WorkspaceRuntime, workspace_config_session_id};
use ligus_term::backend::model::*;
use ligus_term::backend::presence::{Opener, urlencode};
use ligus_term::backend::real::{RealBackend, RealConfig};
use ligus_term::backend::{Backend, Event};
use uuid::Uuid;

static ENV: Once = Once::new();

/// File secrets for every device in this process: never the login keychain.
fn file_secrets() {
    ENV.call_once(|| unsafe { std::env::set_var("LIGUS_SECRET_BACKEND", "file") });
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

/// An OpenAI-compatible model that streams one reply, word by word.
async fn fake_model() -> String {
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
            async move {
                axum::response::Response::builder().header("content-type", "text/event-stream").body(axum::body::Body::from(body)).unwrap()
            }
        }),
    );
    format!("{}/v1/chat/completions", serve(router).await)
}

/// Plays the system browser with a software passkey: reads the hand-off
/// page, runs `create` or `get` over its challenge and posts the result
/// back to the loopback, as the hand-off page does.
struct SoftBrowser(Arc<SoftAuthenticator>);

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
struct Device {
    b: RealBackend,
    events: Arc<Mutex<Vec<Event>>>,
    /// A recovery-words file to answer `joinApproved` prompts with.
    words_file: Arc<Mutex<Option<PathBuf>>>,
}

fn device(dir: &Path, name: &str, relay: &str) -> Device {
    file_secrets();
    let mut cfg = RealConfig::new(dir.to_path_buf());
    cfg.device_name = name.into();
    cfg.relay_url = Some(relay.into());
    cfg.sync_every = Duration::from_millis(150);
    cfg.opener = Arc::new(SoftBrowser(Arc::new(SoftAuthenticator::es256())));
    cfg.presence_wait = Duration::from_secs(20);
    cfg.cli_fallback = false;
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
    fn saw(&self, f: impl Fn(&Event) -> bool) -> bool {
        self.events.lock().unwrap().iter().any(f)
    }
}

/// Poll `f` until it returns `Some`, up to 30 s.
async fn until<T, F, Fut>(what: &str, mut f: F) -> T
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

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const DIFF: &str = "diff --git a/hive.txt b/hive.txt\nnew file mode 100644\n--- /dev/null\n+++ b/hive.txt\n@@ -0,0 +1 @@\n+ligustica\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_devices_work_together_through_a_relay() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(hive_runtime::test_relay::router()).await;
    let model = fake_model().await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let b = device(&tmp.path().join("ben"), "lt Ben", &relay);

    // ── First run: no identity, nothing to show.
    let st = a.b.startup().await.unwrap();
    assert_eq!((st.your_name, st.workspaces.len()), (None, 0));

    // ── Create: a founder passkey (browser `create`) and a recovery key.
    let created = a.b.create_workspace("Ana", "Bee Yard").await.unwrap();
    let ws = created.workspace.id.clone();
    let wsu = Uuid::parse_str(ws.as_str()).unwrap();
    assert_eq!(created.recovery_words.len(), 24);
    assert_eq!(created.workspace.your_role, Role::Owner);
    assert_eq!(created.workspace.can_invite, Allowed::Yes);
    assert_eq!(created.workspace.sync, SyncMode::Relay(relay.clone()));
    assert!(a.saw(|e| matches!(e, Event::PresencePrompt(p) if p.op == PresenceOp::Create)));
    assert!(a.saw(|e| matches!(e, Event::PresenceDone { ok: true, .. })));
    assert_eq!(a.b.startup().await.unwrap().your_name.as_deref(), Some("Ana"));
    assert!(!a.b.device().safety_code.is_empty());
    assert_eq!(a.b.device().secret_store, "file");

    // Seed a workspace runtime (the fake model) and an agent Ana owns. A
    // runtime change needs her passkey: the prompt goes to the browser.
    let svc = a.b.service().unwrap();
    tokio::task::spawn_blocking(move || {
        let mut svc = svc.lock().unwrap();
        let mut rt = WorkspaceRuntime::new("gw", "Lab gateway", ModelProviderKind::Custom, "test-model");
        rt.endpoint = model;
        svc.add_workspace_runtime(wsu, rt).unwrap();
        let mut agent = WorkspaceAgent::new("coder", "gw");
        agent.owner_actor_id = svc.author().id.clone();
        svc.add_agent(workspace_config_session_id(wsu), wsu, agent).unwrap();
    })
    .await
    .unwrap();
    assert!(a.b.agents(&ws).await.unwrap().iter().any(|x| x.handle == "coder" && x.runtime.0 == "gw"));
    let rts = a.b.runtimes(&ws).await.unwrap();
    assert!(rts.iter().any(|r| r.id.0 == "gw" && r.configured && r.kind == RuntimeKind::Custom));

    // ── Invite: confirmed with the recovery words from a 0600 file.
    let words = tmp.path().join("words.txt");
    std::fs::write(&words, created.recovery_words.join(" ")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    *a.words_file.lock().unwrap() = Some(words);
    assert!(a.b.create_invite(&ws, Role::Admin).await.is_err(), "invites admit Contributors");
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    assert!(invite.starts_with("hivews1:"));
    assert!(a.saw(|e| matches!(e, Event::PresencePrompt(p) if p.kind == "joinApproved")));

    // ── Join by invite.
    assert!(b.b.join_workspace("Ben", "not an invite").await.is_err());
    let joined = b.b.join_workspace("Ben", &invite).await.unwrap();
    assert_eq!(joined.id, ws);
    assert_eq!(joined.name, "Bee Yard");
    assert_eq!(joined.your_role, Role::Contributor);
    assert!(matches!(joined.can_invite, Allowed::No(_)));

    // ── People, as each device sees them.
    let people = until("Ana to see Ben", || async {
        let m = a.b.members(&ws).await.unwrap();
        (m.len() == 2).then_some(m)
    })
    .await;
    let ben = people.iter().find(|m| m.name == "Ben").unwrap().clone();
    let ana = people.iter().find(|m| m.you).unwrap();
    assert!(ana.founder && ana.role == Role::Owner);
    assert_eq!(ben.role, Role::Contributor);
    assert!(ben.can_remove.is_yes(), "the founder may remove a Contributor");
    let ben_view = b.b.members(&ws).await.unwrap();
    let ana_from_ben = ben_view.iter().find(|m| m.name == "Ana").unwrap();
    assert!(matches!(ana_from_ben.can_remove, Allowed::No(_)), "a Contributor can't cut the founder");

    // ── Hosts: each device registered itself.
    let hosts = until("Ben to see Ana's host", || async {
        let h = b.b.hosts(&ws).await.unwrap();
        h.iter().any(|h| h.label == "lt Ana").then_some(h)
    })
    .await;
    assert!(hosts.iter().any(|h| h.this_device && h.label == "This device"));

    // ── Chat: Ana opens one, Ben sees it.
    let chat = a.b.create_chat(&ws, "Varroa plan").await.unwrap();
    until("Ben to see the chat", || async { b.b.chats(&ws).await.unwrap().into_iter().find(|c| c.id == chat.id) }).await;

    // ── Relay-sync gate: Ben asks Ana's agent. Two people are here and the
    // agent is Ana's, so Ben's device defers and Ana's answers once the
    // message syncs to it. The reply streams back to Ben.
    let sent = b.b.send_message(&ws, &chat.id, "@coder what's the plan?").await.unwrap();
    assert!(matches!(sent.author, Author::Member { ref name, .. } if name == "Ben"));
    let reply = until("the agent's reply on Ben's device", || async {
        let t = b.b.transcript(&ws, &chat.id).await.unwrap();
        t.into_iter().find(|m| matches!(&m.author, Author::Agent { handle, .. } if handle == "coder") && !m.streaming)
    })
    .await;
    assert_eq!(reply.text, "Hello from the hive.");
    let rid = reply.id.clone();
    // Ben's device saw it finish: a done event, or the whole reply at once
    // when one sync brought all of it.
    until("Ben's events for the reply", || async {
        b.saw(|e| match e {
            Event::MessageDone { message, error, .. } => *message == rid && error.is_none(),
            Event::MessageAdded { message, .. } => message.id == rid && !message.streaming,
            _ => false,
        })
        .then_some(())
    })
    .await;
    assert!(a.saw(|e| matches!(e, Event::MessageDelta { .. })), "Ana's device streamed it");
    let answers = b.b.transcript(&ws, &chat.id).await.unwrap().iter().filter(|m| matches!(m.author, Author::Agent { .. })).count();
    assert_eq!(answers, 1, "answered once");

    // ── Local-send gate: Ana asks her own agent; her device answers at once.
    a.b.send_message(&ws, &chat.id, "@coder and again?").await.unwrap();
    until("the second reply", || async {
        let t = a.b.transcript(&ws, &chat.id).await.unwrap();
        (t.iter().filter(|m| matches!(m.author, Author::Agent { .. }) && !m.streaming).count() == 2).then_some(())
    })
    .await;
    // Human to human: nobody answers a message for Ana in a two-person room.
    b.b.send_message(&ws, &chat.id, "thanks @Ana").await.unwrap();
    until("Ana's mention notification", || async {
        a.b.notifications().await.unwrap().into_iter().find(|n| n.text.contains("Ben mentioned you"))
    })
    .await;

    // ── Proposals: Ana files one; Ben sees it, votes, and asks Ana's
    // device to apply it on her checkout.
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "core.autocrlf", "false"]);
    std::fs::write(repo.join("README.md"), "bees\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    a.b.bind_checkout(&ws, &repo).unwrap();
    let chat_u = Uuid::parse_str(chat.id.as_str()).unwrap();
    let svc = a.b.service().unwrap();
    let pid = tokio::task::spawn_blocking(move || {
        let mut svc = svc.lock().unwrap();
        let author = svc.author().id.clone();
        let p = hive_core::ActionProposal::file_diff("Add hive.txt", author, DIFF.into(), vec!["hive.txt".into()]);
        let id = p.id;
        svc.upsert_proposal(chat_u, wsu, p).unwrap();
        id
    })
    .await
    .unwrap();
    let pid = ProposalId::new(pid.to_string());
    let seen = until("Ben to see the proposal", || async { b.b.proposals(&ws).await.unwrap().into_iter().find(|p| p.id == pid) }).await;
    assert_eq!((seen.status, seen.can_vote.clone()), (ProposalStatus::Open, Allowed::Yes));
    let diff = b.b.proposal_diff(&ws, &pid).await.unwrap();
    assert_eq!(diff.files[0].path, "hive.txt");
    assert_eq!(diff.files[0].added, 1);
    let voted = b.b.vote(&ws, &pid, VoteChoice::Approve).await.unwrap();
    assert_eq!(voted.status, ProposalStatus::Approved);
    assert!(matches!(voted.can_vote, Allowed::No(_)), "one vote each");
    let ana_host = HostId::new(a.b.device_id().unwrap().to_string());
    until("Ben to see the approval synced", || async {
        (a.b.proposals(&ws).await.unwrap().iter().any(|p| p.id == pid && p.status == ProposalStatus::Approved)).then_some(())
    })
    .await;
    let asked = b.b.request_apply(&ws, &pid, &ana_host).await.unwrap();
    assert!(matches!(asked.apply, ApplyState::Waiting { .. }), "{:?}", asked.apply);
    let applied = until("the apply on Ana's host to reach Ben", || async {
        b.b.proposals(&ws).await.unwrap().into_iter().find(|p| p.id == pid && matches!(p.apply, ApplyState::Applied { .. }))
    })
    .await;
    let ApplyState::Applied { host, commit } = applied.apply else { unreachable!() };
    assert_eq!(host, "lt Ana");
    assert_eq!(commit.len(), 40);
    assert_eq!(applied.status, ProposalStatus::Applied);
    assert!(b.saw(|e| matches!(e, Event::ProposalChanged { proposal, .. } if proposal.id == pid)));

    // ── Roles: Ana makes Ben an Admin (her passkey confirms it).
    a.b.set_role(&ws, &ben.id, Role::Admin).await.unwrap();
    until("Ben to become an Admin", || async {
        let w = b.b.workspaces().await.unwrap();
        w.into_iter().find(|w| w.id == ws && w.your_role == Role::Admin && w.can_invite.is_yes())
    })
    .await;

    // ── Lockdown: Ben locks the chat; Ana sees it and clears it.
    b.b.lock_chat(&ws, &chat.id).await.unwrap();
    let locked = until("Ana to see the lockdown", || async {
        a.b.chats(&ws).await.unwrap().into_iter().find(|c| c.id == chat.id).and_then(|c| c.lockdown)
    })
    .await;
    assert!(locked.clear.is_yes());
    a.b.clear_lockdown(&ws, &chat.id).await.unwrap();
    until("Ben to see it cleared", || async {
        b.b.chats(&ws).await.unwrap().into_iter().find(|c| c.id == chat.id && c.lockdown.is_none())
    })
    .await;

    a.b.shutdown();
    b.b.shutdown();
}

/// Reopening a data dir finds the same device and its workspace; a cancelled
/// passkey prompt aborts creation quietly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_persists_and_cancel_is_quiet() {
    file_secrets();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("solo");
    let mut cfg = RealConfig::new(dir.clone());
    cfg.opener = Arc::new(SoftBrowser(Arc::new(SoftAuthenticator::es256())));
    cfg.cli_fallback = false;
    cfg.sync_every = Duration::from_millis(150);
    let b = RealBackend::open(cfg.clone()).unwrap();
    // Cancel the passkey prompt: no workspace.
    let mut ev = b.events();
    let b2 = b.clone();
    let canceller = tokio::spawn(async move {
        while let Some(e) = ev.next().await {
            if let Event::PresencePrompt(p) = e {
                b2.answer_presence(p.id, PresenceAnswer::Cancel).await.unwrap();
                return;
            }
        }
    });
    let e = b.create_workspace("Ana", "Solo").await.unwrap_err();
    assert_eq!(e.0, "cancelled");
    canceller.await.unwrap();
    assert!(b.workspaces().await.unwrap().is_empty());

    // Now with the browser: a local workspace (no relay), solo, so an
    // unaddressed message would go to the primary; there is no runtime, so
    // the turn reports why instead of hanging.
    let mut ev = b.events();
    let b3 = b.clone();
    tokio::spawn(async move {
        while let Some(e) = ev.next().await {
            if let Event::PresencePrompt(p) = e {
                let _ = b3.answer_presence(p.id, PresenceAnswer::Browser).await;
            }
        }
    });
    let created = b.create_workspace("Ana", "Solo").await.unwrap();
    assert_eq!(created.workspace.sync, SyncMode::Local);
    assert!(matches!(created.workspace.can_invite, Allowed::No(ref r) if r.contains("no relay")));
    let ws = created.workspace.id.clone();
    let chat = b.create_chat(&ws, "Notes").await.unwrap();
    b.send_message(&ws, &chat.id, "hello").await.unwrap();
    b.mark_read(&ws, &chat.id).await.unwrap();
    b.confirm_recovery_saved(&ws).await.unwrap();
    let settings = b.set_setting(SettingKey::NotifyMentions, false).await.unwrap();
    assert!(!settings.notify_mentions);
    b.shutdown();
    drop(b);

    let again = RealBackend::open(cfg).unwrap();
    let st = again.startup().await.unwrap();
    assert_eq!(st.your_name.as_deref(), Some("Ana"));
    assert_eq!(st.workspaces.len(), 1);
    assert_eq!(st.workspaces[0].name, "Solo");
    let t = again.transcript(&ws, &chat.id).await.unwrap();
    assert_eq!(t[0].text, "hello");
    assert!(!again.settings().await.unwrap().notify_mentions);
    // No secret in the files lt writes itself.
    for f in ["workspaces.json", "lt-state.json"] {
        let text = std::fs::read_to_string(dir.join(f)).unwrap();
        assert!(!text.contains("\"key\""), "{f}: {text}");
    }
    again.shutdown();
}
