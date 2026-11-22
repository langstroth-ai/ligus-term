//! Agents and runtimes, hosts, people, notifications, settings, workflows
//! and memory.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap};

use unicode_width::UnicodeWidthStr;

use super::{pane, trunc};
use crate::app::state::WorkflowItem;
use crate::app::state::{State, ago, ago_phrase};
use crate::backend::model::{AgentStatus, Allowed, HostKind, Role, RunStatus, StageStatus};
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
    let available = s.available_local();
    let ([top, av, us, ho, bottom], hidden) = agents_layout(s, available.len(), area);
    let title = if hidden.is_empty() { "Agents".to_string() } else { format!("Agents · a taller terminal shows {}", hidden.join(", ")) };
    let rows: Vec<Row> = s
        .agents
        .iter()
        .map(|a| {
            let (badge, ok) = host_badge(s, a.host.as_ref());
            let status = match a.status {
                AgentStatus::Busy => Span::styled("busy", t.fg(t.warn).add_modifier(Modifier::BOLD)),
                AgentStatus::Idle => Span::styled("idle", t.fg(t.good)),
                AgentStatus::Offline => Span::styled("offline", t.dim()),
            };
            let doing = a.doing.clone().or_else(|| a.role.clone()).unwrap_or_default();
            // Who may use it; an agent you may not use is dimmed, with why.
            let (handle, access) = match &a.unavailable {
                Some(why) => (Span::styled(format!("@{}", a.handle), t.dim()), Span::styled(why.clone(), t.dim())),
                None => (
                    Span::styled(format!("@{}", a.handle), t.fg(t.agent).add_modifier(Modifier::BOLD)),
                    Span::styled(access_text(a), t.text()),
                ),
            };
            Row::new(vec![
                handle,
                status,
                Span::styled(a.runtime.0.clone(), t.text()),
                Span::styled(badge, if ok { t.fg(t.good) } else { t.fg(t.warn) }),
                access,
                Span::styled(doing, if a.doing.is_some() { t.text() } else { t.dim() }),
            ])
        })
        .collect();
    let header =
        Row::new(vec!["Agent", "Status", "Runtime", "Runs on", "Who can use", "Doing"]).style(t.dim().add_modifier(Modifier::BOLD));
    let table = Table::new(rows, columns(&[12, 8, 12, 18, 22, 12], top.width))
        .header(header)
        .block(pane(&title, s.agent_sel.index < s.agents.len(), t))
        .row_highlight_style(t.selected());
    let on_agents = s.agent_sel.index < s.agents.len();
    let mut st = TableState::default().with_selected(on_agents.then_some(s.agent_sel.index));
    f.render_stateful_widget(table, top, &mut st);
    local_runtimes(f, av, s, &available, t);

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
    let table =
        Table::new(rows, columns_with_fill(&[13, 24, 14, 13, 20], 2, bottom.width)).header(header).block(pane("Runtimes", false, t));
    if bottom.height > 0 {
        f.render_widget(table, bottom);
    }
    let w = ho.width.saturating_sub(4) as usize;
    let lines: Vec<Line> = if s.handoffs.is_empty() {
        vec![Line::styled("No hand-offs yet.", t.dim())]
    } else {
        s.handoffs
            .iter()
            .take(ho.height.saturating_sub(2) as usize)
            .map(|h| {
                Line::from(vec![
                    Span::styled(format!("@{} → @{} ", h.from, h.to), t.fg(t.agent)),
                    Span::styled(trunc(&h.task, w.saturating_sub(h.from.len() + h.to.len() + 12)), t.text()),
                    Span::styled(format!(" {}", ago(s.now, h.at)), t.dim()),
                ])
            })
            .collect()
    };
    if ho.height > 0 {
        f.render_widget(Paragraph::new(lines).block(pane("Hand-offs", false, t)), ho);
    }
    if us.height > 0 {
        let usage = usage_lines(s.agents.get(s.agent_sel.index), t);
        f.render_widget(Paragraph::new(usage).wrap(Wrap { trim: true }).block(pane("Usage on this device", false, t)), us);
    }
}

/// Table columns of these widths, the last one taking what is left, in a
/// table `width` wide (borders included). When they don't fit, each is
/// shrunk in proportion: the widths are worked out here, so the layout
/// never asks the solver for more than there is (which it settles
/// differently from one run to the next).
fn columns(widths: &[u16], width: u16) -> Vec<Constraint> {
    columns_with_fill(widths, widths.len().saturating_sub(1), width)
}

/// As [`columns`], with column `fill` taking what is left.
fn columns_with_fill(widths: &[u16], fill: usize, width: u16) -> Vec<Constraint> {
    // Borders, and one column of spacing between columns.
    let room = width.saturating_sub(2 + widths.len().saturating_sub(1) as u16);
    let total: u16 = widths.iter().sum();
    if total <= room {
        return widths.iter().enumerate().map(|(i, w)| if i == fill { Constraint::Min(*w) } else { Constraint::Length(*w) }).collect();
    }
    let mut out: Vec<u16> = widths.iter().map(|w| ((u32::from(*w) * u32::from(room)) / u32::from(total.max(1))) as u16).collect();
    let used: u16 = out.iter().sum();
    if let Some(f) = out.get_mut(fill) {
        *f += room.saturating_sub(used);
    }
    out.into_iter().map(Constraint::Length).collect()
}

/// The Agents view's panes, by priority: the agents, what this device could
/// add, the runtimes, the selected agent's usage, then the hand-offs. Each
/// pane in turn gets what it wants (or what is left, if that is at least its
/// smallest size); one that won't fit is left out (an empty `Rect`) and
/// named in the second value, rather than squeezing every pane to a header.
/// What is left over goes to the agents. On a wide terminal usage and
/// hand-offs share a row. On screen the order stays agents, available,
/// usage, hand-offs, runtimes.
fn agents_layout(s: &State, available: usize, area: Rect) -> ([Rect; 5], Vec<&'static str>) {
    const AGENTS: usize = 0;
    const AVAIL: usize = 1;
    const USAGE: usize = 2;
    const HANDOFFS: usize = 3;
    const RUNTIMES: usize = 4;
    let pair = area.width >= 140;
    // (wanted, smallest) heights, borders and table headers included.
    let mut wants = [(0u16, 0u16); 5];
    wants[AGENTS] = ((s.agents.len() as u16 + 3).min((area.height / 2).max(5)), 4);
    wants[AVAIL] = (3 + available.clamp(2, 5) as u16, 4);
    wants[RUNTIMES] = ((s.runtimes.len() as u16 + 3).clamp(4, 8), 4);
    wants[USAGE] = (5, 4);
    wants[HANDOFFS] = (s.handoffs.len().clamp(1, 3) as u16 + 2, 3);
    if pair {
        // One row for both: the taller of the two.
        wants[USAGE] = (wants[USAGE].0.max(wants[HANDOFFS].0), 4);
        wants[HANDOFFS] = (0, 0);
    }
    let priority = [AGENTS, AVAIL, RUNTIMES, USAGE, HANDOFFS];
    let mut h = [0u16; 5];
    let mut left = area.height;
    let mut hidden = vec![];
    for i in priority {
        let give = wants[i].0.min(left);
        if wants[i].1 > 0 && give >= wants[i].1 {
            h[i] = give;
            left -= give;
        } else if wants[i].1 > 0 {
            hidden.push(match i {
                AVAIL => "available",
                RUNTIMES => "runtimes",
                USAGE if pair => "usage and hand-offs",
                USAGE => "usage",
                HANDOFFS => "hand-offs",
                _ => "agents",
            });
        }
    }
    h[AGENTS] += left;
    let mut y = area.y;
    let mut take = |height: u16| {
        let r = Rect { y, height, ..area };
        y += height;
        r
    };
    let top = take(h[AGENTS]);
    let av = take(h[AVAIL]);
    let (us, ho) = if pair {
        let row = take(h[USAGE]);
        let [a, b] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(row);
        (a, b)
    } else {
        (take(h[USAGE]), take(h[HANDOFFS]))
    };
    let rt = take(h[RUNTIMES]);
    ([top, av, us, ho, rt], hidden)
}

/// What this device could run agents on and no agent here uses yet, each
/// one a row to add as an agent; or what to install when nothing was found.
fn local_runtimes(f: &mut Frame, area: Rect, s: &State, available: &[&crate::backend::model::LocalRuntime], t: &Theme) {
    let selected = s.agent_sel.index.checked_sub(s.agents.len());
    let add = s.keymap.hint(Context::Agents, Action::AddAgent);
    let title = if available.is_empty() {
        "Available on this device".to_string()
    } else {
        format!("Available on this device · {add} or enter adds an agent")
    };
    let block = pane(&title, selected.is_some(), t);
    let empty = |lines: Vec<Line<'static>>| Paragraph::new(lines).wrap(Wrap { trim: true }).block(block.clone());
    match &s.local {
        None => return f.render_widget(empty(vec![Line::styled("Looking for agent runtimes on this device\u{2026}", t.dim())]), area),
        Some(all) if all.is_empty() => {
            return f.render_widget(
                empty(vec![
                    Line::styled("No agent runtime found on this device.", t.text()),
                    Line::styled(
                        "Install Claude Code, Codex, the GitHub Copilot CLI or the Antigravity CLI (agy), start Ollama, or set \
                         ANTHROPIC_API_KEY / OPENAI_API_KEY / OPENROUTER_API_KEY, then open this tab again.",
                        t.dim(),
                    ),
                ]),
                area,
            );
        }
        Some(_) if available.is_empty() => {
            return f
                .render_widget(empty(vec![Line::styled("Every runtime found on this device already runs an agent here.", t.dim())]), area);
        }
        Some(_) => {}
    }
    let rows: Vec<Row> = available
        .iter()
        .map(|l| {
            let status = match (&l.unsupported, l.ready) {
                (Some(_), _) => Span::styled("can't run", t.dim()),
                (None, true) => Span::styled("ready", t.fg(t.good)),
                (None, false) => Span::styled("not ready", t.fg(t.warn)),
            };
            let detail = l.unsupported.clone().unwrap_or_else(|| l.detail.clone());
            Row::new(vec![
                Span::styled(l.kind.label(), t.text()),
                Span::styled(l.model.clone().unwrap_or_else(|| "default".into()), t.dim()),
                Span::styled(l.version.clone().unwrap_or_default(), t.dim()),
                status,
                Span::styled(detail, t.dim()),
            ])
        })
        .collect();
    let header = Row::new(vec!["Runtime", "Model", "Version", "Status", "Detail"]).style(t.dim().add_modifier(Modifier::BOLD));
    let table = Table::new(rows, columns(&[24, 16, 9, 10, 12], area.width)).header(header).block(block).row_highlight_style(t.selected());
    let mut st = TableState::default().with_selected(selected);
    f.render_stateful_widget(table, area, &mut st);
}

/// "Everyone" or "Only its owner" (a shared agent: "Admins only").
pub fn access_text(a: &crate::backend::model::Agent) -> String {
    match (a.access.as_str(), a.shared) {
        ("everyone", _) => "Everyone".into(),
        (_, true) => "Admins only".into(),
        (_, false) if a.can_manage => "Only me".into(),
        _ => "Only its owner".into(),
    }
}

/// The selected agent's usage on this device (its owner's to see): per
/// member, turns and estimated cost, today and this month (UTC days).
fn usage_lines<'a>(a: Option<&crate::backend::model::Agent>, t: &Theme) -> Vec<Line<'a>> {
    let Some(a) = a else { return vec![] };
    let Some(u) = &a.usage else {
        return vec![Line::styled("Usage is shown to the agent's owner, on the device that runs it.", t.dim())];
    };
    let row = |label: &str, rows: &[crate::backend::model::UsageRow]| -> Line<'a> {
        let text = if rows.is_empty() {
            "none".to_string()
        } else {
            rows.iter().map(|r| format!("{}: {}", r.member, r.summary())).collect::<Vec<_>>().join(" · ")
        };
        Line::from(vec![Span::styled(format!("{label:<11} "), t.dim()), Span::styled(text, t.text())])
    };
    vec![
        row(&format!("Today {}", u.day.get(5..).unwrap_or("")), &u.today),
        row("This month", &u.month),
        Line::styled("UTC days. Runtimes that report no token usage (most CLIs) count turns only.", t.dim()),
    ]
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
                h.last_seen.map(|at| ago_phrase(s.now, at)).unwrap_or_else(|| "never seen".into())
            };
            let kind = match h.kind {
                HostKind::Device => "DEVICE",
                HostKind::Worker => "WORKER",
            };
            let agents: Vec<String> = h.agents.iter().map(|a| format!("@{a}")).collect();
            let label = if h.this_device && h.label != "This device" { format!("{} (this device)", h.label) } else { h.label.clone() };
            let work = if h.busy { Span::styled("busy", t.fg(t.warn).add_modifier(Modifier::BOLD)) } else { Span::styled("idle", t.dim()) };
            Row::new(vec![
                dot,
                Span::styled(label, t.text().add_modifier(Modifier::BOLD)),
                Span::styled(kind, t.dim()),
                Span::styled(seen, if h.online { t.fg(t.good) } else { t.dim() }),
                work,
                Span::styled(agents.join(" "), t.fg(t.agent)),
                Span::styled(h.repos.join(" "), t.dim()),
            ])
        })
        .collect();
    let header = Row::new(vec!["", "Host", "Kind", "Status", "Work", "Agents", "Repos"]).style(t.dim().add_modifier(Modifier::BOLD));
    let table = Table::new(rows, columns_with_fill(&[1, 20, 7, 10, 5, 10, 12], 5, area.width))
        .header(header)
        .block(pane("Workers & hosts", true, t))
        .row_highlight_style(t.selected());
    let mut st = TableState::default().with_selected(if s.hosts.is_empty() { None } else { Some(s.host_sel.index) });
    let [table_area, note] = Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).areas(area);
    if s.hosts.is_empty() {
        f.render_widget(Paragraph::new(Line::styled("No hosts registered.", t.dim())).block(pane("Workers & hosts", true, t)), table_area);
    } else {
        f.render_stateful_widget(table, table_area, &mut st);
    }
    let bind = s.keymap.hint(Context::Hosts, Action::BindCheckout);
    let checkout = match &s.checkout {
        Some(p) => Line::from(vec![
            Span::styled(" This device applies approved proposals in ", t.dim()),
            Span::styled(p.clone(), t.text()),
            Span::styled(format!("  ({bind} change)"), t.dim()),
        ]),
        None => Line::from(vec![
            Span::styled(" No checkout bound: this device can't apply proposals here. ", t.dim()),
            Span::styled(format!("{bind} bind one"), t.accent()),
        ]),
    };
    f.render_widget(
        Paragraph::new(vec![
            checkout,
            Line::styled(" A host runs agents and applies approved proposals on its checkout. A worker is headless.", t.dim()),
        ])
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
    let admit = k.hint(Context::People, Action::AdmitBox);
    if s.active_workspace().is_some_and(|w| w.your_role >= Role::Admin) {
        lines.push(Line::from(vec![
            Span::styled(admit, t.accent()),
            Span::styled(" Admit a box (ligus serve or a worker) as a service member", t.text()),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(format!("{admit} Admit a box"), t.disabled()),
            Span::styled("  only an Admin or Owner admits a box", t.dim()),
        ]));
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
        {
            let sel = s.settings_sel.index == 2;
            let style = if sel { t.selected() } else { t.text() };
            Line::from(vec![
                Span::styled(format!(" Theme  {} ", s.theme_name), style),
                Span::styled("  enter switches: dark, light, mono (this session)", t.dim()),
            ])
        },
        kv("Config file", s.config_path.clone().unwrap_or_else(|| "none".into())),
        Line::styled("  Keep a theme and rebind keys in the config file; lt --print-config shows every default.", t.dim()),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), area);
}

fn run_status_style(st: RunStatus, t: &Theme) -> ratatui::style::Style {
    match st {
        RunStatus::Running => t.fg(t.accent),
        RunStatus::AwaitingGate => t.fg(t.warn).add_modifier(Modifier::BOLD),
        RunStatus::Completed => t.fg(t.good),
        RunStatus::Failed | RunStatus::Halted => t.fg(t.bad),
        RunStatus::Canceled => t.dim(),
    }
}

fn stage_mark(st: StageStatus, t: &Theme) -> Span<'static> {
    match st {
        StageStatus::Pending => Span::styled("○", t.dim()),
        StageStatus::Running => Span::styled("◐", t.fg(t.accent)),
        StageStatus::AwaitingApproval => Span::styled("◆", t.fg(t.warn)),
        StageStatus::Succeeded => Span::styled("●", t.fg(t.good)),
        StageStatus::Failed | StageStatus::Rejected => Span::styled("✗", t.fg(t.bad)),
        StageStatus::Skipped => Span::styled("–", t.dim()),
    }
}

/// Workflow definitions and runs (live), with the selected one's detail.
pub fn workflows(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let [list, detail] = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let w = list.width.saturating_sub(4) as usize;
    let mut items: Vec<ListItem> = s
        .workflows
        .iter()
        .map(|d| {
            let style = if d.enabled { t.text().add_modifier(Modifier::BOLD) } else { t.disabled() };
            // The name first; the chat's title gets what is left.
            let name = trunc(&d.name, w.saturating_sub(2));
            let room = w.saturating_sub(2 + name.width() + 2);
            let chat = if room >= 6 { format!("  {}", trunc(&d.chat_title, room)) } else { String::new() };
            ListItem::new(Line::from(vec![Span::styled("▸ ", t.accent()), Span::styled(name, style), Span::styled(chat, t.dim())]))
        })
        .collect();
    items.extend(s.runs.iter().map(|r| {
        let marks: Vec<Span> = r.stages.iter().map(|st| stage_mark(st.status, t)).collect();
        let age = ago(s.now, r.updated_at);
        let tail = marks.len() + 1 + r.status.label().width() + 1 + age.width();
        let name = trunc(&r.name, w.saturating_sub(tail + 3).max(8));
        let mut spans = vec![Span::styled(format!("  {name} "), t.text())];
        spans.extend(marks);
        spans.push(Span::styled(format!(" {}", r.status.label()), run_status_style(r.status, t)));
        spans.push(Span::styled(format!(" {age}"), t.dim()));
        ListItem::new(Line::from(spans))
    }));
    let title = format!("Workflows {} · Runs {}", s.workflows.len(), s.runs.len());
    if items.is_empty() {
        f.render_widget(
            Paragraph::new(vec![
                Line::styled("No workflows yet.", t.dim()),
                Line::styled("Agents propose them (approve under Proposals); the Ligus app's editor makes them too.", t.dim()),
            ])
            .wrap(Wrap { trim: false })
            .block(pane(&title, true, t)),
            list,
        );
    } else {
        let lw = List::new(items).block(pane(&title, true, t)).highlight_style(t.selected());
        let mut st = ListState::default().with_selected(Some(s.workflow_sel.index));
        f.render_stateful_widget(lw, list, &mut st);
    }
    let k = &s.keymap;
    let hint = |a| k.hint(Context::Workflows, a);
    let dw = detail.width.saturating_sub(4) as usize;
    let mut lines: Vec<Line> = vec![];
    match s.selected_workflow_item() {
        None => lines.push(Line::styled("Select a workflow or a run.", t.dim())),
        Some(WorkflowItem::Def(d)) => {
            lines.push(Line::styled(d.name.clone(), t.accent()));
            lines.push(Line::styled(format!("in {}", d.chat_title), t.dim()));
            if !d.description.is_empty() {
                lines.push(Line::raw(""));
                lines.extend(super::wrap(&d.description, dw).into_iter().map(|l| Line::styled(l, t.text())));
            }
            lines.push(Line::raw(""));
            for st in &d.stages {
                let after = if st.depends_on.is_empty() { String::new() } else { format!("  after {}", st.depends_on.join(", ")) };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", st.name), t.text().add_modifier(Modifier::BOLD)),
                    Span::styled(st.what.clone(), t.fg(t.agent)),
                    Span::styled(after, t.dim()),
                ]));
            }
            lines.push(Line::raw(""));
            match &d.can_start {
                Allowed::Yes => lines.push(Line::from(vec![
                    Span::styled(hint(Action::StartRun), t.accent()),
                    Span::styled(" Start a run: this device drives it; stages go to their agents' hosts", t.text()),
                ])),
                Allowed::No(r) => lines.push(Line::from(vec![
                    Span::styled(format!("{} Start a run", hint(Action::StartRun)), t.disabled()),
                    Span::styled(format!("  {r}"), t.dim()),
                ])),
            }
        }
        Some(WorkflowItem::Run(r)) => {
            lines.push(Line::from(vec![
                Span::styled(r.name.clone(), t.accent()),
                Span::styled(format!("  {}", r.status.label()), run_status_style(r.status, t)),
            ]));
            let driver = r.driver.clone().unwrap_or_else(|| "no device".into());
            lines.push(Line::styled(
                format!("started by {} {} · driven by {driver}", r.started_by, ago_phrase(s.now, r.started_at)),
                t.dim(),
            ));
            if !r.input.is_empty() {
                lines
                    .push(Line::from(vec![Span::styled("input ", t.dim()), Span::styled(trunc(&r.input, dw.saturating_sub(6)), t.text())]));
            }
            lines.push(Line::raw(""));
            for st in &r.stages {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    stage_mark(st.status, t),
                    Span::styled(format!(" {} ", st.name), t.text().add_modifier(Modifier::BOLD)),
                    Span::styled(st.what.clone(), t.fg(t.agent)),
                    Span::styled(format!("  {}", st.status.label()), t.dim()),
                ]));
                let note = if !st.error.is_empty() { Some((st.error.clone(), t.fg(t.bad))) } else { Some((st.excerpt.clone(), t.dim())) };
                if let Some((text, style)) = note.filter(|(n, _)| !n.is_empty()) {
                    lines.push(Line::styled(format!("      {}", trunc(&text.replace('\n', " "), dw.saturating_sub(6))), style));
                }
            }
            lines.push(Line::raw(""));
            if r.stages.iter().any(|st| st.status == StageStatus::AwaitingApproval) {
                lines.push(Line::from(vec![
                    Span::styled(hint(Action::ApproveGate), t.accent()),
                    Span::styled(" approve the gate  ", t.text()),
                    Span::styled(hint(Action::RejectGate), t.accent()),
                    Span::styled(" reject it", t.text()),
                ]));
            }
            if !r.status.is_terminal() {
                lines.push(Line::from(vec![Span::styled(hint(Action::CancelRun), t.accent()), Span::styled(" cancel the run", t.text())]));
            }
            lines.push(Line::from(vec![
                Span::styled(k.hint(Context::Nav, Action::Select), t.accent()),
                Span::styled(" open its chat", t.text()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(pane("Detail", false, t)), detail);
}

/// Agents' live task state, then the workspace memory.
pub fn memory(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let [list, detail] = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let w = list.width.saturating_sub(4) as usize;
    let mut items: Vec<ListItem> = s
        .tasks
        .iter()
        .map(|ts| {
            let pct = ts.progress.map(|p| format!(" {p}%")).unwrap_or_default();
            let style = if ts.active { t.text() } else { t.dim() };
            ListItem::new(Line::from(vec![
                Span::styled(format!("@{} ", ts.agent), t.fg(t.agent).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}{pct} ", ts.status), if ts.active { t.fg(t.warn) } else { t.dim() }),
                Span::styled(trunc(&ts.summary, w.saturating_sub(ts.agent.len() + 16)), style),
            ]))
        })
        .collect();
    items.extend(s.memory.entries.iter().map(|e| {
        ListItem::new(Line::from(vec![
            Span::styled("◇ ", t.accent()),
            Span::styled(trunc(&e.title, w.saturating_sub(20)), t.text().add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", e.tags.join(" ")), t.dim()),
        ]))
    }));
    let pending = if s.memory.open_proposals > 0 { format!(" · {} proposed", s.memory.open_proposals) } else { String::new() };
    let title = format!("Task state {} · Memory {}{pending}", s.tasks.len(), s.memory.entries.len());
    if items.is_empty() {
        f.render_widget(Paragraph::new(Line::styled("No task state or memory yet.", t.dim())).block(pane(&title, true, t)), list);
    } else {
        let lw = List::new(items).block(pane(&title, true, t)).highlight_style(t.selected());
        let mut st = ListState::default().with_selected(Some(s.memory_sel.index));
        f.render_stateful_widget(lw, list, &mut st);
    }
    let dw = detail.width.saturating_sub(4) as usize;
    let i = s.memory_sel.index;
    let mut lines: Vec<Line> = vec![];
    if let Some(ts) = s.tasks.get(i) {
        let chat = s.chats.iter().find(|c| c.id == ts.chat).map(|c| c.title.clone()).unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(format!("@{}", ts.agent), t.fg(t.agent).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", ts.status), t.accent()),
        ]));
        lines.push(Line::styled(format!("in {chat} · {}", ago_phrase(s.now, ts.at)), t.dim()));
        lines.push(Line::raw(""));
        lines.extend(super::wrap(&ts.summary, dw).into_iter().map(|l| Line::styled(l, t.text())));
        if let Some(p) = ts.progress {
            let filled = (p as usize * 20 / 100).min(20);
            lines.push(Line::styled(format!("[{}{}] {p}%", "█".repeat(filled), "░".repeat(20 - filled)), t.fg(t.good)));
        }
        if !ts.open_questions.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled("Open questions", t.accent()));
            for q in &ts.open_questions {
                lines.push(Line::styled(format!("  ? {q}"), t.text()));
            }
        }
    } else if let Some(e) = s.memory.entries.get(i - s.tasks.len().min(i)) {
        lines.push(Line::styled(e.title.clone(), t.accent()));
        let revs = if e.revisions > 1 { format!(" · {} revisions", e.revisions) } else { String::new() };
        lines.push(Line::styled(format!("by {} · {}{revs}", e.author, ago_phrase(s.now, e.at)), t.dim()));
        lines.push(Line::raw(""));
        lines.extend(super::wrap(&e.body, dw).into_iter().map(|l| Line::styled(l, t.text())));
        if !e.tags.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled(format!("tags: {}", e.tags.join(", ")), t.dim()));
        }
    } else {
        lines.push(Line::styled("Agents' task state and the workspace's shared memory. Read-only here.", t.dim()));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(pane("Detail", false, t)), detail);
}
