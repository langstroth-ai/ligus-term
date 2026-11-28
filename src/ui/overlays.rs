//! Everything drawn over the screen: help, the workspace switcher, pickers,
//! confirmations, invites, and the passkey sheet (always on top).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::{center, onboarding, pane};
use unicode_width::UnicodeWidthStr;

use crate::app::state::{MenuItem, MenuSection, Overlay, PresenceSheet, PresenceStage, State, View};
use crate::backend::model::{Allowed, PresenceOp, RepoChoices, Role, initials};
use crate::keymap::{BINDINGS, Context};
use crate::theme::Theme;

pub fn draw(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    match &s.overlay {
        None => {}
        Some(Overlay::Help { scroll, .. }) => help(f, area, s, *scroll, t),
        Some(Overlay::Setup { sel }) => super::setup::list(f, area, s, *sel, t),
        Some(Overlay::SetupFlow(w)) => super::setup::flow(f, area, s, w, t),
        Some(Overlay::Messages { scroll }) => messages(f, area, s, *scroll, t),
        Some(Overlay::Palette { query, sel }) => super::palette::draw(f, area, s, query, *sel, t),
        Some(Overlay::History { query, sel }) => history(f, area, s, query, *sel, t),
        Some(Overlay::Switcher { sel }) => switcher(f, area, s, *sel, t),
        Some(Overlay::AddWorkspace(form)) => onboarding::draw_form(f, area, s, form, t, false),
        Some(Overlay::StartRun { name, label, input, .. }) => {
            let r = center(area, 64, 6);
            f.render_widget(Clear, r);
            let title = format!("Start “{name}”");
            let block = pane(&title, true, t);
            let inner = block.inner(r);
            f.render_widget(block, r);
            let ask = label.clone().unwrap_or_else(|| "Input (optional)".into());
            let lines = vec![
                Line::styled(ask, t.dim()),
                Line::styled(input.text.clone(), t.text()),
                Line::styled("enter start · esc cancel · this device drives the run", t.dim()),
            ];
            f.render_widget(Paragraph::new(lines), inner);
            f.set_cursor_position(Position::new(inner.x + input.cursor_pos().1 as u16, inner.y + 1));
        }
        Some(Overlay::AskReply { agent, text, .. }) => {
            let r = center(area, 72, 7);
            f.render_widget(Clear, r);
            let title = format!("Reply to {agent}");
            let block = pane(&title, true, t);
            let inner = block.inner(r);
            f.render_widget(block, r);
            // The reply scrolls sideways in one line; up to 1000 characters.
            let w = inner.width as usize;
            let (_, col) = text.cursor_pos();
            let skip = col.saturating_sub(w.saturating_sub(1));
            let shown: String = text.text.chars().skip(skip).take(w).collect();
            let lines = vec![
                Line::styled(format!("{agent} reads it and decides what to do next. It grants no new permission."), t.dim()),
                Line::styled(shown, t.text()),
                Line::raw(""),
                Line::styled("enter send · esc cancel", t.dim()),
            ];
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
            f.set_cursor_position(Position::new(inner.x + (col - skip) as u16, inner.y + 1));
        }
        Some(Overlay::BindCheckout(buf)) => {
            let r = center(area, 72, 7);
            f.render_widget(Clear, r);
            let block = pane("This device's checkout", true, t);
            let inner = block.inner(r);
            f.render_widget(block, r);
            let lines = vec![
                Line::styled("The git checkout approved proposals are applied in when a host asks this device.", t.dim()),
                Line::styled(buf.text.clone(), t.text()),
                Line::raw(""),
                Line::styled("enter save · empty to unbind · esc cancel", t.dim()),
            ];
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
            f.set_cursor_position(Position::new(inner.x + buf.cursor_pos().1 as u16, inner.y + 1));
        }
        Some(Overlay::RepoPicker { chat, sel, choices }) => repo_picker(f, area, s, chat, *sel, choices.as_ref(), t),
        Some(Overlay::KeepScratch { name, path, .. }) => {
            let r = center(area, 72, 9);
            f.render_widget(Clear, r);
            let block = pane("Keep it as a repo", true, t);
            let inner = block.inner(r);
            f.render_widget(block, r);
            let lines = vec![
                Line::styled(format!("Moves {name} out of the scratch folder; it stays in the workspace as an ordinary repo."), t.dim()),
                Line::styled(path.text.clone(), t.text()),
                Line::raw(""),
                Line::styled("enter keep · esc cancel", t.dim()),
                Line::styled("To publish it to GitHub (private), use the desktop or the phone.", t.dim()),
            ];
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
            let row = inner.y
                + wrapped_rows(
                    &format!("Moves {name} out of the scratch folder; it stays in the workspace as an ordinary repo."),
                    inner.width,
                );
            f.set_cursor_position(Position::new(inner.x + path.cursor_pos().1 as u16, row));
        }
        Some(Overlay::AdmitBox { request, busy, checked, result }) => {
            let r = center(area, 84, 11);
            f.render_widget(Clear, r);
            let block = pane("Admit a box", true, t);
            let inner = block.inner(r);
            f.render_widget(block, r);
            let mut lines = vec![];
            match (result, checked) {
                (None, Some(c)) => {
                    let label = c.label.clone().map(|l| format!(" ({l})")).unwrap_or_default();
                    lines.push(Line::from(vec![
                        Span::styled(format!("Check the box{label} shows this fingerprint: "), t.text()),
                        Span::styled(c.fingerprint.clone(), t.accent()),
                    ]));
                    lines.push(Line::styled(
                        "Its terminal printed it with the join request. A different one means the request was swapped: don't admit it.",
                        t.dim(),
                    ));
                    lines.push(Line::raw(""));
                    if *busy {
                        lines.push(Line::styled("Admitting… confirm the passkey prompt", t.fg(t.warn)));
                    } else {
                        lines.push(Line::styled("y it matches, admit (needs your passkey) · c copy · esc back", t.dim()));
                    }
                }
                (None, None) => {
                    lines.push(Line::styled(
                        "A ligus serve or ligus worker box joins as its own service member (Contributor). Paste the join request it printed (ligusjoin1:…), or leave it empty for a code to put in a Kubernetes Secret or LIGUS_ADMISSION_FILE.",
                        t.text(),
                    ));
                    lines.push(Line::raw(""));
                    lines.push(Line::styled(
                        format!("› {}", super::trunc(&request.text, (inner.width as usize).saturating_sub(4))),
                        t.accent(),
                    ));
                    lines.push(Line::raw(""));
                    if *busy {
                        lines.push(Line::styled("Working… (a bearer code needs your passkey)", t.fg(t.warn)));
                    } else {
                        lines.push(Line::styled("enter check its fingerprint (empty: a bearer code) · esc cancel", t.dim()));
                    }
                }
                (Some(a), _) => {
                    if a.sealed {
                        let fp = a.fingerprint.clone().unwrap_or_default();
                        let label = a.label.clone().map(|l| format!(" ({l})")).unwrap_or_default();
                        lines.push(Line::from(vec![
                            Span::styled("Sealed to the box", t.fg(t.good)),
                            Span::styled(format!("{label}: check its fingerprint matches "), t.text()),
                            Span::styled(fp, t.accent()),
                        ]));
                        lines.push(Line::styled("On the box: ligus admit redeem <code>, then start it again.", t.dim()));
                    } else {
                        lines.push(Line::styled(
                            "A bearer code: it carries the workspace key. Hand it over privately and delete it once the box joined.",
                            t.fg(t.warn),
                        ));
                        lines.push(Line::styled(
                            "kubectl create secret generic <release>-serve-admission --from-file=admission=<file>, or LIGUS_ADMISSION_FILE.",
                            t.dim(),
                        ));
                    }
                    lines.push(Line::raw(""));
                    lines.push(Line::styled(a.code.clone(), t.text()));
                    lines.push(Line::raw(""));
                    lines.push(Line::styled("c copy · o full screen (to select it) · any other key closes", t.dim()));
                }
            }
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        Some(Overlay::NewChat(buf)) => {
            let r = center(area, 56, 5);
            f.render_widget(Clear, r);
            let block = pane("New chat", true, t);
            let inner = block.inner(r);
            f.render_widget(block, r);
            let lines =
                vec![Line::styled(buf.text.clone(), t.text()), Line::styled("enter create · esc cancel · empty for Untitled", t.dim())];
            f.render_widget(Paragraph::new(lines), inner);
            f.set_cursor_position(Position::new(inner.x + buf.cursor_pos().1 as u16, inner.y));
        }
        Some(Overlay::RowMenu { title, items, sel }) => row_menu(f, area, s, title, items, *sel, t),
        Some(Overlay::AddAgentPicker { sel }) => add_agent_picker(f, area, s, *sel, t),
        Some(Overlay::AddAgent { runtime, name }) => {
            let r = center(area, 72, 8);
            f.render_widget(Clear, r);
            let block = pane("Add an agent on this device", true, t);
            let inner = block.inner(r);
            f.render_widget(block, r);
            let what = match &runtime.model {
                Some(m) => format!("{} · {m}", runtime.kind.label()),
                None => runtime.kind.label().to_string(),
            };
            let state = if runtime.ready {
                Span::styled(format!("  ready · {}", runtime.detail), t.fg(t.good))
            } else {
                Span::styled(format!("  not ready · {}", runtime.detail), t.fg(t.warn))
            };
            let lines = vec![
                Line::from(vec![Span::styled("@", t.fg(t.agent)), Span::styled(name.text.clone(), t.text())]),
                Line::from(vec![Span::styled(what, t.text()), state]),
                Line::styled("Runs on this device. Only you can use it until you share it (E).", t.dim()),
                Line::raw(""),
                Line::styled("enter add · esc cancel", t.dim()),
            ];
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
            f.set_cursor_position(Position::new(inner.x + 1 + name.cursor_pos().1 as u16, inner.y));
        }
        Some(Overlay::HostPicker { sel, .. }) => {
            let items: Vec<ListItem> = s
                .hosts
                .iter()
                .map(|h| {
                    let state = if h.online { Span::styled(" (online)", t.fg(t.good)) } else { Span::styled(" (offline)", t.dim()) };
                    let me = if h.this_device { "  this device" } else { "" };
                    ListItem::new(Line::from(vec![
                        Span::styled(if h.this_device { "Apply here".to_string() } else { format!("Apply on {}", h.label) }, t.text()),
                        state,
                        Span::styled(me, t.dim()),
                    ]))
                })
                .collect();
            picker(f, area, "Apply on…", items, *sel, "", t);
        }
        Some(Overlay::RolePicker { member, sel }) => {
            let m = s.members.iter().find(|m| &m.id == member);
            let items: Vec<ListItem> = Role::ALL
                .iter()
                .map(|r| {
                    let allowed =
                        m.and_then(|m| m.can_set_role.iter().find(|(x, _)| x == r).map(|(_, a)| a.clone())).unwrap_or(Allowed::Yes);
                    match allowed {
                        Allowed::Yes => ListItem::new(Line::styled(r.label(), t.text())),
                        Allowed::No(why) => ListItem::new(Line::from(vec![
                            Span::styled(r.label(), t.disabled()),
                            Span::styled(format!("  {why}"), t.dim()),
                        ])),
                    }
                })
                .collect();
            let title = format!("Role for {}", m.map(|m| m.name.as_str()).unwrap_or("?"));
            picker(f, area, &title, items, *sel, "Needs your passkey.", t);
        }
        Some(Overlay::RuntimePicker { sel, agent }) => {
            let current = s.agents.iter().find(|a| &a.id == agent).map(|a| a.runtime.clone());
            let items: Vec<ListItem> = s
                .runtimes
                .iter()
                .filter(|r| r.configured)
                .map(|r| {
                    let model = r.model.clone().unwrap_or_else(|| "default".into());
                    let now = if current.as_ref() == Some(&r.id) { " · current" } else { "" };
                    ListItem::new(Line::from(vec![
                        Span::styled(r.id.0.clone(), t.text()),
                        Span::styled(format!("  {} · {model} · {}{now}", r.kind.label(), r.kind.place()), t.dim()),
                    ]))
                })
                .collect();
            let handle = s.agents.iter().find(|a| &a.id == agent).map(|a| a.handle.clone()).unwrap_or_default();
            picker(f, area, &format!("Runtime for @{handle}"), items, *sel, "", t);
        }
        Some(Overlay::ConfirmRemove { name, member }) => {
            let you = s.members.iter().any(|m| &m.id == member && m.you);
            let q = if you {
                "Leave this workspace? Its chats stop syncing on this device.".to_string()
            } else {
                format!("Remove {name} from the workspace?")
            };
            message(
                f,
                area,
                if you { "Leave" } else { "Remove member" },
                vec![
                    Line::styled(q, t.text()),
                    Line::raw(""),
                    Line::styled("y confirm (needs your passkey) · any other key cancels", t.dim()),
                ],
                t,
            );
        }
        Some(Overlay::Confirm { title, question, yes, .. }) => {
            message(
                f,
                area,
                title,
                vec![
                    Line::styled(question.clone(), t.text()),
                    Line::raw(""),
                    Line::from(vec![Span::styled("y", t.accent()), Span::styled(format!(" {yes} · any other key backs out"), t.dim())]),
                ],
                t,
            );
        }
        Some(Overlay::Invite { code }) => {
            let lines = match code {
                None => vec![Line::styled("Creating an invite…", t.dim())],
                Some(c) => vec![
                    Line::styled("Share this invite code. It joins as a contributor and works once.", t.text()),
                    Line::raw(""),
                    Line::styled(c.clone(), t.accent()),
                    Line::raw(""),
                    Line::styled("c copy · o full screen (to select it) · any other key closes", t.dim()),
                ],
            };
            message(f, area, "Invite", lines, t);
        }
        Some(Overlay::Approvals { list }) => approvals(f, area, s, list.as_ref(), t),
        Some(Overlay::LinkCode { view }) => link_code(f, area, s, view.as_ref(), t),
        Some(Overlay::Code { title, code, fingerprint, .. }) => {
            // Alone on the screen, no border in the code's lines: a mouse
            // selection takes only the code (a paste ignores the breaks).
            f.render_widget(Clear, area);
            let w = (area.width as usize).max(8);
            let mut lines = vec![Line::styled(title.clone(), t.accent()), Line::raw("")];
            let chars: Vec<char> = code.chars().collect();
            for piece in chars.chunks(w) {
                lines.push(Line::styled(piece.iter().collect::<String>(), t.text()));
            }
            lines.push(Line::raw(""));
            if let Some(fp) = fingerprint {
                lines.push(Line::from(vec![Span::styled("Fingerprint ", t.dim()), Span::styled(fp.clone(), t.accent())]));
                lines.push(Line::raw(""));
            }
            let select = if s.mouse { "shift-drag selects (lt has the mouse)" } else { "select it with the mouse" };
            lines.push(Line::styled(format!("c copy · {select} · any other key back"), t.dim()));
            f.render_widget(Paragraph::new(lines), area);
        }
    }
    if let Some(p) = s.presence.front() {
        presence(f, area, p, s.presence.len(), super::spinner(s, t), t);
    }
}

/// The workspace's live approval requests, read only: kind, label, age and
/// who says they are checking it. No code: only the requester and its
/// approver can compute one.
fn approvals(
    f: &mut Frame,
    area: Rect,
    s: &State,
    list: Option<&std::result::Result<Vec<crate::backend::model::ApprovalRequest>, String>>,
    t: &Theme,
) {
    let mut lines = vec![Line::styled("Approve on your desktop or phone.", t.text()), Line::raw("")];
    match list {
        None => lines.push(Line::styled("Reading the approval mailbox…", t.dim())),
        Some(Err(e)) => lines.push(Line::styled(format!("✗ Couldn't read them: {e}"), t.fg(t.bad))),
        Some(Ok(l)) if l.is_empty() => lines.push(Line::styled("No requests waiting.", t.dim())),
        Some(Ok(l)) => {
            for r in l {
                lines.push(Line::styled(r.statement(), t.text()));
                let ago = crate::app::state::ago_phrase(s.now, r.at);
                if r.contested {
                    let by = r.checking.as_ref().map(|n| format!(" · claimed by “{n}”")).unwrap_or_default();
                    lines.push(Line::styled(format!("  {ago}{by}"), t.dim()));
                    for l in super::wrap(&r.checking_line(), 58) {
                        lines.push(Line::styled(format!("  {l}"), t.fg(t.warn)));
                    }
                } else {
                    lines.push(Line::styled(format!("  {ago} · {}", r.checking_line()), t.dim()));
                }
            }
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("r refresh · esc close", t.dim()));
    message(f, area, "Approval requests", lines, t);
}

/// Link a device: the short code, how to type it, and where it stands.
fn link_code(
    f: &mut Frame,
    area: Rect,
    s: &State,
    view: Option<&std::result::Result<crate::backend::model::LinkCodeView, String>>,
    t: &Theme,
) {
    use crate::backend::model::LinkCodeState as St;
    let mut lines = Vec::new();
    let mut keys = "esc close";
    match view {
        None => lines.push(Line::styled("Getting a code…", t.dim())),
        Some(Err(e)) => lines.push(Line::styled(format!("✗ Couldn't get a code: {e}"), t.fg(t.bad))),
        Some(Ok(v)) => match &v.state {
            St::Live => {
                lines.push(Line::styled("On the new device, choose I already use Ligus and type:", t.text()));
                lines.push(Line::raw(""));
                lines.push(Line::styled(format!("   {}", v.code), t.accent().add_modifier(Modifier::BOLD)));
                lines.push(Line::raw(""));
                if let Some(r) = &v.relay {
                    lines.push(Line::styled(format!("with the relay {r}."), t.text()));
                }
                let left = ((v.expires_at - s.now) / 60_000).max(0) + 1;
                lines.push(Line::styled(
                    format!("It works once, for about {left} more minute{}.", if left == 1 { "" } else { "s" }),
                    t.dim(),
                ));
                let relay = v.relay.as_ref().map(|r| format!(" --relay {r}")).unwrap_or_default();
                lines.push(Line::styled(format!("In a terminal: ligus devices request --code {}{relay}", v.code), t.dim()));
                lines.push(Line::styled("Then approve its request on your desktop or phone.", t.dim()));
                if let Some(n) = &v.notice {
                    lines.push(Line::styled(n.clone(), t.fg(t.warn)));
                }
            }
            St::Used => {
                lines.push(Line::styled(format!("✓ The new device has the code {}.", v.code), t.text()));
                lines.push(Line::styled("Approve its request in your desktop's Inbox (or on your phone).", t.dim()));
            }
            St::Expired => {
                lines.push(Line::styled(format!("The code {} expired.", v.code), t.dim()));
                keys = "n new code · esc close";
            }
            St::NoRoutes { address } => {
                lines.push(Line::styled("This relay has no short codes yet.", t.text()));
                if let Some(a) = address {
                    lines.push(Line::styled("Paste this approval address on the new device instead:", t.dim()));
                    lines.push(Line::styled(a.clone(), t.text()));
                } else {
                    lines.push(Line::styled("Use Add to my devices on the new device and paste its request on your desktop.", t.dim()));
                }
            }
            St::Failed(e) => lines.push(Line::styled(format!("✗ {e}"), t.fg(t.bad))),
        },
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(keys, t.dim()));
    message(f, area, "Link a device", lines, t);
}

fn message(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line>, t: &Theme) {
    let w = 64.min(area.width);
    let rows: usize = lines.iter().map(|l| super::wrap(&l.to_string(), w.saturating_sub(2) as usize).len()).sum();
    let r = center(area, w, rows as u16 + 2);
    f.render_widget(Clear, r);
    let block = pane(title, true, t);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), r);
}

/// Rows a line of `text` takes wrapped at `width` (at least one).
fn wrapped_rows(text: &str, width: u16) -> u16 {
    super::wrap(text, width as usize).len().max(1) as u16
}

/// The small "scratch" tag after a repository's name.
pub(crate) fn scratch_tag(t: &Theme) -> Span<'static> {
    Span::styled(" scratch", t.fg(t.accent))
}

/// The chat repo picker: "New scratch repo" first, then the workspace's
/// repositories (scratch ones tagged), then "No repository".
fn repo_picker(
    f: &mut Frame,
    area: Rect,
    s: &State,
    chat: &crate::backend::model::ChatId,
    sel: usize,
    choices: Option<&RepoChoices>,
    t: &Theme,
) {
    let current = s.chats.iter().find(|c| &c.id == chat).and_then(|c| c.repo.as_ref()).map(|r| r.id.clone());
    let now = |on: bool| Span::styled(if on { "  current" } else { "" }, t.dim());
    let mut items = vec![match choices.map(|c| &c.new_scratch) {
        Some(Allowed::No(why)) => {
            ListItem::new(Line::from(vec![Span::styled("New scratch repo", t.disabled()), Span::styled(format!("  {why}"), t.dim())]))
        }
        _ => ListItem::new(Line::from(vec![
            Span::styled("New scratch repo", t.text()),
            Span::styled("  an empty repo on this device", t.dim()),
        ])),
    }];
    for r in choices.map(|c| c.repos.as_slice()).unwrap_or_default() {
        let mut spans = vec![Span::styled(r.name.clone(), t.text())];
        if r.scratch {
            spans.push(scratch_tag(t));
        }
        if !r.here {
            spans.push(Span::styled("  not on this device", t.dim()));
        }
        spans.push(now(current.as_deref() == Some(r.id.as_str())));
        items.push(ListItem::new(Line::from(spans)));
    }
    items.push(ListItem::new(Line::from(vec![Span::styled("No repository", t.text()), now(current.is_none())])));
    let note = if choices.is_none() { "Loading the workspace's repositories…" } else { "This chat's agents work in it on this device." };
    picker(f, area, "Repository for this chat", items, sel, note, t);
}

fn picker(f: &mut Frame, area: Rect, title: &str, items: Vec<ListItem>, sel: usize, note: &str, t: &Theme) {
    let n = items.len() as u16;
    let r = center(area, 64, n + if note.is_empty() { 4 } else { 5 });
    f.render_widget(Clear, r);
    let block = pane(title, true, t);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let list_area = Rect { height: n.min(inner.height), ..inner };
    let mut st = ListState::default().with_selected(Some(sel));
    f.render_stateful_widget(List::new(items).highlight_style(t.selected()).highlight_symbol("› "), list_area, &mut st);
    let note_area = Rect { y: inner.y + n + 1, height: inner.height.saturating_sub(n + 1), ..inner };
    let mut lines = vec![];
    if !note.is_empty() {
        lines.push(Line::styled(note, t.dim()));
    }
    lines.push(Line::styled("enter choose · esc cancel", t.dim()));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), note_area);
}

/// The row menu: the row's actions, the view's, then the destructive ones,
/// a hairline between the groups. Each entry shows its own key; a refused
/// one is dimmed with the reason.
fn row_menu(f: &mut Frame, area: Rect, s: &State, title: &str, items: &[MenuItem], sel: usize, t: &Theme) {
    let w = 60.min(area.width);
    let inner_w = w.saturating_sub(2) as usize;
    let keys: Vec<String> = items.iter().map(|it| s.keymap.hint(it.context, it.action)).collect();
    let key_w = keys.iter().map(|k| k.width()).max().unwrap_or(1);
    let mut lines: Vec<Line> = vec![];
    for (i, (it, key)) in items.iter().zip(&keys).enumerate() {
        if i > 0 && items[i - 1].section != it.section {
            lines.push(Line::styled("─".repeat(inner_w), t.dim()));
        }
        let mark = if i == sel { "› " } else { "  " };
        let style = |base: ratatui::style::Style| if i == sel { t.selected() } else { base };
        let mut spans = vec![Span::styled(mark, t.accent()), Span::styled(format!("{key:<key_w$}  "), style(t.accent()))];
        match &it.allowed {
            Allowed::Yes => {
                let base = if it.section == MenuSection::Danger { t.fg(t.bad) } else { t.text() };
                spans.push(Span::styled(it.label.clone(), style(base)));
            }
            Allowed::No(r) => {
                spans.push(Span::styled(it.label.clone(), style(t.disabled())));
                let room = inner_w.saturating_sub(2 + key_w + 2 + it.label.width() + 2);
                spans.push(Span::styled(format!("  {}", super::trunc(r, room)), t.dim()));
            }
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("enter choose · or its key · esc close", t.dim()));
    let r = center(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let title = super::trunc(title, inner_w.saturating_sub(2));
    f.render_widget(Paragraph::new(lines).block(pane(&title, true, t)), r);
}

/// "Add an agent": what this device could run one on (the runtimes no
/// agent here uses yet), or one line when there is nothing.
fn add_agent_picker(f: &mut Frame, area: Rect, s: &State, sel: usize, t: &Theme) {
    let available = s.available_local();
    let empty = match &s.local {
        None => Some("Looking for agent runtimes on this device\u{2026}"),
        Some(all) if all.is_empty() => Some("No agent runtime found on this device; ? lists what to install"),
        Some(_) if available.is_empty() => Some("Every runtime here already runs an agent"),
        Some(_) => None,
    };
    if let Some(line) = empty {
        message(f, area, "Add an agent", vec![Line::styled(line, t.dim()), Line::styled("esc close", t.dim())], t);
        return;
    }
    let items: Vec<ListItem> = available
        .iter()
        .map(|l| {
            let (state, style) = match (&l.unsupported, l.ready) {
                (Some(_), _) => ("can't run", t.dim()),
                (None, true) => ("ready", t.fg(t.good)),
                (None, false) => ("not ready", t.fg(t.warn)),
            };
            let what = match &l.model {
                Some(m) => format!("{} · {m}", l.kind.label()),
                None => l.kind.label().to_string(),
            };
            ListItem::new(Line::from(vec![Span::styled(what, t.text()), Span::styled(format!("  {state}"), style)]))
        })
        .collect();
    // The selected one's detail: why it isn't ready, or where it was found.
    let note = available.get(sel).map(|l| l.unsupported.clone().unwrap_or_else(|| l.detail.clone())).unwrap_or_default();
    picker(f, area, "Add an agent on this device", items, sel, &note, t);
}

fn switcher(f: &mut Frame, area: Rect, s: &State, sel: usize, t: &Theme) {
    let items: Vec<ListItem> = s
        .workspaces
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let active = s.active.as_ref() == Some(&w.id);
            let mut spans = vec![
                Span::styled(format!("{} ", i + 1), t.dim()),
                Span::styled(
                    format!("[{}] ", initials(&w.name)),
                    if active { t.accent().add_modifier(Modifier::REVERSED) } else { t.accent() },
                ),
                Span::styled(w.name.clone(), t.text().add_modifier(Modifier::BOLD)),
                Span::styled(format!("  {} · {}", w.your_role.label(), w.sync.label()), t.dim()),
            ];
            if w.mentioned {
                spans.push(Span::styled("  • mentioned", t.fg(t.mention)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let n = items.len() as u16;
    let r = center(area, 72, n + 5);
    f.render_widget(Clear, r);
    let block = pane("Workspaces", true, t);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let mut st = ListState::default().with_selected(Some(sel));
    f.render_stateful_widget(List::new(items).highlight_style(t.selected()), Rect { height: n.min(inner.height), ..inner }, &mut st);
    let foot = Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner };
    f.render_widget(Paragraph::new(Line::styled("enter switch · 1-9 jump · + create or join · esc close", t.dim())), foot);
}

/// The help overlay's `/` search, if one is typed.
fn help_query(s: &State) -> (&str, bool) {
    match &s.overlay {
        Some(Overlay::Help { query, searching, .. }) => (query.text.trim(), *searching),
        _ => ("", false),
    }
}

/// The help overlay's lines: the keys of what has focus first (as
/// configured, marked "here"), then every other context, then the passkey
/// prompt's. A `/` search keeps only the bindings whose keys, words or
/// context match.
pub fn help_lines(s: &State, t: &Theme) -> Vec<Line<'static>> {
    let first = s.view_contexts();
    let order: Vec<Context> = first.iter().copied().chain(Context::ALL.into_iter().filter(|c| !first.contains(c))).collect();
    let (query, _) = help_query(s);
    let q = query.to_lowercase();
    let matches = |ctx: Context, keys: &str, help: &str, name: &str| {
        q.is_empty()
            || keys.to_lowercase().contains(&q)
            || help.to_lowercase().contains(&q)
            || name.replace('_', " ").contains(&q)
            || ctx.title().to_lowercase().contains(&q)
    };
    let mut lines = vec![];
    if q.is_empty() {
        // What the view is for: the explanations the view itself leaves out.
        for l in super::wrap(about(s.view), 72) {
            lines.push(Line::styled(l, t.text()));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(
                "{} opens a row's actions · / searches · rebind keys in the config file",
                s.keymap.hint(Context::Nav, crate::keymap::Action::RowMenu)
            ),
            t.dim(),
        ));
        lines.push(Line::raw(""));
    }
    let row = |keys: &str, help: &str| {
        Line::from(vec![
            Span::styled(format!("  {keys:<24} "), t.text().add_modifier(Modifier::BOLD)),
            Span::styled(help.to_string(), t.text()),
        ])
    };
    for ctx in order {
        let rows: Vec<Line> = BINDINGS
            .iter()
            .filter(|b| b.context == ctx)
            .filter_map(|b| {
                let keys: Vec<String> = s.keymap.keys(ctx, b.action).iter().map(|c| c.to_string()).collect();
                let keys = if keys.is_empty() { "unbound".to_string() } else { keys.join(", ") };
                matches(ctx, &keys, b.help, b.name).then(|| row(&keys, b.help))
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        let mut head = vec![Span::styled(ctx.title(), t.accent())];
        if first.contains(&ctx) && ctx != Context::Global {
            head.push(Span::styled("  here", t.dim()));
        }
        lines.push(Line::from(head));
        lines.extend(rows);
        lines.push(Line::raw(""));
    }
    let passkey = [("enter, o", "Open the browser to confirm"), ("f", "Use your recovery-words file instead"), ("esc, c", "Cancel")];
    let passkey: Vec<Line> = passkey
        .iter()
        .filter(|(k, h)| q.is_empty() || k.contains(&q) || h.to_lowercase().contains(&q) || "passkey prompt".contains(&q))
        .map(|(k, h)| row(k, h))
        .collect();
    if !passkey.is_empty() {
        lines.push(Line::styled("Passkey prompt", t.accent()));
        lines.extend(passkey);
    }
    if lines.is_empty() {
        lines.push(Line::styled(format!("No key matches \u{201c}{query}\u{201d}. esc clears the search."), t.dim()));
    }
    lines
}

/// What a view is for, and what its rows and actions mean: the help
/// overlay's first lines, so the view itself opens on its content.
pub fn about(v: View) -> &'static str {
    match v {
        View::Chats => {
            "Each chat is a room with people and agents. @ mentions an agent or a person; enter on a chat opens it and puts you in the composer."
        }
        View::Workflows => {
            "Workflow definitions, then their runs. Agents propose workflows (approve them under Proposals); the Ligus app's editor makes them too. Starting a run makes this device drive it; its stages go to their agents' hosts. Enter on a run opens its chat."
        }
        View::Proposals => {
            "Changes agents propose, waiting for votes. Once approved, one host applies a proposal once, on its checkout, and reports the result; nobody else applies it meanwhile."
        }
        View::Agents => {
            "The workspace's agents and where they run. An agent's usage is shown to its owner, on the device that runs it (UTC days; runtimes that report no token usage, like most CLIs, count turns only). To add one here, install Claude Code, Codex, the GitHub Copilot CLI or the Antigravity CLI (agy), start Ollama, or set ANTHROPIC_API_KEY / OPENAI_API_KEY / OPENROUTER_API_KEY."
        }
        View::Hosts => {
            "A host runs agents and applies approved proposals on its checkout; a worker is a headless host. This device applies proposals only once its checkout is bound."
        }
        View::Memory => "Agents' live task state, then the workspace's shared memory. Read-only here.",
        View::People => {
            "Who is in the workspace and their role. Changes to who's in it need your passkey. A founder can't be removed or demoted; a service member (a box: ligus serve or a worker) is at most Contributor and has no passkey."
        }
        View::Notifications => "@mentions of you and what needs you, in every workspace. Enter opens the chat.",
        View::Settings => {
            "Notify: a bell, and a desktop notification while lt is in the background. The theme switch lasts this session; keep a theme and rebind keys in the config file (lt --print-config shows every default)."
        }
    }
}

/// How far the help overlay scrolls in a terminal `height` rows tall (the
/// reducer clamps to it, so scrolling back up never hits a dead zone).
pub fn help_max_scroll(s: &State, height: u16) -> usize {
    let len = help_lines(s, &Theme::MONO).len();
    if height == 0 { len.saturating_sub(1) } else { len.saturating_sub(help_body_height(help_rect(Rect::new(0, 0, 80, height)), s)) }
}

fn help_rect(area: Rect) -> Rect {
    center(area, 76, area.height.saturating_sub(2))
}

/// Rows for bindings inside the help box: the border, and the search line
/// while there is one.
fn help_body_height(r: Rect, s: &State) -> usize {
    let (query, searching) = help_query(s);
    let search = usize::from(searching || !query.is_empty());
    (r.height as usize).saturating_sub(2 + search)
}

fn help(f: &mut Frame, area: Rect, s: &State, scroll: usize, t: &Theme) {
    let lines = help_lines(s, t);
    let r = help_rect(area);
    f.render_widget(Clear, r);
    let inner_h = help_body_height(r, s);
    let max = lines.len().saturating_sub(inner_h);
    let start = scroll.min(max);
    let end = (start + inner_h).min(lines.len());
    let more = if max == 0 { String::new() } else { format!(" · {}-{end} of {}", start + 1, lines.len()) };
    let title = format!("Help{more} · / search · esc closes");
    let block = pane(&title, true, t);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let (query, searching) = match &s.overlay {
        Some(Overlay::Help { query, searching, .. }) => (Some(query), *searching),
        _ => (None, false),
    };
    let mut body = inner;
    if let Some(q) = query.filter(|q| searching || !q.text.trim().is_empty()) {
        let hint = if searching { "  enter keep · esc clear" } else { "  / edit · esc clear" };
        let line = Line::from(vec![Span::styled("/ ", t.accent()), Span::styled(q.text.clone(), t.text()), Span::styled(hint, t.dim())]);
        f.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
        if searching {
            f.set_cursor_position(Position::new(inner.x + 2 + q.cursor_pos().1 as u16, inner.y));
        }
        body = Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner };
    }
    let shown: Vec<Line> = lines.into_iter().skip(start).collect();
    f.render_widget(Paragraph::new(shown), body);
}

/// `ctrl-r`: what you sent, newest first, narrowed by the query; enter
/// puts it in the composer to edit or send again.
fn history(f: &mut Frame, area: Rect, s: &State, query: &crate::app::input::TextBuf, sel: usize, t: &Theme) {
    let q = query.text.trim().to_lowercase();
    let items: Vec<String> = s.my_history().into_iter().filter(|m| m.to_lowercase().contains(&q)).collect();
    let w = 76.min(area.width.saturating_sub(2));
    let rows = (items.len().max(1) as u16).min(area.height.saturating_sub(8).max(1)).min(12);
    let r =
        Rect { x: area.x + area.width.saturating_sub(w) / 2, y: area.bottom().saturating_sub(rows + 4 + 4), width: w, height: rows + 4 };
    f.render_widget(Clear, r);
    let block = pane("What you sent · enter use · esc", true, t);
    let inner = block.inner(r);
    f.render_widget(block, r);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("search › ", t.accent()), Span::styled(query.text.clone(), t.text())])),
        Rect { height: 1, ..inner },
    );
    f.set_cursor_position(Position::new(inner.x + 9 + query.cursor_pos().1 as u16, inner.y));
    let list = Rect { y: inner.y + 2, height: inner.height.saturating_sub(2), ..inner };
    if items.is_empty() {
        let none = if s.my_history().is_empty() { "Nothing sent in this chat yet." } else { "Nothing matches." };
        f.render_widget(Paragraph::new(Line::styled(none, t.dim())), list);
        return;
    }
    let n = list.height as usize;
    let first = sel.saturating_sub(n.saturating_sub(1));
    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .skip(first)
        .take(n)
        .map(|(i, m)| {
            let one = m.replace('\n', " ⏎ ");
            let text = super::trunc(&one, (inner.width as usize).saturating_sub(2));
            if i == sel {
                Line::from(vec![Span::styled("▶ ", t.accent()), Span::styled(text, t.selected())])
            } else {
                Line::from(vec![Span::raw("  "), Span::styled(text, t.text())])
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), list);
}

/// The messages log: every notice and error of this session, newest
/// first, wrapped in full (a toast is cut to the status bar).
fn messages(f: &mut Frame, area: Rect, s: &State, scroll: usize, t: &Theme) {
    let r = center(area, 84, area.height.saturating_sub(2));
    f.render_widget(Clear, r);
    let n = s.log.len();
    let title = if n == 0 { "Messages · esc closes".to_string() } else { format!("Messages · {n} this session · esc closes") };
    let block = pane(&title, true, t);
    let inner = block.inner(r);
    f.render_widget(block, r);
    if n == 0 {
        f.render_widget(Paragraph::new(Line::styled("No messages yet", t.dim())), inner);
        return;
    }
    let body_w = (inner.width as usize).saturating_sub(8).max(10);
    let mut lines = vec![];
    for e in s.log.iter().rev().skip(scroll) {
        let (mark, style) = match e.kind {
            crate::app::state::ToastKind::Info => ("•", t.dim()),
            crate::app::state::ToastKind::Ok => ("✓", t.fg(t.good)),
            crate::app::state::ToastKind::Error => ("✗", t.fg(t.bad).add_modifier(Modifier::BOLD)),
        };
        let age = crate::app::state::ago(s.now, e.at);
        for (i, l) in super::wrap(&e.text, body_w).into_iter().enumerate() {
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled(format!("{mark} "), style),
                    Span::styled(format!("{age:>4}  "), t.dim()),
                    Span::styled(l, t.text()),
                ]));
            } else {
                lines.push(Line::from(vec![Span::raw("        "), Span::styled(l, t.text())]));
            }
        }
        if lines.len() >= inner.height as usize {
            break;
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn presence(f: &mut Frame, area: Rect, p: &PresenceSheet, queued: usize, spin: &str, t: &Theme) {
    let create = p.prompt.op == PresenceOp::Create;
    let elsewhere = matches!(p.stage, PresenceStage::Handoff | PresenceStage::HandoffStopped(_));
    let title = if create {
        "Create a passkey"
    } else if elsewhere {
        "Confirm on your desktop or phone"
    } else {
        "Confirm with passkey"
    };
    let mut lines = vec![];
    if p.stage != PresenceStage::Handoff {
        lines.extend([
            Line::styled(p.prompt.summary.clone(), t.accent()),
            Line::raw(""),
            Line::from(vec![Span::styled("Event   ", t.dim()), Span::styled(p.prompt.kind.clone(), t.text())]),
            Line::from(vec![
                Span::styled("Digest  ", t.dim()),
                Span::styled(p.prompt.digest_short.clone(), t.text().add_modifier(Modifier::BOLD)),
            ]),
            Line::raw(""),
        ]);
    }
    // The ways to confirm here instead (none for a resumed hand-off).
    let here_keys = |keys: &mut Vec<Span<'static>>| {
        if !p.prompt.resumed {
            keys.extend([Span::styled("o", t.accent()), Span::styled(" use the browser here   ", t.text())]);
            if !create {
                keys.extend([Span::styled("f", t.accent()), Span::styled(" use recovery-words file   ", t.text())]);
            }
        }
    };
    match &p.stage {
        PresenceStage::Handoff => {
            let code = p.handoff.clone().unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(format!("{spin} "), t.fg(t.warn)),
                Span::styled(format!("Confirm on your desktop or phone: {} (code {code})", p.prompt.summary), t.accent()),
            ]));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "It shows this change with the same code. Nothing is written until you confirm it there with your passkey; Not me there stops it.",
                t.text(),
            ));
            lines.push(Line::raw(""));
            let mut keys = vec![];
            here_keys(&mut keys);
            keys.extend([Span::styled("esc", t.accent()), Span::styled(" cancel", t.text())]);
            lines.push(Line::from(keys));
        }
        PresenceStage::HandoffStopped(why) => {
            lines.push(Line::styled(format!("{why}: nothing was written."), t.fg(t.warn)));
            lines.push(Line::raw(""));
            let mut keys = vec![Span::styled("enter", t.accent()), Span::styled(" ask your desktop or phone again   ", t.text())];
            here_keys(&mut keys);
            keys.extend([Span::styled("esc", t.accent()), Span::styled(" cancel", t.text())]);
            lines.push(Line::from(keys));
        }
        PresenceStage::Asking => {
            lines.push(Line::styled(
                "Your browser opens the workspace's passkey page (ligus.ai by default). Check the digest there matches, then use Touch ID, Windows Hello, your phone or a security key.",
                t.text(),
            ));
            lines.push(Line::raw(""));
            let mut keys = vec![Span::styled("enter", t.accent()), Span::styled(" open the browser   ", t.text())];
            if !create {
                keys.extend([Span::styled("f", t.accent()), Span::styled(" use recovery-words file   ", t.text())]);
            }
            keys.extend([Span::styled("esc", t.accent()), Span::styled(" cancel", t.text())]);
            lines.push(Line::from(keys));
        }
        PresenceStage::WaitingBrowser if p.remote.is_some() => {
            let port = p.remote.as_ref().map(|r| r.1).unwrap_or(0);
            lines.push(Line::styled(
                format!(
                    "No browser on this device: open the address below in a browser on another device whose \
                     127.0.0.1:{port} reaches this one (ssh -L 127.0.0.1:{port}:127.0.0.1:{port} to this host)."
                ),
                t.text(),
            ));
            lines.push(Line::styled("It is a one-time secret for this prompt: don't paste it anywhere else.", t.dim()));
            lines.push(Line::raw(""));
            let mut keys = vec![];
            if !create {
                keys.extend([Span::styled("f", t.accent()), Span::styled(" use recovery-words file   ", t.text())]);
            }
            keys.extend([Span::styled("esc", t.accent()), Span::styled(" cancel", t.text())]);
            lines.push(Line::from(keys));
        }
        PresenceStage::WaitingBrowser => {
            lines.push(Line::styled("Waiting for your passkey… finish in the browser.", t.fg(t.warn)));
            lines.push(Line::styled("No browser here (SSH)? Use the recovery-words file instead.", t.dim()));
            lines.push(Line::raw(""));
            let mut keys = vec![];
            if !create {
                keys.extend([Span::styled("f", t.accent()), Span::styled(" use recovery-words file   ", t.text())]);
            }
            keys.extend([Span::styled("esc", t.accent()), Span::styled(" cancel", t.text())]);
            lines.push(Line::from(keys));
        }
        PresenceStage::EnteringPath(buf) => {
            lines.push(Line::styled("Path to a file holding your 24 recovery words (chmod 600):", t.text()));
            let mut text = buf.text.clone();
            text.insert(buf.cursor, '▏');
            lines.push(Line::styled(format!("› {text}"), t.accent()));
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("enter", t.accent()),
                Span::styled(" confirm   ", t.text()),
                Span::styled("esc", t.accent()),
                Span::styled(" back", t.text()),
            ]));
        }
    }
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("✗ {e}"), t.fg(t.bad).add_modifier(Modifier::BOLD)));
    }
    if queued > 1 {
        lines.push(Line::styled(format!("{} more waiting", queued - 1), t.dim()));
    }
    // The address goes below the sheet, full width and borderless, so a
    // selection copies only the address (a browser drops the line breaks).
    let url_rows = match (&p.stage, &p.remote) {
        (PresenceStage::WaitingBrowser, Some((url, _))) => super::wrap(url, area.width.max(1) as usize).len() as u16,
        _ => 0,
    };
    let sheet_w = 70.min(area.width);
    let sheet_h = lines.iter().map(|l| super::wrap(&l.to_string(), sheet_w.saturating_sub(2) as usize).len() as u16).sum::<u16>() + 2;
    let r = center(area, sheet_w, sheet_h + url_rows);
    let r = Rect { height: r.height.saturating_sub(url_rows), ..r };
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(pane(title, true, t)), r);
    if let (PresenceStage::WaitingBrowser, Some((url, _))) = (&p.stage, &p.remote) {
        let u = Rect { x: area.x, y: r.bottom(), width: area.width, height: url_rows.min(area.bottom().saturating_sub(r.bottom())) };
        f.render_widget(Clear, u);
        let rows: Vec<Line> = super::wrap(url, area.width.max(1) as usize).into_iter().map(|l| Line::styled(l, t.accent())).collect();
        f.render_widget(Paragraph::new(rows), u);
    }
}
