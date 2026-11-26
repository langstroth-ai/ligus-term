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
        LoadAsks(w, c) => Msg::Asks(w.clone(), c.clone(), b.asks(&w, &c).await),
        AnswerAsk(w, c, a, reply) => done("Answer sent", b.answer_ask(&w, &c, &a, reply).await),
        LoadMentionables(w) => Msg::Mentionables(w.clone(), b.mentionables(&w).await),
        LoadProposals(w) => Msg::Proposals(w.clone(), b.proposals(&w).await),
        LoadDiff(w, p) => Msg::Diff(w.clone(), p.clone(), b.proposal_diff(&w, &p).await),
        PreviewDiff(w, p) => {
            tokio::time::sleep(crate::app::update::DIFF_PREVIEW_DELAY).await;
            Msg::PreviewDue(w, p)
        }
        LoadAgents(w) => Msg::Agents(w.clone(), b.agents(&w).await),
        LoadRuntimes(w) => Msg::Runtimes(w.clone(), b.runtimes(&w).await),
        LoadLocalRuntimes(w) => Msg::LocalRuntimes(w.clone(), b.local_runtimes(&w).await),
        AddLocalAgent(w, r, name) => Msg::AgentAdded(w.clone(), b.add_local_agent(&w, &r, &name).await),
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
        ArchiveProposal(w, p, on) => Msg::Shelved(w.clone(), b.archive_proposal(&w, &p, on).await),
        HideProposal(w, p, on) => Msg::Shelved(w.clone(), b.hide_proposal(&w, &p, on).await),
        SetAgentRuntime(w, a, r) => done("Runtime changed", b.set_agent_runtime(&w, &a, &r).await),
        SetAgentAccess(w, a, everyone) => done(
            if everyone { "Everyone in the workspace can use it now" } else { "Only you can use it now" },
            b.set_agent_access(&w, &a, everyone).await,
        ),
        SetRole(w, m, r) => done("Role changed", b.set_role(&w, &m, r).await),
        RemoveMember(w, m) => done("Removed", b.remove_member(&w, &m).await),
        CreateInvite(w, r) => Msg::Invite(b.create_invite(&w, r).await),
        MarkNotificationsRead => done("", b.mark_notifications_read().await),
        SetSetting(k, on) => Msg::Settings(b.set_setting(k, on).await),
        AnswerPresence(id, a) => done("", b.answer_presence(id, a).await),
        InspectInvite(i) => Msg::InviteChecked(i.clone(), b.inspect_invite(&i).await),
        LinkRequest => Msg::LinkRequested(b.link_request().await),
        InspectLink(g) => Msg::LinkChecked(g.clone(), b.inspect_link_grant(&g).await),
        RedeemLink(g) => Msg::Linked(b.redeem_link(&g).await),
        LinkLive(a) => Msg::LiveStarted(b.link_live(&a).await),
        LinkLiveCode(rid, yes) => Msg::LiveAnswered(b.link_live_code(&rid, yes).await),
        LinkLivePerson(rid, yes) => Msg::LivePerson(b.link_live_person(&rid, yes).await),
        LinkLiveCancel(rid) => Msg::LiveAnswered(b.link_live_cancel(&rid).await),
        CancelRedeem => {
            b.cancel_link_redeem();
            done("", Ok(()))
        }
        ConfirmLink => Msg::LinkConfirmed(b.confirm_link().await),
        LoadApprovals(w) => Msg::Approvals(w.clone(), b.approvals(&w).await),
        ConnectDeployment { your_name, spec } => Msg::Created(b.connect_deployment(&your_name, spec).await),
        InspectJoinRequest(r) => Msg::JoinRequestChecked(r.clone(), b.inspect_join_request(&r).await),
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
        LoadRepoChoices(w) => Msg::RepoChoices(w.clone(), b.repo_choices(&w).await),
        SetChatRepo(w, c, r) => {
            let ok = if r.is_some() { "Repository set: this chat's agents work in it" } else { "This chat has no repository now" };
            done(ok, b.set_chat_repo(&w, &c, r.as_deref()).await)
        }
        NewScratchRepo(w, c) => Msg::ScratchMade(w.clone(), b.new_scratch_repo(&w, &c).await),
        KeepScratch(w, c, dest) => Msg::ScratchKept(w.clone(), b.keep_scratch_repo(&w, &c, &dest).await),
        ShowText(_) => Msg::Shown(Err(crate::backend::BackendError("the pager runs only in the full-screen app".into()))),
        StopTurn(w, c) => Msg::Stopped(w.clone(), c.clone(), b.stop_turn(&w, &c).await),
        // The full-screen loop runs the editor itself (it owns the
        // terminal); anywhere else there is no terminal to lend it.
        EditDraft(_) => Msg::Edited(Err(crate::backend::BackendError("the editor runs only in the full-screen app".into()))),
        LoadSetup(w) => Msg::SetupRows(b.setup_status(w.as_ref()).await),
        SetupForm(w, flow, inputs) => Msg::SetupForm(flow, b.setup_form(w.as_ref(), flow, inputs).await),
        SetupRun { workspace, flow, inputs, secrets, dry_run, cancel } => {
            Msg::SetupRan(flow, b.setup_run(workspace.as_ref(), flow, inputs, secrets, dry_run, cancel).await)
        }
        CancelSetup(c) => {
            c.cancel();
            Msg::Done { ok: String::new(), result: Ok(()) }
        }
        // No terminal here: only the platform's tool.
        Copy(c) => {
            let text = c.text.clone();
            let native = tokio::task::spawn_blocking(move || crate::clipboard::copy_native(&text)).await.ok().flatten();
            Msg::Copied(crate::clipboard::outcome(native, false, c.fallback))
        }
        CompletePath(field, text) => {
            // A directory listing: off the async threads.
            let home = dirs::home_dir();
            let c = tokio::task::spawn_blocking(move || crate::setup::complete::complete(&text, home.as_deref()))
                .await
                .unwrap_or_else(|_| crate::setup::complete::Completion { text: String::new(), candidates: vec![] });
            Msg::PathCompleted(field, c)
        }
    }
}

/// What Settings and the palette show of the theme session.
pub fn show_theme(state: &mut State, session: &crate::theme::Session, shown: &Theme) {
    state.theme_name = session.theme_label(shown);
    state.theme_id.clone_from(&session.theme);
    state.appearance = session.appearance;
    state.appearance_name = session.appearance_label();
    state.themes = session.list();
}

/// A desktop notification as the terminal's OSC 9 (inside tmux, through
/// its passthrough). The text is cut to 200 characters and loses any
/// control character, so it can't end the sequence early.
pub fn osc9(text: &str, tmux: bool) -> Vec<u8> {
    let clean: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
    let seq = format!("\x1b]9;{clean}\x07");
    if tmux { format!("\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b")).into_bytes() } else { seq.into_bytes() }
}

/// Read terminal input into messages until aborted.
fn spawn_input(t: mpsc::UnboundedSender<Msg>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut input = EventStream::new();
        while let Some(Ok(ev)) = input.next().await {
            let msg = match ev {
                TermEvent::Key(k) if k.kind != KeyEventKind::Release => Msg::Key(k),
                TermEvent::Paste(p) => Msg::Paste(p),
                TermEvent::Mouse(m) => Msg::Mouse(m),
                TermEvent::FocusGained => Msg::Focus(true),
                TermEvent::FocusLost => Msg::Focus(false),
                TermEvent::Resize(w, h) => Msg::Resize(w, h),
                _ => continue,
            };
            if t.send(msg).is_err() {
                break;
            }
        }
    })
}

/// The editor to run: `$VISUAL`, else `$EDITOR`, else `vi`, split on
/// spaces so `code --wait` works.
pub fn editor_command(env: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let cmd = env("VISUAL").filter(|v| !v.trim().is_empty()).or_else(|| env("EDITOR").filter(|v| !v.trim().is_empty()));
    cmd.unwrap_or_else(|| "vi".into()).split_whitespace().map(String::from).collect()
}

/// Leave the screen, show `text` in `$PAGER` (or print it, so it stays in
/// the terminal's scrollback), and take the screen back. `Ok(true)` when a
/// pager showed it.
fn show_text(terminal: &mut ratatui::DefaultTerminal, text: &str) -> Result<bool, String> {
    use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
    use std::io::Write;
    let mouse = MOUSE.load(std::sync::atomic::Ordering::Relaxed);
    let mut stdout = std::io::stdout();
    if mouse {
        let _ = crossterm::execute!(stdout, crossterm::event::DisableMouseCapture);
    }
    let _ = crossterm::execute!(stdout, crossterm::event::DisableBracketedPaste, LeaveAlternateScreen, crossterm::cursor::Show);
    let _ = disable_raw_mode();
    let pager: Vec<String> = std::env::var("PAGER")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .map(|p| p.split_whitespace().map(String::from).collect())
        .unwrap_or_default();
    let result = if pager.is_empty() {
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
        Ok(false)
    } else {
        match std::process::Command::new(&pager[0]).args(&pager[1..]).stdin(std::process::Stdio::piped()).spawn() {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait().map(|_| true).map_err(|e| e.to_string())
            }
            Err(e) => Err(format!("{}: {e} (check $PAGER)", pager[0])),
        }
    };
    let _ = enable_raw_mode();
    let _ = crossterm::execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    );
    if mouse {
        let _ = crossterm::execute!(stdout, crossterm::event::EnableMouseCapture);
    }
    terminal.swap_buffers();
    terminal.swap_buffers();
    result
}

/// Write the draft to a private temporary file (0600, a fresh name), run
/// the editor on it with the terminal restored, read it back and delete it.
fn suspend_for_editor(terminal: &mut ratatui::DefaultTerminal, text: &str) -> Result<String, String> {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!("lt-draft-{}.md", uuid::Uuid::new_v4()));
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    let mut file = open.open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    drop(file);
    // Hand the terminal over as it was before lt started, and take it back
    // the same way (the same Terminal, so nothing queries the cursor).
    use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
    let mut stdout = std::io::stdout();
    let mouse = MOUSE.load(std::sync::atomic::Ordering::Relaxed);
    if mouse {
        let _ = crossterm::execute!(stdout, crossterm::event::DisableMouseCapture);
    }
    let _ = crossterm::execute!(stdout, crossterm::event::DisableBracketedPaste, LeaveAlternateScreen, crossterm::cursor::Show);
    let _ = disable_raw_mode();
    let cmd = editor_command(|k| std::env::var(k).ok());
    let status = std::process::Command::new(&cmd[0]).args(&cmd[1..]).arg(&path).status();
    let _ = enable_raw_mode();
    let _ = crossterm::execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    );
    if mouse {
        let _ = crossterm::execute!(stdout, crossterm::event::EnableMouseCapture);
    }
    // Both buffers blank, so the next draw repaints everything (ratatui's
    // own clear() would ask the terminal for the cursor first).
    terminal.swap_buffers();
    terminal.swap_buffers();
    let out = match status {
        Ok(st) if st.success() => std::fs::read_to_string(&path).map_err(|e| e.to_string()),
        Ok(st) => Err(format!("{} exited with {st}; the draft is unchanged", cmd[0])),
        Err(e) => Err(format!("{}: {e} (set $VISUAL or $EDITOR)", cmd[0])),
    };
    let _ = std::fs::remove_file(&path);
    out
}

/// Run the full-screen app until the person quits.
pub async fn run_tui(backend: Arc<dyn Backend>, mut state: State, session: crate::theme::Session, mouse: bool) -> anyhow::Result<State> {
    let mut terminal = ratatui::init();
    let mut stdout = std::io::stdout();
    let _ = crossterm::execute!(stdout, crossterm::event::EnableBracketedPaste);
    MOUSE.store(mouse, std::sync::atomic::Ordering::Relaxed);
    // Focus reports let a needs-you signal reach you only while you're away.
    let _ = crossterm::execute!(stdout, crossterm::event::EnableFocusChange);
    if mouse {
        let _ = crossterm::execute!(stdout, crossterm::event::EnableMouseCapture);
    }
    // Shift-Enter is only distinguishable with the kitty keyboard protocol.
    let enhanced = matches!(crossterm::terminal::supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        let _ = crossterm::execute!(
            stdout,
            crossterm::event::PushKeyboardEnhancementFlags(crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }

    let result = event_loop(&mut terminal, backend, &mut state, session).await;

    if enhanced {
        let _ = crossterm::execute!(stdout, crossterm::event::PopKeyboardEnhancementFlags);
    }
    if mouse {
        let _ = crossterm::execute!(stdout, crossterm::event::DisableMouseCapture);
    }
    let _ = crossterm::execute!(stdout, crossterm::event::DisableFocusChange, crossterm::event::DisableBracketedPaste);
    ratatui::restore();
    result.map(|()| state)
}

/// Whether the app captured the mouse (so the editor hand-over gives it
/// back and takes it again).
static MOUSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    backend: Arc<dyn Backend>,
    state: &mut State,
    mut session: crate::theme::Session,
) -> anyhow::Result<()> {
    let mut theme = session.current();
    show_theme(state, &session, &theme);
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let mut tasks = tokio::task::JoinSet::new();

    let mut input = spawn_input(tx.clone());
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
    if let Ok(size) = terminal.size() {
        update(state, Msg::Resize(size.width, size.height));
    }

    terminal.draw(|f| crate::ui::draw(f, state, &theme))?;
    while let Some(msg) = rx.recv().await {
        let mut edit = None;
        let mut page = None;
        let run = |tasks: &mut tokio::task::JoinSet<()>, fx: Effect, edit: &mut Option<String>, page: &mut Option<String>| match fx {
            Effect::EditDraft(text) => *edit = Some(text),
            Effect::ShowText(text) => *page = Some(text),
            // The terminal's clipboard (OSC 52) from here, then the
            // platform's tool.
            Effect::Copy(c) => {
                use std::io::Write;
                let mut out = std::io::stdout();
                let sent =
                    out.write_all(&crate::clipboard::osc52(&c.text, std::env::var_os("TMUX").is_some())).is_ok() && out.flush().is_ok();
                let t = tx.clone();
                tasks.spawn(async move {
                    let text = c.text.clone();
                    let native = tokio::task::spawn_blocking(move || crate::clipboard::copy_native(&text)).await.ok().flatten();
                    let _ = t.send(Msg::Copied(crate::clipboard::outcome(native, sent, c.fallback)));
                });
            }
            fx => spawn(tasks, fx),
        };
        for fx in update(state, msg) {
            run(&mut tasks, fx, &mut edit, &mut page);
        }
        // Drain whatever else is queued before drawing once.
        while let Ok(msg) = rx.try_recv() {
            for fx in update(state, msg) {
                run(&mut tasks, fx, &mut edit, &mut page);
            }
        }
        if state.quit {
            break;
        }
        // The editor gets the terminal: stop reading keys, leave the
        // alternate screen, wait for it, and take the screen back.
        if let Some(text) = page {
            input.abort();
            let _ = input.await;
            let result = show_text(terminal, &text);
            input = spawn_input(tx.clone());
            let _ = tx.send(Msg::Shown(result.map_err(crate::backend::BackendError)));
        }
        if let Some(text) = edit {
            input.abort();
            let _ = input.await;
            let result = suspend_for_editor(terminal, &text);
            input = spawn_input(tx.clone());
            let _ = tx.send(Msg::Edited(result.map_err(crate::backend::BackendError)));
        }
        if let Some(req) = state.theme_request.take() {
            use crate::theme::ThemeRequest;
            match req {
                ThemeRequest::Set { theme: t, appearance: a } => {
                    if let Some(t) = &t {
                        session.theme.clone_from(t);
                    }
                    if let Some(a) = a {
                        session.appearance = a;
                    }
                    theme = session.current();
                    show_theme(state, &session, &theme);
                    update(state, if t.is_some() { Msg::ThemeChanged } else { Msg::AppearanceChanged });
                }
                ThemeRequest::Preview { theme: t, appearance: a } => {
                    theme = session.build(t.as_deref().unwrap_or(&session.theme), a.unwrap_or(session.appearance));
                }
                ThemeRequest::Revert => theme = session.current(),
            }
        }
        if std::mem::take(&mut state.bell) {
            use std::io::Write;
            let _ = std::io::stdout().write_all(b"\x07");
        }
        let signals = std::mem::take(&mut state.signals);
        if !signals.is_empty() && !state.focused {
            use std::io::Write;
            let text = if signals.len() == 1 { signals[0].clone() } else { format!("{} things need you in lt", signals.len()) };
            let mut out = std::io::stdout();
            let _ = out.write_all(&osc9(&text, std::env::var_os("TMUX").is_some()));
            let _ = out.flush();
        }
        terminal.draw(|f| crate::ui::draw(f, state, &theme))?;
    }
    tasks.abort_all();
    Ok(())
}
