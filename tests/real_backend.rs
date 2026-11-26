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

mod support;

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use ligus_core::presence::soft::SoftAuthenticator;
use ligus_core::{ModelProviderKind, WorkspaceAgent, WorkspaceRuntime, workspace_config_session_id};
use ligus_term::backend::model::*;
use ligus_term::backend::real::{RealBackend, RealConfig};
use ligus_term::backend::{Backend, Event};
use support::*;
use uuid::Uuid;

const DIFF: &str = "diff --git a/notes.txt b/notes.txt\nnew file mode 100644\n--- /dev/null\n+++ b/notes.txt\n@@ -0,0 +1 @@\n+ligustica\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_devices_work_together_through_a_relay() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let (model, calls) = fake_model_counted().await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let b = device(&tmp.path().join("ben"), "lt Ben", &relay);

    // ── First run: no identity, nothing to show.
    let st = a.b.startup().await.unwrap();
    assert_eq!((st.your_name, st.workspaces.len()), (None, 0));

    // ── Create: a founder passkey (browser `create`) and a recovery key.
    let created = a.b.create_workspace("Ana", "Acme Web").await.unwrap();
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
    assert_eq!(joined.name, "Acme Web");
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
    let chat = a.b.create_chat(&ws, "Flaky tests plan").await.unwrap();
    until("Ben to see the chat", || async { b.b.chats(&ws).await.unwrap().into_iter().find(|c| c.id == chat.id) }).await;

    // ── Agent access: coder is Ana's personal agent, "Only me" by default.
    // Ben sees it greyed out with the reason; his mention is answered by
    // Ana's device (where it runs) with a short note, and no model runs.
    let coder = a.b.agents(&ws).await.unwrap().into_iter().find(|x| x.handle == "coder").unwrap();
    assert_eq!((coder.access.as_str(), coder.can_manage, coder.unavailable.clone()), ("owner", true, None));
    assert!(coder.usage.is_some(), "its owner sees its usage");
    let bens = until("Ben to see coder", || async { b.b.agents(&ws).await.unwrap().into_iter().find(|x| x.handle == "coder") }).await;
    assert_eq!(bens.unavailable.as_deref(), Some("Only its owner can use this agent"));
    assert!(!bens.can_manage && bens.usage.is_none());
    let ment = b.b.mentionables(&ws).await.unwrap();
    assert!(ment.iter().any(|m| m.handle == "coder" && m.unavailable.is_some()));
    b.b.send_message(&ws, &chat.id, "@coder are you there?").await.unwrap();
    until("the refusal on Ben's device", || async {
        let t = b.b.transcript(&ws, &chat.id).await.unwrap();
        t.into_iter().find(|m| m.text == "coder is only available to its owner." && !m.streaming)
    })
    .await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no model call for a refused turn");
    // The chat's primary runtime runs on Ana's device and account: Ben's
    // `@ligus` is answered with a note there, and no model runs either.
    b.b.send_message(&ws, &chat.id, "@ligus summarize this").await.unwrap();
    until("the primary's refusal on Ben's device", || async {
        let t = b.b.transcript(&ws, &chat.id).await.unwrap();
        t.into_iter().find(|m| m.text.starts_with("Ligus in this chat runs on another member's device") && !m.streaming)
    })
    .await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no model call for a teammate's @ligus");
    assert!(b.b.set_agent_access(&ws, &coder.id, true).await.is_err(), "only its owner opens it");
    // Ana opens it to everyone.
    a.b.set_agent_access(&ws, &coder.id, true).await.unwrap();
    until("Ben to see coder open", || async {
        b.b.agents(&ws).await.unwrap().into_iter().find(|x| x.handle == "coder" && x.access == "everyone" && x.unavailable.is_none())
    })
    .await;

    // ── Relay-sync gate: Ben asks Ana's agent. Two people are here and the
    // agent is Ana's, so Ben's device defers and Ana's answers once the
    // message syncs to it. The reply streams back to Ben.
    let sent = b.b.send_message(&ws, &chat.id, "@coder what's the plan?").await.unwrap();
    assert!(matches!(sent.author, Author::Member { ref name, .. } if name == "Ben"));
    let reply = until("the agent's reply on Ben's device", || async {
        let t = b.b.transcript(&ws, &chat.id).await.unwrap();
        t.into_iter().find(|m| {
            matches!(&m.author, Author::Agent { handle, .. } if handle == "coder") && !m.streaming && m.text == "Hello from the agent."
        })
    })
    .await;
    assert_eq!(reply.text, "Hello from the agent.");
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
    let answers = b.b.transcript(&ws, &chat.id).await.unwrap().iter().filter(|m| m.text == "Hello from the agent.").count();
    assert_eq!(answers, 1, "answered once");

    // ── Local-send gate: Ana asks her own agent; her device answers at once.
    a.b.send_message(&ws, &chat.id, "@coder and again?").await.unwrap();
    until("the second reply", || async {
        let t = a.b.transcript(&ws, &chat.id).await.unwrap();
        (t.iter().filter(|m| matches!(m.author, Author::Agent { .. }) && !m.streaming && m.text == "Hello from the agent.").count() == 2)
            .then_some(())
    })
    .await;
    // Ana's device ran both turns and counted them: one each (UTC day).
    let usage = a.b.agents(&ws).await.unwrap().into_iter().find(|x| x.handle == "coder").unwrap().usage.unwrap();
    let turns = |who: &str| usage.today.iter().find(|r| r.member == who).map(|r| r.turns);
    assert_eq!((turns("Ben"), turns("Ana")), (Some(1), Some(1)), "{usage:?}");
    assert_eq!(usage.month.iter().map(|r| r.turns).sum::<u64>(), 2);
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
    std::fs::write(repo.join("README.md"), "readme\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    a.b.bind_checkout(&ws, &repo).unwrap();
    let chat_u = Uuid::parse_str(chat.id.as_str()).unwrap();
    let svc = a.b.service().unwrap();
    let pid = tokio::task::spawn_blocking(move || {
        let mut svc = svc.lock().unwrap();
        let author = svc.author().id.clone();
        let p = ligus_core::ActionProposal::file_diff("Add notes.txt", author, DIFF.into(), vec!["notes.txt".into()]);
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
    assert_eq!(diff.files[0].path, "notes.txt");
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
    let ApplyState::Applied { host, commit } = applied.apply.clone() else { unreachable!() };
    assert_eq!(host, "lt Ana");
    assert_eq!(commit.len(), 40);
    assert_eq!(applied.status, ProposalStatus::Applied);
    assert!(b.saw(|e| matches!(e, Event::ProposalChanged { proposal, .. } if proposal.id == pid)));

    // ── Archive: Ben archives the applied proposal for everyone; Ana sees
    // it archived. Hide for me stays on Ana's device; Restore brings it back.
    assert!(applied.in_inbox() && !applied.waiting && applied.can_archive.is_yes(), "{applied:?}");
    let archived = b.b.archive_proposal(&ws, &pid, true).await.unwrap();
    assert_eq!(archived.shelf, Shelf::Archived);
    until("Ana to see it archived", || async {
        a.b.proposals(&ws).await.unwrap().into_iter().find(|p| p.id == pid && p.shelf == Shelf::Archived)
    })
    .await;
    let hidden = a.b.hide_proposal(&ws, &pid, true).await.unwrap();
    assert_eq!(hidden.shelf, Shelf::Hidden);
    let restored = b.b.archive_proposal(&ws, &pid, false).await.unwrap();
    assert!(restored.in_inbox(), "{restored:?}");
    assert_eq!(a.b.proposals(&ws).await.unwrap().into_iter().find(|p| p.id == pid).unwrap().shelf, Shelf::Hidden, "hidden wins");
    a.b.hide_proposal(&ws, &pid, false).await.unwrap();
    until("Ana to see it restored once she shows it again", || async {
        a.b.proposals(&ws).await.unwrap().into_iter().find(|p| p.id == pid && p.in_inbox())
    })
    .await;

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

/// The agent sandbox denies this device's own data dir (its keys, database
/// and secrets) to every agent child, as the desktop and the CLI register
/// theirs at startup.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_registers_the_data_dir_with_the_agent_sandbox() {
    file_secrets();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("sandboxed");
    let mut cfg = RealConfig::new(dir.clone());
    cfg.cli_fallback = false;
    let b = RealBackend::open(cfg).unwrap();
    let denied = ligus_runtime::executor::host_paths().data_dirs;
    assert!(denied.iter().any(|d| d == &dir), "{dir:?} not in {denied:?}");
    b.shutdown();
}

/// A relay URL that isn't an http(s) address is refused when lt starts, not
/// turned silently into device-only workspaces.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unusable_relay_url_is_refused_at_open() {
    file_secrets();
    let tmp = tempfile::tempdir().unwrap();
    for bad in ["relay.example", "ftp://relay.example", "wss://relay.example"] {
        let mut cfg = RealConfig::new(tmp.path().join("bad-relay"));
        cfg.cli_fallback = false;
        cfg.relay_url = Some(bad.into());
        let r = RealBackend::open(cfg);
        assert!(r.as_ref().is_err_and(|e| e.contains("LT_RELAY_URL")), "{bad}: accepted");
    }
}

/// Agent runtimes on this device: stub `claude` and `codex` CLIs on a
/// temporary `PATH` (they answer `--version` and nothing else) and an
/// Ollama-shaped server with one model are found, each "available"; adding
/// one as an agent writes one signed roster event (yours, "Only me", pinned
/// to this device), after which it is no longer offered. Nothing here reads
/// the real environment, keychain or home dir.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_runtimes_are_found_and_added_as_agents() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let (bin, home) = (tmp.path().join("bin"), tmp.path().join("home"));
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    // Codex is signed in (its auth file exists; its content is never read).
    std::fs::write(home.join(".codex/auth.json"), "not read").unwrap();
    let misuse = tmp.path().join("misuse.log");
    for (name, version) in [("claude", "2.1.3 (Claude Code)"), ("codex", "codex-cli 0.98.0")] {
        let p = bin.join(name);
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '{version}'; exit 0; fi\necho \"{name} $*\" >> '{}'\nexit 9\n",
                misuse.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let ollama = serve(axum::Router::new().route(
        "/api/tags",
        axum::routing::get(|| async { axum::Json(serde_json::json!({"models": [{"name": "qwen3:8b", "size": 1, "details": {}}]})) }),
    ))
    .await;
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let env = ligus_runtime::cli_detect::DetectEnv { path: bin.clone().into_os_string(), home: home.clone(), vars: Default::default() };
    let d = device_with(&tmp.path().join("ana"), "lt Ana", &relay, |c| {
        c.detect_env = Some(env);
        c.ollama_url = Some(ollama.clone());
    });
    let ws = d.b.create_workspace("Ana", "Acme Web").await.unwrap().workspace.id;
    let wsu = Uuid::parse_str(ws.as_str()).unwrap();
    assert!(d.b.agents(&ws).await.unwrap().is_empty());

    let found = d.b.local_runtimes(&ws).await.unwrap();
    let ids: Vec<&str> = found.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, vec!["local:claude-code", "local:codex", "local:ollama/qwen3:8b"], "{found:?}");
    let codex = &found[1];
    assert_eq!((codex.kind, codex.version.as_deref(), codex.ready), (RuntimeKind::Codex, Some("0.98.0"), true));
    assert_eq!(codex.detail, "signed in (codex login)");
    assert_eq!(found[0].version.as_deref(), Some("2.1.3"));
    assert_eq!(found[2].model.as_deref(), Some("qwen3:8b"));
    assert!(found.iter().all(|l| l.available() && l.unsupported.is_none()));
    assert_eq!(found.iter().map(|l| l.suggested_name.as_str()).collect::<Vec<_>>(), vec!["claude", "codex", "qwen3"]);

    // Add it: one signed roster event on the workspace-config log.
    assert_eq!(d.b.add_local_agent(&ws, &RuntimeId::new("local:codex"), "codex").await.unwrap(), "codex");
    let me = d.b.device_id().unwrap();
    let svc = d.b.service().unwrap();
    let (author, events) = tokio::task::spawn_blocking(move || {
        let svc = svc.lock().unwrap();
        (svc.author().id.clone(), svc.store().list(workspace_config_session_id(wsu)).unwrap())
    })
    .await
    .unwrap();
    let added: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            ligus_core::SessionEvent::AgentUpserted { agent } if agent.name == "codex" => Some((e, agent)),
            _ => None,
        })
        .collect();
    assert_eq!(added.len(), 1);
    let (env, agent) = added[0];
    assert!(env.signature.is_some() && env.signer_device_id == Some(me), "signed by this device");
    assert_eq!(agent.runtime_id, "local:codex");
    assert_eq!(agent.owner_actor_id, author);
    assert_eq!(agent.access, ligus_core::agent::ACCESS_OWNER);
    assert_eq!(agent.host_id, me.to_string(), "the founder pins it to this device");

    let a = d.b.agents(&ws).await.unwrap();
    let a = a.iter().find(|a| a.handle == "codex").unwrap();
    assert_eq!((a.host.as_ref(), a.access.as_str(), a.can_manage, a.shared), (None, "owner", true, false));
    assert!(d.b.runtimes(&ws).await.unwrap().iter().any(|r| r.id.0 == "local:codex" && r.kind == RuntimeKind::Codex && r.configured));
    let after = d.b.local_runtimes(&ws).await.unwrap();
    let codex = after.iter().find(|l| l.id.0 == "local:codex").unwrap();
    assert_eq!(codex.used_by, vec!["codex".to_string()]);
    assert!(after.iter().find(|l| l.id.0 == "local:claude-code").unwrap().available());

    // A taken or reserved name, or a runtime this device doesn't name, is refused.
    let e = d.b.add_local_agent(&ws, &RuntimeId::new("local:claude-code"), "codex").await.unwrap_err();
    assert!(e.0.contains("already taken"), "{e}");
    let e = d.b.add_local_agent(&ws, &RuntimeId::new("local:claude-code"), "all").await.unwrap_err();
    assert!(e.0.contains("reserved"), "{e}");
    assert!(d.b.add_local_agent(&ws, &RuntimeId::new("gw"), "gateway").await.is_err());

    // The stubs were only ever asked for their version.
    assert!(!misuse.exists(), "{}", std::fs::read_to_string(&misuse).unwrap_or_default());
    d.b.shutdown();
}

/// Guided setup on a real device: the list, an agent on a stub Codex
/// (signed in by its auth file), a code folder bound from a git checkout,
/// and a dry run of the relay flow against the test relay. The engine
/// writes through the same paths as the rest of lt.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn setup_flows_run_on_a_real_device() {
    use ligus_runtime::setup::FlowId;
    use ligus_term::setup::{CancelFlag, LtFlow, Progress, SetupInputs, SetupSecrets};
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let (bin, home) = (tmp.path().join("bin"), tmp.path().join("home"));
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(home.join(".codex/auth.json"), "not read").unwrap();
    let p = bin.join("codex");
    std::fs::write(&p, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'codex-cli 0.98.0'; exit 0; fi\nexit 9\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let env = ligus_runtime::cli_detect::DetectEnv { path: bin.clone().into_os_string(), home: home.clone(), vars: Default::default() };
    let d = device_with(&tmp.path().join("ana"), "lt Ana", &relay, |c| c.detect_env = Some(env));
    let ws = d.b.create_workspace("Ana", "Acme Web").await.unwrap().workspace.id;

    let rows = d.b.setup_status(Some(&ws)).await.unwrap();
    assert_eq!(rows.len(), 8, "the seven and the device row");
    assert_eq!(rows[0].progress, Progress::Todo, "{rows:?}");
    assert_eq!(rows[6].progress, Progress::Done, "the relay it was made on: {rows:?}");

    let inputs = |kv: &[(&str, &str)]| -> SetupInputs { kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
    let run =
        |flow, i: SetupInputs, dry| d.b.setup_run(Some(&ws), LtFlow::Engine(flow), i, SetupSecrets::new(), dry, CancelFlag::default());
    let o = run(FlowId::Agent, inputs(&[("harness", "codex"), ("name", "fixer")]), false).await.unwrap();
    if ligus_runtime::setup::SandboxFact::here().confined() {
        assert_eq!(o.exit, 0, "{o:?}");
        let agents = d.b.agents(&ws).await.unwrap();
        let a = agents.iter().find(|a| a.handle == "fixer").unwrap();
        assert_eq!(a.runtime.as_str(), "local:codex", "lt's device-local runtime id");
        // Run again: already set up, nothing changes.
        let again = run(FlowId::Agent, inputs(&[("harness", "codex"), ("name", "fixer")]), false).await.unwrap();
        assert_eq!(again.exit, 0, "{again:?}");
        assert_eq!(d.b.agents(&ws).await.unwrap().iter().filter(|a| a.handle == "fixer").count(), 1);
    } else {
        assert_eq!(o.exit, 3, "no agent sandbox here: {o:?}");
    }
    // lt can't run Aider: a usage error before anything.
    let o = run(FlowId::Agent, inputs(&[("harness", "aider"), ("name", "a2")]), false).await.unwrap();
    assert_eq!(o.exit, 2, "{o:?}");

    // A git checkout becomes a workspace repository bound here.
    let repo = tmp.path().join("web");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git").args(args).current_dir(&repo).env("HOME", &home).output().unwrap().status.success(),
            "git {args:?}"
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-q", "--allow-empty", "-m", "root"]);
    git(&["remote", "add", "origin", "https://github.com/acme/web"]);
    let path = repo.display().to_string();
    let o = run(FlowId::Folder, inputs(&[("path", path.as_str())]), false).await.unwrap();
    assert_eq!(o.exit, 0, "{o:?}");
    assert!(d.b.checkout(&ws).await.unwrap().is_some(), "lt's checkout is set when none was");

    // The relay flow, checked only: the test relay answers.
    let o = run(FlowId::Relay, inputs(&[("kind", "own"), ("url", relay.as_str())]), true).await.unwrap();
    assert!(o.exit == 0 && o.dry_run, "{o:?}");
    d.b.shutdown();
}

/// A chat's scratch repo on a real device: an empty repository made under
/// lt's data dir (one empty commit on main), in the workspace as scratch,
/// checked out here and set as the chat's repo; the agent sandbox may read
/// it (it denies the rest of the data dir). Kept, it moves out to the
/// folder chosen and is an ordinary repository.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chat_works_in_a_scratch_repo_and_keeps_it() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let dir = tmp.path().join("ana");
    let d = device(&dir, "lt Ana", &relay);
    let ws = d.b.create_workspace("Ana", "Acme Web").await.unwrap().workspace.id;
    let chat = d.b.create_chat(&ws, "CSV export").await.unwrap();
    assert!(chat.repo.is_none());

    let r = d.b.new_scratch_repo(&ws, &chat.id).await.unwrap();
    assert!(r.scratch && r.mine && r.removes_on.is_some() && !r.notice, "{r:?}");
    assert!(r.keep_to.as_deref().is_some_and(|k| k.ends_with("/csv-export")), "{r:?}");
    let listed = d.b.chats(&ws).await.unwrap().into_iter().find(|c| c.id == chat.id).unwrap();
    assert_eq!(listed.repo.as_ref(), Some(&r));
    let c = d.b.repo_choices(&ws).await.unwrap();
    assert!(c.new_scratch.is_yes());
    assert!(c.repos.iter().any(|x| x.id == r.id && x.scratch && x.here), "{:?}", c.repos);

    // The folder: under <data dir>/scratch, one empty commit on main.
    let root = dir.join("scratch");
    let made: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).collect();
    assert_eq!(made.len(), 1, "{made:?}");
    let log = std::process::Command::new("git").args(["log", "--format=%s", "main"]).current_dir(&made[0]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout).lines().count(), 1);
    assert!(ligus_runtime::executor::host_paths().allow_read.iter().any(|p| p == &root), "agents may work in it");

    // No repository, then the scratch repo again.
    d.b.set_chat_repo(&ws, &chat.id, None).await.unwrap();
    assert!(d.b.chats(&ws).await.unwrap().iter().find(|c| c.id == chat.id).unwrap().repo.is_none());
    d.b.set_chat_repo(&ws, &chat.id, Some(&r.id)).await.unwrap();
    assert_eq!(d.b.chats(&ws).await.unwrap().iter().find(|c| c.id == chat.id).unwrap().repo.as_ref().map(|x| &x.id), Some(&r.id));

    // Kept: moved out, no longer scratch, still the chat's repo.
    let dest = tmp.path().join("kept-csv");
    let path = d.b.keep_scratch_repo(&ws, &chat.id, &dest.display().to_string()).await.unwrap();
    assert!(std::path::Path::new(&path).join(".git").exists(), "{path}");
    assert!(!made[0].exists());
    let kept = d.b.chats(&ws).await.unwrap().into_iter().find(|c| c.id == chat.id).unwrap().repo.unwrap();
    assert!(kept.id == r.id && !kept.scratch && !kept.mine && kept.removes_on.is_none(), "{kept:?}");
    // Only a scratch repo this device made can be kept.
    let e = d.b.keep_scratch_repo(&ws, &chat.id, &tmp.path().join("again").display().to_string()).await.unwrap_err();
    assert!(e.0.contains("isn't a scratch repo this device made"), "{e}");
    d.b.shutdown();
}
