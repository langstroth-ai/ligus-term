//! Drawing. Every function here reads [`State`] and writes to a frame;
//! nothing changes state.

mod chat;
mod onboarding;
mod overlays;
mod proposals;
mod splash;
mod views;

pub use proposals::apply_text;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::state::{Focus, Phase, State, ToastKind, View};
use crate::backend::model::initials;
use crate::keymap::{Action, Context};
use crate::theme::Theme;

pub fn draw(f: &mut Frame, s: &State, t: &Theme) {
    let area = f.area();
    match &s.phase {
        Phase::Loading => {
            let p = Paragraph::new(Line::styled("Starting Ligus…", t.dim())).centered();
            f.render_widget(p, center(area, 30, 1));
        }
        Phase::Onboarding(form) => onboarding::draw_form(f, area, s, form, t, true),
        Phase::Recovery(c) => onboarding::draw_recovery(f, area, c, t),
        Phase::Main => draw_main(f, area, s, t),
    }
    overlays::draw(f, area, s, t);
    draw_toasts(f, area, s, t);
}

fn draw_main(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let [top, body, status] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(area);
    draw_top_bar(f, top, s, t);
    match s.view {
        View::Chats => chat::draw(f, body, s, t),
        View::Proposals => proposals::draw(f, body, s, t),
        View::Agents => views::agents(f, body, s, t),
        View::Hosts => views::hosts(f, body, s, t),
        View::People => views::people(f, body, s, t),
        View::Notifications => views::notifications(f, body, s, t),
        View::Settings => views::settings(f, body, s, t),
        View::Workflows => views::workflows(f, body, s, t),
        View::Memory => views::memory(f, body, s, t),
    }
    draw_status(f, status, s, t);
}

/// `[AT] Apiary Tracker · owner │ Chats  Proposals …` with a mention dot on
/// other workspaces.
fn draw_top_bar(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let mut spans = vec![];
    for (i, w) in s.workspaces.iter().enumerate() {
        let active = s.active.as_ref() == Some(&w.id);
        let tile = format!("[{}]", initials(&w.name));
        let style = if active { t.accent().add_modifier(Modifier::REVERSED) } else { t.dim() };
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(tile, style));
        if w.mentioned && !active {
            spans.push(Span::styled("•", t.fg(t.mention)));
        }
    }
    if let Some(w) = s.active_workspace() {
        spans.push(Span::styled(format!(" {} ", w.name), t.accent()));
        spans.push(Span::styled(format!("· {} ", w.your_role.label()), t.dim()));
    }
    spans.push(Span::styled("│ ", t.dim()));
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let label = |v: View, short: bool| {
        let mut l = if short { v.short() } else { v.title() }.to_string();
        if v == View::Notifications {
            let n = s.unread_notifications();
            if n > 0 {
                l = format!("{l} {n}");
            }
        }
        l
    };
    // Full titles two apart when they fit, else shorter ones one apart.
    let fits =
        |short: bool, gap: usize| View::ALL.iter().map(|v| label(*v, short).width() + gap).sum::<usize>() + used <= area.width as usize;
    let (short, gap) = [(false, 2), (false, 1), (true, 1)].into_iter().find(|(s, g)| fits(*s, *g)).unwrap_or((true, 1));
    for v in View::ALL {
        let style = if v == s.view { t.accent().add_modifier(Modifier::UNDERLINED) } else { t.text() };
        spans.push(Span::styled(label(v, short), style));
        spans.push(Span::raw(" ".repeat(gap)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Key hints for what has focus, and the device on the right.
fn draw_status(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let k = &s.keymap;
    let h = |ctx, a, label: &str| format!("{} {label}", k.hint(ctx, a));
    use Action::*;
    use Context as C;
    let hints: Vec<String> = match (s.view, s.focus) {
        (View::Chats, Focus::Composer) => vec![
            h(C::Composer, Send, "send"),
            h(C::Composer, Newline, "newline"),
            "@ mention".into(),
            h(C::Composer, CancelCompose, "leave"),
        ],
        (View::Chats, _) => vec![
            "↑↓ chats".into(),
            h(C::Chat, Compose, "write"),
            h(C::Chat, NewChat, "new chat"),
            h(C::Global, FocusNext, "pane"),
            h(C::Global, SwitchWorkspace, "workspaces"),
            h(C::Global, Help, "help"),
        ],
        (View::Proposals, Focus::Detail) => vec![
            "↑↓ scroll".into(),
            h(C::Diff, NextHunk, "next hunk"),
            h(C::Diff, PrevHunk, "prev hunk"),
            h(C::Review, Approve, "approve"),
            h(C::Review, Reject, "reject"),
            h(C::Review, ApplyOnHost, "apply"),
        ],
        (View::Proposals, _) => vec![
            "↑↓ proposals".into(),
            h(C::Nav, Select, "diff"),
            h(C::Review, Approve, "approve"),
            h(C::Review, Reject, "reject"),
            h(C::Review, ApplyOnHost, "apply"),
            h(C::Global, Help, "help"),
        ],
        (View::People, _) => vec![
            "↑↓ people".into(),
            h(C::People, ChangeRole, "role"),
            h(C::People, RemoveMember, "remove"),
            h(C::People, Invite, "invite"),
            h(C::People, AdmitBox, "admit box"),
            h(C::Global, Help, "help"),
        ],
        (View::Agents, _) => vec![
            "↑↓ agents".into(),
            h(C::Agents, ChangeRuntime, "runtime"),
            h(C::Agents, ToggleAccess, "who can use"),
            h(C::Agents, AddAgent, "add agent"),
            h(C::Global, Help, "help"),
        ],
        (View::Notifications, _) => {
            vec!["↑↓".into(), h(C::Nav, Select, "open"), h(C::Notifications, MarkAllRead, "mark all read"), h(C::Global, Help, "help")]
        }
        (View::Settings, _) => vec!["↑↓".into(), h(C::Settings, Toggle, "toggle"), h(C::Global, Help, "help")],
        (View::Hosts, _) => vec!["↑↓ hosts".into(), h(C::Hosts, BindCheckout, "bind checkout"), h(C::Global, Help, "help")],
        (View::Workflows, _) => vec![
            "↑↓".into(),
            h(C::Workflows, StartRun, "start"),
            h(C::Workflows, ApproveGate, "approve gate"),
            h(C::Workflows, RejectGate, "reject"),
            h(C::Workflows, CancelRun, "cancel"),
            h(C::Global, Help, "help"),
        ],
        (View::Memory, _) => vec!["↑↓ task state and memory".into(), h(C::Global, Help, "help")],
    };
    let right = format!("{} · {}", s.device.device_name, s.device.backend_label);
    let left = hints.join("  ");
    let pad = (area.width as usize).saturating_sub(left.width() + right.width() + 1);
    let line = Line::from(vec![Span::styled(left, t.dim()), Span::raw(" ".repeat(pad)), Span::styled(right, t.dim())]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_toasts(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let n = s.toasts.len() as u16;
    if n == 0 || area.height < n + 2 {
        return;
    }
    for (i, toast) in s.toasts.iter().enumerate() {
        let (mark, color) = match toast.kind {
            ToastKind::Info => ("•", t.accent),
            ToastKind::Ok => ("✓", t.good),
            ToastKind::Error => ("✗", t.bad),
        };
        let text = format!(" {mark} {} ", toast.text);
        let w = (text.width() as u16).min(area.width);
        let y = area.bottom().saturating_sub(1 + n - i as u16);
        let r = Rect { x: area.right().saturating_sub(w), y, width: w, height: 1 };
        f.render_widget(Clear, r);
        let style = if t.mono {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(color).add_modifier(Modifier::BOLD | Modifier::REVERSED)
        };
        f.render_widget(Paragraph::new(Line::styled(text, style)), r);
    }
}

/// A bordered pane with a title; the border lights up when focused.
pub(crate) fn pane<'a>(title: &'a str, focused: bool, t: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(if focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(t.border(focused))
        .title(Span::styled(format!(" {title} "), if focused { t.accent() } else { t.text() }))
}

/// A centered rect of at most `w`×`h` inside `area`.
pub(crate) fn center(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

/// Greedy word wrap to `width` display columns; long words are split.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = vec![];
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let mut word = word.to_string();
            loop {
                let sep = if line.is_empty() { 0 } else { 1 };
                if line.width() + sep + word.width() <= width {
                    if sep == 1 {
                        line.push(' ');
                    }
                    line.push_str(&word);
                    break;
                }
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                    continue;
                }
                // A word wider than the line: split it.
                let mut cut = String::new();
                let mut rest = String::new();
                for c in word.chars() {
                    if rest.is_empty() && (cut.clone() + &c.to_string()).width() <= width {
                        cut.push(c);
                    } else {
                        rest.push(c);
                    }
                }
                out.push(cut);
                word = rest;
                if word.is_empty() {
                    break;
                }
            }
        }
        out.push(line);
    }
    out
}

/// Truncate to `width` columns with an ellipsis.
pub(crate) fn trunc(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.to_string().width() + 1 > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words_and_splits_long_ones() {
        assert_eq!(wrap("the quick brown fox", 9), vec!["the quick", "brown fox"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
        assert_eq!(trunc("hello world", 6), "hello…");
    }
}
