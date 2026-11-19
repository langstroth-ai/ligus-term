//! The loop: terminal input, backend events and a clock tick all become
//! [`Msg`]s; each goes through [`update`]; the [`Effect`]s it returns run as
//! tasks against the backend and come back as more `Msg`s. Effects run
//! concurrently because some wait on a person (a passkey prompt).

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event as TermEvent, EventStream, KeyEventKind};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::app::{Effect, Msg, State, update};
use crate::backend::Backend;
use crate::backend::model::Millis;
use crate::theme::Theme;

pub fn now_ms() -> Millis {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as Millis).unwrap_or(0)
}

/// Make one backend call and turn its answer into a [`Msg`].
pub async fn run_effect(b: Arc<dyn Backend>, fx: Effect) -> Msg {
    use Effect::*;
    let done = |ok: &str, r| Msg::Done { ok: ok.into(), result: r };
    match fx {
        Startup => Msg::Startup(b.startup().await),
        LoadWorkspaces => Msg::Workspaces(b.workspaces().await),
        LoadChats(w) => Msg::Chats(w.clone(), b.chats(&w).await),
        LoadTranscript(w, c) => Msg::Transcript(w.clone(), c.clone(), b.transcript(&w, &c).await),
        LoadMentionables(w) => Msg::Mentionables(w.clone(), b.mentionables(&w).await),
        LoadProposals(w) => Msg::Proposals(w.clone(), b.proposals(&w).await),
        LoadDiff(w, p) => Msg::Diff(w.clone(), p.clone(), b.proposal_diff(&w, &p).await),
        LoadAgents(w) => Msg::Agents(w.clone(), b.agents(&w).await),
        LoadRuntimes(w) => Msg::Runtimes(w.clone(), b.runtimes(&w).await),
        LoadHosts(w) => Msg::Hosts(w.clone(), b.hosts(&w).await),
        LoadMembers(w) => Msg::Members(w.clone(), b.members(&w).await),
        LoadNotifications => Msg::Notifications(b.notifications().await),
        LoadSettings => Msg::Settings(b.settings().await),
        CreateWorkspace { your_name, name } => Msg::Created(b.create_workspace(&your_name, &name).await),
        JoinWorkspace { your_name, invite } => Msg::Joined(b.join_workspace(&your_name, &invite).await),
        ConfirmRecovery(w) => done("", b.confirm_recovery_saved(&w).await),
        CreateChat(w, t) => Msg::ChatCreated(w.clone(), b.create_chat(&w, &t).await),
        Send { workspace, chat, text } => Msg::Sent(b.send_message(&workspace, &chat, &text).await),
        MarkRead(w, c) => done("", b.mark_read(&w, &c).await),
        LockChat(w, c) => done("Chat locked down", b.lock_chat(&w, &c).await),
        ClearLockdown(w, c) => done("Lockdown cleared", b.clear_lockdown(&w, &c).await),
        Vote(w, p, choice) => Msg::ProposalUpdated(w.clone(), b.vote(&w, &p, choice).await),
        RequestApply(w, p, h) => Msg::ProposalUpdated(w.clone(), b.request_apply(&w, &p, &h).await),
        WithdrawApply(w, p) => Msg::ProposalUpdated(w.clone(), b.withdraw_apply(&w, &p).await),
        SetAgentRuntime(w, a, r) => done("Runtime changed", b.set_agent_runtime(&w, &a, &r).await),
        SetRole(w, m, r) => done("Role changed", b.set_role(&w, &m, r).await),
        RemoveMember(w, m) => done("Removed", b.remove_member(&w, &m).await),
        CreateInvite(w, r) => Msg::Invite(b.create_invite(&w, r).await),
        MarkNotificationsRead => done("", b.mark_notifications_read().await),
        SetSetting(k, on) => Msg::Settings(b.set_setting(k, on).await),
        AnswerPresence(id, a) => done("", b.answer_presence(id, a).await),
        InspectInvite(i) => Msg::InviteChecked(i.clone(), b.inspect_invite(&i).await),
        ConnectDeployment { your_name, spec } => Msg::Created(b.connect_deployment(&your_name, spec).await),
        AdmitBox(w, r) => Msg::Admitted(b.admit_box(&w, r.as_deref()).await),
        LoadWorkflows(w) => Msg::Workflows(w.clone(), b.workflows(&w).await),
        LoadRuns(w) => Msg::Runs(w.clone(), b.runs(&w).await),
        StartRun(w, wf, input) => Msg::RunUpdated(w.clone(), b.start_run(&w, &wf, &input).await),
        CancelRun(w, r) => Msg::RunUpdated(w.clone(), b.cancel_run(&w, &r).await),
        DecideGate(w, r, stage, approve) => Msg::RunUpdated(w.clone(), b.decide_gate(&w, &r, &stage, approve).await),
        LoadMemory(w) => Msg::Memory(w.clone(), b.memory(&w).await),
        LoadTasks(w) => Msg::Tasks(w.clone(), b.task_state(&w).await),
        LoadHandoffs(w) => Msg::Handoffs(w.clone(), b.handoffs(&w).await),
        LoadCheckout(w) => Msg::Checkout(w.clone(), b.checkout(&w).await),
        SetCheckout(w, p) => Msg::Checkout(w.clone(), b.set_checkout(&w, p.as_deref()).await),
    }
}

/// Run the full-screen app until the person quits.
pub async fn run_tui(backend: Arc<dyn Backend>, mut state: State, theme: Theme) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let mut stdout = std::io::stdout();
    let _ = crossterm::execute!(stdout, crossterm::event::EnableBracketedPaste);
    // Shift-Enter is only distinguishable with the kitty keyboard protocol.
    let enhanced = matches!(crossterm::terminal::supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        let _ = crossterm::execute!(
            stdout,
            crossterm::event::PushKeyboardEnhancementFlags(crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    let result = event_loop(&mut terminal, backend, &mut state, theme).await;

    if enhanced {
        let _ = crossterm::execute!(stdout, crossterm::event::PopKeyboardEnhancementFlags);
    }
    let _ = crossterm::execute!(stdout, crossterm::event::DisableBracketedPaste);
    ratatui::restore();
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    backend: Arc<dyn Backend>,
    state: &mut State,
    theme: Theme,
) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let mut tasks = tokio::task::JoinSet::new();

    let t = tx.clone();
    tasks.spawn(async move {
        let mut input = EventStream::new();
        while let Some(Ok(ev)) = input.next().await {
            let msg = match ev {
                TermEvent::Key(k) if k.kind != KeyEventKind::Release => Msg::Key(k),
                TermEvent::Paste(p) => Msg::Paste(p),
                TermEvent::Resize(..) => Msg::Tick(now_ms()),
                _ => continue,
            };
            if t.send(msg).is_err() {
                break;
            }
        }
    });
    let t = tx.clone();
    let mut events = backend.events();
    tasks.spawn(async move {
        while let Some(e) = events.next().await {
            if t.send(Msg::Event(e)).is_err() {
                break;
            }
        }
    });
    let t = tx.clone();
    tasks.spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            tick.tick().await;
            if t.send(Msg::Tick(now_ms())).is_err() {
                break;
            }
        }
    });

    let spawn = |tasks: &mut tokio::task::JoinSet<()>, fx: Effect| {
        let (b, t) = (backend.clone(), tx.clone());
        tasks.spawn(async move {
            let _ = t.send(run_effect(b, fx).await);
        });
    };
    spawn(&mut tasks, Effect::Startup);

    terminal.draw(|f| crate::ui::draw(f, state, &theme))?;
    while let Some(msg) = rx.recv().await {
        for fx in update(state, msg) {
            spawn(&mut tasks, fx);
        }
        // Drain whatever else is queued before drawing once.
        while let Ok(msg) = rx.try_recv() {
            for fx in update(state, msg) {
                spawn(&mut tasks, fx);
            }
        }
        if state.quit {
            break;
        }
        if std::mem::take(&mut state.bell) {
            use std::io::Write;
            let _ = std::io::stdout().write_all(b"\x07");
        }
        terminal.draw(|f| crate::ui::draw(f, state, &theme))?;
    }
    tasks.abort_all();
    Ok(())
}
