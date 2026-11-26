//! Proposals: the list, the votes, approve/reject, "Apply on ‹host›", and
//! a diff viewer that scrolls and jumps between hunks and files.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use unicode_width::UnicodeWidthStr;

use super::{pane, trunc};
use crate::app::state::{DiffRow, Focus, State, ago};
use crate::backend::model::{ApplyState, DiffLine, Proposal, ProposalStatus, Shelf};
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
    if y <= area.y || y >= area.bottom().saturating_sub(1) || s.shown_proposals().is_empty() {
        return ProposalHit::List(None);
    }
    // One line a proposal; the list scrolls to keep the selection in view.
    let visible = inner_h.max(1) as usize;
    let offset = (s.proposal_sel.index + 1).saturating_sub(visible);
    let row = offset + (y - area.y - 1) as usize;
    ProposalHit::List((row < s.shown_proposals().len()).then_some(row))
}

pub fn draw(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let left_w = (area.width * 2 / 5).clamp(30, 52);
    let [list, right] = Layout::horizontal([Constraint::Length(left_w), Constraint::Min(30)]).areas(area);
    draw_list(f, list, s, t);
    let Some(p) = s.selected_proposal() else {
        let block = pane("Review", false, t);
        f.render_widget(Paragraph::new(Line::styled(empty_text(s), t.dim())).block(block), right);
        return;
    };
    let refused = p.status == ProposalStatus::Open && !p.can_vote.is_yes();
    let detail_h = 3
        + p.votes.len() as u16
        + u16::from(p.needs_presence)
        + u16::from(apply_text(&p.apply).is_some())
        + u16::from(refused)
        + u16::from(shelf_text(p.shelf).is_some());
    let [detail, diff] = Layout::vertical([Constraint::Length(detail_h), Constraint::Min(3)]).areas(right);
    draw_detail(f, detail, p, t);
    draw_diff(f, diff, s, t);
}

/// What the list says when it is empty.
fn empty_text(s: &State) -> &'static str {
    if s.show_archived { "Nothing archived or hidden." } else { "Nothing is waiting for review." }
}

/// Where an archived or hidden proposal is, for its detail.
pub fn shelf_text(shelf: Shelf) -> Option<&'static str> {
    match shelf {
        Shelf::Inbox => None,
        Shelf::Archived => Some("Archived for everyone"),
        Shelf::AutoArchived => Some("Archived automatically after it settled"),
        Shelf::Hidden => Some("Hidden on this device only"),
    }
}

fn draw_list(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let title = if s.show_archived { "Archived proposals" } else { "Proposals" };
    let block = pane(title, s.focus == Focus::List, t);
    if s.shown_proposals().is_empty() {
        f.render_widget(Paragraph::new(Line::styled(empty_text(s), t.dim())).block(block), area);
        return;
    }
    let w = area.width.saturating_sub(4) as usize;
    // The title, then where it stands: its status (in its colour), the
    // votes and its age.
    let items: Vec<ListItem> = s
        .shown_proposals()
        .iter()
        .map(|p| {
            let applying = if matches!(p.apply, ApplyState::Waiting { .. }) { " · applying" } else { "" };
            // Off the list: where it is instead of its votes (its status is
            // in the detail).
            let (label, meta, label_style) = if p.in_inbox() {
                let meta = format!(" {}/{} · {}{applying}", p.approvals, p.quorum, ago(s.now, p.created_at));
                (p.status.label(), meta, status_style(p.status, t).remove_modifier(Modifier::BOLD))
            } else {
                (p.shelf.label(), format!(" · {}", ago(s.now, p.created_at)), t.dim())
            };
            let tail = 2 + label.width() + meta.width();
            let title = trunc(&p.title, w.saturating_sub(tail).max(8));
            let pad = w.saturating_sub(title.width() + tail);
            let needs = p.status == ProposalStatus::Open && p.can_vote.is_yes();
            let style = if needs { t.text().add_modifier(Modifier::BOLD) } else { t.text() };
            ListItem::new(Line::from(vec![
                Span::styled(title, style),
                Span::raw(" ".repeat(pad + 2)),
                Span::styled(label, label_style),
                Span::styled(meta, t.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).block(block).highlight_style(t.selected());
    let mut st = ListState::default().with_selected(Some(s.proposal_sel.index));
    f.render_stateful_widget(list, area, &mut st);
}

fn draw_detail(f: &mut Frame, area: Rect, p: &Proposal, t: &Theme) {
    let block = pane(&p.title, false, t);
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
    // Why you can't vote stays in view (the actions are in the row menu).
    if let (ProposalStatus::Open, Some(r)) = (p.status, p.can_vote.reason()) {
        lines.push(Line::styled(format!("You can't vote: {r}"), t.dim()));
    }
    if let Some(a) = apply_text(&p.apply) {
        let c = match p.apply {
            ApplyState::Failed { .. } => t.bad,
            ApplyState::Applied { .. } => t.good,
            _ => t.warn,
        };
        lines.push(Line::styled(a, t.fg(c)));
    }
    if let Some(w) = shelf_text(p.shelf) {
        lines.push(Line::styled(w, t.dim()));
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
