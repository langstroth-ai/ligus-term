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
    update(&mut s, Msg::Hosts(w.clone(), Ok(vec![host("here", false, true, true), host("sff", true, true, false)])));
    assert_eq!(key(&mut s, KeyCode::Char('A')), vec![Effect::Vote(w.clone(), ProposalId::new("p1"), VoteChoice::Approve)]);
    // Applying an open proposal is refused locally with the reason.
    assert!(key(&mut s, KeyCode::Char('O')).is_empty());
    assert!(s.toasts.last().unwrap().text.contains("Only an approved proposal"));
    // Approved: the host picker opens on the online worker.
    key(&mut s, KeyCode::Down);
    key(&mut s, KeyCode::Char('O'));
    assert_eq!(s.overlay, Some(Overlay::HostPicker { proposal: ProposalId::new("p2"), sel: 1 }));
    assert_eq!(key(&mut s, KeyCode::Enter), vec![Effect::RequestApply(w.clone(), ProposalId::new("p2"), HostId::new("sff"))]);
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
    let check = JoinRequestCheck { fingerprint: "AAAA-BBBB".into(), label: Some("sff".into()) };
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
    assert_eq!(s.agent_rows(), 2);
    assert!(s.selected_local().is_none());
    // `N` on an agent row says where to look.
    assert!(key(&mut s, KeyCode::Char('N')).is_empty());
    assert!(s.overlay.is_none());
    key(&mut s, KeyCode::Down);
    assert_eq!(s.selected_local().map(|l| l.id.as_str()), Some("local:codex"));
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
    assert!(s.overlay.is_none());
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
    assert!(all[2].starts_with("Proposals"), "the focused view's keys come first: {all:?}");
    assert!(all[2].contains("here"), "{all:?}");
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
