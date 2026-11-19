//! Everything drawn over the screen: help, the workspace switcher, pickers,
//! confirmations, invites, and the passkey sheet (always on top).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::{center, onboarding, pane};
use crate::app::state::{Overlay, PresenceSheet, PresenceStage, State};
use crate::backend::model::{Allowed, PresenceOp, Role, initials};
use crate::keymap::{BINDINGS, Context};
use crate::theme::Theme;

pub fn draw(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    match &s.overlay {
        None => {}
        Some(Overlay::Help { scroll }) => help(f, area, s, *scroll, t),
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
                        lines.push(Line::styled("y it matches, admit (needs your passkey) · esc back", t.dim()));
                    }
                }
                (None, None) => {
                    lines.push(Line::styled(
                        "A ligus serve or ligus worker box joins as its own service member (Contributor). Paste the join request it printed (ligusjoin1:…), or leave it empty for a code to put in a Kubernetes Secret or LIGUS_ADMISSION_FILE.",
                        t.text(),
                    ));
                    lines.push(Line::raw(""));
                    lines.push(Line::styled(format!("› {}", super::trunc(&request.text, inner.width as usize - 4)), t.accent()));
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
                    lines.push(Line::styled("any key closes", t.dim()));
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
            picker(
                f,
                area,
                "Apply on…",
                items,
                *sel,
                "That host applies it once, on its checkout, and reports the result. Nobody else applies it meanwhile.",
                t,
            );
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
            picker(f, area, &title, items, *sel, "Refused roles are crossed out with the reason. Changing a role needs your passkey.", t);
        }
        Some(Overlay::RuntimePicker { sel, agent }) => {
            let items: Vec<ListItem> = s
                .runtimes
                .iter()
                .filter(|r| r.configured)
                .map(|r| {
                    ListItem::new(Line::from(vec![
                        Span::styled(r.id.0.clone(), t.text()),
                        Span::styled(format!("  {}", r.kind.label()), t.dim()),
                    ]))
                })
                .collect();
            let handle = s.agents.iter().find(|a| &a.id == agent).map(|a| a.handle.clone()).unwrap_or_default();
            picker(f, area, &format!("Runtime for @{handle}"), items, *sel, "Only configured runtimes are listed.", t);
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
        Some(Overlay::Invite { code }) => {
            let lines = match code {
                None => vec![Line::styled("Creating an invite…", t.dim())],
                Some(c) => vec![
                    Line::styled("Share this invite code. It joins as a contributor and works once.", t.text()),
                    Line::raw(""),
                    Line::styled(c.clone(), t.accent()),
                    Line::raw(""),
                    Line::styled("Select it with your terminal to copy. Any key closes.", t.dim()),
                ],
            };
            message(f, area, "Invite", lines, t);
        }
    }
    if let Some(p) = s.presence.front() {
        presence(f, area, p, s.presence.len(), t);
    }
}

fn message(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line>, t: &Theme) {
    let r = center(area, 64, lines.len() as u16 + 4);
    f.render_widget(Clear, r);
    let block = pane(title, true, t);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), r);
}

fn picker(f: &mut Frame, area: Rect, title: &str, items: Vec<ListItem>, sel: usize, note: &str, t: &Theme) {
    let n = items.len() as u16;
    let r = center(area, 64, n + 6);
    f.render_widget(Clear, r);
    let block = pane(title, true, t);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let list_area = Rect { height: n.min(inner.height), ..inner };
    let mut st = ListState::default().with_selected(Some(sel));
    f.render_stateful_widget(List::new(items).highlight_style(t.selected()).highlight_symbol("› "), list_area, &mut st);
    let note_area = Rect { y: inner.y + n + 1, height: inner.height.saturating_sub(n + 1), ..inner };
    f.render_widget(
        Paragraph::new(vec![Line::styled(note, t.dim()), Line::styled("enter choose · esc cancel", t.dim())]).wrap(Wrap { trim: true }),
        note_area,
    );
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

fn help(f: &mut Frame, area: Rect, s: &State, scroll: usize, t: &Theme) {
    let mut lines = vec![Line::styled("lt keybindings. Rebind any of them in the config file ([keys.<context>]).", t.dim()), Line::raw("")];
    for ctx in Context::ALL {
        lines.push(Line::styled(ctx.title(), t.accent()));
        for b in BINDINGS.iter().filter(|b| b.context == ctx) {
            let keys: Vec<String> = s.keymap.keys(ctx, b.action).iter().map(|c| c.to_string()).collect();
            let keys = if keys.is_empty() { "unbound".to_string() } else { keys.join(", ") };
            lines.push(Line::from(vec![
                Span::styled(format!("  {keys:<32} "), t.text().add_modifier(Modifier::BOLD)),
                Span::styled(b.help, t.text()),
            ]));
        }
        lines.push(Line::raw(""));
    }
    lines.push(Line::styled("Passkey prompt", t.accent()));
    for (k, h) in [("enter, o", "Open the browser to confirm"), ("f", "Use your recovery-words file instead"), ("esc, c", "Cancel")] {
        lines.push(Line::from(vec![Span::styled(format!("  {k:<32} "), t.text().add_modifier(Modifier::BOLD)), Span::styled(h, t.text())]));
    }
    let r = center(area, 76, area.height.saturating_sub(2));
    f.render_widget(Clear, r);
    let block = pane("Help · ↑↓ scroll · any other key closes", true, t);
    let inner_h = r.height.saturating_sub(2) as usize;
    let max = lines.len().saturating_sub(inner_h);
    let start = scroll.min(max);
    let shown: Vec<Line> = lines.into_iter().skip(start).collect();
    f.render_widget(Paragraph::new(shown).block(block), r);
}

fn presence(f: &mut Frame, area: Rect, p: &PresenceSheet, queued: usize, t: &Theme) {
    let create = p.prompt.op == PresenceOp::Create;
    let title = if create { "Create a passkey" } else { "Confirm with passkey" };
    let mut lines = vec![
        Line::styled(p.prompt.summary.clone(), t.accent()),
        Line::raw(""),
        Line::from(vec![Span::styled("Event   ", t.dim()), Span::styled(p.prompt.kind.clone(), t.text())]),
        Line::from(vec![
            Span::styled("Digest  ", t.dim()),
            Span::styled(p.prompt.digest_short.clone(), t.text().add_modifier(Modifier::BOLD)),
        ]),
        Line::raw(""),
    ];
    match &p.stage {
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
    let r = center(area, 70, lines.len() as u16 + 6);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(pane(title, true, t)), r);
}
