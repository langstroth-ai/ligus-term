//! Agents and runtimes, hosts, people, notifications, settings, workflows
//! and memory.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

use unicode_width::UnicodeWidthStr;

use super::{pane, trunc};
use crate::app::state::WorkflowItem;
use crate::app::state::{State, ago, ago_phrase};
use crate::backend::model::{AgentStatus, Allowed, HostKind, RunStatus, StageStatus, UNCONFIRMED};
use crate::keymap::{Action, Context};
use crate::theme::Theme;

fn host_badge(s: &State, host: Option<&crate::backend::model::HostId>) -> (String, bool) {
    match host.and_then(|id| s.hosts.iter().find(|h| &h.id == id)) {
        None => ("This device".into(), true),
        Some(h) if h.this_device => ("This device".into(), true),
        Some(h) => (format!("{} · {}", h.label, if h.online { "online" } else { "offline" }), h.online),
    }
}

/// A list beside the selected row's detail; in a narrow terminal the
/// detail goes under the list instead.
pub(crate) fn list_and_detail(area: Rect) -> [Rect; 2] {
    if area.width >= 72 {
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area)
    } else {
        Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area)
    }
}

/// Spans cut to `width` columns (the last one that doesn't fit ends in
/// "…").
pub(crate) fn fit_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut out = vec![];
    let mut used = 0;
    for sp in spans {
        let w = sp.content.width();
        if used + w <= width {
            used += w;
            out.push(sp);
            continue;
        }
        let room = width.saturating_sub(used);
        if room > 0 {
            out.push(Span::styled(trunc(&sp.content, room), sp.style));
        }
        break;
    }
    out
}

/// One-line rows: who or what each is, then one muted status in a column
/// after the longest name (at most half the row), cut to `width`.
pub(crate) fn one_line_rows(rows: Vec<(Vec<Span<'static>>, Vec<Span<'static>>)>, width: usize) -> Vec<ListItem<'static>> {
    let name_w = rows.iter().map(|(n, _)| spans_w(n)).max().unwrap_or(0).min(width / 2);
    rows.into_iter()
        .map(|(name, status)| {
            let mut spans = fit_spans(name, name_w);
            let pad = name_w.saturating_sub(spans_w(&spans)) + 2;
            spans.push(Span::raw(" ".repeat(pad)));
            spans.extend(fit_spans(status, width.saturating_sub(name_w + 2)));
            ListItem::new(Line::from(spans))
        })
        .collect()
}

fn spans_w(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// A label and its value on one line of a detail pane.
fn kv(label: &str, value: Vec<Span<'static>>, t: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{label:<12}"), t.dim())];
    spans.extend(value);
    Line::from(spans)
}

/// One line saying a list is empty, in its pane.
fn empty_pane(f: &mut Frame, area: Rect, title: &str, text: &str, focused: bool, t: &Theme) {
    f.render_widget(Paragraph::new(Line::styled(text.to_string(), t.dim())).block(pane(title, focused, t)), area);
}

pub fn agents(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let [list, detail] = list_and_detail(area);
    if s.agents.is_empty() {
        empty_pane(f, list, "Agents", "No agents yet · enter set one up", true, t);
    } else {
        let w = list.width.saturating_sub(4) as usize;
        let rows = s
            .agents
            .iter()
            .map(|a| {
                let name = match &a.unavailable {
                    Some(_) => Span::styled(format!("@{}", a.handle), t.dim()),
                    None => Span::styled(format!("@{}", a.handle), t.fg(t.agent).add_modifier(Modifier::BOLD)),
                };
                let mut status = vec![match a.status {
                    AgentStatus::Busy => Span::styled("busy", t.fg(t.warn)),
                    AgentStatus::Idle => Span::styled("idle", t.dim()),
                    AgentStatus::Offline => Span::styled("offline", t.dim()),
                }];
                // Where it runs, when that isn't here.
                if let Some(h) = a.host.as_ref().and_then(|id| s.hosts.iter().find(|h| &h.id == id)).filter(|h| !h.this_device) {
                    if h.online {
                        status.push(Span::styled(format!(" · {}", h.label), t.dim()));
                    } else {
                        status.push(Span::styled(format!(" · {} offline", h.label), t.fg(t.warn)));
                    }
                }
                (vec![name], status)
            })
            .collect();
        let lw = List::new(one_line_rows(rows, w)).block(pane("Agents", true, t)).highlight_style(t.selected());
        let mut st = ListState::default().with_selected(Some(s.agent_sel.index.min(s.agents.len() - 1)));
        f.render_stateful_widget(lw, list, &mut st);
    }
    let Some(a) = s.agents.get(s.agent_sel.index) else {
        f.render_widget(Paragraph::new(Line::raw("")).block(pane("Agent", false, t)), detail);
        return;
    };
    let dw = detail.width.saturating_sub(4) as usize;
    let (badge, ok) = host_badge(s, a.host.as_ref());
    let status = match a.status {
        AgentStatus::Busy => Span::styled("busy", t.fg(t.warn).add_modifier(Modifier::BOLD)),
        AgentStatus::Idle => Span::styled("idle", t.fg(t.good)),
        AgentStatus::Offline => Span::styled("offline", t.dim()),
    };
    let mut lines = vec![
        Line::from(vec![Span::styled(format!("@{}", a.handle), t.fg(t.agent).add_modifier(Modifier::BOLD)), Span::raw("  "), status]),
        Line::raw(""),
        kv("Runtime", vec![Span::styled(a.runtime.0.clone(), t.text())], t),
        kv("Runs on", vec![Span::styled(badge, if ok { t.text() } else { t.fg(t.warn) })], t),
        kv(
            "Who can use",
            vec![match &a.unavailable {
                Some(why) => Span::styled(why.clone(), t.fg(t.warn)),
                None => Span::styled(access_text(a), t.text()),
            }],
            t,
        ),
    ];
    if let Some(d) = a.doing.clone().or_else(|| a.role.clone()) {
        lines.push(kv("Doing", vec![Span::styled(trunc(&d, dw.saturating_sub(12)), t.text())], t));
    }
    lines.push(Line::raw(""));
    lines.extend(usage_lines(a, t));
    let handoffs: Vec<_> = s.handoffs.iter().filter(|h| h.from == a.handle || h.to == a.handle).collect();
    if !handoffs.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled("Hand-offs", t.dim()));
        for h in handoffs.iter().take(5) {
            let head = format!("@{} → @{} ", h.from, h.to);
            let age = format!(" {}", ago(s.now, h.at));
            lines.push(Line::from(vec![
                Span::styled(head.clone(), t.fg(t.agent)),
                Span::styled(trunc(&h.task, dw.saturating_sub(head.width() + age.width())), t.text()),
                Span::styled(age, t.dim()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(pane("Agent", false, t)), detail);
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

/// The agent's usage on this device (its owner's to see): per member,
/// turns and estimated cost, today and this month (UTC days).
fn usage_lines(a: &crate::backend::model::Agent, t: &Theme) -> Vec<Line<'static>> {
    let Some(u) = &a.usage else {
        return vec![kv("Usage", vec![Span::styled("its owner sees it", t.dim())], t)];
    };
    // One member a line, under the period's label.
    let rows = |label: &str, rows: &[crate::backend::model::UsageRow]| -> Vec<Line<'static>> {
        if rows.is_empty() {
            return vec![kv(label, vec![Span::styled("none", t.dim())], t)];
        }
        rows.iter()
            .enumerate()
            .map(|(i, r)| kv(if i == 0 { label } else { "" }, vec![Span::styled(format!("{}: {}", r.member, r.summary()), t.text())], t))
            .collect()
    };
    let mut out = rows(&format!("Today {}", u.day.get(5..).unwrap_or("")), &u.today);
    out.extend(rows("This month", &u.month));
    out
}

pub fn hosts(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let [table_area, note] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    if s.hosts.is_empty() {
        empty_pane(f, table_area, "Hosts", "No hosts yet · enter add a box", true, t);
    } else {
        let w = table_area.width.saturating_sub(4) as usize;
        let rows = s
            .hosts
            .iter()
            .map(|h| {
                let dot = if h.online { Span::styled("● ", t.fg(t.good)) } else { Span::styled("○ ", t.dim()) };
                let label = if h.this_device && h.label != "This device" { format!("{} (this device)", h.label) } else { h.label.clone() };
                let mut parts: Vec<Span<'static>> = vec![];
                let mut push = |sp: Span<'static>| {
                    if !parts.is_empty() {
                        parts.push(Span::styled(" · ", t.dim()));
                    }
                    parts.push(sp);
                };
                if h.kind == HostKind::Worker {
                    push(Span::styled("worker", t.dim()));
                }
                if !h.online {
                    push(Span::styled(h.last_seen.map(|at| ago_phrase(s.now, at)).unwrap_or_else(|| "never seen".into()), t.dim()));
                }
                if h.busy {
                    push(Span::styled("busy", t.fg(t.warn)));
                }
                if !h.agents.is_empty() {
                    push(Span::styled(h.agents.iter().map(|a| format!("@{a}")).collect::<Vec<_>>().join(" "), t.fg(t.agent)));
                }
                if !h.repos.is_empty() {
                    push(Span::styled(h.repos.join(" "), t.dim()));
                }
                (vec![dot, Span::styled(label, t.text().add_modifier(Modifier::BOLD))], parts)
            })
            .collect();
        let lw = List::new(one_line_rows(rows, w)).block(pane("Hosts", true, t)).highlight_style(t.selected());
        let mut st = ListState::default().with_selected(Some(s.host_sel.index));
        f.render_stateful_widget(lw, table_area, &mut st);
    }
    // This device's checkout stays in view: without one it can't apply.
    let bind = s.keymap.hint(Context::Hosts, Action::BindCheckout);
    let w = note.width as usize;
    let checkout = match &s.checkout {
        Some(p) => vec![Span::styled(" Applies proposals in ", t.dim()), Span::styled(p.clone(), t.text())],
        None => vec![
            Span::styled(" No checkout bound: this device can't apply proposals ", t.fg(t.warn)),
            Span::styled(format!("· {bind} bind"), t.dim()),
        ],
    };
    f.render_widget(Paragraph::new(Line::from(fit_spans(checkout, w))), note);
}

/// "This terminal is its own member · D link to my devices": one muted
/// line, while this install is a member of its own.
fn own_member_line(s: &State, ctx: Context, indent: &str, t: &Theme) -> Line<'static> {
    let key = s.keymap.hint(ctx, Action::LinkDevice);
    Line::from(vec![
        Span::styled(format!("{indent}This terminal is its own member"), t.dim()),
        Span::styled(format!(" · {key} link to my devices"), t.dim()),
    ])
}

pub fn people(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    // Someone here looks like you on your desktop: say so on one line.
    let area = if s.likely_me_elsewhere().is_some() {
        let [list, note] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
        f.render_widget(Paragraph::new(own_member_line(s, Context::People, " ", t)), note);
        list
    } else {
        area
    };
    if s.members.is_empty() {
        return empty_pane(f, area, "People", "Nobody else here yet · enter invite someone", true, t);
    }
    let w = area.width.saturating_sub(4) as usize;
    let rows = s
        .members
        .iter()
        .map(|m| {
            let mut name = vec![Span::styled(m.name.clone(), t.text().add_modifier(Modifier::BOLD))];
            if m.you {
                name.push(Span::styled(" (you)", t.dim()));
            }
            let mut status = vec![Span::styled(m.role.label(), t.dim())];
            if m.founder {
                status.push(Span::styled(" · founder", t.dim()));
            }
            if m.service {
                status.push(Span::styled(" · box", t.dim()));
            }
            (name, status)
        })
        .collect();
    let mut items = one_line_rows(rows, w);
    // Just you: the view's one action is inviting someone.
    if s.members.len() == 1 {
        items.push(ListItem::new(Line::raw("")));
        items.push(ListItem::new(Line::styled("Nobody else here yet · enter invite someone", t.dim())));
    }
    let lw = List::new(items).block(pane("People", true, t)).highlight_style(t.selected());
    let mut st = ListState::default().with_selected(Some(s.member_sel.index));
    f.render_stateful_widget(lw, area, &mut st);
}

pub fn notifications(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let block = pane("Notifications", true, t);
    if s.notifications.is_empty() {
        f.render_widget(Paragraph::new(Line::styled("No notifications", t.dim())).block(block), area);
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
        let mut spans = vec![Span::styled(format!(" {mark} {label} "), style)];
        if !note.is_empty() {
            spans.push(Span::styled(format!(" {note}"), t.dim()));
        }
        Line::from(spans)
    };
    let kv = |k: &str, v: String| Line::from(vec![Span::styled(format!("  {k:<16}"), t.dim()), Span::styled(v, t.text())]);
    let head = |h: &str| Line::styled(h.to_string(), t.accent());
    let d = &s.device;
    let lines = vec![
        head("You"),
        kv("Name", if st.your_name.is_empty() { "—".into() } else { st.your_name.clone() }),
        kv("Safety code", d.safety_code.clone()),
    ];
    let mut lines = lines;
    match st.linked_to.as_ref() {
        Some(p) => {
            lines.push(kv("Device of", format!("{p} (linked to their devices)")));
            // One line, one action: why is in the toast and in R's answer.
            if st.link_unconfirmed.is_some() {
                lines.push(Line::from(vec![Span::styled(format!("  {UNCONFIRMED}"), t.fg(t.warn)), Span::styled(" · R retry", t.dim())]));
            }
        }
        None if s.own_member_unlinked() => lines.push(own_member_line(s, Context::Settings, "  ", t)),
        None => {}
    }
    // A link started and not finished.
    if let Some(line) = s.link_pending_line() {
        lines.push(Line::from(Span::styled(format!("  {line}"), t.accent())));
    }
    lines.extend([
        Line::raw(""),
        head("Notifications and agents"),
        toggle(0, st.notify_mentions, "Notify me when I'm @mentioned or needed", ""),
        // What it reaches stays beside it: it lets agents change files.
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
            Line::from(vec![Span::styled(format!(" Theme       {} ", s.theme_name), style), Span::styled("  this session", t.dim())])
        },
        {
            let sel = s.settings_sel.index == 3;
            let style = if sel { t.selected() } else { t.text() };
            Line::from(vec![Span::styled(format!(" Appearance  {} ", s.appearance_name), style)])
        },
        kv("Config file", s.config_path.clone().unwrap_or_else(|| "none".into())),
    ]);
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
    let [list, detail] = list_and_detail(area);
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
        empty_pane(f, list, &title, "No workflows yet", true, t);
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
            // Why it can't start stays in view; starting is the footer's key.
            if let Allowed::No(r) = &d.can_start {
                lines.push(Line::raw(""));
                lines.push(Line::styled(format!("Can't start here: {r}"), t.fg(t.warn)));
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
            // A gate waiting on a person keeps its one action in view.
            if r.stages.iter().any(|st| st.status == StageStatus::AwaitingApproval) {
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::styled("◆ Waiting for a sign-off  ", t.fg(t.warn)),
                    Span::styled(hint(Action::ApproveGate), t.accent()),
                    Span::styled(" approve", t.text()),
                ]));
            }
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(pane("Detail", false, t)), detail);
}

/// Agents' live task state, then the workspace memory.
pub fn memory(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let [list, detail] = list_and_detail(area);
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
        empty_pane(f, list, &title, "No task state or memory yet", true, t);
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
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(pane("Detail", false, t)), detail);
}
