//! Proposals: the list, the votes, approve/reject, "Apply on ‹host›", and
//! a diff viewer that scrolls and jumps between hunks and files.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use super::{pane, trunc};
use crate::app::state::{DiffRow, Focus, State, ago};
use crate::backend::model::{Allowed, ApplyState, DiffLine, Proposal, ProposalStatus};
use crate::keymap::{Action, Context};
use crate::theme::Theme;

pub(crate) fn status_style(st: ProposalStatus, t: &Theme) -> ratatui::style::Style {
    let c = match st {
        ProposalStatus::Open => t.warn,
        ProposalStatus::Approved => t.good,
        ProposalStatus::Rejected => t.bad,
        ProposalStatus::Applied => t.hunk,
    };
    t.fg(c).add_modifier(Modifier::BOLD)
}

pub fn apply_text(a: &ApplyState) -> Option<String> {
    match a {
        ApplyState::NotRequested => None,
        ApplyState::Waiting { host } => Some(format!("Waiting for {host} to apply it")),
        ApplyState::Applied { host, commit } => Some(format!("Applied on {host} ({commit})")),
        ApplyState::Failed { host, reason } => Some(format!("{host} couldn't apply it: {reason}")),
        ApplyState::Withdrawn => Some("Withdrawn".into()),
    }
}

/// What a click or the wheel lands on in the Proposals view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalHit {
    /// The list: a row, or its empty space.
    List(Option<usize>),
    /// The detail and the diff.
    Detail,
}

/// What is under (`x`, `y`) in the Proposals view drawn in the body `area`.
pub fn proposal_hit(s: &State, area: Rect, x: u16, y: u16) -> ProposalHit {
    let left_w = (area.width * 2 / 5).clamp(30, 52);
    if x >= area.x + left_w {
        return ProposalHit::Detail;
    }
    let inner_h = area.height.saturating_sub(2);
    if y <= area.y || y >= area.bottom().saturating_sub(1) || s.proposals.is_empty() {
        return ProposalHit::List(None);
    }
    // Two lines a proposal; the list scrolls to keep the selection in view.
    let visible = (inner_h / 2).max(1) as usize;
    let offset = (s.proposal_sel.index + 1).saturating_sub(visible);
    let row = offset + ((y - area.y - 1) / 2) as usize;
    ProposalHit::List((row < s.proposals.len()).then_some(row))
}

pub fn draw(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let left_w = (area.width * 2 / 5).clamp(30, 52);
    let [list, right] = Layout::horizontal([Constraint::Length(left_w), Constraint::Min(30)]).areas(area);
    draw_list(f, list, s, t);
    let Some(p) = s.selected_proposal() else {
        let block = pane("Review", false, t);
        f.render_widget(Paragraph::new(Line::styled("Nothing is waiting for review.", t.dim())).block(block), right);
        return;
    };
    let detail_h = 7 + p.votes.len() as u16;
    let [detail, diff] = Layout::vertical([Constraint::Length(detail_h), Constraint::Min(3)]).areas(right);
    draw_detail(f, detail, s, p, t);
    draw_diff(f, diff, s, t);
}

fn draw_list(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let block = pane("Proposals", s.focus == Focus::List, t);
    if s.proposals.is_empty() {
        f.render_widget(Paragraph::new(Line::styled("Nothing is waiting for review.", t.dim())).block(block), area);
        return;
    }
    let w = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = s
        .proposals
        .iter()
        .map(|p| {
            let pill = format!("{:<9}", p.status.label());
            let l1 = Line::from(vec![
                Span::styled(pill, status_style(p.status, t)),
                Span::styled(trunc(&p.title, w.saturating_sub(10)), t.text()),
            ]);
            let mut meta = format!("         {}/{} approvals · {} · {}", p.approvals, p.quorum, p.author, ago(s.now, p.created_at));
            if matches!(p.apply, ApplyState::Waiting { .. }) {
                meta.push_str(" · applying");
            }
            ListItem::new(vec![l1, Line::styled(trunc(&meta, w), t.dim())])
        })
        .collect();
    let list = List::new(items).block(block).highlight_style(t.selected());
    let mut st = ListState::default().with_selected(Some(s.proposal_sel.index));
    f.render_stateful_widget(list, area, &mut st);
}

/// An action hint, crossed out with the reason when refused.
fn action<'a>(key: String, label: String, allowed: &Allowed, t: &Theme) -> Vec<Span<'a>> {
    match allowed {
        Allowed::Yes => vec![Span::styled(key, t.accent()), Span::styled(format!(" {label}   "), t.text())],
        Allowed::No(r) => vec![Span::styled(format!("{key} {label}"), t.disabled()), Span::styled(format!(" ({r})   "), t.dim())],
    }
}

fn draw_detail(f: &mut Frame, area: Rect, s: &State, p: &Proposal, t: &Theme) {
    let block = pane(&p.title, false, t);
    let k = &s.keymap;
    let mut lines = vec![Line::from(vec![
        Span::styled(p.status.label(), status_style(p.status, t)),
        Span::styled(
            format!(
                "  by {} · {} file{} changed · {}/{} approvals",
                p.author,
                p.files_changed,
                if p.files_changed == 1 { "" } else { "s" },
                p.approvals,
                p.quorum
            ),
            t.dim(),
        ),
    ])];
    for v in &p.votes {
        let (mark, verb, c) = if v.approve { ("✓", "approved", t.good) } else { ("✗", "rejected", t.bad) };
        lines.push(Line::from(vec![
            Span::styled(format!("{mark} "), t.fg(c)),
            Span::styled(format!("{} {verb} · {}", v.member, v.role.label()), t.text()),
        ]));
    }
    if p.needs_presence {
        lines.push(Line::styled("Presence (passkey) required", t.fg(t.warn)));
    }
    let vote = if p.status == ProposalStatus::Open { p.can_vote.clone() } else { Allowed::No(format!("already {}", p.status.label())) };
    let mut spans = vec![];
    let (ka, kr) = (k.hint(Context::Review, Action::Approve), k.hint(Context::Review, Action::Reject));
    match &vote {
        Allowed::Yes => {
            spans.extend(action(ka, "Approve".into(), &vote, t));
            spans.extend(action(kr, "Reject".into(), &vote, t));
        }
        // One reason for both buttons.
        Allowed::No(_) => spans.extend(action(format!("{ka} Approve  {kr}"), "Reject".into(), &vote, t)),
    }
    let host = s
        .default_apply_host()
        .map(|h| if h.this_device { "this device".to_string() } else { h.label.clone() })
        .unwrap_or_else(|| "a host".into());
    let apply_ok = match (&p.status, &p.apply) {
        (_, ApplyState::Waiting { .. }) => Allowed::No("one applier per proposal".into()),
        (ProposalStatus::Approved, _) => Allowed::Yes,
        (ProposalStatus::Applied, _) => Allowed::No("already applied".into()),
        _ => Allowed::No("needs approval first".into()),
    };
    let more = if s.hosts.len() > 1 { "…" } else { "" };
    spans.extend(action(k.hint(Context::Review, Action::ApplyOnHost), format!("Apply on {host}{more}"), &apply_ok, t));
    if matches!(p.apply, ApplyState::Waiting { .. }) {
        spans.extend(action(k.hint(Context::Review, Action::WithdrawApply), "Withdraw".into(), &Allowed::Yes, t));
    }
    lines.push(Line::from(spans));
    if let Some(a) = apply_text(&p.apply) {
        let c = match p.apply {
            ApplyState::Failed { .. } => t.bad,
            ApplyState::Applied { .. } => t.good,
            _ => t.warn,
        };
        lines.push(Line::styled(a, t.fg(c)));
    } else {
        lines.push(Line::styled("That host applies it once, on its checkout, and reports the result.", t.dim()));
    }
    f.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }).block(block), area);
}

fn draw_diff(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let focused = s.focus == Focus::Detail;
    let Some(d) = &s.diff else {
        f.render_widget(Paragraph::new(Line::styled("Loading the diff…", t.dim())).block(pane("Diff", focused, t)), area);
        return;
    };
    let rows = d.rows();
    let file_count = d.diff.files.len();
    let hunks_before = rows.iter().take(d.scroll + 1).filter(|r| matches!(r, DiffRow::Hunk(_))).count();
    let hunk_total = rows.iter().filter(|r| matches!(r, DiffRow::Hunk(_))).count();
    let title = format!(
        "Diff · {file_count} file{} · hunk {}/{}",
        if file_count == 1 { "" } else { "s" },
        hunks_before.max(1).min(hunk_total.max(1)),
        hunk_total
    );
    let block = pane(&title, focused, t);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if rows.is_empty() {
        f.render_widget(Paragraph::new(Line::styled("No file changes.", t.dim())), inner);
        return;
    }
    let w = inner.width as usize;
    let lines: Vec<Line> = rows
        .iter()
        .skip(d.scroll)
        .take(inner.height as usize)
        .map(|r| match r {
            DiffRow::File { path, added, removed } => Line::from(vec![
                Span::styled(format!("▌{path} "), t.text().add_modifier(Modifier::BOLD)),
                Span::styled(format!("+{added}"), t.fg(t.added)),
                Span::styled(format!(" −{removed}"), t.fg(t.removed)),
            ]),
            DiffRow::Hunk(h) => Line::styled(trunc(h, w), t.fg(t.hunk)),
            DiffRow::Line(DiffLine::Added(l)) => Line::styled(trunc(&format!("+{l}"), w), t.fg(t.added)),
            DiffRow::Line(DiffLine::Removed(l)) => Line::styled(trunc(&format!("-{l}"), w), t.fg(t.removed)),
            DiffRow::Line(DiffLine::Context(l)) => Line::styled(trunc(&format!(" {l}"), w), t.dim()),
            DiffRow::Blank => Line::raw(""),
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}
