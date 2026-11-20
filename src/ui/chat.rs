//! The chat list, the transcript with streaming replies, the lockdown bar,
//! which agents are thinking, and the composer with its @mention popup.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{pane, trunc, wrap};
use crate::app::state::{Focus, State, ago};
use crate::backend::model::{Allowed, Author, Message};
use crate::keymap::{Action, Context};
use crate::theme::Theme;

pub fn draw(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let side = if area.width >= 90 { 32 } else { 24 };
    let [list, main] = Layout::horizontal([Constraint::Length(side), Constraint::Min(20)]).areas(area);
    draw_list(f, list, s, t);
    draw_main(f, main, s, t);
}

fn draw_list(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let block = pane("Chats", s.focus == Focus::List, t);
    let inner_w = area.width.saturating_sub(4) as usize;
    if s.chats.is_empty() {
        let p = Paragraph::new(vec![
            Line::styled("No chats yet.", t.dim()),
            Line::styled(format!("{} start a chat", s.keymap.hint(Context::Chat, Action::NewChat)), t.dim()),
        ])
        .block(block);
        f.render_widget(p, area);
        return;
    }
    let items: Vec<ListItem> = s
        .chats
        .iter()
        .map(|c| {
            let dot = if c.mentioned {
                Span::styled("@ ", t.fg(t.mention).add_modifier(Modifier::BOLD))
            } else if c.unread {
                Span::styled("● ", t.fg(t.accent))
            } else {
                Span::raw("  ")
            };
            let age = ago(s.now, c.updated_at);
            let lock = if c.lockdown.is_some() { "⊘ " } else { "" };
            let title_w = inner_w.saturating_sub(age.width() + 3 + lock.width());
            let title = trunc(&c.title, title_w);
            let pad = title_w.saturating_sub(title.width());
            let mut spans = vec![dot];
            if !lock.is_empty() {
                spans.push(Span::styled(lock, t.fg(t.bad)));
            }
            let style = if c.unread { t.text().add_modifier(Modifier::BOLD) } else { t.text() };
            spans.extend([Span::styled(title, style), Span::raw(" ".repeat(pad + 1)), Span::styled(age, t.dim())]);
            let mut lines = vec![Line::from(spans)];
            if let Some(ch) = &c.channel {
                lines.push(Line::styled(format!("  #{ch}"), t.dim()));
            }
            ListItem::new(lines)
        })
        .collect();
    let list = List::new(items).block(block).highlight_style(t.selected());
    let mut st = ListState::default().with_selected(Some(s.chat_sel.index));
    f.render_stateful_widget(list, area, &mut st);
}

fn draw_main(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let chat = s.open_chat_summary();
    let lock_h = if chat.is_some_and(|c| c.lockdown.is_some()) { 2 } else { 0 };
    let comp_lines = s.composer.lines().len().clamp(1, 6) as u16;
    let [lock, transcript, thinking, composer] =
        Layout::vertical([Constraint::Length(lock_h), Constraint::Min(3), Constraint::Length(1), Constraint::Length(comp_lines + 2)])
            .areas(area);

    if let Some(l) = chat.and_then(|c| c.lockdown.as_ref()) {
        let reasons = l.reasons.join("; ");
        let action = match &l.clear {
            Allowed::Yes => format!("{} clear lockdown", s.keymap.hint(Context::Chat, Action::ClearLockdown)),
            Allowed::No(r) => r.clone(),
        };
        let lines = vec![
            Line::from(vec![
                Span::styled(" Locked down ", t.banner(t.bad).add_modifier(Modifier::REVERSED)),
                Span::styled(format!(" {reasons}"), t.fg(t.bad)),
            ]),
            Line::styled(format!(" no network for agents · every proposal needs a human · {action}"), t.fg(t.bad)),
        ];
        f.render_widget(Paragraph::new(lines), lock);
    }

    let title = chat.map(|c| c.title.as_str()).unwrap_or("No chat open");
    let block = pane(title, s.focus == Focus::Detail, t);
    let inner = block.inner(transcript);
    f.render_widget(block, transcript);
    if chat.is_some() && s.transcript.is_empty() {
        let lines = vec![
            Line::styled("Start a conversation", t.accent()),
            Line::styled("Ask about your code, or @mention an agent to have it make changes.", t.dim()),
        ];
        // The mark above the welcome when the pane has room for both.
        let with_mark = inner.width >= super::splash::WIDTH + 2 && inner.height >= super::splash::HEIGHT + 6;
        if with_mark {
            let r = super::center(inner, inner.width, super::splash::HEIGHT + 4);
            super::splash::draw(f, Rect { height: super::splash::HEIGHT, ..r }, t);
            let text = Rect { y: r.y + super::splash::HEIGHT + 2, height: 2, ..r };
            f.render_widget(Paragraph::new(lines).centered(), text);
        } else {
            f.render_widget(Paragraph::new(lines).centered(), super::center(inner, inner.width, 2));
        }
    } else {
        let lines = transcript_lines(s, inner.width as usize, t);
        let h = inner.height as usize;
        let scroll = s.transcript_scroll.min(lines.len().saturating_sub(h));
        let end = lines.len() - scroll;
        let start = end.saturating_sub(h);
        f.render_widget(Paragraph::new(lines[start..end].to_vec()), inner);
        if scroll > 0 {
            let more = format!(" ↓ {scroll} more ");
            let r = Rect {
                x: inner.right().saturating_sub(more.width() as u16),
                y: inner.bottom().saturating_sub(1),
                width: more.width() as u16,
                height: 1,
            };
            f.render_widget(Paragraph::new(Line::styled(more, t.dim().add_modifier(Modifier::REVERSED))), r);
        }
    }

    if let Some(c) = &s.open_chat {
        let parts: Vec<String> = s.thinking.get(c).map(|w| w.iter().map(|a| format!("{a} is thinking…")).collect()).unwrap_or_default();
        if !parts.is_empty() {
            f.render_widget(
                Paragraph::new(Line::styled(format!(" {}", parts.join("  ")), t.dim().add_modifier(Modifier::ITALIC))),
                thinking,
            );
        }
    }

    draw_composer(f, composer, s, t);
}

fn transcript_lines(s: &State, width: usize, t: &Theme) -> Vec<Line<'static>> {
    let mut out = vec![];
    let body_w = width.saturating_sub(2).max(10);
    for (i, m) in s.transcript.iter().enumerate() {
        if i > 0 {
            out.push(Line::raw(""));
        }
        out.push(header(s, m, t));
        for call in &m.tools {
            let status = if call.ok { Span::styled(" done", t.fg(t.good)) } else { Span::styled(" error", t.fg(t.bad)) };
            out.push(Line::from(vec![
                Span::styled("  ⚒ ", t.dim()),
                Span::styled(call.name.clone(), t.text()),
                Span::styled(format!(" {}", trunc(&call.preview, body_w.saturating_sub(call.name.width() + 10))), t.dim()),
                status,
            ]));
        }
        let mut text = m.text.clone();
        if m.streaming {
            text.push('▍');
        }
        if text.is_empty() {
            continue;
        }
        for l in wrap(&text, body_w) {
            let mut spans = vec![Span::raw("  ")];
            spans.extend(highlight_mentions(&l, t));
            out.push(Line::from(spans));
        }
    }
    out
}

fn header(s: &State, m: &Message, t: &Theme) -> Line<'static> {
    let mut spans = match &m.author {
        Author::Member { name, .. } => vec![Span::styled(name.clone(), t.text().add_modifier(Modifier::BOLD))],
        Author::Agent { handle, runtime, .. } => vec![
            Span::styled(format!("@{handle}"), t.fg(t.agent).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  via {runtime}"), t.dim()),
        ],
        Author::System => vec![Span::styled("ligus", t.dim())],
    };
    spans.push(Span::styled(format!("  {}", ago(s.now, m.at)), t.dim()));
    if m.streaming {
        spans.push(Span::styled("  generating", t.fg(t.warn)));
    }
    Line::from(spans)
}

fn highlight_mentions(line: &str, t: &Theme) -> Vec<Span<'static>> {
    let mut spans = vec![];
    for (i, word) in line.split(' ').enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        if word.starts_with('@') && word.len() > 1 {
            spans.push(Span::styled(word.to_string(), t.fg(t.mention).add_modifier(Modifier::BOLD)));
        } else {
            spans.push(Span::styled(word.to_string(), t.text()));
        }
    }
    spans
}

fn draw_composer(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let focused = s.focus == Focus::Composer;
    let block = pane("Message the room", focused, t);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if s.open_chat.is_none() {
        return;
    }
    if s.composer.text.is_empty() && !focused {
        let hint = format!("{} to write · @ mentions an agent or a person", s.keymap.hint(Context::Chat, Action::Compose));
        f.render_widget(Paragraph::new(Line::styled(hint, t.dim())), inner);
        return;
    }
    let lines = s.composer.lines();
    let (row, col) = s.composer.cursor_pos();
    let h = inner.height as usize;
    let first = row.saturating_sub(h.saturating_sub(1));
    let shown: Vec<Line> = lines.iter().skip(first).take(h).map(|l| Line::styled(l.to_string(), t.text())).collect();
    f.render_widget(Paragraph::new(shown), inner);
    if focused {
        let x = inner.x + (col as u16).min(inner.width.saturating_sub(1));
        let y = inner.y + (row - first) as u16;
        f.set_cursor_position(Position::new(x, y));
        draw_mentions(f, area, s, t);
    }
}

/// The @mention popup, just above the composer.
fn draw_mentions(f: &mut Frame, composer: Rect, s: &State, t: &Theme) {
    let c = s.mention_candidates();
    if c.is_empty() {
        return;
    }
    let h = c.len() as u16 + 2;
    let w = 36.min(composer.width);
    let r = Rect { x: composer.x + 2, y: composer.y.saturating_sub(h), width: w, height: h };
    f.render_widget(Clear, r);
    let items: Vec<ListItem> = c
        .iter()
        .map(|m| {
            // An agent you may not use: dimmed, with why.
            match &m.unavailable {
                None => ListItem::new(Line::from(vec![
                    Span::styled(format!("@{:<16}", m.handle), t.text()),
                    Span::styled(m.kind.clone(), t.dim()),
                ])),
                Some(why) => {
                    ListItem::new(Line::from(vec![Span::styled(format!("@{:<16}", m.handle), t.dim()), Span::styled(why.clone(), t.dim())]))
                }
            }
        })
        .collect();
    let list = List::new(items).block(pane("Mention", true, t)).highlight_style(t.selected());
    let mut st = ListState::default().with_selected(Some(s.mention_sel.min(c.len() - 1)));
    f.render_stateful_widget(list, r, &mut st);
}
