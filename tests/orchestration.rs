//! Orchestration through the real backend, end to end: `lt` founds a
//! deployment's workspace, admits a worker-like box as a service member,
//! and drives a workflow run whose stages go to the box's agent and to
//! `lt`'s own, through a gate approved from `lt`. Also: the invite-signature
//! check on join, the deferred-claim fallback, the checkout binding, plain
//! mode over the real backend, and that DM invites and people's presence
//! beats from other devices are ignored.
//!
//! Everything runs in this process: the crates' in-process test relay, a
//! fake OpenAI-compatible model, temporary data dirs with file secrets and
//! a software passkey. The "box" is a `ChatService` driven by hand the way
//! `ligus worker` drives one (`pending_turns`, `claim_turn`, a reply).

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ligus_core::workflow::{GateRejectPolicy, WorkflowDefinition, WorkflowNode, WorkflowNodeKind};
use ligus_core::{ModelProviderKind, Timestamp, WorkspaceAgent, WorkspaceHost, WorkspaceRuntime, workspace_config_session_id};
use ligus_runtime::box_admission::{self, JoinRequest};
use ligus_runtime::keyring::KeySource;
use ligus_runtime::secret_store::StoreKeyVault;
use ligus_runtime::workspace_invite::{WorkspaceConn, decode_workspace_invite, encode_workspace_invite};
use ligus_runtime::{ChatService, EventStore, IdentityStore, RelayClient, StoredIdentity, SyncEngine};
use ligus_term::backend::model::*;
use ligus_term::backend::{Backend, Event};
use support::*;
use uuid::Uuid;

/// A worker-like box: its own identity, marked a service member, joined by
/// an admission, answering mentions of the agents it hosts.
struct BoxDevice {
    dir: PathBuf,
    identity: StoredIdentity,
}

impl BoxDevice {
    fn new(dir: &Path) -> Self {
        file_secrets();
        std::fs::create_dir_all(dir).unwrap();
        let ids = IdentityStore::new(dir, StoreKeyVault::for_data_dir(dir));
        ids.bootstrap("box", "box", "box").unwrap();
        let identity = ids.mark_service().unwrap();
        BoxDevice { dir: dir.to_path_buf(), identity }
    }

    fn ids(&self) -> IdentityStore<StoreKeyVault> {
        IdentityStore::new(&self.dir, StoreKeyVault::for_data_dir(&self.dir))
    }

    fn db(&self) -> PathBuf {
        self.dir.join("hive.db")
    }

    fn device(&self) -> Uuid {
        self.identity.device.id
    }

    /// What `ligus admit request` prints.
    fn join_request(&self) -> JoinRequest {
        let ka = self.ids().key_agreement(self.device()).unwrap();
        JoinRequest::new(self.identity.account.id, self.device(), &self.identity.device.signing_public_key, &ka.public_key_bytes(), "box")
    }

    fn service(&self) -> ChatService {
        let ids = self.ids();
        let dk = ids.device_keypair(self.device()).unwrap();
        let ak = ids.account_keypair(self.identity.account.id).unwrap();
        ChatService::new(EventStore::open(self.db()).unwrap(), self.device(), dk, ak, self.identity.actor())
    }

    /// `ligus admit redeem`: keep the invite, its redeem key and the founder
    /// pin; the connection for syncing.
    fn redeem(&self, code: &str) -> WorkspaceConn {
        let ka = self.ids().key_agreement(self.device()).unwrap();
        let invite = box_admission::open_admission(code, &ka).unwrap();
        let conn = decode_workspace_invite(&invite).unwrap();
        let seed = box_admission::redeem_key_of(&invite).expect("an admission carries its redeem key");
        let store = EventStore::open(self.db()).unwrap();
        let ws = conn.id();
        store.set_join_invite(ws, conn.invite_id().unwrap()).unwrap();
        store.set_join_redeem_key(ws, &seed).unwrap();
        let (f, fk) = conn.founder_pin().unwrap();
        store.set_workspace_pin(ws, &f, &fk).unwrap();
        conn
    }

    fn engine(&self, conn: &WorkspaceConn) -> SyncEngine {
        let ka = self.ids().key_agreement(self.device()).unwrap();
        let src = KeySource::for_device(self.device(), ka, conn.key.as_deref(), &conn.ek);
        SyncEngine::new(RelayClient::new(&conn.relay_url), conn.room.clone()).with_key_source(src)
    }
}

/// Sync the box once (pull, then push what it wrote).
async fn box_sync(engine: &mut SyncEngine, db: &Path) {
    let mut store = EventStore::open(db).unwrap();
    let _ = ligus_runtime::sync_service::sync_round(engine, &mut store).await;
}

/// The box's worker loop: heartbeat, then answer each pending mention of an
/// agent it hosts with a reply to that mention.
fn spawn_worker(bx: Arc<BoxDevice>, conn: WorkspaceConn, stop: tokio::sync::watch::Receiver<bool>) -> tokio::task::JoinHandle<usize> {
    tokio::spawn(async move {
        let ws = conn.id();
        let mut engine = bx.engine(&conn);
        let relay = RelayClient::new(&conn.relay_url);
        let mut answered = 0;
        while !*stop.borrow() {
            box_sync(&mut engine, &bx.db()).await;
            let store = EventStore::open(bx.db()).unwrap();
            let src = {
                let ka = bx.ids().key_agreement(bx.device()).unwrap();
                KeySource::for_device(bx.device(), ka, conn.key.as_deref(), &conn.ek)
            };
            let ring = ligus_runtime::keyring::KeyringEngine::from_store(&store, ws, &src);
            let _ = ligus_runtime::host_presence::publish_host_beat(&relay, &conn.room, &ring, &bx.device().to_string()).await;
            let bx2 = bx.clone();
            answered += tokio::task::spawn_blocking(move || {
                let mut svc = bx2.service();
                let me = bx2.device().to_string();
                let mut n = 0;
                for sid in svc.store().sessions_of_workspace(ws).unwrap() {
                    let Some(s) = svc.load(sid).unwrap() else { continue };
                    for p in ligus_runtime::pending_turns(&s) {
                        let Some(agent) = s.workspace_agents.iter().find(|a| a.id == p.agent_id && a.host_id == me) else { continue };
                        if !svc.claim_turn(sid, ws, p.message_id).unwrap() {
                            continue;
                        }
                        let mid = svc
                            .begin_assistant_message(
                                sid,
                                ws,
                                agent.name.clone(),
                                agent.runtime_id.clone(),
                                Some(agent.id),
                                Some(p.message_id),
                            )
                            .unwrap();
                        svc.append_chunk(sid, ws, mid, "Built it on the box.".to_string()).unwrap();
                        svc.complete_assistant_message(sid, ws, mid, "Built it on the box.".to_string()).unwrap();
                        n += 1;
                    }
                }
                n
            })
            .await
            .unwrap();
            box_sync(&mut engine, &bx.db()).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        answered
    })
}

fn workflow(builder: Uuid) -> WorkflowDefinition {
    let node = |id: &str, name: &str, deps: &[&str], kind| WorkflowNode {
        id: id.into(),
        name: name.into(),
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        kind,
        x: None,
        y: None,
    };
    WorkflowDefinition {
        id: Uuid::new_v4(),
        name: "Build and ship".into(),
        description: "The box builds, a person signs off, the primary writes the notes.".into(),
        input_label: Some("What to build".into()),
        nodes: vec![
            node("build", "Build", &[], WorkflowNodeKind::Agent { agent_id: Some(builder), prompt_template: "Build: {{input}}".into() }),
            node(
                "signoff",
                "Sign-off",
                &["build"],
                WorkflowNodeKind::Gate {
                    title_template: "Ship {{input}}?".into(),
                    body_template: "{{nodes.build.output}}".into(),
                    required_approvals: 1,
                    on_reject: GateRejectPolicy::Halt,
                },
            ),
            node("notes", "Notes", &["signoff"], WorkflowNodeKind::Agent { agent_id: None, prompt_template: "Write the notes".into() }),
        ],
        created_at: Timestamp::now(),
        disabled: false,
    }
}

/// Run blocking crate calls on `lt`'s own write service (seeding only).
async fn on_service<T: Send + 'static>(d: &Device, f: impl FnOnce(&mut ChatService) -> T + Send + 'static) -> T {
    let svc = d.b.service().unwrap();
    tokio::task::spawn_blocking(move || f(&mut svc.lock().unwrap())).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lt_founds_a_deployment_admits_a_box_and_drives_a_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let model = fake_model().await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let other = device(&tmp.path().join("ben"), "lt Ben", &relay);

    // ── Connect: found the workspace the deployment serves (its room and key
    // as ligus-k8s puts them in the workspace Secret).
    let spec = DeploymentSpec {
        name: "Cluster".into(),
        relay_url: relay.clone(),
        room: "k8s-trial-test".into(),
        key: "the-workspace-key".into(),
        token: None,
    };
    let bad = DeploymentSpec { relay_url: "relay.example".into(), ..spec.clone() };
    assert!(a.b.connect_deployment("Ana", bad).await.unwrap_err().0.contains("isn't a relay address"));
    let created = a.b.connect_deployment("Ana", spec.clone()).await.unwrap();
    let ws = created.workspace.id.clone();
    let wsu = Uuid::parse_str(ws.as_str()).unwrap();
    assert_eq!(created.workspace.your_role, Role::Owner);
    assert_eq!(created.workspace.sync, SyncMode::Relay(relay.clone()));
    assert_eq!(created.recovery_words.len(), 24);
    // Once it exists on the relay, nobody founds it again: that takes an invite.
    let again = other.b.connect_deployment("Ben", spec).await.unwrap_err();
    assert!(again.0.contains("already holds"), "{again}");

    // ── Admit the box: sealed to its join request, confirmed with Ana's passkey.
    let bx = Arc::new(BoxDevice::new(&tmp.path().join("box")));
    let request = bx.join_request();
    assert!(other.b.admit_box(&ws, Some(&request.encode())).await.is_err(), "Ben isn't in the workspace");
    let admission = a.b.admit_box(&ws, Some(&request.encode())).await.unwrap();
    assert!(admission.sealed && admission.code.starts_with(box_admission::SEALED_PREFIX));
    assert_eq!(admission.fingerprint.as_deref(), Some(request.fingerprint().as_str()));
    assert!(a.saw(|e| matches!(e, Event::PresencePrompt(p) if p.kind == "joinApproved")));
    // A bearer code (for a Kubernetes Secret) carries the workspace key.
    let bearer = a.b.admit_box(&ws, None).await.unwrap();
    assert!(!bearer.sealed && bearer.code.starts_with("hivews1:"));

    // The box redeems it, syncs the config log and joins as its own member.
    let conn = bx.redeem(&admission.code);
    assert_eq!(conn.id(), wsu);
    let mut engine = bx.engine(&conn);
    until("the box to pull the workspace", || {
        let bx = bx.clone();
        let mut e = bx.engine(&conn);
        async move {
            box_sync(&mut e, &bx.db()).await;
            bx.service().load(workspace_config_session_id(wsu)).unwrap().map(|_| ())
        }
    })
    .await;
    {
        let mut svc = bx.service();
        svc.ensure_self_member(workspace_config_session_id(wsu), wsu).unwrap();
        svc.ensure_host(wsu, WorkspaceHost::new(bx.device().to_string(), ligus_core::HostKind::Worker, "box".to_string())).unwrap();
    }
    box_sync(&mut engine, &bx.db()).await;
    let people = until("Ana to see the box as a member", || async {
        let m = a.b.members(&ws).await.unwrap();
        m.iter().any(|m| m.service).then_some(m)
    })
    .await;
    let boxm = people.iter().find(|m| m.service).unwrap();
    assert_eq!(boxm.role, Role::Contributor);
    // As an Admin's device, lt closes the redeemed admission (so nobody
    // holding the code can slot a backdated redemption in later), as the
    // desktop and `ligus admit close` do.
    until("Ana's device to close the redeemed admission", || async {
        let closed =
            on_service(&a, move |svc| svc.load(workspace_config_session_id(wsu)).unwrap().unwrap().guards.closed_admissions.len()).await;
        (closed >= 1).then_some(())
    })
    .await;

    // ── Agents: a runtime (the fake model) and an agent hosted on the box.
    let builder = {
        let model = model.clone();
        let host = bx.device().to_string();
        on_service(&a, move |svc| {
            let mut rt = WorkspaceRuntime::new("gw", "Lab gateway", ModelProviderKind::Custom, "test-model");
            rt.endpoint = model;
            svc.add_workspace_runtime(wsu, rt).unwrap();
            let mut agent = WorkspaceAgent::new("builder", "gw");
            agent.host_id = host;
            let id = agent.id;
            svc.add_agent(workspace_config_session_id(wsu), wsu, agent).unwrap();
            id
        })
        .await
    };
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let worker = spawn_worker(bx.clone(), conn.clone(), stop_rx);

    // Hosts: the box's heartbeat makes it online, with its agent.
    let hosts = until("Ana to see the box online with its agent", || async {
        let h = a.b.hosts(&ws).await.unwrap();
        h.iter().any(|h| h.label == "box" && h.online && h.agents == vec!["builder".to_string()]).then_some(h)
    })
    .await;
    assert!(hosts.iter().any(|h| h.this_device));
    let agents = a.b.agents(&ws).await.unwrap();
    let b_agent = agents.iter().find(|x| x.handle == "builder").unwrap();
    assert_eq!(b_agent.host.as_ref().map(|h| h.0.clone()), Some(bx.device().to_string()));
    assert_ne!(b_agent.status, AgentStatus::Offline);

    // ── A workflow in a chat (saving a definition takes Ana's passkey).
    let chat = a.b.create_chat(&ws, "Release").await.unwrap();
    let chat_u = Uuid::parse_str(chat.id.as_str()).unwrap();
    let def = workflow(builder);
    let def_id = WorkflowId::new(def.id.to_string());
    on_service(&a, move |svc| svc.save_workflow_definition(chat_u, wsu, def).unwrap()).await;
    let defs = until("the workflow to be listed", || async {
        let w = a.b.workflows(&ws).await.unwrap();
        (!w.is_empty()).then_some(w)
    })
    .await;
    assert_eq!(defs[0].name, "Build and ship");
    assert_eq!(defs[0].can_start, Allowed::Yes);
    assert_eq!(defs[0].stages.iter().map(|s| s.what.as_str()).collect::<Vec<_>>(), ["@builder", "gate (1 approval)", "@primary"]);

    // ── Run it: lt drives; the box answers the build stage.
    let run = a.b.start_run(&ws, &def_id, "v1.2").await.unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.driver.as_deref(), Some("This device"));
    let gated = until("the build stage, then the gate", || async {
        let r = a.b.runs(&ws).await.unwrap().into_iter().find(|r| r.id == run.id)?;
        (r.status == RunStatus::AwaitingGate).then_some(r)
    })
    .await;
    assert_eq!(gated.stages[0].status, StageStatus::Succeeded);
    assert_eq!(gated.stages[0].excerpt, "Built it on the box.");
    assert_eq!(gated.stages[1].status, StageStatus::AwaitingApproval);
    let gate = gated.stages[1].gate.clone().unwrap();
    let gp = a.b.proposals(&ws).await.unwrap().into_iter().find(|p| p.id == gate).unwrap();
    assert_eq!((gp.kind, gp.title.as_str(), gp.author.as_str()), (ProposalKind::Gate, "Ship v1.2?", "workflow"));
    // A stage that isn't a waiting gate can't be decided.
    assert!(a.b.decide_gate(&ws, &run.id, "build", true).await.is_err());

    // ── Approve the gate; the notes stage runs here (the primary runtime).
    a.b.decide_gate(&ws, &run.id, "signoff", true).await.unwrap();
    let done = until("the run to complete", || async {
        let r = a.b.runs(&ws).await.unwrap().into_iter().find(|r| r.id == run.id)?;
        r.status.is_terminal().then_some(r)
    })
    .await;
    assert_eq!(done.status, RunStatus::Completed, "{done:?}");
    assert_eq!(done.stages[2].excerpt, "Hello from the hive.");
    assert!(a.saw(|e| matches!(e, Event::RunChanged { run: r, .. } if r.id == run.id && r.status == RunStatus::AwaitingGate)));
    // The record can be read completed a moment before its event goes out.
    until("the run's completed event", || async {
        a.saw(|e| matches!(e, Event::RunChanged { run: r, .. } if r.id == run.id && r.status == RunStatus::Completed)).then_some(())
    })
    .await;
    // The box sees the same run, completed, through the relay.
    until("the box to see the run completed", || {
        let bx = bx.clone();
        let mut e = bx.engine(&conn);
        async move {
            box_sync(&mut e, &bx.db()).await;
            let s = bx.service().load(chat_u).unwrap()?;
            s.workflow_runs.iter().any(|r| r.status == ligus_core::workflow::WorkflowRunStatus::Completed).then_some(())
        }
    })
    .await;
    // Hand-offs: the stages were recorded as work handed to each agent.
    let _ = a.b.handoffs(&ws).await.unwrap();

    // ── Plain mode over the real backend: stable records, exit codes.
    let (code, out) = plain(&a, "runs").await;
    assert_eq!(code, 0, "{out}");
    let line = out.lines().find(|l| l.starts_with("run\t")).unwrap();
    let f: Vec<&str> = line.split('\t').collect();
    assert_eq!((f[2], f[3], f[4], f[5]), (run.id.as_str(), "completed", "Build and ship", "3/3"), "{line}");
    let (code, out) = plain(&a, "hosts").await;
    assert_eq!(code, 0);
    assert!(out.lines().any(|l| l.contains("\tbox\tworker\tonline\t") && l.ends_with("@builder\t")), "{out}");
    let (code, out) = plain(&a, &format!("watch {}", run.id)).await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(&format!("run-done\t{}\tcompleted", run.id)), "{out}");
    let (code, out) = plain(&a, "gate 1 approve").await;
    assert_eq!(code, 1, "no gate is waiting: {out}");

    stop_tx.send(true).unwrap();
    assert!(worker.await.unwrap() >= 1);
    a.b.shutdown();
    other.b.shutdown();
}

/// `lt --plain COMMAND` against a device: (exit code, output).
async fn plain(d: &Device, cmd: &str) -> (i32, String) {
    struct Sink(Arc<std::sync::Mutex<Vec<u8>>>);
    impl tokio::io::AsyncWrite for Sink {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.0.lock().unwrap().extend_from_slice(buf);
            std::task::Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }
    let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
    let opts = ligus_term::plain::PlainOpts { one_shot: Some(cmd.into()), ..Default::default() };
    let b: Arc<dyn Backend> = Arc::new(d.b.clone());
    let code = ligus_term::plain::run_with(b, &b""[..], Box::new(Sink(buf.clone())), opts).await.unwrap();
    let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    (code, out)
}

/// The invite-signature check: an invite is labeled by its founder's
/// signature against the keys this device's workspaces know, and one whose
/// founder key contradicts them is refused. DM invites aren't lt's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invites_are_checked_before_joining() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let b = device(&tmp.path().join("ben"), "lt Ben", &relay);
    let created = a.b.create_workspace("Ana", "Bee Yard").await.unwrap();
    let words = tmp.path().join("words.txt");
    std::fs::write(&words, created.recovery_words.join(" ")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    *a.words_file.lock().unwrap() = Some(words);

    // A stranger's invite: signed, but Ben shares no workspace with Ana yet.
    let invite = a.b.create_invite(&created.workspace.id, Role::Contributor).await.unwrap();
    let check = b.b.inspect_invite(&invite).await.unwrap();
    assert_eq!(check.trust, InviteTrust::Unknown);
    assert_eq!(check.workspace_name, "Bee Yard");
    assert!(check.safety_code.is_some());
    b.b.join_workspace("Ben", &invite).await.unwrap();

    // Now Ana is a co-member: her next invite (to a second workspace) is known…
    let second = a.b.create_workspace("Ana", "Queen Rearing").await.unwrap();
    let words2 = tmp.path().join("words2.txt");
    std::fs::write(&words2, second.recovery_words.join(" ")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&words2, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    *a.words_file.lock().unwrap() = Some(words2);
    let invite2 = a.b.create_invite(&second.workspace.id, Role::Contributor).await.unwrap();
    let ok = b.b.inspect_invite(&invite2).await.unwrap();
    assert_eq!(ok.trust, InviteTrust::Known, "{ok:?}");
    assert_eq!(ok.founder_name.as_deref(), Some("Ana"));
    // …a copy pointing at another relay no longer carries her signature…
    let mut moved = decode_workspace_invite(&invite2).unwrap();
    moved.relay_url = "https://evil.example".into();
    let claimed = b.b.inspect_invite(&encode_workspace_invite(&moved)).await.unwrap();
    assert_eq!(claimed.trust, InviteTrust::Claimed);
    // …and one naming Ana with another key isn't from her: refused.
    let mut forged = decode_workspace_invite(&invite2).unwrap();
    forged.fk = Some("00112233445566778899aabbccddeeff".into());
    let forged = encode_workspace_invite(&forged);
    assert_eq!(b.b.inspect_invite(&forged).await.unwrap().trust, InviteTrust::Mismatch);
    let refused = b.b.join_workspace("Ben", &forged).await.unwrap_err();
    assert!(refused.0.contains("doesn't match"), "{refused}");
    assert!(!b.b.workspaces().await.unwrap().iter().any(|w| w.id == second.workspace.id));

    // A direct-message invite: lt doesn't do DMs, and says so.
    let mut dm = decode_workspace_invite(&invite2).unwrap();
    dm.dm_account = Some(Uuid::new_v4().to_string());
    let dm = encode_workspace_invite(&dm);
    assert!(b.b.inspect_invite(&dm).await.unwrap_err().0.contains("direct messages"));
    assert!(b.b.join_workspace("Ben", &dm).await.unwrap_err().0.contains("direct messages"));
    a.b.shutdown();
    b.b.shutdown();
}

/// The deferred-claim fallback: a turn deferred to its agent's owner, whose
/// device never answers, is answered here once the wait is over. And people's
/// presence beats (online, typing) from other devices change nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deferred_turn_nobody_answers_is_answered_here() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let model = fake_model().await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let b = device_with(&tmp.path().join("ben"), "lt Ben", &relay, |c| c.deferred_timeout = Duration::from_secs(2));
    let created = a.b.create_workspace("Ana", "Bee Yard").await.unwrap();
    let ws = created.workspace.id.clone();
    let wsu = Uuid::parse_str(ws.as_str()).unwrap();
    on_service(&a, move |svc| {
        let mut rt = WorkspaceRuntime::new("gw", "Lab gateway", ModelProviderKind::Custom, "test-model");
        rt.endpoint = model;
        svc.add_workspace_runtime(wsu, rt).unwrap();
        let mut agent = WorkspaceAgent::new("coder", "gw");
        agent.owner_actor_id = svc.author().id.clone();
        svc.add_agent(workspace_config_session_id(wsu), wsu, agent).unwrap();
    })
    .await;
    let words = tmp.path().join("words.txt");
    std::fs::write(&words, created.recovery_words.join(" ")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    *a.words_file.lock().unwrap() = Some(words);
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    b.b.join_workspace("Ben", &invite).await.unwrap();
    let chat = a.b.create_chat(&ws, "Varroa").await.unwrap();
    until("Ben to see the chat and the agent", || async {
        let c = b.b.chats(&ws).await.unwrap().into_iter().any(|c| c.id == chat.id);
        let ag = b.b.agents(&ws).await.unwrap().into_iter().any(|x| x.handle == "coder");
        (c && ag).then_some(())
    })
    .await;

    // Someone else's device publishes people's presence (online, typing in
    // this chat), sealed as the desktop does: lt reads none of it.
    {
        let conn = decode_workspace_invite(&invite).unwrap();
        let ring = ligus_runtime::keyring::KeyringEngine::passphrase(wsu, ligus_core::derive_workspace_key(conn.key.as_deref().unwrap()));
        let beat = ligus_runtime::human_presence::HumanBeat {
            device_id: Uuid::new_v4().to_string(),
            actor_id: "someone".into(),
            name: "Someone".into(),
            session_id: chat.id.0.clone(),
            typing: true,
            ts: ligus_runtime::human_presence::now_ms(),
        };
        let _ = ligus_runtime::human_presence::publish_human_beat(&RelayClient::new(&relay), &conn.room, &ring, &beat).await;
    }

    // Ana's device goes away; Ben asks her agent. The turn is hers, so Ben's
    // device defers to it; nobody answers, so after the wait Ben's does.
    a.b.shutdown();
    let sent = b.b.send_message(&ws, &chat.id, "@coder are the mites under 3?").await.unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let early = b.b.transcript(&ws, &chat.id).await.unwrap();
    assert!(!early.iter().any(|m| matches!(m.author, Author::Agent { .. })), "answered before the deferral ran out");
    let reply = until("the fallback reply on Ben's device", || async {
        let t = b.b.transcript(&ws, &chat.id).await.unwrap();
        t.into_iter().find(|m| matches!(&m.author, Author::Agent { handle, .. } if handle == "coder") && !m.streaming)
    })
    .await;
    assert_eq!(reply.text, "Hello from the hive.");
    assert!(reply.at >= sent.at);
    // People's beats never became events: only agents think, nobody types.
    assert!(!b.saw(|e| matches!(e, Event::Thinking { agent, .. } if !agent.starts_with('@'))));
    b.b.shutdown();
}

/// The checkout this device applies proposals in: set, refused when it isn't
/// a git checkout, cleared.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_checkout_is_bound_per_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let ws = a.b.create_workspace("Ana", "Bee Yard").await.unwrap().workspace.id;
    assert_eq!(a.b.checkout(&ws).await.unwrap(), None);
    let plain_dir = tmp.path().join("not-a-repo");
    std::fs::create_dir_all(&plain_dir).unwrap();
    let e = a.b.set_checkout(&ws, Some(plain_dir.to_str().unwrap())).await.unwrap_err();
    assert!(e.0.contains("isn't a git checkout"), "{e}");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    let bound = a.b.set_checkout(&ws, Some(repo.to_str().unwrap())).await.unwrap().unwrap();
    assert_eq!(Path::new(&bound), std::fs::canonicalize(&repo).unwrap());
    assert_eq!(a.b.checkout(&ws).await.unwrap(), Some(bound));
    assert_eq!(a.b.set_checkout(&ws, None).await.unwrap(), None);
    assert_eq!(a.b.checkout(&ws).await.unwrap(), None);
    a.b.shutdown();
}

/// An invite for a workspace this device already holds never rewrites it:
/// not its founder pin (a co-member who knows the room could otherwise
/// name a founder of their choosing, or none), nor its relay or key.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_workspace_is_never_rejoined_from_another_invite() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let evil = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let b = device(&tmp.path().join("ben"), "lt Ben", &relay);
    let created = a.b.create_workspace("Ana", "Bee Yard").await.unwrap();
    let words = tmp.path().join("words.txt");
    std::fs::write(&words, created.recovery_words.join(" ")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    *a.words_file.lock().unwrap() = Some(words);
    let invite = a.b.create_invite(&created.workspace.id, Role::Contributor).await.unwrap();
    b.b.join_workspace("Ben", &invite).await.unwrap();

    // Someone who knows the room hands Ben an "invite" to it with no founder
    // and their own relay.
    let mut hostile = decode_workspace_invite(&invite).unwrap();
    hostile.relay_url = evil.clone();
    hostile.founder_actor_id = None;
    hostile.fk = None;
    hostile.sig = None;
    let refused = b.b.join_workspace("Ben", &encode_workspace_invite(&hostile)).await.unwrap_err();
    assert!(refused.0.contains("already"), "{refused}");
    let held = b.b.workspaces().await.unwrap().into_iter().find(|w| w.id == created.workspace.id).unwrap();
    assert_eq!(held.sync, SyncMode::Relay(relay.clone()), "the relay was rewritten");
    assert_eq!(held.your_role, Role::Contributor);
    // A failed join of a new workspace leaves nothing behind, so it can be retried.
    let mut stray = decode_workspace_invite(&invite).unwrap();
    stray.room = "a-room-nobody-founded".into();
    stray.sig = None;
    assert!(b.b.join_workspace("Ben", &encode_workspace_invite(&stray)).await.is_err());
    assert!(!b.b.workspaces().await.unwrap().iter().any(|w| w.name == stray.name && w.id != created.workspace.id));
    a.b.shutdown();
    b.b.shutdown();
}

/// The deferred-claim fallback re-resolves who answers when it fires, as
/// the desktop's does: an agent removed while the turn waited isn't run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_fallback_re_resolves_its_responder_when_it_fires() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let model = fake_model().await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let b = device_with(&tmp.path().join("ben"), "lt Ben", &relay, |c| c.deferred_timeout = Duration::from_secs(3));
    let created = a.b.create_workspace("Ana", "Bee Yard").await.unwrap();
    let ws = created.workspace.id.clone();
    let wsu = Uuid::parse_str(ws.as_str()).unwrap();
    // Ana's agent lives on a worker that is offline: nobody answers it, so
    // Ben's device would after the wait.
    let coder = on_service(&a, move |svc| {
        let mut rt = WorkspaceRuntime::new("gw", "Lab gateway", ModelProviderKind::Custom, "test-model");
        rt.endpoint = model;
        svc.add_workspace_runtime(wsu, rt).unwrap();
        let mut agent = WorkspaceAgent::new("coder", "gw");
        agent.owner_actor_id = svc.author().id.clone();
        agent.host_id = Uuid::new_v4().to_string();
        let id = agent.id;
        svc.add_agent(workspace_config_session_id(wsu), wsu, agent).unwrap();
        id
    })
    .await;
    let words = tmp.path().join("words.txt");
    std::fs::write(&words, created.recovery_words.join(" ")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    *a.words_file.lock().unwrap() = Some(words);
    let invite = a.b.create_invite(&ws, Role::Contributor).await.unwrap();
    b.b.join_workspace("Ben", &invite).await.unwrap();
    let chat = a.b.create_chat(&ws, "Varroa").await.unwrap();
    until("Ben to see the chat and the agent", || async {
        let c = b.b.chats(&ws).await.unwrap().into_iter().any(|c| c.id == chat.id);
        let ag = b.b.agents(&ws).await.unwrap().into_iter().any(|x| x.handle == "coder");
        (c && ag).then_some(())
    })
    .await;

    b.b.send_message(&ws, &chat.id, "@coder are the mites under 3?").await.unwrap();
    // While the turn waits, Ana removes the agent.
    on_service(&a, move |svc| svc.remove_agent(workspace_config_session_id(wsu), wsu, coder).unwrap()).await;
    until("Ben to see the agent gone", || async { (!b.b.agents(&ws).await.unwrap().iter().any(|x| x.handle == "coder")).then_some(()) })
        .await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let t = b.b.transcript(&ws, &chat.id).await.unwrap();
    assert!(!t.iter().any(|m| matches!(m.author, Author::Agent { .. })), "a removed agent answered: {t:?}");
    a.b.shutdown();
    b.b.shutdown();
}

/// Host beats count only in the workspace they were published to: a member
/// of one workspace can't make another workspace's host look online.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_beat_counts_only_in_its_own_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let yard = a.b.create_workspace("Ana", "Bee Yard").await.unwrap().workspace.id;
    let yard_u = Uuid::parse_str(yard.as_str()).unwrap();
    // A worker of the Bee Yard, last seen long ago: offline.
    let worker = Uuid::new_v4().to_string();
    {
        let worker = worker.clone();
        on_service(&a, move |svc| {
            let mut h = WorkspaceHost::new(worker, ligus_core::HostKind::Worker, "far box".to_string());
            h.last_seen = Timestamp::epoch();
            svc.ensure_host(yard_u, h).unwrap();
        })
        .await;
    }
    let offline = |hosts: Vec<Host>, id: &str| hosts.iter().find(|h| h.id.as_str() == id).map(|h| !h.online);
    assert_eq!(offline(a.b.hosts(&yard).await.unwrap(), &worker), Some(true));
    // Another workspace on the same device, whose key someone else holds.
    let spec =
        DeploymentSpec { name: "Other".into(), relay_url: relay.clone(), room: "other-room".into(), key: "other-key".into(), token: None };
    let other = a.b.connect_deployment("Ana", spec).await.unwrap().workspace.id;
    let other_u = Uuid::parse_str(other.as_str()).unwrap();
    // They publish a beat for the Bee Yard's worker in their own room.
    let ring = ligus_runtime::keyring::KeyringEngine::passphrase(other_u, ligus_core::derive_workspace_key("other-key"));
    for _ in 0..10 {
        ligus_runtime::host_presence::publish_host_beat(&RelayClient::new(&relay), "other-room", &ring, &worker).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(offline(a.b.hosts(&yard).await.unwrap(), &worker), Some(true), "a beat from another workspace made the host online");
    a.b.shutdown();
}
