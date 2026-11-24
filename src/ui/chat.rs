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
use crate::backend::model::{Allowed, Author, Message, SessionState};
use crate::keymap::{Action, Context};
use crate::theme::Theme;

/// Below this many columns the chat list shows only while it has the
/// keyboard (`esc` or `ctrl-l`), and otherwise the transcript has the width.
pub const LIST_OVERLAY_BELOW: u16 = 100;
/// From this many columns a third column shows what the chat has going on.
pub const RAIL_FROM: u16 = 160;
/// The transcript's longest line, so a wide terminal doesn't make
/// 170-column lines.
pub const READING_WIDTH: u16 = 100;

/// What a click or the wheel at a point of the Chats view lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatHit {
    /// A row of the chat list (an index into `chats`), or the list's
    /// empty space.
    List(Option<usize>),
    Transcript,
    Composer,
    Rail,
}

/// The chat list, transcript and composer areas for a body `area`, as
/// `draw` lays them out.
fn areas(area: Rect, s: &State) -> (Option<Rect>, Rect, Option<Rect>) {
    if area.width < LIST_OVERLAY_BELOW {
        if s.focus == Focus::List {
            let [list, main] = Layout::horizontal([Constraint::Length(28.min(area.width / 2)), Constraint::Min(20)]).areas(area);
            return (Some(list), main, None);
        }
        return (None, area, None);
    }
    if area.width >= RAIL_FROM {
        let [list, main, rail] = Layout::horizontal([Constraint::Length(32), Constraint::Min(40), Constraint::Length(40)]).areas(area);
        return (Some(list), main, Some(rail));
    }
    let [list, main] = Layout::horizontal([Constraint::Length(32), Constraint::Min(20)]).areas(area);
    (Some(list), main, None)
}

/// The rows a list scrolled to keep `selected` in view starts at, as
/// ratatui's `List` does from a fresh state.
fn list_offset(heights: &[u16], selected: usize, inner_h: u16) -> usize {
    let mut offset = 0;
    while offset < selected && heights[offset..=selected.min(heights.len().saturating_sub(1))].iter().sum::<u16>() > inner_h {
        offset += 1;
    }
    offset
}

/// What is under (`x`, `y`) in the Chats view drawn in the body `area`.
pub fn chat_hit(s: &State, area: Rect, x: u16, y: u16) -> Option<ChatHit> {
    let (list, main, rail) = areas(area, s);
    let inside = |r: Rect| x >= r.x && x < r.right() && y >= r.y && y < r.bottom();
    if let Some(l) = list.filter(|l| inside(*l)) {
        let heights: Vec<u16> = s.chats.iter().map(|_| 1).collect();
        let inner_h = l.height.saturating_sub(2);
        if y <= l.y || y >= l.bottom().saturating_sub(1) || heights.is_empty() {
            return Some(ChatHit::List(None));
        }
        let mut row = list_offset(&heights, s.chat_sel.index, inner_h);
        let mut top = l.y + 1;
        while row < heights.len() {
            if y < top + heights[row] {
                return Some(ChatHit::List(Some(row)));
            }
            top += heights[row];
            row += 1;
        }
        return Some(ChatHit::List(None));
    }
    if rail.is_some_and(inside) {
        return Some(ChatHit::Rail);
    }
    if inside(main) {
        let comp_lines = s.composer.lines().len().clamp(1, 6) as u16;
        let composer_top = main.bottom().saturating_sub(comp_lines + 2);
        return Some(if y >= composer_top { ChatHit::Composer } else { ChatHit::Transcript });
    }
    None
}

pub fn draw(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    if area.width < LIST_OVERLAY_BELOW {
        // The list only while it has the keyboard; the transcript rewraps
        // beside it rather than being covered.
        if s.focus == Focus::List {
            let [list, main] = Layout::horizontal([Constraint::Length(28.min(area.width / 2)), Constraint::Min(20)]).areas(area);
            draw_list(f, list, s, t);
            draw_main(f, main, s, t);
        } else {
            draw_main(f, area, s, t);
        }
        return;
    }
    if area.width >= RAIL_FROM {
        let [list, main, rail] = Layout::horizontal([Constraint::Length(32), Constraint::Min(40), Constraint::Length(40)]).areas(area);
        draw_list(f, list, s, t);
        draw_main(f, main, s, t);
        draw_rail(f, rail, s, t);
        return;
    }
    let [list, main] = Layout::horizontal([Constraint::Length(32), Constraint::Min(20)]).areas(area);
    draw_list(f, list, s, t);
    draw_main(f, main, s, t);
}

/// The third column on a wide terminal: who is working in this chat, the
/// proposals it produced and the runs it belongs to.
fn draw_rail(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let block = pane("This chat", false, t);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(chat) = s.open_chat.as_ref() else { return };
    let w = inner.width as usize;
    let mut lines: Vec<Line> = vec![];
    let working: Vec<&String> = s.thinking.get(chat).map(|x| x.iter().collect()).unwrap_or_default();
    if !working.is_empty() {
        lines.push(Line::styled("Working", t.dim()));
        for a in working {
            let glyph = super::state_glyph(s, SessionState::Working, t);
            lines.push(Line::from(vec![
                Span::styled(format!("{glyph} "), t.state(SessionState::Working)),
                Span::styled(trunc(a, w - 2), t.text()),
            ]));
        }
        lines.push(Line::raw(""));
    }
    let props: Vec<_> = s.proposals.iter().filter(|p| p.chat.as_ref() == Some(chat)).collect();
    if !props.is_empty() {
        lines.push(Line::styled("Proposals", t.dim()));
        for p in props {
            let needs = p.status == crate::backend::model::ProposalStatus::Open && p.can_vote.is_yes();
            let (glyph, style) =
                if needs { (SessionState::NeedsYou.glyph(t.ascii), t.state(SessionState::NeedsYou)) } else { ("·", t.dim()) };
            let state = format!(" {}/{}", p.approvals, p.quorum);
            lines.push(Line::from(vec![
                Span::styled(format!("{glyph} "), style),
                Span::styled(trunc(&p.title, w.saturating_sub(2 + state.width())), t.text()),
                Span::styled(state, t.dim()),
            ]));
        }
        lines.push(Line::raw(""));
    }
    let runs: Vec<_> = s.runs.iter().filter(|r| &r.chat == chat).collect();
    if !runs.is_empty() {
        lines.push(Line::styled("Runs", t.dim()));
        for r in runs {
            let state = format!(" {}", r.status.label());
            lines.push(Line::from(vec![
                Span::styled("· ", t.dim()),
                Span::styled(trunc(&r.name, w.saturating_sub(2 + state.width())), t.text()),
                Span::styled(state, t.dim()),
            ]));
        }
    }
    if lines.is_empty() {
        lines.push(Line::styled("Nothing going on here.", t.dim()));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_list(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let block = pane("Chats", s.focus == Focus::List, t);
    let inner_w = area.width.saturating_sub(4) as usize;
    if s.chats.is_empty() {
        let p = Paragraph::new(Line::styled(format!("No chats yet · {} new", s.keymap.hint(Context::Chat, Action::NewChat)), t.dim()))
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
            // Where the chat stands, as a glyph (the colour only repeats it).
            let st = s.session_state(&c.id);
            let chip = if st == SessionState::Idle { String::new() } else { format!("{} ", super::state_glyph(s, st, t)) };
            // One line: the title, then its channel and age, muted.
            let room = inner_w.saturating_sub(age.width() + 3 + chip.width());
            let title = trunc(&c.title, room);
            let channel = c
                .channel
                .as_ref()
                .map(|ch| format!("#{ch} "))
                .filter(|ch| room.saturating_sub(title.width()) > ch.width())
                .unwrap_or_default();
            let pad = room.saturating_sub(title.width() + channel.width());
            let mut spans = vec![dot];
            if !chip.is_empty() {
                spans.push(Span::styled(chip, t.state(st)));
            }
            let style = if c.unread { t.text().add_modifier(Modifier::BOLD) } else { t.text() };
            spans.extend([
                Span::styled(title, style),
                Span::raw(" ".repeat(pad + 1)),
                Span::styled(channel, t.dim()),
                Span::styled(age, t.dim()),
            ]);
            ListItem::new(Line::from(spans))
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
        // In a narrow pane the reasons end in "…" and the way out comes
        // first, so it is never the part cut off.
        let w = lock.width as usize;
        let label = " Locked down ";
        let lines = vec![
            Line::from(vec![
                Span::styled(label, t.banner(t.bad).add_modifier(Modifier::REVERSED)),
                Span::styled(format!(" {}", trunc(&reasons, w.saturating_sub(label.width() + 1))), t.fg(t.bad)),
            ]),
            Line::styled(
                format!(" {}", super::fit_parts(&[&action, "no network for agents", "every proposal needs a human"], w.saturating_sub(1))),
                t.fg(t.bad),
            ),
        ];
        f.render_widget(Paragraph::new(lines), lock);
    }

    let mut title = chat.map(|c| c.title.clone()).unwrap_or_else(|| "No chat open".into());
    // With the list out of sight, say where this chat sits in it.
    if area.width < LIST_OVERLAY_BELOW && chat.is_some() && s.chats.len() > 1 {
        title = format!("{title} · {}/{}", s.chat_sel.index + 1, s.chats.len());
    }
    let block = pane(&title, s.focus == Focus::Detail, t);
    let inner = block.inner(transcript);
    // A comfortable line length, centred, however wide the pane.
    let inner = if inner.width > READING_WIDTH {
        Rect { x: inner.x + (inner.width - READING_WIDTH) / 2, width: READING_WIDTH, ..inner }
    } else {
        inner
    };
    f.render_widget(block, transcript);
    if chat.is_some() && s.transcript.is_empty() && !checklist(s).iter().all(|i| i.done) {
        draw_checklist(f, inner, s, t);
    } else if chat.is_some() && s.transcript.is_empty() {
        // The welcome wraps in a narrow pane rather than losing its end.
        let mut lines = vec![Line::styled("Start a conversation", t.accent())];
        let ask = "Ask about your code, or @mention an agent to have it make changes.";
        lines.extend(wrap(ask, inner.width as usize).into_iter().map(|l| Line::styled(l, t.dim())));
        let text_h = lines.len() as u16;
        let welcome = Paragraph::new(lines).centered();
        // The mark above the welcome when the pane has room for both.
        let with_mark = inner.width >= super::splash::WIDTH + 2 && inner.height >= super::splash::HEIGHT + 4 + text_h;
        if with_mark {
            let r = super::center(inner, inner.width, super::splash::HEIGHT + 2 + text_h);
            super::splash::draw(f, Rect { height: super::splash::HEIGHT, ..r }, t);
            let text = Rect { y: r.y + super::splash::HEIGHT + 2, height: text_h, ..r };
            f.render_widget(welcome, text);
        } else {
            f.render_widget(welcome, super::center(inner, inner.width, text_h));
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
    // Proposals from this chat, oldest first: each gets a link line after
    // the last message before it was filed.
    let mut props: Vec<&crate::backend::model::Proposal> =
        s.proposals.iter().filter(|p| p.chat.is_some() && p.chat.as_ref() == s.open_chat.as_ref()).collect();
    props.sort_by_key(|p| p.created_at);
    let mut next_prop = 0;
    for (i, m) in s.transcript.iter().enumerate() {
        if i > 0 {
            out.push(Line::raw(""));
        }
        out.push(header(s, m, t));
        for call in &m.tools {
            out.extend(tool_lines(call, s.tools_expanded, body_w, t));
        }
        let mut text = m.text.clone();
        if m.streaming {
            text.push('▍');
        }
        if !text.is_empty() {
            out.extend(markdown_lines(&text, body_w, t));
        }
        let next_at = s.transcript.get(i + 1).map(|n| n.at);
        while let Some(p) = props.get(next_prop) {
            if next_at.is_some_and(|at| p.created_at >= at) {
                break;
            }
            out.push(proposal_line(s, p, body_w, t));
            next_prop += 1;
        }
    }
    out
}

/// One step of a new workspace's getting-started list.
pub struct Step {
    pub done: bool,
    pub what: &'static str,
    pub keys: String,
    pub note: String,
}

/// A new workspace's first steps, read from its real state: agents,
/// people, boxes and workflow runs.
pub fn checklist(s: &State) -> Vec<Step> {
    use crate::backend::model::{HostKind, SyncMode};
    let k = &s.keymap;
    let two = |a: Action, b: (Context, Action)| format!("{} then {}", k.hint(Context::Global, a), k.hint(b.0, b.1));
    let found: Vec<&str> = s.available_local().iter().map(|l| l.kind.label()).collect();
    let found = if found.is_empty() { String::new() } else { format!("found here: {}", found.join(", ")) };
    let relay = s.active_workspace().is_some_and(|w| matches!(w.sync, SyncMode::Relay(_)));
    vec![
        Step { done: true, what: "Workspace created", keys: String::new(), note: String::new() },
        Step {
            done: !s.agents.is_empty(),
            what: "Add an agent",
            keys: two(Action::GoAgents, (Context::Agents, Action::AddAgent)),
            note: found,
        },
        Step {
            done: s.members.iter().filter(|m| !m.service).count() > 1,
            what: "Invite someone",
            keys: two(Action::GoPeople, (Context::People, Action::Invite)),
            note: if relay { String::new() } else { "needs a relay (LT_RELAY_URL)".into() },
        },
        Step {
            done: s.hosts.iter().any(|h| h.kind == HostKind::Worker),
            what: "Connect a box",
            keys: two(Action::GoPeople, (Context::People, Action::AdmitBox)),
            note: "ligus serve or ligus worker".into(),
        },
        Step {
            done: !s.runs.is_empty(),
            what: "Start a workflow",
            keys: two(Action::GoWorkflows, (Context::Workflows, Action::StartRun)),
            note: String::new(),
        },
    ]
}

/// The getting-started list in an empty chat of a new workspace, in place
/// of the welcome, until every step is done.
fn draw_checklist(f: &mut Frame, inner: Rect, s: &State, t: &Theme) {
    let name = s.active_workspace().map(|w| w.name.clone()).unwrap_or_default();
    let mut lines = vec![Line::styled(format!("{name}: getting started"), t.accent()), Line::raw("")];
    for st in checklist(s) {
        let (mark, style) = if st.done { ("✓ ", t.fg(t.good)) } else { ("○ ", t.dim()) };
        let mut spans = vec![Span::styled(mark, style), Span::styled(format!("{:<18}", st.what), if st.done { t.dim() } else { t.text() })];
        if !st.done {
            spans.push(Span::styled(format!("{:<10}", st.keys), t.accent()));
            if !st.note.is_empty() {
                spans.push(Span::styled(format!("  {}", st.note), t.dim()));
            }
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Or just type: ask about your code, or @mention an agent.", t.dim()));
    let w = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16;
    let h = lines.len() as u16;
    // The mark above the list when the pane has room for both.
    if inner.width >= super::splash::WIDTH.max(w) + 2 && inner.height >= super::splash::HEIGHT + 2 + h {
        let r = super::center(inner, inner.width, super::splash::HEIGHT + 2 + h);
        super::splash::draw(f, Rect { height: super::splash::HEIGHT, ..r }, t);
        let list = super::center(Rect { y: r.y + super::splash::HEIGHT + 2, height: h, ..r }, w, h);
        f.render_widget(Paragraph::new(lines), list);
        return;
    }
    let r = super::center(inner, w.min(inner.width), h.min(inner.height));
    f.render_widget(Paragraph::new(lines), r);
}

/// `  → proposal “Fix the clock” · open 1/2`: a proposal this chat produced.
fn proposal_line(s: &State, p: &crate::backend::model::Proposal, width: usize, t: &Theme) -> Line<'static> {
    let state = format!(" · {} {}/{}", p.status.label(), p.approvals, p.quorum);
    let needs = p.status == crate::backend::model::ProposalStatus::Open && p.can_vote.is_yes();
    let key = s.keymap.hint(Context::Global, Action::GoProposals);
    let tail = if needs { format!("{state} · {key} to review") } else { state };
    let title = trunc(&p.title, width.saturating_sub(tail.width() + 16));
    let mark = if needs {
        Span::styled(format!("  {} ", SessionState::NeedsYou.glyph(t.ascii)), t.state(SessionState::NeedsYou))
    } else {
        Span::styled("  → ", t.dim())
    };
    Line::from(vec![mark, Span::styled(format!("proposal \u{201c}{title}\u{201d}"), t.text()), Span::styled(tail, t.dim())])
}

/// One tool call: `▸ read_file  src/checkout.rs  done 0.2s`, and with
/// `o`, `▾` and the first lines it returned.
fn tool_lines(call: &crate::backend::model::ToolCall, expanded: bool, width: usize, t: &Theme) -> Vec<Line<'static>> {
    let took = call
        .duration_ms
        .map(|ms| if ms < 1000 { format!(" {:.1}s", ms as f64 / 1000.0) } else { format!(" {}s", ms / 1000) })
        .unwrap_or_default();
    let (word, style) = if call.ok { ("done", t.fg(t.good)) } else { ("error", t.fg(t.bad)) };
    let status = format!(" {word}{took}");
    let has_output = !call.output.trim().is_empty();
    let arrow = if expanded && has_output { "▾ " } else { "▸ " };
    let preview_w = width.saturating_sub(call.name.width() + status.width() + 6);
    let mut out = vec![Line::from(vec![
        Span::styled(format!("  {arrow}"), t.dim()),
        Span::styled(call.name.clone(), t.text()),
        Span::styled(format!(" {}", trunc(&call.preview, preview_w)), t.dim()),
        Span::styled(status, style),
    ])];
    if expanded && has_output {
        for l in call.output.lines().take(6) {
            out.push(Line::styled(format!("      {}", trunc(l, width.saturating_sub(6))), t.dim()));
        }
    }
    out
}

/// A message body with the little Markdown agents write: `**bold**`,
/// `` `code` ``, `# headings`, `-`/`*` lists as bullets, and fenced code
/// blocks in a box labelled with their language. Code isn't wrapped; long
/// lines end in "…".
fn markdown_lines(text: &str, width: usize, t: &Theme) -> Vec<Line<'static>> {
    let mut out = vec![];
    let mut fence: Option<usize> = None;
    let code_w = width.saturating_sub(4).max(8);
    for raw in text.split('\n') {
        let trimmed = raw.trim_start();
        if let Some(lang) = trimmed.strip_prefix("```") {
            if fence.is_some() {
                out.push(Line::styled(format!("  └{}", "─".repeat(code_w)), t.dim()));
                fence = None;
            } else {
                let label = lang.trim();
                let head = if label.is_empty() { String::new() } else { format!(" {label} ") };
                out.push(Line::styled(format!("  ┌{head}{}", "─".repeat(code_w.saturating_sub(head.width()))), t.dim()));
                fence = Some(out.len());
            }
            continue;
        }
        if fence.is_some() {
            out.push(Line::from(vec![Span::styled("  │ ", t.dim()), Span::styled(trunc(raw, code_w.saturating_sub(1)), t.fg(t.hunk))]));
            continue;
        }
        let (prefix, rest, bold) = if let Some(r) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            let indent = raw.len() - trimmed.len();
            (format!("{}• ", " ".repeat(indent.min(6))), r, false)
        } else if let Some(r) = trimmed.strip_prefix('#') {
            (String::new(), r.trim_start_matches('#').trim_start(), true)
        } else {
            (String::new(), raw, false)
        };
        let lines = wrap(rest, width.saturating_sub(prefix.width()).max(4));
        for (i, l) in lines.into_iter().enumerate() {
            let lead = if i == 0 { prefix.clone() } else { " ".repeat(prefix.width()) };
            let mut spans = vec![Span::raw(format!("  {lead}"))];
            spans.extend(inline_spans(&l, bold, t));
            out.push(Line::from(spans));
        }
    }
    // An unclosed fence (a reply still streaming) gets no bottom edge.
    out
}

/// `**bold**` and `` `code` `` inside one line, with @mentions picked out.
fn inline_spans(line: &str, bold: bool, t: &Theme) -> Vec<Span<'static>> {
    let mut spans = vec![];
    let base = if bold { t.text().add_modifier(Modifier::BOLD) } else { t.text() };
    let mut rest = line;
    while !rest.is_empty() {
        let code = rest.find('`');
        let strong = rest.find("**");
        let next = match (code, strong) {
            (Some(c), Some(b)) => Some(c.min(b)),
            (a, b) => a.or(b),
        };
        let Some(at) = next else {
            spans.extend(mention_spans(rest, base, t));
            break;
        };
        spans.extend(mention_spans(&rest[..at], base, t));
        let is_code = code == Some(at);
        let marker = if is_code { "`" } else { "**" };
        let after = &rest[at + marker.len()..];
        match after.find(marker) {
            Some(end) if end > 0 => {
                let inner = after[..end].to_string();
                let style = if is_code { t.fg(t.hunk) } else { base.add_modifier(Modifier::BOLD) };
                spans.push(Span::styled(inner, style));
                rest = &after[end + marker.len()..];
            }
            _ => {
                spans.push(Span::styled(marker.to_string(), base));
                rest = after;
            }
        }
    }
    spans
}

fn header(s: &State, m: &Message, t: &Theme) -> Line<'static> {
    let mut spans = match &m.author {
        Author::Member { name, .. } => vec![Span::styled(name.clone(), t.text().add_modifier(Modifier::BOLD))],
        Author::Agent { handle, runtime, .. } => vec![
            Span::styled(format!("@{handle}"), t.fg(t.agent).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  via {runtime}"), t.dim()),
            Span::styled(runs_on(s, handle).map(|h| format!(" · on {h}")).unwrap_or_default(), t.dim()),
        ],
        Author::System => vec![Span::styled("ligus", t.dim())],
    };
    spans.push(Span::styled(format!("  {}", ago(s.now, m.at)), t.dim()));
    if m.streaming {
        // One spinner cell and the time it has been working.
        let secs = ((s.now - m.at) / 1000).max(0);
        let glyph = super::state_glyph(s, SessionState::Working, t);
        spans.push(Span::styled(format!("  {glyph} {:02}:{:02}", secs / 60, secs % 60), t.state(SessionState::Working)));
    }
    Line::from(spans)
}

/// Where an agent's turns run: "this device" or its host's name.
fn runs_on(s: &State, handle: &str) -> Option<String> {
    let a = s.agents.iter().find(|a| a.handle == handle)?;
    match &a.host {
        None => Some("this device".into()),
        Some(id) => s.hosts.iter().find(|h| &h.id == id).map(|h| if h.this_device { "this device".into() } else { h.label.clone() }),
    }
}

fn mention_spans(text: &str, base: ratatui::style::Style, t: &Theme) -> Vec<Span<'static>> {
    let mut spans = vec![];
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", base));
        }
        if word.starts_with('@') && word.len() > 1 {
            spans.push(Span::styled(word.to_string(), t.fg(t.mention).add_modifier(Modifier::BOLD)));
        } else if !word.is_empty() {
            spans.push(Span::styled(word.to_string(), base));
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
        let write = format!("{} to write", s.keymap.hint(Context::Chat, Action::Compose));
        let hint = super::fit_parts(&[&write, "@ mentions an agent or a person"], inner.width as usize);
        f.render_widget(Paragraph::new(Line::styled(hint, t.dim())), inner);
        return;
    }
    if s.composer.text.is_empty() {
        let esc = s.keymap.hint(Context::Composer, Action::CancelCompose);
        let keys = format!("{esc} for keys");
        let hint = super::fit_parts(&["Type a message", "@ mentions an agent or a person", &keys], inner.width as usize);
        f.render_widget(Paragraph::new(Line::styled(hint, t.dim())), inner);
        f.set_cursor_position(Position::new(inner.x, inner.y));
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
