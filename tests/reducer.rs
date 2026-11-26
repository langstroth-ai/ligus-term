//! Reducer tests: plain values in, state and effects out. No backend.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ligus_term::app::state::*;
use ligus_term::app::{Effect, Msg, State, update};
use ligus_term::backend::model::*;
use ligus_term::backend::{BackendError, Event};
use ligus_term::keymap::Keymap;

const NOW: Millis = 1_794_992_400_000;

fn device() -> DeviceInfo {
    DeviceInfo {
        device_name: "test".into(),
        safety_code: "0000".into(),
        data_dir: "/tmp/x".into(),
        secret_store: "file".into(),
        backend_label: "test".into(),
    }
}

fn state() -> State {
    State::new(device(), Keymap::default(), NOW)
}

fn key(s: &mut State, code: KeyCode) -> Vec<Effect> {
    update(s, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn mods(s: &mut State, code: KeyCode, m: KeyModifiers) -> Vec<Effect> {
    update(s, Msg::Key(KeyEvent::new(code, m)))
}

fn typed(s: &mut State, text: &str) {
    for c in text.chars() {
        key(s, KeyCode::Char(c));
    }
}

fn ws(id: &str, role: Role) -> WorkspaceSummary {
    WorkspaceSummary {
        id: WorkspaceId::new(id),
        name: format!("WS {id}"),
        mentioned: false,
        sync: SyncMode::Peer,
        your_role: role,
        can_invite: Allowed::Yes,
    }
}

fn chat(id: &str) -> ChatSummary {
    ChatSummary { id: ChatId::new(id), title: id.into(), channel: None, updated_at: NOW, unread: true, mentioned: false, lockdown: None }
}

fn proposal(id: &str, status: ProposalStatus) -> Proposal {
    Proposal {
        id: ProposalId::new(id),
        chat: None,
        kind: ProposalKind::Change,
        title: id.into(),
        author: "@coder".into(),
        status,
        files_changed: 1,
        approvals: 0,
        quorum: 1,
        votes: vec![],
        created_at: NOW,
        needs_presence: false,
        apply: ApplyState::NotRequested,
        can_vote: Allowed::Yes,
    }
}

fn host(id: &str, worker: bool, online: bool, this: bool) -> Host {
    Host {
        id: HostId::new(id),
        label: id.into(),
        kind: if worker { HostKind::Worker } else { HostKind::Device },
        online,
        last_seen: None,
        this_device: this,
        busy: false,
        agents: vec![],
        repos: vec![],
    }
}

/// A started state with one workspace, two chats, the first one open.
fn main_state() -> State {
    let mut s = state();
    let fx = update(
        &mut s,
        Msg::Startup(Ok(StartupState { your_name: Some("Mara".into()), workspaces: vec![ws("a", Role::Owner), ws("b", Role::Viewer)] })),
    );
    assert!(fx.contains(&Effect::LoadChats(WorkspaceId::new("a"))));
    let fx = update(&mut s, Msg::Chats(WorkspaceId::new("a"), Ok(vec![chat("c1"), chat("c2")])));
    assert_eq!(
        fx,
        vec![Effect::LoadTranscript(WorkspaceId::new("a"), ChatId::new("c1")), Effect::MarkRead(WorkspaceId::new("a"), ChatId::new("c1"))]
    );
    // The first chat opens with the cursor in its composer; esc leaves it
    // for the list, where the other tests start.
    assert_eq!(s.focus, Focus::Composer);
    key(&mut s, KeyCode::Esc);
    assert_eq!(s.focus, Focus::List);
    update(
        &mut s,
        Msg::Mentionables(
            WorkspaceId::new("a"),
            Ok(vec![
                Mentionable { handle: "primary".into(), kind: "Default agent".into(), unavailable: None },
                Mentionable { handle: "coder".into(), kind: "Agent".into(), unavailable: None },
                Mentionable { handle: "Cora".into(), kind: "admin".into(), unavailable: None },
                Mentionable {
                    handle: "corvid".into(),
                    kind: "Agent".into(),
                    unavailable: Some("Only its owner can use this agent".into()),
                },
            ]),
        ),
    );
    s
}

#[test]
fn empty_startup_is_first_run_and_asks_for_a_name() {
    let mut s = state();
    update(&mut s, Msg::Startup(Ok(StartupState { your_name: None, workspaces: vec![] })));
    let Phase::Onboarding(f) = &s.phase else { panic!("{:?}", s.phase) };
    assert!(f.ask_name);
    assert_eq!(f.field, FormField::Name);
    // Enter on the value with no name: an error, no effect.
    key(&mut s, KeyCode::Tab);
    key(&mut s, KeyCode::Tab);
    typed(&mut s, "Northwind");
    assert!(key(&mut s, KeyCode::Enter).is_empty());
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert_eq!(f.error.as_deref(), Some("Tell us your name first"));
    assert_eq!(f.field, FormField::Name);
}

#[test]
fn create_then_recovery_then_main() {
    let mut s = state();
    update(&mut s, Msg::Startup(Ok(StartupState { your_name: None, workspaces: vec![] })));
    typed(&mut s, "Mara");
    key(&mut s, KeyCode::Tab);
    key(&mut s, KeyCode::Tab);
    typed(&mut s, "Acme Web");
    let fx = key(&mut s, KeyCode::Enter);
    assert_eq!(fx, vec![Effect::CreateWorkspace { your_name: "Mara".into(), name: "Acme Web".into() }]);
    // Busy: keys are ignored while the passkey is created.
    assert!(key(&mut s, KeyCode::Enter).is_empty());
    let created = Created { workspace: ws("new", Role::Owner), recovery_words: vec!["amber".into(); 24], invite: None };
    update(&mut s, Msg::Created(Ok(created)));
    assert!(matches!(s.phase, Phase::Recovery(_)));
    let fx = key(&mut s, KeyCode::Enter);
    assert_eq!(fx[0], Effect::ConfirmRecovery(WorkspaceId::new("new")));
    assert!(fx.contains(&Effect::LoadChats(WorkspaceId::new("new"))));
    assert_eq!(s.phase, Phase::Main);
}

#[test]
fn a_cancelled_passkey_returns_to_the_form_with_a_message() {
    let mut s = state();
    update(&mut s, Msg::Startup(Ok(StartupState { your_name: Some("M".into()), workspaces: vec![] })));
    key(&mut s, KeyCode::Tab);
    typed(&mut s, "X");
    key(&mut s, KeyCode::Enter);
    update(&mut s, Msg::Created(Err(BackendError("cancelled".into()))));
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert!(!f.busy);
    assert_eq!(f.error.as_deref(), Some("Cancelled: the passkey wasn't created"));
}

#[test]
fn pasting_on_the_mode_row_switches_to_join() {
    let mut s = state();
    update(&mut s, Msg::Startup(Ok(StartupState { your_name: Some("M".into()), workspaces: vec![] })));
    update(&mut s, Msg::Paste("  code-123456789  \n".into()));
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert_eq!(f.mode, FormMode::Join);
    assert_eq!(f.invite.text, "code-123456789");
    // Enter first checks who the invite is from; a second Enter joins.
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::InspectInvite("code-123456789".into())]);
    update(&mut s, Msg::InviteChecked("code-123456789".into(), Ok(check(InviteTrust::Known))));
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::JoinWorkspace { your_name: "M".into(), invite: "code-123456789".into() }]);
}

fn check(trust: InviteTrust) -> InviteCheck {
    InviteCheck {
        workspace_name: "Acme Web".into(),
        relay_url: "https://relay.example".into(),
        founder_name: Some("Ana".into()),
        safety_code: Some("aaaa-bbbb".into()),
        trust,
        already_joined: false,
    }
}

/// An invite whose founder key contradicts this device's logs is never
/// joined; editing the code checks again.
#[test]
fn a_mismatched_invite_is_refused_before_joining() {
    let mut s = state();
    update(&mut s, Msg::Startup(Ok(StartupState { your_name: Some("M".into()), workspaces: vec![] })));
    update(&mut s, Msg::Paste("code-forged-12345".into()));
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::InspectInvite("code-forged-12345".into())]);
    update(&mut s, Msg::InviteChecked("code-forged-12345".into(), Ok(check(InviteTrust::Mismatch))));
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert!(f.error.as_deref().is_some_and(|e| e.contains("doesn't match")));
    assert_eq!(key(&mut s, KeyCode::Enter), vec![]);
    typed(&mut s, "x");
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::InspectInvite("code-forged-12345x".into())]);
}

/// Connect: the deployment's relay, room and key, then found it.
#[test]
fn connect_mode_founds_a_deployments_workspace() {
    let mut s = state();
    update(&mut s, Msg::Startup(Ok(StartupState { your_name: Some("M".into()), workspaces: vec![] })));
    key(&mut s, KeyCode::Char('d'));
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert_eq!(f.mode, FormMode::Connect);
    // Down to the relay field; Enter on an empty form points at what's missing.
    key(&mut s, KeyCode::Tab);
    for _ in 0..4 {
        key(&mut s, KeyCode::Tab);
    }
    assert_eq!(key(&mut s, KeyCode::Enter), vec![]);
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert_eq!(f.field, FormField::Relay);
    typed(&mut s, "http://127.0.0.1:18443");
    key(&mut s, KeyCode::Tab);
    typed(&mut s, "k8s-trial-1");
    key(&mut s, KeyCode::Tab);
    typed(&mut s, "secret");
    key(&mut s, KeyCode::Tab);
    key(&mut s, KeyCode::Tab);
    assert_eq!(
        key(&mut s, KeyCode::Enter),
        vec![Effect::ConnectDeployment {
            your_name: "M".into(),
            spec: DeploymentSpec {
                name: String::new(),
                relay_url: "http://127.0.0.1:18443".into(),
                room: "k8s-trial-1".into(),
                key: "secret".into(),
                token: None,
            }
        }]
    );
}

#[test]
fn switching_workspace_reloads_and_keeps_drafts() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('i'));
    typed(&mut s, "draft");
    key(&mut s, KeyCode::Esc);
    key(&mut s, KeyCode::Char('w'));
    assert!(matches!(s.overlay, Some(Overlay::Switcher { sel: 0 })));
    let fx = key(&mut s, KeyCode::Char('2'));
    assert_eq!(s.active, Some(WorkspaceId::new("b")));
    assert!(fx.contains(&Effect::LoadProposals(WorkspaceId::new("b"))));
    assert!(s.composer.text.is_empty());
    assert_eq!(s.drafts.get(&ChatId::new("c1")).map(String::as_str), Some("draft"));
    // Results for the old workspace are dropped.
    update(&mut s, Msg::Chats(WorkspaceId::new("a"), Ok(vec![chat("stale")])));
    assert!(s.chats.is_empty());
}

#[test]
fn composer_sends_and_keeps_letters_out_of_global_keys() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('i'));
    assert_eq!(s.focus, Focus::Composer);
    // 'q' and 'w' type rather than quit or switch.
    typed(&mut s, "qw hi");
    assert!(!s.quit && s.overlay.is_none());
    mods(&mut s, KeyCode::Enter, KeyModifiers::ALT);
    typed(&mut s, "there");
    let fx = key(&mut s, KeyCode::Enter);
    assert_eq!(fx, vec![Effect::Send { workspace: WorkspaceId::new("a"), chat: ChatId::new("c1"), text: "qw hi\nthere".into() }]);
    assert!(s.composer.text.is_empty());
    // An empty composer sends nothing.
    assert!(key(&mut s, KeyCode::Enter).is_empty());
    // Ctrl-C quits from anywhere.
    mods(&mut s, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(s.quit);
}

#[test]
fn mention_popup_filters_completes_and_dismisses() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('i'));
    typed(&mut s, "hey @c");
    let c: Vec<String> = s.mention_candidates().iter().map(|m| m.handle.clone()).collect();
    assert_eq!(c, vec!["coder", "Cora", "corvid"]);
    key(&mut s, KeyCode::Down);
    key(&mut s, KeyCode::Tab);
    assert_eq!(s.composer.text, "hey @Cora ");
    typed(&mut s, "@p");
    // Enter accepts the mention instead of sending.
    assert!(key(&mut s, KeyCode::Enter).is_empty());
    assert_eq!(s.composer.text, "hey @Cora @primary ");
    typed(&mut s, "@co");
    key(&mut s, KeyCode::Esc);
    assert!(s.mention_candidates().is_empty());
    assert_eq!(s.focus, Focus::Composer, "Esc closes the popup first");
}

/// Agent access controls: an agent you may not use is listed (dimmed, with
/// why) but never completed.
#[test]
fn an_unavailable_agent_is_shown_but_never_completed() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('i'));
    typed(&mut s, "hey @corv");
    let c = s.mention_candidates();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].unavailable.as_deref(), Some("Only its owner can use this agent"));
    assert!(key(&mut s, KeyCode::Tab).is_empty());
    assert_eq!(s.composer.text, "hey @corv", "not completed");
    assert!(s.toasts.last().unwrap().text.contains("Only its owner can use this agent"));
    assert!(key(&mut s, KeyCode::Enter).is_empty(), "Enter neither completes nor sends");
    assert_eq!(s.composer.text, "hey @corv");
}

#[test]
fn streaming_events_build_the_reply_and_thinking_set() {
    let mut s = main_state();
    let (w, c) = (WorkspaceId::new("a"), ChatId::new("c1"));
    update(&mut s, Msg::Event(Event::Thinking { workspace: w.clone(), chat: c.clone(), agent: "@coder".into(), on: true }));
    assert!(s.thinking[&c].contains("@coder"));
    let m = Message {
        id: MessageId::new("m"),
        chat: c.clone(),
        author: Author::System,
        text: String::new(),
        at: NOW,
        streaming: true,
        tools: vec![],
    };
    update(&mut s, Msg::Event(Event::MessageAdded { workspace: w.clone(), message: m }));
    for d in ["a ", "b"] {
        update(
            &mut s,
            Msg::Event(Event::MessageDelta { workspace: w.clone(), chat: c.clone(), message: MessageId::new("m"), delta: d.into() }),
        );
    }
    update(&mut s, Msg::Event(Event::MessageDone { workspace: w.clone(), chat: c.clone(), message: MessageId::new("m"), error: None }));
    update(&mut s, Msg::Event(Event::Thinking { workspace: w.clone(), chat: c.clone(), agent: "@coder".into(), on: false }));
    assert_eq!(s.transcript[0].text, "a b");
    assert!(!s.transcript[0].streaming);
    assert!(s.thinking[&c].is_empty());
    // A message in another chat marks it unread instead.
    let other = Message { id: MessageId::new("o"), chat: ChatId::new("c2"), ..s.transcript[0].clone() };
    s.chats[1].unread = false;
    update(&mut s, Msg::Event(Event::MessageAdded { workspace: w, message: other }));
    assert!(s.chats[1].unread);
    assert_eq!(s.transcript.len(), 1);
}

#[test]
fn notifications_ring_the_bell_unless_turned_off() {
    let mut s = main_state();
    let n = Notification {
        id: NotificationId::new("n"),
        workspace: WorkspaceId::new("a"),
        chat: None,
        text: "hi".into(),
        at: NOW,
        read: false,
    };
    update(&mut s, Msg::Event(Event::Notification(n.clone())));
    assert!(s.bell);
    assert_eq!(s.unread_notifications(), 1);
    s.bell = false;
    update(
        &mut s,
        Msg::Settings(Ok(Settings {
            your_name: "M".into(),
            device_name: "d".into(),
            notify_mentions: false,
            agents_may_edit: false,
            relay_url: None,
            linked_to: None,
            link_pending: None,
        })),
    );
    update(&mut s, Msg::Event(Event::Notification(n)));
    assert!(!s.bell);
}

#[test]
fn presence_sheet_browser_recovery_file_and_cancel() {
    let mut s = main_state();
    let prompt = PresencePrompt {
        id: PresenceId(7),
        kind: "memberRemoved".into(),
        summary: "Remove Lin".into(),
        digest_short: "abcd".into(),
        op: PresenceOp::Get,
    };
    update(&mut s, Msg::Event(Event::PresencePrompt(prompt.clone())));
    assert!(!s.typing_text());
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::AnswerPresence(PresenceId(7), PresenceAnswer::Browser)]);
    assert_eq!(s.presence[0].stage, PresenceStage::WaitingBrowser);
    // Switch to the recovery-words file while waiting.
    key(&mut s, KeyCode::Char('f'));
    assert!(s.typing_text());
    typed(&mut s, "/k/words");
    assert_eq!(
        key(&mut s, KeyCode::Enter),
        vec![Effect::AnswerPresence(PresenceId(7), PresenceAnswer::RecoveryWordsFile("/k/words".into()))]
    );
    update(&mut s, Msg::Event(Event::PresenceProblem { id: PresenceId(7), error: "unreadable".into() }));
    assert_eq!(s.presence[0].error.as_deref(), Some("unreadable"));
    assert_eq!(key(&mut s, KeyCode::Esc), vec![Effect::AnswerPresence(PresenceId(7), PresenceAnswer::Cancel)]);
    update(&mut s, Msg::Event(Event::PresenceDone { id: PresenceId(7), ok: false, error: Some("cancelled".into()) }));
    assert!(s.presence.is_empty());
    // A create prompt has no recovery-file option.
    update(&mut s, Msg::Event(Event::PresencePrompt(PresencePrompt { id: PresenceId(8), op: PresenceOp::Create, ..prompt })));
    key(&mut s, KeyCode::Char('f'));
    assert_eq!(s.presence[0].stage, PresenceStage::Asking);
}

#[test]
fn proposals_vote_apply_and_hunk_navigation() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    key(&mut s, KeyCode::Char('p'));
    let mut refused = proposal("p0", ProposalStatus::Open);
    refused.can_vote = Allowed::No("you already voted".into());
    update(
        &mut s,
        Msg::Proposals(w.clone(), Ok(vec![proposal("p1", ProposalStatus::Open), proposal("p2", ProposalStatus::Approved), refused])),
    );
    update(&mut s, Msg::Hosts(w.clone(), Ok(vec![host("here", false, true, true), host("build-box", true, true, false)])));
    assert_eq!(key(&mut s, KeyCode::Char('A')), vec![Effect::Vote(w.clone(), ProposalId::new("p1"), VoteChoice::Approve)]);
    // Applying an open proposal is refused locally with the reason.
    assert!(key(&mut s, KeyCode::Char('O')).is_empty());
    assert!(s.toasts.last().unwrap().text.contains("Only an approved proposal"));
    // Approved: the host picker opens on the online worker.
    key(&mut s, KeyCode::Down);
    key(&mut s, KeyCode::Char('O'));
    assert_eq!(s.overlay, Some(Overlay::HostPicker { proposal: ProposalId::new("p2"), sel: 1 }));
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::RequestApply(w.clone(), ProposalId::new("p2"), HostId::new("build-box"))]);
    // A refused vote never reaches the backend.
    key(&mut s, KeyCode::Down);
    assert!(key(&mut s, KeyCode::Char('R')).is_empty());
    assert!(s.toasts.last().unwrap().text.contains("you already voted"));

    // Diff: Enter loads it and focuses the viewer; ] and [ jump hunks.
    key(&mut s, KeyCode::Up);
    key(&mut s, KeyCode::Up);
    let fx = key(&mut s, KeyCode::Enter);
    assert!(fx.contains(&Effect::LoadDiff(w.clone(), ProposalId::new("p1"))) || s.diff.is_some());
    let hunk = |h: &str, n: usize| Hunk { header: h.into(), lines: vec![DiffLine::Context("x".into()); n] };
    let diff = Diff {
        files: vec![
            FileDiff { path: "a".into(), added: 0, removed: 0, hunks: vec![hunk("@@ 1", 3), hunk("@@ 2", 3)] },
            FileDiff { path: "b".into(), added: 0, removed: 0, hunks: vec![hunk("@@ 3", 3)] },
        ],
    };
    update(&mut s, Msg::Diff(w.clone(), ProposalId::new("p1"), Ok(diff)));
    assert_eq!(s.focus, Focus::Detail);
    let rows = |s: &State| s.diff.as_ref().unwrap().rows()[s.diff.as_ref().unwrap().scroll].clone();
    key(&mut s, KeyCode::Char(']'));
    assert_eq!(rows(&s), DiffRow::Hunk("@@ 1".into()));
    key(&mut s, KeyCode::Char(']'));
    assert_eq!(rows(&s), DiffRow::Hunk("@@ 2".into()));
    key(&mut s, KeyCode::Char('}'));
    assert!(matches!(rows(&s), DiffRow::File { ref path, .. } if path == "b"));
    key(&mut s, KeyCode::Char('['));
    assert_eq!(rows(&s), DiffRow::Hunk("@@ 2".into()));
    key(&mut s, KeyCode::Char('G'));
    key(&mut s, KeyCode::Char('j'));
    assert_eq!(s.diff.as_ref().unwrap().scroll, s.diff.as_ref().unwrap().rows().len() - 1, "scroll stops at the end");
    key(&mut s, KeyCode::Esc);
    assert_eq!(s.focus, Focus::List);
}

/// The selected proposal's diff shows without enter: at once when the list
/// arrives, and after the selection rests when it moves, so holding ↓
/// through the list asks for one diff, not one per row.
#[test]
fn proposals_preview_the_selected_diff() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    let (p1, p2, p3) = (ProposalId::new("p1"), ProposalId::new("p2"), ProposalId::new("p3"));
    let list = vec![proposal("p1", ProposalStatus::Open), proposal("p2", ProposalStatus::Open), proposal("p3", ProposalStatus::Open)];
    // Arriving outside Proposals loads nothing extra.
    assert!(update(&mut s, Msg::Proposals(w.clone(), Ok(list.clone()))).is_empty());
    // Opening Proposals previews the selected one at once.
    let fx = key(&mut s, KeyCode::Char('p'));
    assert!(fx.contains(&Effect::LoadDiff(w.clone(), p1.clone())), "{fx:?}");
    update(&mut s, Msg::Diff(w.clone(), p1.clone(), Ok(Diff { files: vec![] })));
    assert_eq!(s.diff.as_ref().map(|d| d.proposal.clone()), Some(p1.clone()));
    assert_eq!(s.focus, Focus::List, "the preview doesn't take focus");
    // A refreshed list with the same selection keeps the diff.
    assert!(update(&mut s, Msg::Proposals(w.clone(), Ok(list))).is_empty());
    // Moving asks for a preview after a rest, not the diff itself.
    assert_eq!(key(&mut s, KeyCode::Down), vec![Effect::PreviewDiff(w.clone(), p2.clone())]);
    assert!(s.diff.is_none(), "the old diff isn't shown under the new selection");
    assert_eq!(key(&mut s, KeyCode::Down), vec![Effect::PreviewDiff(w.clone(), p3.clone())]);
    // The rest for p2 comes due after the selection moved on: nothing loads.
    assert!(update(&mut s, Msg::PreviewDue(w.clone(), p2.clone())).is_empty());
    // The selection rests on p3: its diff loads.
    assert_eq!(update(&mut s, Msg::PreviewDue(w.clone(), p3.clone())), vec![Effect::LoadDiff(w.clone(), p3.clone())]);
    // A slow diff for an earlier selection is dropped.
    update(&mut s, Msg::Diff(w.clone(), p1.clone(), Ok(Diff { files: vec![] })));
    assert!(s.diff.is_none());
    update(&mut s, Msg::Diff(w.clone(), p3.clone(), Ok(Diff { files: vec![] })));
    assert_eq!(s.diff.as_ref().map(|d| d.proposal.clone()), Some(p3.clone()));
    // Leaving Proposals: a rest that comes due later loads nothing.
    key(&mut s, KeyCode::Up);
    key(&mut s, KeyCode::Char('c'));
    assert!(update(&mut s, Msg::PreviewDue(w.clone(), p2)).is_empty());
}

fn member(id: &str, role: Role, remove: Allowed, roles: Allowed) -> Member {
    Member {
        id: MemberId::new(id),
        name: id.into(),
        role,
        founder: false,
        service: false,
        you: false,
        can_remove: remove,
        can_set_role: Role::ALL
            .iter()
            .map(|r| (*r, if *r == role { Allowed::No("already that role".into()) } else { roles.clone() }))
            .collect(),
    }
}

#[test]
fn people_actions_follow_the_backends_refusals() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    key(&mut s, KeyCode::Char('m'));
    let no = |r: &str| Allowed::No(r.into());
    update(
        &mut s,
        Msg::Members(
            w.clone(),
            Ok(vec![
                member(
                    "founder",
                    Role::Owner,
                    no("nobody cuts the workspace's founder"),
                    no("no one changes the role of a member above them"),
                ),
                member("lin", Role::Contributor, Allowed::Yes, Allowed::Yes),
            ]),
        ),
    );
    // Refused: no effect, a toast with the reason.
    assert!(key(&mut s, KeyCode::Char('X')).is_empty());
    assert!(s.toasts.last().unwrap().text.ends_with("nobody cuts the workspace's founder"));
    assert!(key(&mut s, KeyCode::Char('r')).is_empty());
    assert!(s.toasts.last().unwrap().text.contains("no one changes the role of a member above them"));
    assert!(s.overlay.is_none());
    // Allowed: confirm, then the effect.
    key(&mut s, KeyCode::Down);
    key(&mut s, KeyCode::Char('X'));
    assert!(matches!(s.overlay, Some(Overlay::ConfirmRemove { .. })));
    assert_eq!(key(&mut s, KeyCode::Char('y')), vec![Effect::RemoveMember(w.clone(), MemberId::new("lin"))]);
    // The role picker skips refused roles with a toast.
    key(&mut s, KeyCode::Char('r'));
    assert!(matches!(s.overlay, Some(Overlay::RolePicker { sel: 2, .. })));
    assert!(key(&mut s, KeyCode::Enter).is_empty(), "contributor is already their role");
    key(&mut s, KeyCode::Up);
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::SetRole(w, MemberId::new("lin"), Role::Admin)]);
}

/// `.` opens the row menu: the row's actions, the view's, then the
/// destructive ones last; esc or `.` closes it, enter or an entry's own key
/// runs it as the key would (a destructive one still asks first), and a
/// refused entry says why instead.
#[test]
fn row_menu_lists_actions_destructive_last_and_runs_them() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    key(&mut s, KeyCode::Char('m'));
    let no = |r: &str| Allowed::No(r.into());
    update(
        &mut s,
        Msg::Members(
            w.clone(),
            Ok(vec![
                member("founder", Role::Owner, no("nobody cuts the workspace's founder"), no("no one above them")),
                member("lin", Role::Contributor, Allowed::Yes, Allowed::Yes),
            ]),
        ),
    );
    key(&mut s, KeyCode::Down);
    assert!(key(&mut s, KeyCode::Char('.')).is_empty());
    let Some(Overlay::RowMenu { title, items, sel: 0 }) = &s.overlay else { panic!("{:?}", s.overlay) };
    assert_eq!(title, "lin");
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, vec!["Change role…", "Create an invite", "Admit a box…", "Approval requests…", "Remove from the workspace…"]);
    let sections: Vec<MenuSection> = items.iter().map(|i| i.section).collect();
    assert_eq!(sections, vec![MenuSection::Row, MenuSection::Pane, MenuSection::Pane, MenuSection::Pane, MenuSection::Danger]);
    // esc closes it, and so does `.` again.
    key(&mut s, KeyCode::Esc);
    assert!(s.overlay.is_none());
    key(&mut s, KeyCode::Char('.'));
    key(&mut s, KeyCode::Char('.'));
    assert!(s.overlay.is_none());
    // Enter runs the selected entry: the role picker.
    key(&mut s, KeyCode::Char('.'));
    assert!(key(&mut s, KeyCode::Enter).is_empty());
    assert!(matches!(s.overlay, Some(Overlay::RolePicker { .. })), "{:?}", s.overlay);
    s.overlay = None;
    // The destructive entry (last) still asks first.
    key(&mut s, KeyCode::Char('.'));
    key(&mut s, KeyCode::Char('G'));
    key(&mut s, KeyCode::Enter);
    assert!(matches!(s.overlay, Some(Overlay::ConfirmRemove { .. })), "{:?}", s.overlay);
    assert_eq!(key(&mut s, KeyCode::Char('y')), vec![Effect::RemoveMember(w, MemberId::new("lin"))]);
    // An entry's own key chooses it; a refused one says why and stays open.
    key(&mut s, KeyCode::Up);
    key(&mut s, KeyCode::Char('.'));
    assert!(key(&mut s, KeyCode::Char('X')).is_empty());
    assert!(s.toasts.last().unwrap().text.contains("nobody cuts the workspace's founder"));
    assert!(matches!(s.overlay, Some(Overlay::RowMenu { .. })));
}

/// A view with nothing to act on says so instead of opening an empty menu.
#[test]
fn row_menu_with_nothing_to_do_is_a_toast() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('y'));
    assert!(key(&mut s, KeyCode::Char('.')).is_empty());
    assert!(s.overlay.is_none());
    assert_eq!(s.toasts.last().unwrap().text, "Nothing to do here");
}

#[test]
fn lockdown_clear_is_refused_with_the_reason() {
    let mut s = main_state();
    s.chats[0].lockdown = Some(Lockdown { reasons: vec!["locked by a member".into()], clear: Allowed::No("a member can clear it".into()) });
    assert!(key(&mut s, KeyCode::Char('U')).is_empty());
    assert!(s.toasts.last().unwrap().text.contains("a member can clear it"));
    s.chats[0].lockdown.as_mut().unwrap().clear = Allowed::Yes;
    assert_eq!(key(&mut s, KeyCode::Char('U')), vec![Effect::ClearLockdown(WorkspaceId::new("a"), ChatId::new("c1"))]);
    assert_eq!(key(&mut s, KeyCode::Char('L')), vec![Effect::LockChat(WorkspaceId::new("a"), ChatId::new("c1"))]);
}

#[test]
fn views_and_help_and_toasts_expire() {
    let mut s = main_state();
    for (k, v) in [
        ('p', View::Proposals),
        ('a', View::Agents),
        ('h', View::Hosts),
        ('m', View::People),
        ('n', View::Notifications),
        ('s', View::Settings),
        ('c', View::Chats),
    ] {
        key(&mut s, KeyCode::Char(k));
        assert_eq!(s.view, v);
    }
    key(&mut s, KeyCode::Char('?'));
    assert!(matches!(s.overlay, Some(Overlay::Help { .. })));
    key(&mut s, KeyCode::Char('j'));
    assert!(matches!(s.overlay, Some(Overlay::Help { scroll: 1, .. })));
    key(&mut s, KeyCode::Char('x'));
    assert!(s.overlay.is_none());
    s.toast(ToastKind::Info, "hello");
    for _ in 0..16 {
        update(&mut s, Msg::Tick(NOW));
    }
    assert!(s.toasts.is_empty());
    assert_eq!(ago(NOW, NOW - 90_000), "1m");
    assert_eq!(ago(NOW, NOW - 3 * 3_600_000), "3h");
}

#[test]
fn errors_become_toasts_and_cancel_is_quiet() {
    let mut s = main_state();
    update(&mut s, Msg::Done { ok: "x".into(), result: Err(BackendError("relay unreachable".into())) });
    assert_eq!(s.toasts.last().unwrap().kind, ToastKind::Error);
    update(&mut s, Msg::Done { ok: "x".into(), result: Err(BackendError("cancelled".into())) });
    assert_eq!(s.toasts.last().unwrap().text, "Cancelled");
}

fn run_waiting(id: &str) -> Run {
    let stage = |id: &str, status, gate: Option<&str>| StageRun {
        id: id.into(),
        name: id.into(),
        what: if gate.is_some() { "gate (1 approval)".into() } else { "@coder".into() },
        status,
        gate: gate.map(ProposalId::new),
        excerpt: String::new(),
        error: String::new(),
        attempts: 1,
    };
    Run {
        id: RunId::new(id),
        workflow: WorkflowId::new("wf"),
        chat: ChatId::new("c1"),
        name: "Ship".into(),
        input: "v1".into(),
        status: RunStatus::AwaitingGate,
        stages: vec![stage("build", StageStatus::Succeeded, None), stage("signoff", StageStatus::AwaitingApproval, Some("p-g"))],
        started_by: "Mara".into(),
        started_at: NOW,
        updated_at: NOW,
        driver: Some("This device".into()),
    }
}

fn workflow_def(id: &str, can_start: Allowed) -> Workflow {
    Workflow {
        id: WorkflowId::new(id),
        chat: ChatId::new("c1"),
        chat_title: "c1".into(),
        name: "Ship".into(),
        description: String::new(),
        input_label: Some("What".into()),
        stages: vec![],
        enabled: true,
        can_start,
    }
}

/// The Workflows view: start a run (with its input), decide a waiting gate,
/// cancel; live run changes land in place; refusals come from the backend.
#[test]
fn workflows_view_starts_runs_and_decides_gates() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    let fx = key(&mut s, KeyCode::Char('f'));
    assert_eq!(s.view, View::Workflows);
    assert_eq!(fx, vec![Effect::LoadWorkflows(w.clone()), Effect::LoadRuns(w.clone())]);
    update(
        &mut s,
        Msg::Workflows(w.clone(), Ok(vec![workflow_def("wf", Allowed::Yes), workflow_def("off", Allowed::No("turned off".into()))])),
    );
    update(&mut s, Msg::Runs(w.clone(), Ok(vec![run_waiting("r1")])));
    assert_eq!(s.workflow_rows(), 3);

    // Start: the input overlay, then the effect.
    key(&mut s, KeyCode::Char('S'));
    assert!(matches!(s.overlay, Some(Overlay::StartRun { .. })));
    typed(&mut s, "v2");
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::StartRun(w.clone(), WorkflowId::new("wf"), "v2".into())]);
    // A refused definition says why, and asks nothing.
    key(&mut s, KeyCode::Down);
    assert!(key(&mut s, KeyCode::Char('S')).is_empty());
    assert!(s.overlay.is_none());
    assert!(s.toasts.last().is_some_and(|t| t.text.contains("turned off")));

    // The run: approve its waiting gate.
    key(&mut s, KeyCode::Down);
    assert_eq!(key(&mut s, KeyCode::Char('A')), vec![Effect::DecideGate(w.clone(), RunId::new("r1"), "signoff".into(), true)]);
    // Rejecting the gate and cancelling the run ask first: y goes ahead,
    // any other key backs out.
    assert!(key(&mut s, KeyCode::Char('R')).is_empty());
    assert!(matches!(&s.overlay, Some(Overlay::Confirm { action: ConfirmAction::RejectGate { .. }, .. })));
    assert_eq!(key(&mut s, KeyCode::Char('y')), vec![Effect::DecideGate(w.clone(), RunId::new("r1"), "signoff".into(), false)]);
    assert!(key(&mut s, KeyCode::Char('X')).is_empty());
    assert!(key(&mut s, KeyCode::Char('n')).is_empty());
    assert!(s.overlay.is_none(), "n backs out");
    assert!(key(&mut s, KeyCode::Char('X')).is_empty());
    assert_eq!(key(&mut s, KeyCode::Char('y')), vec![Effect::CancelRun(w.clone(), RunId::new("r1"))]);

    // A change from the driver lands in place; a settled run has no gate.
    let mut done = run_waiting("r1");
    done.status = RunStatus::Completed;
    done.stages[1].status = StageStatus::Succeeded;
    update(&mut s, Msg::Event(Event::RunChanged { workspace: w.clone(), run: done }));
    assert_eq!(s.runs.len(), 1);
    assert!(key(&mut s, KeyCode::Char('A')).is_empty());
    assert!(key(&mut s, KeyCode::Char('X')).is_empty());
    // A run from another workspace is ignored.
    update(&mut s, Msg::Event(Event::RunChanged { workspace: WorkspaceId::new("b"), run: run_waiting("r9") }));
    assert_eq!(s.runs.len(), 1);
}

/// Admitting a box: Admin only; the request's fingerprint is shown (and
/// confirmed) before the admission, and so before any passkey prompt.
#[test]
fn admitting_a_box_asks_for_its_request() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('m'));
    key(&mut s, KeyCode::Char('B'));
    assert!(matches!(s.overlay, Some(Overlay::AdmitBox { .. })));
    update(&mut s, Msg::Paste(" ligusjoin1:abc ".into()));
    // Enter only reads the request: nothing that needs a passkey yet.
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::InspectJoinRequest("ligusjoin1:abc".into())]);
    let check = JoinRequestCheck { fingerprint: "AAAA-BBBB".into(), label: Some("build-box".into()) };
    update(&mut s, Msg::JoinRequestChecked("ligusjoin1:abc".into(), Ok(check.clone())));
    assert!(matches!(&s.overlay, Some(Overlay::AdmitBox { checked: Some(c), busy: false, .. }) if *c == check));
    // Any other key (Enter included) doesn't admit; esc goes back to edit.
    assert!(key(&mut s, KeyCode::Enter).is_empty());
    assert!(key(&mut s, KeyCode::Char('x')).is_empty());
    key(&mut s, KeyCode::Esc);
    assert!(matches!(&s.overlay, Some(Overlay::AdmitBox { checked: None, result: None, .. })));
    key(&mut s, KeyCode::Enter);
    update(&mut s, Msg::JoinRequestChecked("ligusjoin1:abc".into(), Ok(check)));
    // y, with the fingerprint on screen, admits.
    assert_eq!(key(&mut s, KeyCode::Char('y')), vec![Effect::AdmitBox(WorkspaceId::new("a"), Some("ligusjoin1:abc".into()))]);
    update(
        &mut s,
        Msg::Admitted(Ok(BoxAdmission {
            code: "ligusadmit1:x".into(),
            sealed: true,
            fingerprint: Some("AAAA".into()),
            label: None,
            expires_at: NOW,
        })),
    );
    assert!(matches!(&s.overlay, Some(Overlay::AdmitBox { result: Some(a), .. }) if a.code == "ligusadmit1:x"));
    key(&mut s, KeyCode::Esc);
    assert!(s.overlay.is_none());
    // An unreadable request is a toast and no admission; empty is a
    // bearer code, admitted at once (no box to compare with).
    key(&mut s, KeyCode::Char('B'));
    update(&mut s, Msg::Paste("nonsense".into()));
    key(&mut s, KeyCode::Enter);
    update(&mut s, Msg::JoinRequestChecked("nonsense".into(), Err(BackendError("not a join request".into()))));
    assert!(matches!(&s.overlay, Some(Overlay::AdmitBox { checked: None, busy: false, .. })));
    assert!(s.toasts.iter().any(|t| t.text.contains("not a join request")));
    key(&mut s, KeyCode::Esc);
    key(&mut s, KeyCode::Char('B'));
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::AdmitBox(WorkspaceId::new("a"), None)]);
    update(&mut s, Msg::Admitted(Err(BackendError("cancelled".into()))));
    key(&mut s, KeyCode::Esc);
    assert!(s.overlay.is_none());
    // A Viewer is told no before anything is asked.
    let fx = update(&mut s, Msg::Key(crossterm::event::KeyEvent::new(KeyCode::Char('w'), crossterm::event::KeyModifiers::NONE)));
    assert!(fx.is_empty());
    key(&mut s, KeyCode::Char('2'));
    key(&mut s, KeyCode::Char('m'));
    key(&mut s, KeyCode::Char('B'));
    assert!(s.overlay.is_none());
}

/// The checkout binding: the overlay starts from the current path.
#[test]
fn binding_a_checkout() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    key(&mut s, KeyCode::Char('h'));
    update(&mut s, Msg::Checkout(w.clone(), Ok(Some("/srv/northwind-api".into()))));
    key(&mut s, KeyCode::Char('b'));
    assert!(matches!(&s.overlay, Some(Overlay::BindCheckout(b)) if b.text == "/srv/northwind-api"));
    for _ in 0.."/srv/northwind-api".len() {
        key(&mut s, KeyCode::Backspace);
    }
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::SetCheckout(w, None)]);
}

fn local(id: &str, kind: RuntimeKind, ready: bool, used_by: &[&str], name: &str) -> LocalRuntime {
    LocalRuntime {
        id: RuntimeId::new(id),
        kind,
        model: None,
        version: Some("1.0.0".into()),
        ready,
        detail: if ready { "signed in".into() } else { "run `codex login`".into() },
        used_by: used_by.iter().map(|s| s.to_string()).collect(),
        unsupported: None,
        suggested_name: name.into(),
    }
}

fn agent(handle: &str) -> Agent {
    Agent {
        id: AgentId::new(format!("a-{handle}")),
        handle: handle.into(),
        runtime: RuntimeId::new("claude-code"),
        role: None,
        host: None,
        shared: false,
        status: AgentStatus::Idle,
        doing: None,
        access: "owner".into(),
        unavailable: None,
        can_manage: true,
        usage: None,
    }
}

/// Opening the Agents tab looks for runtimes on this device; the ones no
/// agent uses follow the agents in the list, and adding one goes through a
/// name prompt to the backend's add-agent call.
#[test]
fn agents_tab_offers_what_this_device_has() {
    let mut s = main_state();
    let a = WorkspaceId::new("a");
    let fx = key(&mut s, KeyCode::Char('a'));
    assert!(fx.contains(&Effect::LoadLocalRuntimes(a.clone())), "{fx:?}");
    assert!(s.local.is_none(), "still looking");
    update(&mut s, Msg::Agents(a.clone(), Ok(vec![agent("primary")])));
    update(
        &mut s,
        Msg::LocalRuntimes(
            a.clone(),
            Ok(vec![
                local("local:claude-code", RuntimeKind::ClaudeCode, true, &["primary"], "claude"),
                local("local:codex", RuntimeKind::Codex, false, &[], "codex"),
            ]),
        ),
    );
    assert_eq!(s.available_local().len(), 1, "a runtime an agent uses isn't offered again");
    assert_eq!(s.agent_rows(), 1, "the list is the agents; what to add opens from N");
    // `N`: the picker of what this device has.
    assert!(key(&mut s, KeyCode::Char('N')).is_empty());
    assert_eq!(s.overlay, Some(Overlay::AddAgentPicker { sel: 0 }));
    // Enter on it: the name prompt, prefilled.
    assert!(key(&mut s, KeyCode::Enter).is_empty());
    match &s.overlay {
        Some(Overlay::AddAgent { runtime, name }) => {
            assert_eq!(runtime.id.as_str(), "local:codex");
            assert_eq!(name.text, "codex");
        }
        o => panic!("{o:?}"),
    }
    typed(&mut s, "-2");
    let fx = key(&mut s, KeyCode::Enter);
    assert_eq!(fx, vec![Effect::AddLocalAgent(a.clone(), RuntimeId::new("local:codex"), "codex-2".into())]);
    assert!(s.overlay.is_none());
    // Added: a toast, and the lists reload.
    let fx = update(&mut s, Msg::AgentAdded(a.clone(), Ok("codex-2".into())));
    assert!(fx.contains(&Effect::LoadLocalRuntimes(a.clone())) && fx.contains(&Effect::LoadAgents(a.clone())), "{fx:?}");
    assert!(s.toasts.last().unwrap().text.contains("@codex-2"));
    // A refusal is a toast.
    update(&mut s, Msg::AgentAdded(a.clone(), Err(BackendError("\u{201c}codex\u{201d} is already taken".into()))));
    assert_eq!(s.toasts.last().unwrap().kind, ToastKind::Error);
    // While on the Agents tab, a roster change looks again.
    let fx = update(&mut s, Msg::Event(Event::AgentsChanged { workspace: a.clone() }));
    assert!(fx.contains(&Effect::LoadLocalRuntimes(a)), "{fx:?}");
}

#[test]
fn an_unsupported_cli_is_explained_not_added() {
    let mut s = main_state();
    let a = WorkspaceId::new("a");
    key(&mut s, KeyCode::Char('a'));
    let mut gemini = local("local:gemini", RuntimeKind::Custom, false, &[], "gemini");
    gemini.unsupported = Some("Gemini CLI stopped serving Google AI Pro sign-ins; install agy".into());
    update(&mut s, Msg::LocalRuntimes(a, Ok(vec![gemini])));
    key(&mut s, KeyCode::Char('N'));
    assert_eq!(s.overlay, Some(Overlay::AddAgentPicker { sel: 0 }));
    key(&mut s, KeyCode::Enter);
    assert_eq!(s.overlay, Some(Overlay::AddAgentPicker { sel: 0 }), "refused: the picker stays");
    assert!(s.toasts.last().unwrap().text.contains("install agy"));
    // Esc on the name prompt cancels without a call.
    s.overlay = Some(Overlay::AddAgent {
        runtime: local("local:codex", RuntimeKind::Codex, true, &[], "codex"),
        name: ligus_term::app::input::TextBuf::single(),
    });
    assert!(key(&mut s, KeyCode::Esc).is_empty());
    assert!(s.overlay.is_none());
}

/// `q` quits at once when nothing runs here; while this device drives a
/// run it asks first (any other key stays), and ctrl-c twice always quits.
#[test]
fn quitting_asks_while_this_device_drives_a_run() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('q'));
    assert!(s.quit, "nothing runs here: q quits");

    let mut s = main_state();
    let w = WorkspaceId::new("a");
    update(&mut s, Msg::Runs(w.clone(), Ok(vec![run_waiting("r1")])));
    key(&mut s, KeyCode::Char('q'));
    assert!(!s.quit);
    let Some(Overlay::Confirm { action: ConfirmAction::Quit, question, .. }) = &s.overlay else { panic!("{:?}", s.overlay) };
    assert!(question.contains("drives the run") && question.contains("Ship"), "{question}");
    key(&mut s, KeyCode::Char('n'));
    assert!(!s.quit && s.overlay.is_none());
    key(&mut s, KeyCode::Char('q'));
    key(&mut s, KeyCode::Char('y'));
    assert!(s.quit);

    let mut s = main_state();
    update(&mut s, Msg::Runs(w.clone(), Ok(vec![run_waiting("r1")])));
    mods(&mut s, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!s.quit && s.overlay.is_some());
    mods(&mut s, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(s.quit, "a second ctrl-c quits");

    // A run another device drives, or one that ended, doesn't hold lt.
    let mut s = main_state();
    let mut elsewhere = run_waiting("r2");
    elsewhere.driver = Some("ada-studio".into());
    let mut done = run_waiting("r3");
    done.status = RunStatus::Completed;
    update(&mut s, Msg::Runs(w, Ok(vec![elsewhere, done])));
    key(&mut s, KeyCode::Char('q'));
    assert!(s.quit);
}

/// Rejecting a proposal asks first; approving doesn't.
#[test]
fn rejecting_a_proposal_asks_first() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    key(&mut s, KeyCode::Char('p'));
    update(&mut s, Msg::Proposals(w.clone(), Ok(vec![proposal("p1", ProposalStatus::Open)])));
    assert!(key(&mut s, KeyCode::Char('R')).is_empty());
    assert!(matches!(&s.overlay, Some(Overlay::Confirm { action: ConfirmAction::Reject(_), .. })));
    assert!(key(&mut s, KeyCode::Esc).is_empty());
    assert!(s.overlay.is_none());
    key(&mut s, KeyCode::Char('R'));
    assert_eq!(key(&mut s, KeyCode::Char('y')), vec![Effect::Vote(w.clone(), ProposalId::new("p1"), VoteChoice::Reject)]);
    assert_eq!(key(&mut s, KeyCode::Char('A')), vec![Effect::Vote(w, ProposalId::new("p1"), VoteChoice::Approve)]);
}

/// R means reject only: Notifications marks all read with M; n and N keep
/// their global meaning in the diff viewer (hunks are ] and [).
#[test]
fn keys_mean_one_thing_everywhere() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('n'));
    assert_eq!(s.view, View::Notifications);
    assert!(key(&mut s, KeyCode::Char('R')).is_empty());
    assert_eq!(key(&mut s, KeyCode::Char('M')), vec![Effect::MarkNotificationsRead]);

    let w = WorkspaceId::new("a");
    key(&mut s, KeyCode::Char('p'));
    update(&mut s, Msg::Proposals(w.clone(), Ok(vec![proposal("p1", ProposalStatus::Open)])));
    key(&mut s, KeyCode::Enter);
    let d = Diff { files: vec![FileDiff { path: "a".into(), added: 1, removed: 0, hunks: vec![] }] };
    update(&mut s, Msg::Diff(w, ProposalId::new("p1"), Ok(d)));
    assert_eq!(s.focus, Focus::Detail);
    key(&mut s, KeyCode::Char('n'));
    assert_eq!(s.view, View::Notifications, "n opens Notifications from the diff too");
}

/// A background failure on the sync channel is a toast; sync states are
/// the status bar's.
#[test]
fn sync_states_and_background_failures() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    update(&mut s, Msg::Event(Event::Sync { workspace: w.clone(), status: "synced".into() }));
    assert_eq!(s.sync_status.get(&w).map(String::as_str), Some("synced"));
    assert!(s.toasts.is_empty());
    update(&mut s, Msg::Event(Event::Sync { workspace: w.clone(), status: "turn failed: no runtime".into() }));
    assert_eq!(s.sync_status.get(&w).map(String::as_str), Some("synced"));
    assert!(s.toasts.last().is_some_and(|t| t.kind == ToastKind::Error && t.text.contains("turn failed")));
}

/// Help scrolls to its last line and straight back, in steps the terminal's
/// height allows.
#[test]
fn help_scrolls_within_bounds() {
    let mut s = main_state();
    update(&mut s, Msg::Resize(80, 24));
    key(&mut s, KeyCode::Char('?'));
    key(&mut s, KeyCode::Char('G'));
    let Some(Overlay::Help { scroll: end, .. }) = s.overlay.clone() else { panic!() };
    assert!(end > 0);
    assert_eq!(end, ligus_term::ui::help_max_scroll(&s, 24));
    key(&mut s, KeyCode::Char('j'));
    assert!(matches!(s.overlay, Some(Overlay::Help { scroll, .. }) if scroll == end), "no scrolling past the end");
    key(&mut s, KeyCode::Char('k'));
    assert!(matches!(s.overlay, Some(Overlay::Help { scroll, .. }) if scroll == end - 1));
    key(&mut s, KeyCode::Char('x'));
    assert!(s.overlay.is_none(), "another key closes it");
}

/// `/` in the help narrows it to the bindings that match; esc clears the
/// search before it closes the overlay, and the search never acts as keys.
#[test]
fn help_search_narrows_and_esc_backs_out() {
    let mut s = main_state();
    update(&mut s, Msg::Resize(80, 24));
    key(&mut s, KeyCode::Char('3'));
    key(&mut s, KeyCode::Char('?'));
    let all = ligus_term::ui::help_lines_text(&s);
    // What the view is for, then its keys first.
    assert!(all[0].starts_with("Changes agents propose"), "{all:?}");
    let first = all.iter().find(|l| l.ends_with("  here")).unwrap();
    assert!(first.starts_with("Proposals"), "the focused view's keys come first: {all:?}");
    key(&mut s, KeyCode::Char('/'));
    assert!(s.typing_text());
    for c in "hunk".chars() {
        key(&mut s, KeyCode::Char(c));
    }
    assert_eq!(s.view, View::Proposals, "typing in the search doesn't switch views");
    let found = ligus_term::ui::help_lines_text(&s);
    assert!(found.iter().any(|l| l.contains("Next hunk")), "{found:?}");
    assert!(found.iter().all(|l| !l.contains("Approve")), "{found:?}");
    key(&mut s, KeyCode::Enter);
    assert!(!s.typing_text(), "enter keeps the filter and gives the keys back");
    key(&mut s, KeyCode::Char('j'));
    assert!(matches!(s.overlay, Some(Overlay::Help { .. })), "j scrolls the filtered list");
    key(&mut s, KeyCode::Esc);
    assert!(matches!(&s.overlay, Some(Overlay::Help { query, .. }) if query.text.is_empty()), "esc clears the search first");
    key(&mut s, KeyCode::Char('/'));
    for c in "zzzz".chars() {
        key(&mut s, KeyCode::Char(c));
    }
    let none = ligus_term::ui::help_lines_text(&s);
    assert_eq!(none.len(), 1, "{none:?}");
    assert!(none[0].contains("No key matches"), "{none:?}");
    key(&mut s, KeyCode::Esc);
    key(&mut s, KeyCode::Esc);
    assert!(s.overlay.is_none(), "then esc closes it");
}

/// One toast at a time: a new one replaces the one on screen, errors stay
/// twice as long, and every notice is kept in the messages log (`!`), which
/// keeps the newest 200.
#[test]
fn one_toast_at_a_time_and_a_log_to_read_them_again() {
    let mut s = main_state();
    s.toast(ToastKind::Ok, "first");
    s.toast(ToastKind::Error, "second went wrong");
    assert_eq!(s.toasts.len(), 1);
    assert_eq!(s.toasts[0].text, "second went wrong");
    for _ in 0..16 {
        update(&mut s, Msg::Tick(NOW));
    }
    assert_eq!(s.toasts.len(), 1, "an error outlasts an ordinary notice");
    for _ in 0..16 {
        update(&mut s, Msg::Tick(NOW));
    }
    assert!(s.toasts.is_empty());
    assert_eq!(s.log.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["first", "second went wrong"]);
    key(&mut s, KeyCode::Char('!'));
    assert!(matches!(s.overlay, Some(Overlay::Messages { scroll: 0 })));
    key(&mut s, KeyCode::Char('j'));
    assert!(matches!(s.overlay, Some(Overlay::Messages { scroll: 1 })));
    key(&mut s, KeyCode::Char('j'));
    assert!(matches!(s.overlay, Some(Overlay::Messages { scroll: 1 })), "no scrolling past the oldest");
    key(&mut s, KeyCode::Esc);
    assert!(s.overlay.is_none());
    for i in 0..250 {
        s.toast(ToastKind::Info, format!("n{i}"));
    }
    assert_eq!(s.log.len(), LOG_CAP);
    assert_eq!(s.log.last().unwrap().text, "n249");
}

/// The status bar's "running" counts agents replying and runs moving.
#[test]
fn running_counts_busy_agents_and_moving_runs() {
    let mut s = main_state();
    assert_eq!(s.running(), 0);
    let mut busy = agent("a");
    busy.status = AgentStatus::Busy;
    s.agents = vec![busy, agent("b")];
    assert_eq!(s.running(), 1);
}

/// A chat's state comes from what lt already receives: an agent thinking
/// (working), a turn that finished or failed while you were elsewhere
/// (done, failed: cleared when you open the chat), a proposal waiting on
/// your vote or an agent's open question (needs you), and a lockdown
/// (paused). Needs you outranks everything.
#[test]
fn chat_states_from_live_signals() {
    let mut s = main_state();
    let (w, c1, c2) = (WorkspaceId::new("a"), ChatId::new("c1"), ChatId::new("c2"));
    assert_eq!(s.session_state(&c2), SessionState::Idle);
    update(&mut s, Msg::Event(Event::Thinking { workspace: w.clone(), chat: c2.clone(), agent: "@coder".into(), on: true }));
    assert_eq!(s.session_state(&c2), SessionState::Working);
    update(&mut s, Msg::Event(Event::MessageDone { workspace: w.clone(), chat: c2.clone(), message: MessageId::new("m"), error: None }));
    update(&mut s, Msg::Event(Event::Thinking { workspace: w.clone(), chat: c2.clone(), agent: "@coder".into(), on: false }));
    assert_eq!(s.session_state(&c2), SessionState::Done, "finished while c1 was open");
    // The open chat finishing isn't news.
    update(&mut s, Msg::Event(Event::MessageDone { workspace: w.clone(), chat: c1.clone(), message: MessageId::new("m1"), error: None }));
    assert_eq!(s.session_state(&c1), SessionState::Idle);
    update(
        &mut s,
        Msg::Event(Event::MessageDone {
            workspace: w.clone(),
            chat: c1.clone(),
            message: MessageId::new("m2"),
            error: Some("no runtime".into()),
        }),
    );
    assert_eq!(s.session_state(&c1), SessionState::Failed);
    // Opening c2 clears its "done".
    key(&mut s, KeyCode::Down);
    assert_eq!(s.open_chat.as_ref(), Some(&c2));
    assert_eq!(s.session_state(&c2), SessionState::Idle);
    // A proposal from c1 waiting on your vote: needs you, over failed.
    let mut p = proposal("p1", ProposalStatus::Open);
    p.chat = Some(c1.clone());
    update(&mut s, Msg::Proposals(w.clone(), Ok(vec![p.clone()])));
    assert_eq!(s.session_state(&c1), SessionState::NeedsYou);
    p.can_vote = Allowed::No("you already voted".into());
    update(&mut s, Msg::Proposals(w.clone(), Ok(vec![p])));
    assert_eq!(s.session_state(&c1), SessionState::Failed, "a proposal you can't vote on doesn't need you");
    // An agent's open question in c2.
    let ask = TaskState {
        chat: c2.clone(),
        agent: "coder".into(),
        status: "blocked".into(),
        summary: "which timeout?".into(),
        progress: None,
        open_questions: vec!["30s or 60s?".into()],
        at: NOW,
        active: true,
    };
    update(&mut s, Msg::Tasks(w.clone(), Ok(vec![ask])));
    assert_eq!(s.session_state(&c2), SessionState::NeedsYou);
    // A lockdown pauses a chat that has nothing else going on.
    let mut locked = chat("c3");
    locked.lockdown = Some(Lockdown { reasons: vec!["secret in a diff".into()], clear: Allowed::Yes });
    update(&mut s, Msg::Chats(w.clone(), Ok(vec![chat("c1"), chat("c2"), locked])));
    assert_eq!(s.session_state(&ChatId::new("c3")), SessionState::Paused);
    // Workspace load asks for task state, which feeds "needs you".
    let fx = update(&mut s, Msg::Event(Event::MemoryChanged { workspace: w.clone() }));
    assert!(fx.contains(&Effect::LoadTasks(w)));
}

/// The palette (`:`, `ctrl-p`, `ctrl-k`) lists actions with their keys,
/// views and the workspace's entities; typing filters, enter runs, and a
/// context action names what it acts on and only shows where it applies.
#[test]
fn palette_runs_actions_and_opens_entities() {
    use ligus_term::app::palette::{Target, filtered};
    let mut s = main_state();
    key(&mut s, KeyCode::Char(':'));
    assert!(matches!(s.overlay, Some(Overlay::Palette { .. })));
    assert!(s.typing_text(), "letters type into the palette");
    typed(&mut s, "go prop");
    let items = filtered(&s, "go prop");
    assert_eq!(items[0].target, Target::Action(ligus_term::keymap::Action::GoProposals), "{items:?}");
    assert!(items[0].key.contains('p') && items[0].key.contains('3'), "the key is shown: {:?}", items[0]);
    key(&mut s, KeyCode::Enter);
    assert!(s.overlay.is_none());
    assert_eq!(s.view, View::Proposals);
    // In Proposals, approve names the selected proposal.
    let mut p = proposal("p1", ProposalStatus::Open);
    p.title = "Fix the flaky test".into();
    update(&mut s, Msg::Proposals(WorkspaceId::new("a"), Ok(vec![p])));
    let all = filtered(&s, "approve");
    assert!(all.iter().any(|i| i.label == "Approve \u{201c}Fix the flaky test\u{201d}" && i.scope == "Proposals"), "{all:?}");
    // Not offered where it doesn't apply.
    key(&mut s, KeyCode::Char('c'));
    assert!(filtered(&s, "").iter().all(|i| !i.label.starts_with("Approve")));
    // From the composer, ctrl-p opens it too; an entity opens and selects.
    key(&mut s, KeyCode::Char('i'));
    assert_eq!(s.focus, Focus::Composer);
    mods(&mut s, KeyCode::Char('p'), KeyModifiers::CONTROL);
    assert!(matches!(s.overlay, Some(Overlay::Palette { .. })));
    typed(&mut s, "c2");
    let fx = key(&mut s, KeyCode::Enter);
    assert_eq!(s.open_chat, Some(ChatId::new("c2")));
    assert!(fx.contains(&Effect::LoadTranscript(WorkspaceId::new("a"), ChatId::new("c2"))));
    // ctrl-k is the same; ↓ past the end stays on the last entry; esc closes.
    mods(&mut s, KeyCode::Char('k'), KeyModifiers::CONTROL);
    typed(&mut s, "workspace");
    for _ in 0..50 {
        key(&mut s, KeyCode::Down);
    }
    let n = filtered(&s, "workspace").len();
    assert!(matches!(s.overlay, Some(Overlay::Palette { sel, .. }) if sel == n - 1));
    key(&mut s, KeyCode::Esc);
    assert!(s.overlay.is_none());
    // Switching workspace through it.
    key(&mut s, KeyCode::Char(':'));
    typed(&mut s, "switch ws b");
    key(&mut s, KeyCode::Enter);
    assert_eq!(s.active, Some(WorkspaceId::new("b")));
}

/// `space` waits for one more key: that key acts as it always does, and
/// esc or space again only put the sheet away.
#[test]
fn which_key_then_any_key_acts() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char(' '));
    assert_eq!(s.which_key, Some(NOW));
    key(&mut s, KeyCode::Char('p'));
    assert!(s.which_key.is_none());
    assert_eq!(s.view, View::Proposals);
    key(&mut s, KeyCode::Char(' '));
    key(&mut s, KeyCode::Esc);
    assert!(s.which_key.is_none());
    assert_eq!(s.view, View::Proposals, "esc put the sheet away and didn't go back");
    key(&mut s, KeyCode::Char(' '));
    key(&mut s, KeyCode::Char(' '));
    assert!(s.which_key.is_none(), "space again closes it");
    // In the composer space types.
    key(&mut s, KeyCode::Char('c'));
    key(&mut s, KeyCode::Char('i'));
    key(&mut s, KeyCode::Char(' '));
    assert!(s.which_key.is_none());
}

/// The composer: ↑ on an empty draft recalls what you sent (newest first),
/// ↓ comes back to an empty draft, an edit keeps the recalled text; ctrl-r
/// searches it; ctrl-g hands the draft to the editor and takes the result
/// back; `i` still enters the composer.
#[test]
fn composer_history_search_and_editor() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('i'));
    assert_eq!(s.focus, Focus::Composer);
    for m in ["first try", "second thought", "@coder fix the clock"] {
        typed(&mut s, m);
        let fx = key(&mut s, KeyCode::Enter);
        assert!(fx.iter().any(|f| matches!(f, Effect::Send { .. })));
    }
    key(&mut s, KeyCode::Up);
    assert_eq!(s.composer.text, "@coder fix the clock");
    key(&mut s, KeyCode::Up);
    assert_eq!(s.composer.text, "second thought");
    key(&mut s, KeyCode::Down);
    assert_eq!(s.composer.text, "@coder fix the clock");
    key(&mut s, KeyCode::Down);
    assert_eq!(s.composer.text, "", "↓ past the newest is an empty draft again");
    key(&mut s, KeyCode::Up);
    typed(&mut s, "!");
    assert_eq!(s.composer.text, "@coder fix the clock!");
    key(&mut s, KeyCode::Up);
    assert_eq!(s.composer.text, "@coder fix the clock!", "a draft you edited isn't replaced by ↑");
    s.composer.take();
    // ctrl-r: search what you sent.
    mods(&mut s, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert!(matches!(s.overlay, Some(Overlay::History { .. })));
    typed(&mut s, "thought");
    key(&mut s, KeyCode::Enter);
    assert!(s.overlay.is_none());
    assert_eq!(s.composer.text, "second thought");
    // ctrl-g: the editor gets the draft and its result replaces it.
    let fx = mods(&mut s, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert_eq!(fx, vec![Effect::EditDraft("second thought".into())]);
    update(&mut s, Msg::Edited(Ok("a longer\nmessage\n".into())));
    assert_eq!(s.composer.text, "a longer\nmessage");
    assert_eq!(s.focus, Focus::Composer);
    update(&mut s, Msg::Edited(Err(BackendError("vi: not found".into()))));
    assert_eq!(s.composer.text, "a longer\nmessage", "a failed editor leaves the draft alone");
    assert!(s.toasts.last().unwrap().text.contains("vi: not found"));
    // `i` still works after leaving.
    key(&mut s, KeyCode::Esc);
    key(&mut s, KeyCode::Char('i'));
    assert_eq!(s.focus, Focus::Composer);
}

/// $VISUAL wins over $EDITOR, both split on spaces, and vi is the last
/// resort.
#[test]
fn editor_command_from_the_environment() {
    use ligus_term::app::runtime::editor_command;
    let env =
        |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
    assert_eq!(editor_command(env(&[("VISUAL", "code --wait"), ("EDITOR", "nano")])), ["code", "--wait"]);
    assert_eq!(editor_command(env(&[("VISUAL", " "), ("EDITOR", "nano")])), ["nano"]);
    assert_eq!(editor_command(env(&[])), ["vi"]);
}

/// ctrl-x stops the open chat's turn when one is going; `o` shows what
/// tool calls returned.
#[test]
fn stop_turn_and_tool_output() {
    let mut s = main_state();
    let (w, c1) = (WorkspaceId::new("a"), ChatId::new("c1"));
    let fx = mods(&mut s, KeyCode::Char('x'), KeyModifiers::CONTROL);
    assert!(fx.is_empty(), "nothing to stop");
    assert!(s.toasts.last().unwrap().text.contains("No agent is working"));
    update(&mut s, Msg::Event(Event::Thinking { workspace: w.clone(), chat: c1.clone(), agent: "@coder".into(), on: true }));
    let fx = mods(&mut s, KeyCode::Char('x'), KeyModifiers::CONTROL);
    assert_eq!(fx, vec![Effect::StopTurn(w.clone(), c1.clone())]);
    update(&mut s, Msg::Stopped(w.clone(), c1.clone(), Ok(1)));
    assert!(s.toasts.last().unwrap().text.contains("Stopped the turn (@coder)"), "{:?}", s.toasts);
    update(&mut s, Msg::Stopped(w.clone(), c1.clone(), Ok(0)));
    assert!(s.toasts.last().unwrap().text.contains("host stops it"));
    // From the composer too.
    key(&mut s, KeyCode::Char('i'));
    let fx = mods(&mut s, KeyCode::Char('x'), KeyModifiers::CONTROL);
    assert_eq!(fx, vec![Effect::StopTurn(w, c1)]);
    key(&mut s, KeyCode::Esc);
    assert!(!s.tools_expanded);
    key(&mut s, KeyCode::Char('o'));
    assert!(s.tools_expanded);
}

/// The theme switches in the app: Settings' Theme row cycles every
/// palette then mono, its Appearance row cycles auto, light, dark, and the
/// palette shows a theme while its selection is on it (esc goes back).
#[test]
fn theme_switches_in_the_app() {
    use ligus_term::app::palette::Target;
    use ligus_term::app::state::Overlay;
    use ligus_term::theme::{Appearance, ThemeRequest};
    let set = |t: &str| Some(ThemeRequest::Set { theme: Some(t.into()), appearance: None });
    let mut s = main_state();
    update(
        &mut s,
        Msg::Settings(Ok(Settings {
            your_name: "Mara".into(),
            device_name: "lt".into(),
            notify_mentions: true,
            agents_may_edit: false,
            relay_url: None,
            linked_to: None,
            link_pending: None,
        })),
    );
    key(&mut s, KeyCode::Char('s'));
    key(&mut s, KeyCode::Down);
    key(&mut s, KeyCode::Down);
    key(&mut s, KeyCode::Enter);
    assert_eq!(s.theme_request.take(), set("studio"));
    s.theme_id = "studio".into();
    update(&mut s, Msg::ThemeChanged);
    let toast = &s.toasts.last().unwrap().text;
    assert!(toast.starts_with("Theme: Studio") && toast.contains("theme = \"studio\""), "{toast}");
    s.theme_id = "obsidian".into();
    key(&mut s, KeyCode::Char('t'));
    assert_eq!(s.theme_request.take(), set("mono"));
    s.theme_id = "mono".into();
    key(&mut s, KeyCode::Char('t'));
    assert_eq!(s.theme_request.take(), set("pollen"));
    key(&mut s, KeyCode::Down);
    key(&mut s, KeyCode::Enter);
    assert_eq!(s.theme_request.take(), Some(ThemeRequest::Set { theme: None, appearance: Some(Appearance::Light) }));
    s.appearance = Appearance::Light;
    update(&mut s, Msg::AppearanceChanged);
    assert!(s.toasts.last().unwrap().text.contains("appearance = \"light\""));

    // The palette: typing a theme's name shows it; esc goes back.
    s.theme_id = "pollen".into();
    key(&mut s, KeyCode::Char(':'));
    for c in "harbor".chars() {
        key(&mut s, KeyCode::Char(c));
    }
    let Some(Overlay::Palette { query, sel }) = &s.overlay else { panic!("no palette") };
    let items = ligus_term::app::palette::filtered(&s, &query.text);
    assert_eq!(items[*sel].target, Target::Theme("harbor".into()));
    assert_eq!(s.theme_request.take(), Some(ThemeRequest::Preview { theme: Some("harbor".into()), appearance: None }));
    assert!(s.theme_previewing);
    key(&mut s, KeyCode::Esc);
    assert_eq!(s.theme_request.take(), Some(ThemeRequest::Revert));
    // Enter keeps it.
    key(&mut s, KeyCode::Char(':'));
    for c in "harbor".chars() {
        key(&mut s, KeyCode::Char(c));
    }
    key(&mut s, KeyCode::Enter);
    assert_eq!(s.theme_request.take(), set("harbor"));
    assert!(!s.theme_previewing);
}

fn click(s: &mut State, kind: crossterm::event::MouseEventKind, column: u16, row: u16) -> Vec<Effect> {
    update(s, Msg::Mouse(crossterm::event::MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }))
}

/// The mouse: a click on a tab opens it, on a chat opens the chat, on the
/// transcript or the composer focuses it; the wheel scrolls what is under
/// the pointer, and in an overlay moves like ↑/↓.
#[test]
fn mouse_clicks_and_wheel() {
    use crossterm::event::{MouseButton, MouseEventKind as K};
    let left = K::Down(MouseButton::Left);
    let mut s = main_state();
    update(&mut s, Msg::Resize(110, 32));
    let x = (0..110).find(|x| ligus_term::ui::tab_at(&s, 110, *x) == Some(View::Proposals)).expect("a Proposals tab");
    click(&mut s, left, x, 0);
    assert_eq!(s.view, View::Proposals);
    let x = (0..110).find(|x| ligus_term::ui::tab_at(&s, 110, *x) == Some(View::Chats)).unwrap();
    click(&mut s, left, x, 0);
    assert_eq!(s.view, View::Chats);
    // The list: c1 on row 2, c2 on row 3.
    let fx = click(&mut s, left, 5, 3);
    assert_eq!(s.open_chat, Some(ChatId::new("c2")));
    assert!(fx.contains(&Effect::LoadTranscript(WorkspaceId::new("a"), ChatId::new("c2"))));
    // The transcript, then the composer at the bottom.
    click(&mut s, left, 60, 10);
    assert_eq!(s.focus, Focus::Detail);
    click(&mut s, K::ScrollUp, 60, 10);
    assert_eq!(s.transcript_scroll, 3, "the wheel scrolls three lines");
    click(&mut s, K::ScrollDown, 60, 10);
    assert_eq!(s.transcript_scroll, 0);
    click(&mut s, left, 60, 29);
    assert_eq!(s.focus, Focus::Composer);
    // The wheel over the list moves the selection.
    click(&mut s, K::ScrollUp, 5, 3);
    assert_eq!(s.open_chat, Some(ChatId::new("c1")));
    // In an overlay the wheel is ↑/↓.
    key(&mut s, KeyCode::Esc);
    key(&mut s, KeyCode::Char('?'));
    click(&mut s, K::ScrollDown, 50, 10);
    assert!(matches!(s.overlay, Some(Overlay::Help { scroll: 1, .. })));
    // A right click or a drag does nothing.
    let before = s.view;
    click(&mut s, K::Down(MouseButton::Right), x, 0);
    click(&mut s, K::Drag(MouseButton::Left), 5, 3);
    assert_eq!(s.view, before);
}

/// A click on a proposal selects it and previews its diff; the wheel over
/// the diff scrolls it.
#[test]
fn mouse_on_proposals() {
    use crossterm::event::{MouseButton, MouseEventKind as K};
    let mut s = main_state();
    update(&mut s, Msg::Resize(110, 32));
    key(&mut s, KeyCode::Char('p'));
    let w = WorkspaceId::new("a");
    update(&mut s, Msg::Proposals(w.clone(), Ok(vec![proposal("p1", ProposalStatus::Open), proposal("p2", ProposalStatus::Open)])));
    // One line a proposal: the second is on the list's second row.
    let fx = click(&mut s, K::Down(MouseButton::Left), 5, 3);
    assert_eq!(s.proposal_sel.index, 1);
    assert_eq!(fx, vec![Effect::PreviewDiff(w, ProposalId::new("p2"))]);
    click(&mut s, K::ScrollUp, 5, 3);
    assert_eq!(s.proposal_sel.index, 0);
}

/// The first run offers a look around first: choosing it quits into
/// `lt --demo` without a name or a workspace. "+" later doesn't offer it.
#[test]
fn first_run_can_look_around_in_the_demo() {
    let mut s = state();
    update(&mut s, Msg::Startup(Ok(StartupState { your_name: None, workspaces: vec![] })));
    key(&mut s, KeyCode::Tab);
    key(&mut s, KeyCode::Left);
    assert!(matches!(&s.phase, Phase::Onboarding(f) if f.mode == FormMode::Demo));
    let fx = key(&mut s, KeyCode::Enter);
    assert!(fx.is_empty(), "nothing is created");
    assert!(s.quit && s.exec_demo);
    // Later, "+" cycles past it.
    let mut s = main_state();
    key(&mut s, KeyCode::Char('+'));
    key(&mut s, KeyCode::Left);
    assert!(matches!(&s.overlay, Some(Overlay::AddWorkspace(f)) if f.mode == FormMode::Connect));
    key(&mut s, KeyCode::Right);
    assert!(matches!(&s.overlay, Some(Overlay::AddWorkspace(f)) if f.mode == FormMode::Create));
    // A workspace with no agents looks for runtimes for its first steps.
    let fx = update(&mut s, Msg::Agents(WorkspaceId::new("a"), Ok(vec![])));
    assert_eq!(fx, vec![Effect::LoadLocalRuntimes(WorkspaceId::new("a"))]);
}

/// A chat that newly needs you rings the bell; while the terminal is in
/// the background it also queues one desktop notification. What already
/// waited when the workspace opened is quiet, and so is a focused terminal.
#[test]
fn needs_you_signals_when_away() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    let mut p1 = proposal("p1", ProposalStatus::Open);
    p1.chat = Some(ChatId::new("c1"));
    update(&mut s, Msg::Proposals(w.clone(), Ok(vec![p1])));
    assert!(!s.bell, "already waiting when the workspace opened");
    let mut p2 = proposal("p2", ProposalStatus::Open);
    p2.chat = Some(ChatId::new("c2"));
    update(&mut s, Msg::Event(Event::ProposalChanged { workspace: w.clone(), proposal: p2.clone() }));
    assert!(s.bell, "a new one rings");
    assert!(s.signals.is_empty(), "no desktop notification while focused");
    s.bell = false;
    update(&mut s, Msg::Focus(false));
    let mut p3 = proposal("p3", ProposalStatus::Open);
    p3.chat = Some(ChatId::new("c3"));
    update(&mut s, Msg::Chats(w.clone(), Ok(vec![chat("c1"), chat("c2"), chat("c3")])));
    update(&mut s, Msg::Event(Event::ProposalChanged { workspace: w.clone(), proposal: p3 }));
    assert!(s.bell);
    assert_eq!(s.signals, vec!["\u{201c}c3\u{201d} needs you".to_string()]);
    // A turn you asked for ends while you're away.
    s.signals.clear();
    key(&mut s, KeyCode::Char('i'));
    typed(&mut s, "@coder go");
    key(&mut s, KeyCode::Enter);
    update(
        &mut s,
        Msg::Event(Event::MessageDone { workspace: w.clone(), chat: ChatId::new("c1"), message: MessageId::new("m"), error: None }),
    );
    assert_eq!(s.signals, vec!["A turn finished in \u{201c}c1\u{201d}".to_string()]);
    // Not for a chat you didn't send to.
    s.signals.clear();
    update(&mut s, Msg::Event(Event::MessageDone { workspace: w, chat: ChatId::new("c3"), message: MessageId::new("m3"), error: None }));
    assert!(s.signals.is_empty());
}

/// OSC 9 carries the text without control characters, and goes through
/// tmux's passthrough inside tmux.
#[test]
fn desktop_notification_sequence() {
    use ligus_term::app::runtime::osc9;
    assert_eq!(osc9("“Fix” needs you", false), "\x1b]9;“Fix” needs you\x07".as_bytes());
    assert_eq!(osc9("a\x07b\x1b]c", false), b"\x1b]9;ab]c\x07");
    assert_eq!(osc9("hi", true), b"\x1bPtmux;\x1b\x1b]9;hi\x07\x1b\\");
}

/// `/` finds in the focused list as you type, ↓ goes to the next match,
/// enter keeps it and esc goes back to where the find started.
#[test]
fn find_in_a_list() {
    let mut s = main_state();
    let w = WorkspaceId::new("a");
    update(&mut s, Msg::Chats(w.clone(), Ok(vec![chat("checkout tests"), chat("auth"), chat("checkout docs")])));
    key(&mut s, KeyCode::Char('/'));
    assert!(s.find.is_some() && s.typing_text());
    typed(&mut s, "check");
    assert_eq!(s.chat_sel.index, 0);
    key(&mut s, KeyCode::Down);
    assert_eq!(s.chat_sel.index, 2, "the next match");
    assert_eq!(s.open_chat, Some(ChatId::new("checkout docs")), "moving there opens it, as ↓ does");
    key(&mut s, KeyCode::Down);
    assert_eq!(s.chat_sel.index, 0, "round again");
    key(&mut s, KeyCode::Esc);
    assert!(s.find.is_none());
    assert_eq!(s.chat_sel.index, 0, "back where it started");
    key(&mut s, KeyCode::Char('/'));
    typed(&mut s, "auth");
    key(&mut s, KeyCode::Enter);
    assert!(s.find.is_none());
    assert_eq!(s.chat_sel.index, 1, "enter keeps it");
    assert_eq!(s.view, View::Chats, "typing in the find never switched views");
    // In People too.
    key(&mut s, KeyCode::Char('m'));
    update(
        &mut s,
        Msg::Members(
            w,
            Ok(vec![member("Ada", Role::Admin, Allowed::Yes, Allowed::Yes), member("Lin", Role::Contributor, Allowed::Yes, Allowed::Yes)]),
        ),
    );
    key(&mut s, KeyCode::Char('/'));
    typed(&mut s, "lin");
    assert_eq!(s.member_sel.index, 1);
}

/// esc and ctrl-o go back to the view you came from (not always Chats);
/// alt-right goes forward again.
#[test]
fn view_history_back_and_forward() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('p'));
    key(&mut s, KeyCode::Char('m'));
    assert_eq!(s.view, View::People);
    key(&mut s, KeyCode::Esc);
    assert_eq!(s.view, View::Proposals, "esc goes back, not to Chats");
    mods(&mut s, KeyCode::Right, KeyModifiers::ALT);
    assert_eq!(s.view, View::People);
    mods(&mut s, KeyCode::Char('o'), KeyModifiers::CONTROL);
    mods(&mut s, KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(s.view, View::Chats);
    mods(&mut s, KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(s.view, View::Chats, "nothing further back");
}

/// D shows the whole transcript as plain text (no control or bidi
/// characters), and quitting says what is still going.
#[test]
fn transcript_dump_and_exit_summary() {
    let mut s = main_state();
    assert_eq!(s.exit_summary(), None, "nothing going: nothing said");
    let m = Message {
        id: MessageId::new("m1"),
        chat: ChatId::new("c1"),
        author: Author::Agent { id: AgentId::new("a"), handle: "coder".into(), runtime: "claude-code".into() },
        text: "fixed \u{1b}[2Jit\u{202e}\nline two".into(),
        at: NOW - 60_000,
        streaming: false,
        tools: vec![ToolCall { name: "read_file".into(), preview: "src/a.rs".into(), ok: true, ..ToolCall::default() }],
    };
    update(&mut s, Msg::Transcript(WorkspaceId::new("a"), ChatId::new("c1"), Ok(vec![m])));
    let fx = key(&mut s, KeyCode::Char('D'));
    let [Effect::ShowText(text)] = fx.as_slice() else { panic!("{fx:?}") };
    assert!(text.starts_with("c1\n") && text.contains("@coder  1m ago") && text.contains("[read_file] src/a.rs done"), "{text}");
    assert!(text.contains("  fixed [2Jit\n  line two"), "control and bidi characters are gone: {text:?}");
    update(&mut s, Msg::Shown(Ok(false)));
    assert!(s.toasts.last().unwrap().text.contains("scrollback"));
    let mut r = run_waiting("r1");
    r.status = RunStatus::Running;
    r.driver = Some("build-box".into());
    let mut done = run_waiting("r2");
    done.status = RunStatus::Completed;
    s.runs = vec![r, done];
    s.proposals = vec![proposal("p1", ProposalStatus::Open), proposal("p2", ProposalStatus::Approved)];
    assert_eq!(s.exit_summary().as_deref(), Some("lt: 1 run still going (driven by build-box); 1 proposal waiting on your vote."));
}

fn mention(id: &str, chat: &str) -> Notification {
    Notification {
        id: NotificationId::new(id),
        workspace: WorkspaceId::new("a"),
        chat: Some(ChatId::new(chat)),
        text: format!("Ben mentioned you ({id})"),
        at: NOW,
        read: false,
    }
}

/// Quiet notifications: a mention in the chat you're looking at is
/// already on screen (no toast, no bell, no badge); a burst is one toast
/// that counts them, while the log keeps each.
#[test]
fn quiet_notifications_and_bursts() {
    let mut s = main_state();
    assert_eq!(s.open_chat, Some(ChatId::new("c1")));
    let fx = update(&mut s, Msg::Event(Event::Notification(mention("n1", "c1"))));
    assert_eq!(fx, vec![Effect::MarkRead(WorkspaceId::new("a"), ChatId::new("c1"))]);
    assert!(s.toasts.is_empty() && !s.bell, "it's on screen already");
    assert_eq!(s.unread_notifications(), 0, "and doesn't inflate the badge");
    for id in ["n2", "n3", "n4"] {
        update(&mut s, Msg::Event(Event::Notification(mention(id, "c2"))));
    }
    assert_eq!(s.toasts.len(), 1);
    assert_eq!(s.toasts[0].text, "3 new notifications (n reads them)");
    assert_eq!(s.log.iter().filter(|e| e.text.starts_with("Ben mentioned you")).count(), 3, "the log keeps each");
    assert_eq!(s.unread_notifications(), 3);
    assert_eq!(s.needs_you(), 3);
    // Once the toast is gone, the next one starts a new burst.
    for _ in 0..20 {
        update(&mut s, Msg::Tick(NOW));
    }
    update(&mut s, Msg::Event(Event::Notification(mention("n5", "c2"))));
    assert_eq!(s.toasts[0].text, "Ben mentioned you (n5)");
}

/// Focus mode (ctrl-f) hides the bars and comes back the same way.
#[test]
fn focus_mode_toggles() {
    let mut s = main_state();
    mods(&mut s, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert!(s.focus_mode);
    key(&mut s, KeyCode::Char('i'));
    mods(&mut s, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert!(!s.focus_mode, "from the composer too");
}

/// Add to my devices: the request is made once, a grant is checked first
/// (who approved it) and redeemed only on the second Enter, and editing the
/// code asks again.
#[test]
fn add_to_my_devices_checks_the_grant_before_linking() {
    let mut s = state();
    s.phase = Phase::Onboarding(WorkspaceForm::new(true));
    key(&mut s, KeyCode::Tab);
    let fx = key(&mut s, KeyCode::Char('a'));
    assert_eq!(fx, vec![Effect::LinkRequest]);
    assert!(key(&mut s, KeyCode::Tab).is_empty(), "asked once");
    // No name needed: the person comes from the desktop's code.
    let fx = key(&mut s, KeyCode::Enter);
    assert!(fx.is_empty());
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert_eq!(f.error.as_deref(), Some("Paste the approval address your desktop shows"));
    // A paste is checked at once: straight to who approved it.
    assert_eq!(update(&mut s, Msg::Paste("ligusgrant1:abc".into())), vec![Effect::InspectLink("ligusgrant1:abc".into())]);
    let view = LinkGrantView { person_name: "Alex".into(), workspace_name: "Acme".into(), approver_fingerprint: "2C41".into() };
    update(&mut s, Msg::LinkChecked("ligusgrant1:abc".into(), Ok(view)));
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::RedeemLink("ligusgrant1:abc".into())]);
    // A refusal shows on the form.
    update(&mut s, Msg::Linked(Err(BackendError("the grant wasn't issued by a device of that person".into()))));
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert!(f.error.as_deref().unwrap().contains("wasn't issued"));
    // Editing the code checks it again.
    key(&mut s, KeyCode::Char('x'));
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::InspectLink("ligusgrant1:abcx".into())]);
}

fn settings(linked_to: Option<&str>) -> Settings {
    Settings {
        your_name: "zaz".into(),
        device_name: "lt".into(),
        notify_mentions: true,
        agents_may_edit: false,
        relay_url: None,
        linked_to: linked_to.map(Into::into),
        link_pending: None,
    }
}

fn people(you: &str, other: &str) -> Vec<Member> {
    let mut me = member("m-you", Role::Owner, Allowed::No("that's you".into()), Allowed::Yes);
    me.name = you.into();
    me.you = true;
    let mut desk = member("m-desk", Role::Owner, Allowed::Yes, Allowed::Yes);
    desk.name = other.into();
    desk.founder = true;
    vec![desk, me]
}

/// This terminal as its own member next to the person's desktop: People and
/// Settings say so, D links (in both views), and a linked install isn't
/// asked again.
#[test]
fn an_own_member_terminal_is_offered_link_to_my_devices() {
    let mut s = main_state();
    update(&mut s, Msg::Settings(Ok(settings(None))));
    update(&mut s, Msg::Members(WorkspaceId::new("a"), Ok(people("zaz", "zazulam"))));
    assert!(s.own_member_unlinked());
    assert_eq!(s.likely_me_elsewhere().map(|m| m.name.as_str()), Some("zazulam"));
    // A teammate with another name isn't taken for you (Settings still
    // says this terminal is its own member).
    update(&mut s, Msg::Members(WorkspaceId::new("a"), Ok(people("zaz", "Taylor"))));
    assert!(s.own_member_unlinked());
    assert!(s.likely_me_elsewhere().is_none());
    // Two letters aren't enough to call it a match.
    update(&mut s, Msg::Members(WorkspaceId::new("a"), Ok(people("Al", "Alex"))));
    assert!(s.likely_me_elsewhere().is_none());
    update(&mut s, Msg::Members(WorkspaceId::new("a"), Ok(people("zaz", "zazulam"))));

    key(&mut s, KeyCode::Char('m'));
    assert_eq!(key(&mut s, KeyCode::Char('D')), vec![Effect::LinkRequest]);
    assert!(matches!(&s.overlay, Some(Overlay::AddWorkspace(f)) if f.mode == FormMode::Link));
    key(&mut s, KeyCode::Esc);
    key(&mut s, KeyCode::Char('s'));
    assert_eq!(key(&mut s, KeyCode::Char('D')), vec![Effect::LinkRequest]);
    key(&mut s, KeyCode::Esc);
    // The row menus offer it too.
    key(&mut s, KeyCode::Char('.'));
    let Some(Overlay::RowMenu { items, .. }) = &s.overlay else { panic!("menu") };
    assert!(items.iter().any(|i| i.label == "Link to my devices…"));
    assert!(items.iter().any(|i| i.label == "Copy your safety code"));
    key(&mut s, KeyCode::Esc);
    key(&mut s, KeyCode::Char('m'));
    key(&mut s, KeyCode::Char('.'));
    let Some(Overlay::RowMenu { items, .. }) = &s.overlay else { panic!("menu") };
    assert!(items.iter().any(|i| i.label == "Link to my devices…"));
    key(&mut s, KeyCode::Esc);

    // Linked: nothing to offer, and D says whose device it is.
    update(&mut s, Msg::Settings(Ok(settings(Some("zazulam")))));
    assert!(!s.own_member_unlinked());
    assert!(s.likely_me_elsewhere().is_none());
    assert!(key(&mut s, KeyCode::Char('D')).is_empty());
    assert!(s.toasts.iter().any(|t| t.text == "Already one of zazulam's devices"));
}

/// The palette finds Link this computer to my devices by "link", "computer"
/// and "add a computer", once.
#[test]
fn the_palette_finds_linking_by_its_synonyms() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('s'));
    for q in ["link", "computer", "add a computer", "devices"] {
        let hits = ligus_term::app::palette::filtered(&s, q);
        let n =
            hits.iter().filter(|i| i.target == ligus_term::app::palette::Target::Action(ligus_term::keymap::Action::LinkDevice)).count();
        assert_eq!(n, 1, "{q}: {:?}", hits.iter().take(3).map(|i| &i.label).collect::<Vec<_>>());
    }
}

fn link_view() -> LinkRequestView {
    LinkRequestView { code: "liguslink1:AAAABBBBCCCC".into(), fingerprint: "7F3A-91C0".into(), label: "lt on mbp".into(), new_device: true }
}

/// The link request is copied only on c (or ctrl-y), shown alone with o,
/// and c types once the desktop's code is being pasted.
#[test]
fn the_link_request_copies_on_c_and_shows_full_screen_on_o() {
    let mut s = main_state();
    key(&mut s, KeyCode::Char('s'));
    key(&mut s, KeyCode::Char('D'));
    update(&mut s, Msg::LinkRequested(Ok(link_view())));
    let copy = Effect::Copy(ligus_term::clipboard::Clip::new("liguslink1:AAAABBBBCCCC", "run lt --plain link-request"));
    assert_eq!(key(&mut s, KeyCode::Char('c')), vec![copy.clone()]);
    assert_eq!(mods(&mut s, KeyCode::Char('y'), KeyModifiers::CONTROL), vec![copy.clone()]);
    assert!(key(&mut s, KeyCode::Char('o')).is_empty());
    assert!(
        matches!(&s.overlay, Some(Overlay::Code { code, fingerprint: Some(fp), .. }) if code == "liguslink1:AAAABBBBCCCC" && fp == "7F3A-91C0")
    );
    assert_eq!(key(&mut s, KeyCode::Char('c')), vec![copy.clone()]);
    // Any other key goes back to the form, as it was.
    key(&mut s, KeyCode::Esc);
    assert!(matches!(&s.overlay, Some(Overlay::AddWorkspace(f)) if f.link.is_some()));
    // Pasting the desktop's code: now c types, ctrl-y still copies.
    assert_eq!(update(&mut s, Msg::Paste("ligusgrant1:x".into())), vec![Effect::InspectLink("ligusgrant1:x".into())]);
    update(&mut s, Msg::LinkChecked("ligusgrant1:x".into(), Err(BackendError("damaged".into()))));
    key(&mut s, KeyCode::Char('c'));
    assert!(matches!(&s.overlay, Some(Overlay::AddWorkspace(f)) if f.grant.text == "ligusgrant1:xc"));
    assert_eq!(mods(&mut s, KeyCode::Char('y'), KeyModifiers::CONTROL), vec![copy]);
    // The toast.
    update(&mut s, Msg::Copied(Ok("Copied".into())));
    assert!(s.toasts.iter().any(|t| t.text == "Copied"));
    update(&mut s, Msg::Copied(Err("Couldn't reach the clipboard — run lt --plain link-request".into())));
    assert!(s.toasts.iter().any(|t| t.text.contains("lt --plain link-request")));
}

/// An invite and a box's admission copy only on c; o shows them alone;
/// the copied code never shows in an effect's Debug.
#[test]
fn invites_and_admissions_copy_on_c_and_never_print_the_code() {
    let mut s = main_state();
    s.overlay = Some(Overlay::Invite { code: Some("hivews1:SECRET".into()) });
    let fx = key(&mut s, KeyCode::Char('c'));
    assert!(matches!(&fx[..], [Effect::Copy(c)] if c.text == "hivews1:SECRET"));
    assert!(!format!("{fx:?}").contains("SECRET"));
    key(&mut s, KeyCode::Char('o'));
    assert!(matches!(&s.overlay, Some(Overlay::Code { code, .. }) if code == "hivews1:SECRET"));
    key(&mut s, KeyCode::Esc);
    assert!(matches!(&s.overlay, Some(Overlay::Invite { .. })));
    key(&mut s, KeyCode::Char('x'));
    assert!(s.overlay.is_none());

    s.overlay = Some(Overlay::AdmitBox {
        request: ligus_term::app::input::TextBuf::single(),
        busy: false,
        checked: None,
        result: Some(BoxAdmission {
            code: "ligusadmit1:z".into(),
            sealed: true,
            fingerprint: Some("AAAA".into()),
            label: None,
            expires_at: NOW,
        }),
    });
    let fx = key(&mut s, KeyCode::Char('c'));
    assert!(matches!(&fx[..], [Effect::Copy(c)] if c.text == "ligusadmit1:z"));
    key(&mut s, KeyCode::Char('o'));
    assert!(matches!(&s.overlay, Some(Overlay::Code { fingerprint: Some(f), .. }) if f == "AAAA"));
}

// ── Add to my devices, live (code match) ────────────────────────────────

const ADDR: &str = "ligusaddr1:eyJ2IjoxfQ";

fn live_view() -> LinkRequestView {
    LinkRequestView { code: "liguslink1:abc".into(), fingerprint: "7F3A-91C0-77DE-0B14-5E62".into(), label: "lt".into(), new_device: false }
}

fn code(shown: &str) -> MatchCodeView {
    MatchCodeView { shown: shown.into(), spelled: String::new(), claimed: "Alex's desktop".into(), expires_at: 1_000_000 + 5 * 60_000 }
}

/// The first-run form on Add to my devices with the approval address
/// pasted and sent: live request `r1`, waiting for the desktop.
fn live_waiting() -> State {
    let mut s = state();
    s.now = 1_000_000;
    s.phase = Phase::Onboarding(WorkspaceForm::linking(true));
    update(&mut s, Msg::LinkRequested(Ok(live_view())));
    update(&mut s, Msg::Paste(ADDR.into()));
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::LinkLive(ADDR.into())]);
    assert_eq!(stage(&s), Some(LiveStage::Sending));
    update(&mut s, Msg::LiveStarted(Ok(LiveLinkStart::Posted { rid: "r1".into(), request: live_view() })));
    assert_eq!(stage(&s), Some(LiveStage::Waiting));
    s
}

fn stage(s: &State) -> Option<LiveStage> {
    let Phase::Onboarding(f) = &s.phase else { return None };
    f.live.as_ref().map(|l| l.stage.clone())
}

fn live(s: &mut State, update_: LiveLinkUpdate) {
    update(s, Msg::Event(Event::LiveLink { rid: "r1".into(), update: update_ }));
}

/// No on the code ends the request at once: the abort goes out, and the
/// form says nothing was granted. The first code is the one shown; a later
/// one (a racing desktop) never replaces it.
#[test]
fn live_link_no_on_the_code_ends_it() {
    let mut s = live_waiting();
    live(&mut s, LiveLinkUpdate::Opened(code("K7Q4 M9X2")));
    live(&mut s, LiveLinkUpdate::Opened(code("ZZZZ 0000")));
    let Some(LiveStage::Code { code: c, yes: false }) = stage(&s) else { panic!("{:?}", stage(&s)) };
    assert_eq!(c.shown, "K7Q4 M9X2", "the first code stays");
    // Enter on the default answer is No: a stray Enter grants nothing.
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::LinkLiveCode("r1".into(), false)]);
    let Some(LiveStage::Ended(e)) = stage(&s) else { panic!() };
    assert_eq!(e, LiveLinkEnd::Mismatch);
    assert_eq!(e.describe(), "Request didn't match. Nothing was granted.");
    // Enter starts again with a new request.
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::LinkLive(ADDR.into())]);
}

/// Yes on the code, then the verified person: No links nothing.
#[test]
fn live_link_no_on_the_verified_person_links_nothing() {
    let mut s = live_waiting();
    live(&mut s, LiveLinkUpdate::Opened(code("K7Q4 M9X2")));
    assert_eq!(key(&mut s, KeyCode::Char('y')), vec![Effect::LinkLiveCode("r1".into(), true)]);
    assert!(matches!(stage(&s), Some(LiveStage::Matched { acknowledged: false, .. })));
    live(&mut s, LiveLinkUpdate::Acknowledged);
    assert!(matches!(stage(&s), Some(LiveStage::Matched { acknowledged: true, .. })));
    live(&mut s, LiveLinkUpdate::Checking);
    assert_eq!(stage(&s), Some(LiveStage::Checking));
    live(&mut s, LiveLinkUpdate::Verified { person: "Ben".into(), workspace: "Acme".into() });
    assert!(matches!(stage(&s), Some(LiveStage::Confirm { ref person, yes: false, busy: false, .. }) if person == "Ben"));
    assert_eq!(key(&mut s, KeyCode::Char('n')), vec![Effect::LinkLivePerson("r1".into(), false)]);
    assert_eq!(stage(&s), Some(LiveStage::Ended(LiveLinkEnd::NotLinked { person: "Ben".into() })));
    assert!(matches!(s.phase, Phase::Onboarding(_)), "nothing linked");
}

/// Yes, link: linked as the verified person.
#[test]
fn live_link_yes_links_as_the_verified_person() {
    let mut s = live_waiting();
    live(&mut s, LiveLinkUpdate::Opened(code("K7Q4 M9X2")));
    key(&mut s, KeyCode::Right);
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::LinkLiveCode("r1".into(), true)]);
    live(&mut s, LiveLinkUpdate::Verified { person: "Alex".into(), workspace: "Acme".into() });
    key(&mut s, KeyCode::Left);
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::LinkLivePerson("r1".into(), true)]);
    assert!(matches!(stage(&s), Some(LiveStage::Confirm { busy: true, .. })));
    assert!(key(&mut s, KeyCode::Enter).is_empty(), "one answer");
    let fx = update(&mut s, Msg::LivePerson(Ok(Some(ws("ws-acme", Role::Contributor)))));
    assert!(matches!(s.phase, Phase::Main));
    assert!(fx.contains(&Effect::LoadSettings));
    assert!(s.toasts.iter().any(|t| t.text.contains("Alex")), "{:?}", s.toasts);
}

/// Another request's events change nothing; Esc ends this one without
/// quitting; the code drops at zero; a relay without a mailbox falls back
/// to the paste path.
#[test]
fn live_link_stale_events_esc_expiry_and_fallback() {
    let mut s = live_waiting();
    update(&mut s, Msg::Event(Event::LiveLink { rid: "other".into(), update: LiveLinkUpdate::Opened(code("ZZZZ 0000")) }));
    assert_eq!(stage(&s), Some(LiveStage::Waiting));
    assert_eq!(key(&mut s, KeyCode::Esc), vec![Effect::LinkLiveCancel("r1".into())]);
    assert_eq!(stage(&s), Some(LiveStage::Ended(LiveLinkEnd::Cancelled)));
    assert!(!s.quit);

    let mut s = live_waiting();
    live(&mut s, LiveLinkUpdate::Opened(code("K7Q4 M9X2")));
    assert!(update(&mut s, Msg::Tick(1_000_000 + 60_000)).is_empty());
    assert_eq!(update(&mut s, Msg::Tick(1_000_000 + 5 * 60_000 + 1)), vec![Effect::LinkLiveCancel("r1".into())]);
    assert_eq!(stage(&s), Some(LiveStage::Ended(LiveLinkEnd::Expired)));

    let mut s = state();
    s.phase = Phase::Onboarding(WorkspaceForm::linking(true));
    update(&mut s, Msg::LinkRequested(Ok(live_view())));
    update(&mut s, Msg::Paste(ADDR.into()));
    key(&mut s, KeyCode::Enter);
    update(&mut s, Msg::LiveStarted(Ok(LiveLinkStart::NoMailbox { request: live_view() })));
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert!(f.live.is_none() && f.live_unavailable && f.grant.text.is_empty() && f.error.is_none());
    // The paste path, as before: straight to who approved it.
    assert_eq!(update(&mut s, Msg::Paste("ligusgrant1:abc".into())), vec![Effect::InspectLink("ligusgrant1:abc".into())]);
}

/// While waiting, an older desktop's code pasted in takes the paste path
/// and ends the live request.
#[test]
fn live_link_an_older_desktops_code_ends_the_live_request() {
    let mut s = live_waiting();
    assert_eq!(key(&mut s, KeyCode::Char('c')).len(), 1, "c copies the request while waiting");
    assert_eq!(
        update(&mut s, Msg::Paste("ligusgrant1:abc".into())),
        vec![Effect::LinkLiveCancel("r1".into()), Effect::InspectLink("ligusgrant1:abc".into())]
    );
    assert_eq!(stage(&s), None);
}

/// The desktop's code pasted with whatever came with it (whitespace, line
/// breaks, the whole command someone copied) still goes straight to who
/// approved it; a request or the bare command says what to paste instead.
#[test]
fn a_pasted_grant_is_cut_out_of_what_came_with_it() {
    let g = "ligusgrant1:eyJ2Ijox_LCJh-Zm9v";
    for pasted in [
        format!("\n  {g}\n\n"),
        "ligusgrant1:eyJ2Ijox_\n  LCJh-Zm9v\n".to_string(),
        format!("lt --plain link-redeem '{g}' --approver-fingerprint 2C41-88E0-1B9D-04AF-6E73"),
    ] {
        let mut s = state();
        s.phase = Phase::Onboarding(WorkspaceForm::new(true));
        key(&mut s, KeyCode::Tab);
        key(&mut s, KeyCode::Char('a'));
        key(&mut s, KeyCode::Tab);
        assert_eq!(update(&mut s, Msg::Paste(pasted.clone())), vec![Effect::InspectLink(g.into())], "{pasted:?}");
        let Phase::Onboarding(f) = &s.phase else { panic!() };
        assert_eq!(f.grant.text, g);
        assert!(f.busy);
    }
    // On the mode row, in another mode: a grant still opens the link form.
    let mut s = state();
    s.phase = Phase::Onboarding(WorkspaceForm::new(true));
    key(&mut s, KeyCode::Tab);
    assert_eq!(update(&mut s, Msg::Paste(format!(" {g} "))), vec![Effect::InspectLink(g.into())]);
    let Phase::Onboarding(f) = &s.phase else { panic!() };
    assert_eq!(f.mode, FormMode::Link);
    // The bare command, or this computer's own request: nothing is checked.
    for (pasted, says) in
        [("lt --plain link-redeem --approver-fingerprint 2C41-88E0", "Copy for lt"), ("liguslink1:AAAABBBB", "this computer's request")]
    {
        let mut s = state();
        s.phase = Phase::Onboarding(WorkspaceForm::new(true));
        key(&mut s, KeyCode::Tab);
        key(&mut s, KeyCode::Char('a'));
        key(&mut s, KeyCode::Tab);
        assert!(update(&mut s, Msg::Paste(pasted.into())).is_empty());
        let Phase::Onboarding(f) = &s.phase else { panic!() };
        assert!(f.error.as_deref().unwrap().contains(says), "{:?}", f.error);
    }
}

fn pending_settings() -> Settings {
    // Started 14:02 UTC; the desktop may approve until 14:17.
    let at = 1_795_615_350_000;
    Settings {
        link_pending: Some(LinkPending { started_ms: at, approve_by_ms: at + 15 * 60_000, expires_ms: at + 45 * 60_000, count: 2 }),
        ..settings(None)
    }
}

/// A link started and not finished: one notice on start, and a line in
/// Settings until it is linked.
#[test]
fn a_waiting_link_shows_once_on_start_and_in_settings() {
    let mut s = main_state();
    s.now = 1_795_615_350_000 + 60_000;
    update(&mut s, Msg::Settings(Ok(pending_settings())));
    let want = "Linking waiting for your desktop's code (started 14:02; expires 14:17) · 2 requests waiting";
    assert_eq!(s.toasts[0].text, want);
    assert_eq!(s.link_pending_line().as_deref(), Some(want));
    s.toasts.clear();
    update(&mut s, Msg::Settings(Ok(pending_settings())));
    assert!(s.toasts.is_empty(), "once a run");
    // Past the approval window: the code the desktop showed still works.
    s.now += 20 * 60_000;
    assert_eq!(
        s.link_pending_line().as_deref(),
        Some("Linking waiting: the code your desktop showed works until 14:47 · 2 requests waiting")
    );
    // Linked: nothing waits.
    update(&mut s, Msg::Settings(Ok(Settings { linked_to: Some("zaz".into()), ..pending_settings() })));
    assert!(s.link_pending_line().is_none());
}
