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
        Some(Overlay::Help { scroll, .. }) => help(f, area, s, *scroll, t),
        Some(Overlay::Messages { scroll }) => messages(f, area, s, *scroll, t),
        Some(Overlay::Palette { query, sel }) => super::palette::draw(f, area, s, query, *sel, t),
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
    let w = 64.min(area.width);
    let rows: usize = lines.iter().map(|l| super::wrap(&l.to_string(), w.saturating_sub(2) as usize).len()).sum();
    let r = center(area, w, rows as u16 + 2);
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
        lines.push(Line::styled("What has focus comes first. / searches. Rebind keys in the config file.", t.dim()));
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
        let lines = vec![
            Line::styled("Nothing yet.", t.text()),
            Line::styled("Notices and errors from this session collect here, so one that went by can be read again.", t.dim()),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
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
