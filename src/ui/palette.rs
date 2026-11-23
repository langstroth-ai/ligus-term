//! The command palette: the query, then the matching entries with the key
//! that does the same and where it applies.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{pane, trunc};
use crate::app::input::TextBuf;
use crate::app::palette::filtered;
use crate::app::state::State;
use crate::theme::Theme;

pub fn draw(f: &mut Frame, area: Rect, s: &State, query: &TextBuf, sel: usize, t: &Theme) {
    let items = filtered(s, &query.text);
    let w = 76.min(area.width.saturating_sub(2)).max(20.min(area.width));
    let rows = (items.len().max(1) as u16).min(area.height.saturating_sub(6).max(1)).min(14);
    let h = rows + 4;
    let r = Rect { x: area.x + (area.width.saturating_sub(w)) / 2, y: area.y + 2.min(area.height.saturating_sub(h)), width: w, height: h };
    f.render_widget(Clear, r);
    let title = match items.len() {
        0 => "Commands".to_string(),
        n => format!("Commands · {n}"),
    };
    let block = pane(&title, true, t);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let iw = inner.width as usize;
    let prompt = Line::from(vec![Span::styled("› ", t.accent()), Span::styled(query.text.clone(), t.text())]);
    f.render_widget(Paragraph::new(prompt), Rect { height: 1, ..inner });
    f.set_cursor_position(Position::new(inner.x + 2 + query.cursor_pos().1 as u16, inner.y));
    f.render_widget(Paragraph::new(Line::styled("─".repeat(iw), t.dim())), Rect { y: inner.y + 1, height: 1, ..inner });
    let list = Rect { y: inner.y + 2, height: inner.height.saturating_sub(2), ..inner };
    if items.is_empty() {
        f.render_widget(Paragraph::new(Line::styled("Nothing matches. esc closes.", t.dim())), list);
        return;
    }
    let n = list.height as usize;
    let first = sel.saturating_sub(n.saturating_sub(1));
    let shown: Vec<_> = items.iter().enumerate().skip(first).take(n).collect();
    // Two right-hand columns: the key, then where it applies.
    let kw = shown.iter().map(|(_, it)| it.key.width()).max().unwrap_or(0).min(10);
    let sw = shown.iter().map(|(_, it)| it.scope.width()).max().unwrap_or(0).min(12);
    let mut lines = vec![];
    for (i, it) in shown {
        let key = trunc(&it.key, kw);
        let right = format!("{key:<kw$}  {:>sw$}", trunc(&it.scope, sw));
        let label_w = iw.saturating_sub(right.width() + 4);
        let label = trunc(&it.label, label_w);
        let pad = iw.saturating_sub(2 + label.width() + right.width());
        let selected = i == sel;
        let mark = if selected { "▶ " } else { "  " };
        let style = if selected { t.selected() } else { t.text() };
        lines.push(Line::from(vec![
            Span::styled(mark, if selected { t.accent() } else { t.dim() }),
            Span::styled(format!("{label}{}", " ".repeat(pad)), style),
            Span::styled(right, if selected { t.selected() } else { t.dim() }),
        ]));
    }
    f.render_widget(Paragraph::new(lines), list);
}
