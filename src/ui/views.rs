//! Agents and runtimes, hosts, people, notifications and settings.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap};

use super::{pane, trunc};
use crate::app::state::{State, ago};
use crate::backend::model::{Allowed, HostKind, Role};
use crate::keymap::{Action, Context};
use crate::theme::Theme;

fn host_badge(s: &State, host: Option<&crate::backend::model::HostId>) -> (String, bool) {
    match host.and_then(|id| s.hosts.iter().find(|h| &h.id == id)) {
        None => ("This device".into(), true),
        Some(h) if h.this_device => ("This device".into(), true),
        Some(h) => (format!("{} · {}", h.label, if h.online { "online" } else { "offline" }), h.online),
    }
}

pub fn agents(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let [top, bottom] = Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
    let rows: Vec<Row> = s
        .agents
        .iter()
        .map(|a| {
            let (badge, ok) = host_badge(s, a.host.as_ref());
            Row::new(vec![
                Span::styled(format!("@{}", a.handle), t.fg(t.agent).add_modifier(Modifier::BOLD)),
                Span::styled(a.runtime.0.clone(), t.text()),
                Span::styled(a.role.clone().unwrap_or_else(|| "agent".into()), t.dim()),
                Span::styled(badge, if ok { t.fg(t.good) } else { t.fg(t.warn) }),
                Span::styled(if a.shared { "workspace" } else { "personal" }, t.dim()),
            ])
        })
        .collect();
    let header = Row::new(vec!["Agent", "Runtime", "Role", "Runs on", "Scope"]).style(t.dim().add_modifier(Modifier::BOLD));
    let table = Table::new(
        rows,
        [Constraint::Length(14), Constraint::Length(13), Constraint::Min(16), Constraint::Length(16), Constraint::Length(10)],
    )
    .header(header)
    .block(pane("Agents", true, t))
    .row_highlight_style(t.selected());
    let mut st = TableState::default().with_selected(if s.agents.is_empty() { None } else { Some(s.agent_sel.index) });
    f.render_stateful_widget(table, top, &mut st);

    let rows: Vec<Row> = s
        .runtimes
        .iter()
        .map(|r| {
            let chips = match (r.configured, r.active) {
                (true, true) => Span::styled("Configured · Active", t.fg(t.good)),
                (true, false) => Span::styled("Configured", t.fg(t.good)),
                _ => Span::styled("Not configured", t.dim()),
            };
            let place = r.kind.place();
            Row::new(vec![
                Span::styled(r.id.0.clone(), t.text()),
                Span::styled(r.kind.label(), t.text()),
                Span::styled(r.model.clone().unwrap_or_else(|| "default".into()), t.dim()),
                Span::styled(place, t.dim()),
                chips,
            ])
        })
        .collect();
    let header = Row::new(vec!["Runtime", "Kind", "Model", "Where", "Status"]).style(t.dim().add_modifier(Modifier::BOLD));
    let table = Table::new(
        rows,
        [Constraint::Length(13), Constraint::Length(24), Constraint::Min(14), Constraint::Length(13), Constraint::Length(20)],
    )
    .header(header)
    .block(pane("Runtimes", false, t));
    f.render_widget(table, bottom);
}

pub fn hosts(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let rows: Vec<Row> = s
        .hosts
        .iter()
        .map(|h| {
            let dot = if h.online { Span::styled("●", t.fg(t.good)) } else { Span::styled("○", t.dim()) };
            let seen = if h.online {
                "online".to_string()
            } else {
                h.last_seen.map(|at| format!("{} ago", ago(s.now, at))).unwrap_or_else(|| "never seen".into())
            };
            let kind = match h.kind {
                HostKind::Device => "DEVICE",
                HostKind::Worker => "WORKER",
            };
            let agents: Vec<String> = s
                .agents
                .iter()
                .filter(|a| match &a.host {
                    Some(id) => id == &h.id,
                    None => h.this_device,
                })
                .map(|a| format!("@{}", a.handle))
                .collect();
            let label = if h.this_device && h.label != "This device" { format!("{} (this device)", h.label) } else { h.label.clone() };
            Row::new(vec![
                dot,
                Span::styled(label, t.text().add_modifier(Modifier::BOLD)),
                Span::styled(kind, t.dim()),
                Span::styled(seen, if h.online { t.fg(t.good) } else { t.dim() }),
                Span::styled(agents.join(" "), t.fg(t.agent)),
            ])
        })
        .collect();
    let header = Row::new(vec!["", "Host", "Kind", "Status", "Agents"]).style(t.dim().add_modifier(Modifier::BOLD));
    let table = Table::new(
        rows,
        [Constraint::Length(1), Constraint::Length(20), Constraint::Length(7), Constraint::Length(10), Constraint::Min(10)],
    )
    .header(header)
    .block(pane("Workers & hosts", true, t))
    .row_highlight_style(t.selected());
    let mut st = TableState::default().with_selected(if s.hosts.is_empty() { None } else { Some(s.host_sel.index) });
    let [table_area, note] = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).areas(area);
    if s.hosts.is_empty() {
        f.render_widget(Paragraph::new(Line::styled("No hosts registered.", t.dim())).block(pane("Workers & hosts", true, t)), table_area);
    } else {
        f.render_stateful_widget(table, table_area, &mut st);
    }
    f.render_widget(
        Paragraph::new(Line::styled(" A host runs agents and applies approved proposals on its checkout. A worker is headless.", t.dim()))
            .wrap(Wrap { trim: true }),
        note,
    );
}

pub fn people(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let [list, detail] = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let items: Vec<ListItem> = s
        .members
        .iter()
        .map(|m| {
            let mut spans = vec![Span::raw("  "), Span::styled(m.name.clone(), t.text().add_modifier(Modifier::BOLD))];
            if m.you {
                spans.push(Span::styled(" · you", t.dim()));
            }
            spans.push(Span::styled(format!("  {}", m.role.label()), t.accent()));
            if m.founder {
                spans.push(Span::styled("  founder", t.fg(t.hunk)));
            }
            if m.service {
                spans.push(Span::styled("  service", t.fg(t.hunk)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let empty = s.members.is_empty();
    let list_w = List::new(items).block(pane("People", true, t)).highlight_style(t.selected());
    let mut st = ListState::default().with_selected(if empty { None } else { Some(s.member_sel.index) });
    f.render_stateful_widget(list_w, list, &mut st);

    let Some(m) = s.selected_member() else {
        f.render_widget(
            Paragraph::new(Line::styled("No extra members have been added yet.", t.dim())).block(pane("Member", false, t)),
            detail,
        );
        return;
    };
    let k = &s.keymap;
    let mut lines = vec![
        Line::from(vec![Span::styled(m.name.clone(), t.accent()), Span::styled(format!("  {}", m.role.label()), t.text())]),
        Line::raw(""),
    ];
    if m.founder {
        lines.push(Line::styled("Founder: nobody can remove or demote them.", t.dim()));
    }
    if m.service {
        lines.push(Line::styled("A service member (a box): at most Contributor, no passkey.", t.dim()));
    }
    lines.push(Line::styled(format!("Role ({})", k.hint(Context::People, Action::ChangeRole)), t.text().add_modifier(Modifier::BOLD)));
    for role in Role::ALL {
        let allowed = m.can_set_role.iter().find(|(r, _)| *r == role).map(|(_, a)| a.clone()).unwrap_or(Allowed::No("unknown".into()));
        let current = role == m.role;
        let mark = if current { "(•) " } else { "( ) " };
        let line = match (&allowed, current) {
            (_, true) => Line::from(vec![Span::styled(format!("  {mark}{}", role.label()), t.accent())]),
            (Allowed::Yes, _) => Line::from(vec![Span::styled(format!("  {mark}{}", role.label()), t.text())]),
            (Allowed::No(r), _) => {
                Line::from(vec![Span::styled(format!("  {mark}{}", role.label()), t.disabled()), Span::styled(format!("  {r}"), t.dim())])
            }
        };
        lines.push(line);
    }
    lines.push(Line::raw(""));
    let remove_label = if m.you { "Leave the workspace" } else { "Remove from the workspace" };
    match &m.can_remove {
        Allowed::Yes => lines.push(Line::from(vec![
            Span::styled(k.hint(Context::People, Action::RemoveMember), t.accent()),
            Span::styled(format!(" {remove_label}"), t.text()),
        ])),
        Allowed::No(r) => lines.push(Line::from(vec![
            Span::styled(format!("{} {remove_label}", k.hint(Context::People, Action::RemoveMember)), t.disabled()),
            Span::styled(format!("  {r}"), t.dim()),
        ])),
    }
    match s.active_workspace().map(|w| w.can_invite.clone()).unwrap_or(Allowed::Yes) {
        Allowed::Yes => lines.push(Line::from(vec![
            Span::styled(k.hint(Context::People, Action::Invite), t.accent()),
            Span::styled(" Create an invite", t.text()),
        ])),
        Allowed::No(r) => lines.push(Line::from(vec![
            Span::styled(format!("{} Create an invite", k.hint(Context::People, Action::Invite)), t.disabled()),
            Span::styled(format!("  {r}"), t.dim()),
        ])),
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Changes to who's in the workspace need your passkey.", t.dim()));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(pane("Member", false, t)), detail);
}

pub fn notifications(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let block = pane("Notifications", true, t);
    if s.notifications.is_empty() {
        f.render_widget(Paragraph::new(Line::styled("Nothing new. You'll see @mentions of you here.", t.dim())).block(block), area);
        return;
    }
    let w = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = s
        .notifications
        .iter()
        .map(|n| {
            let ws = s.workspaces.iter().find(|w| w.id == n.workspace).map(|w| w.name.clone()).unwrap_or_default();
            let dot = if n.read { Span::raw("  ") } else { Span::styled("● ", t.fg(t.mention)) };
            let style = if n.read { t.dim() } else { t.text().add_modifier(Modifier::BOLD) };
            ListItem::new(Line::from(vec![
                dot,
                Span::styled(trunc(&n.text, w.saturating_sub(ws.len() + 12)), style),
                Span::styled(format!("  {ws} · {}", ago(s.now, n.at)), t.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).block(block).highlight_style(t.selected());
    let mut st = ListState::default().with_selected(Some(s.notification_sel.index));
    f.render_stateful_widget(list, area, &mut st);
}

pub fn settings(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let block = pane("Settings", true, t);
    let Some(st) = &s.settings else {
        f.render_widget(Paragraph::new(Line::styled("Loading…", t.dim())).block(block), area);
        return;
    };
    let toggle = |i: usize, on: bool, label: &str, note: &str| {
        let sel = s.settings_sel.index == i;
        let mark = if on { "[x]" } else { "[ ]" };
        let style = if sel { t.selected() } else { t.text() };
        Line::from(vec![Span::styled(format!(" {mark} {label} "), style), Span::styled(format!(" {note}"), t.dim())])
    };
    let kv = |k: &str, v: String| Line::from(vec![Span::styled(format!("  {k:<16}"), t.dim()), Span::styled(v, t.text())]);
    let head = |h: &str| Line::styled(h.to_string(), t.accent());
    let d = &s.device;
    let lines = vec![
        head("You"),
        kv("Name", if st.your_name.is_empty() { "—".into() } else { st.your_name.clone() }),
        kv("Safety code", d.safety_code.clone()),
        Line::raw(""),
        head("Notifications and agents"),
        toggle(0, st.notify_mentions, "Notify me when I'm @mentioned", "a terminal bell and a toast"),
        toggle(1, st.agents_may_edit, "Let agents edit files and run commands", "on this device's hosts"),
        Line::raw(""),
        head("This device"),
        kv("Device", st.device_name.clone()),
        kv("Data dir", d.data_dir.clone()),
        kv("Secret store", d.secret_store.clone()),
        kv("Backend", d.backend_label.clone()),
        kv(
            "Sync",
            st.relay_url.clone().map(|u| format!("relay {u}")).unwrap_or_else(|| "no relay: peer to peer, or this device only".into()),
        ),
        Line::raw(""),
        head("Appearance and keys"),
        kv("Theme", s.theme_name.clone()),
        kv("Config file", s.config_path.clone().unwrap_or_else(|| "none".into())),
        Line::styled("  Change the theme and rebind keys in the config file; lt --print-config shows every default.", t.dim()),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), area);
}
