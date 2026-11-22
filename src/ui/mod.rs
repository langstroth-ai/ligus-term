//! Drawing. Every function here reads [`State`] and writes to a frame;
//! nothing changes state.

mod chat;
mod onboarding;
mod overlays;
mod palette;
mod proposals;
mod splash;
mod views;

pub use overlays::help_max_scroll;

/// The help overlay's lines as plain text (for tests and plain mode).
pub fn help_lines_text(s: &State) -> Vec<String> {
    overlays::help_lines(s, &Theme::MONO).iter().map(|l| l.to_string()).collect()
}
pub use proposals::apply_text;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::state::{Focus, Phase, State, ToastKind, View};
use crate::backend::model::{SyncMode, initials};
use crate::keymap::{Action, Context};
use crate::theme::Theme;

/// The smallest terminal lt lays out; below it one line says so (a
/// passkey prompt still shows, since it waits on you).
pub const MIN_SIZE: (u16, u16) = (60, 20);

pub fn draw(f: &mut Frame, s: &State, t: &Theme) {
    let area = f.area();
    if (area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1) && s.presence.is_empty() {
        let msg = format!("Terminal too small: lt needs {}×{}, this is {}×{}", MIN_SIZE.0, MIN_SIZE.1, area.width, area.height);
        let lines: Vec<Line> = wrap(&msg, area.width.max(1) as usize).into_iter().map(|l| Line::styled(l, t.text())).collect();
        let h = lines.len() as u16;
        f.render_widget(Paragraph::new(lines).centered(), center(area, area.width, h));
        return;
    }
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
    // In the main screen the toast sits in the status bar; before it (the
    // first run, start-up) it takes the last row.
    if !matches!(s.phase, Phase::Main) {
        draw_toast_row(f, Rect { y: area.bottom().saturating_sub(1), height: 1.min(area.height), ..area }, s, t);
    }
    if t.ascii {
        asciify(f.buffer_mut());
    }
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
    if s.overlay.is_none() && s.which_key.is_some_and(|at| s.now - at >= crate::app::state::WHICH_KEY_DELAY) {
        draw_which_key(f, body, s, t);
    }
}

/// The keys of one context that are a single key press (no ctrl or alt),
/// as "key label" pairs.
fn single_keys(s: &State, ctx: Context, skip: &[Action]) -> Vec<(String, String)> {
    crate::keymap::BINDINGS
        .iter()
        .filter(|b| b.context == ctx && !skip.contains(&b.action))
        .filter_map(|b| {
            let key = s.keymap.keys(ctx, b.action).iter().find(|c| !c.is_command() && c.code != crossterm::event::KeyCode::Char(' '))?;
            // The config name reads as a short label ("apply on host").
            Some((key.to_string(), b.name.replace('_', " ")))
        })
        .collect()
}

/// The sheet `space` shows after a pause: what one key does here, the
/// views, and what works anywhere. It sits over the bottom of the screen,
/// above the status bar, and goes with the next key.
fn draw_which_key(f: &mut Frame, body: Rect, s: &State, t: &Theme) {
    use Action::*;
    let views = [GoChats, GoWorkflows, GoProposals, GoAgents, GoHosts, GoMemory, GoPeople, GoNotifications, GoSettings];
    let mut groups: Vec<(&str, Vec<(String, String)>)> = vec![];
    let here: Vec<(String, String)> = s
        .view_contexts()
        .into_iter()
        .filter(|c| !matches!(c, Context::Global | Context::Nav | Context::Composer))
        .flat_map(|c| single_keys(s, c, &[]))
        .collect();
    if !here.is_empty() {
        groups.push((s.view.title(), here));
    }
    let go: Vec<(String, String)> = views
        .iter()
        .filter_map(|a| {
            let key = s.keymap.keys(Context::Global, *a).iter().find(|c| !c.is_command())?;
            let v = View::ALL[views.iter().position(|x| x == a)?];
            Some((key.to_string(), v.title().to_lowercase()))
        })
        .collect();
    groups.push(("Go to", go));
    let mut skip = views.to_vec();
    skip.extend([WhichKey, FocusNext, FocusPrev, Back]);
    groups.push(("Anywhere", single_keys(s, Context::Global, &skip)));

    let w = body.width.saturating_sub(2).min(110);
    let inner_w = w.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = vec![];
    for (title, pairs) in &groups {
        let mut row: Vec<Span> = vec![Span::styled(format!("{title:<10}"), t.dim())];
        let mut used = 10;
        for (k, label) in pairs {
            let cell = k.width() + 1 + label.width() + 3;
            if used + cell > inner_w && used > 10 {
                lines.push(Line::from(std::mem::take(&mut row)));
                row.push(Span::raw(" ".repeat(10)));
                used = 10;
            }
            row.push(Span::styled(k.clone(), t.accent()));
            row.push(Span::styled(format!(" {label}   "), t.text()));
            used += cell;
        }
        lines.push(Line::from(row));
    }
    let h = (lines.len() as u16 + 2).min(body.height);
    let r = Rect { x: body.x + (body.width - w) / 2, y: body.bottom().saturating_sub(h), width: w, height: h };
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(pane("space … press a key · esc", true, t)), r);
}

/// `[NA] Northwind API · owner │ Chats  Proposals …` with a mention dot on
/// other workspaces. When the terminal is narrow the bar degrades step by
/// step (shorter labels, one workspace tile, the view digits, then only the
/// current view) instead of cutting views off, and never hides the
/// Notifications badge.
fn draw_top_bar(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let width = area.width as usize;
    let combos = [
        (Ws::Full, Tabs::Titles(2)),
        (Ws::Full, Tabs::Titles(1)),
        (Ws::Full, Tabs::Short),
        (Ws::Full, Tabs::Tiny),
        (Ws::Tile, Tabs::Tiny),
        (Ws::Tile, Tabs::Digits),
        (Ws::Tile, Tabs::Current),
    ];
    let (level, mut spans, tabs) = combos
        .into_iter()
        .map(|(ws, tabs)| (ws, workspace_spans(s, t, ws), tabs_spans(s, t, tabs)))
        .find(|(_, w, tabs)| spans_width(w) + spans_width(tabs) <= width)
        .unwrap_or_else(|| (Ws::Tile, workspace_spans(s, t, Ws::Tile), tabs_spans(s, t, Tabs::Current)));
    spans.extend(tabs);
    // With one tile, the workspace's name goes on the right when it fits.
    let room = width.saturating_sub(spans_width(&spans) + 2);
    if level == Ws::Tile
        && let Some(w) = s.active_workspace()
        && room >= w.name.width().min(8)
    {
        let name = trunc(&w.name, room);
        spans.push(Span::raw(" ".repeat(width - spans_width(&spans) - name.width())));
        spans.push(Span::styled(name, t.accent()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// How much of the workspace part of the top bar to show.
#[derive(Clone, Copy, PartialEq)]
enum Ws {
    /// Every workspace's tile, the active one's name and your role.
    Full,
    /// The active tile and how many others.
    Tile,
}

/// How the view tabs are labelled.
#[derive(Clone, Copy, PartialEq)]
enum Tabs {
    /// Full titles, this many spaces apart.
    Titles(usize),
    /// Short titles, one space apart.
    Short,
    /// Shorter titles, one space apart.
    Tiny,
    /// Digits, with the current view's short title.
    Digits,
    /// Only the current view, between arrows.
    Current,
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// The workspace part of the top bar, ending in `│ `.
fn workspace_spans(s: &State, t: &Theme, level: Ws) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = vec![];
    let active = s.active_workspace();
    let tile_style = t.accent().add_modifier(Modifier::REVERSED);
    if level == Ws::Full {
        for (i, w) in s.workspaces.iter().enumerate() {
            let is_active = s.active.as_ref() == Some(&w.id);
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(format!("[{}]", initials(&w.name)), if is_active { tile_style } else { t.dim() }));
            if w.mentioned && !is_active {
                spans.push(Span::styled("•", t.fg(t.mention)));
            }
        }
        if let Some(w) = active {
            spans.push(Span::styled(format!(" {} ", w.name), t.accent()));
            spans.push(Span::styled(format!("· {} ", w.your_role.label()), t.dim()));
        }
        spans.push(Span::styled("│ ", t.dim()));
        return spans;
    }
    if let Some(w) = active {
        spans.push(Span::styled(format!("[{}]", initials(&w.name)), tile_style));
    }
    let others = s.workspaces.len().saturating_sub(usize::from(active.is_some()));
    if others > 0 {
        spans.push(Span::styled(format!("+{others}"), t.dim()));
        if s.workspaces.iter().any(|w| w.mentioned && s.active.as_ref() != Some(&w.id)) {
            spans.push(Span::styled("•", t.fg(t.mention)));
        }
    }
    spans.push(Span::styled(" │ ", t.dim()));
    spans
}

/// The view tabs, labelled as `level` says.
fn tabs_spans(s: &State, t: &Theme, level: Tabs) -> Vec<Span<'static>> {
    let unread = s.unread_notifications();
    let style = |v: View| if v == s.view { t.accent().add_modifier(Modifier::UNDERLINED) } else { t.text() };
    let digit = |v: View| View::ALL.iter().position(|x| *x == v).map(|i| i + 1).unwrap_or(0);
    let badge = |v: View| if v == View::Notifications && unread > 0 { format!(" {unread}") } else { String::new() };
    let mut spans = vec![];
    match level {
        Tabs::Titles(_) | Tabs::Short | Tabs::Tiny => {
            let gap = if let Tabs::Titles(g) = level { g } else { 1 };
            for (i, v) in View::ALL.into_iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(" ".repeat(gap)));
                }
                let title = match level {
                    Tabs::Short => v.short(),
                    Tabs::Tiny => v.tiny(),
                    _ => v.title(),
                };
                spans.push(Span::styled(format!("{title}{}", badge(v)), style(v)));
            }
        }
        Tabs::Digits => {
            for (i, v) in View::ALL.into_iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                let label = if v == s.view {
                    format!("{} {}{}", digit(v), v.tiny(), badge(v))
                } else if v == View::Notifications && unread > 0 {
                    format!("{}({unread})", digit(v))
                } else {
                    digit(v).to_string()
                };
                spans.push(Span::styled(label, style(v)));
            }
        }
        Tabs::Current => {
            spans.push(Span::styled("‹ ", t.dim()));
            spans.push(Span::styled(format!("{} {}{}", digit(s.view), s.view.tiny(), badge(s.view)), style(s.view)));
            spans.push(Span::styled(" ›", t.dim()));
            if s.view != View::Notifications && unread > 0 {
                spans.push(Span::styled(format!(" {}({unread})", digit(View::Notifications)), t.text()));
            }
        }
    }
    spans
}

/// The right of the status bar: how the active workspace syncs, and whether
/// the last sync worked: "⇅ relay ok", "⚠ offline", "peer to peer".
pub(crate) fn sync_label(s: &State, t: &Theme) -> Option<Span<'static>> {
    let w = s.active_workspace()?;
    Some(match &w.sync {
        SyncMode::Local => Span::styled("this device only", t.dim()),
        SyncMode::Peer => Span::styled("peer to peer", t.dim()),
        SyncMode::Relay(_) => match s.sync_status.get(&w.id).map(String::as_str) {
            Some("synced") => Span::styled("⇅ relay ok", t.dim()),
            Some(st) if st.starts_with("sync off") => Span::styled("⚠ sync off", t.fg(t.warn).add_modifier(Modifier::BOLD)),
            Some(st) if st.starts_with("sync") => Span::styled("⚠ offline", t.fg(t.warn).add_modifier(Modifier::BOLD)),
            _ => Span::styled("⇅ relay", t.dim()),
        },
    })
}

/// The status bar's right zone, fullest first: the sync state, how much
/// is running, and "demo" in the demo; then less, down to nothing.
fn right_zones(s: &State, t: &Theme) -> Vec<Vec<Span<'static>>> {
    let sep = || Span::styled(" · ", t.dim());
    let mut parts: Vec<Span<'static>> = vec![];
    if let Some(sy) = sync_label(s, t) {
        parts.push(sy);
    }
    let running = s.running();
    if running > 0 {
        parts.push(Span::styled(format!("{running} running"), t.dim()));
    }
    if s.device.backend_label == "demo" {
        parts.push(Span::styled("demo", t.dim()));
    }
    let mut out = vec![];
    for n in (0..=parts.len()).rev() {
        let mut spans = vec![];
        for (i, p) in parts.iter().take(n).enumerate() {
            if i > 0 {
                spans.push(sep());
            }
            spans.push(p.clone());
        }
        out.push(spans);
    }
    out
}

/// The toast as status-bar spans, its text cut to `width` columns.
fn toast_spans(toast: &crate::app::state::Toast, width: usize, t: &Theme) -> Vec<Span<'static>> {
    let (mark, color) = match (toast.kind, t.ascii) {
        (ToastKind::Info, false) => ("•", t.accent),
        (ToastKind::Ok, false) => ("✓", t.good),
        (ToastKind::Error, false) => ("✗", t.bad),
        (ToastKind::Info, true) => ("*", t.accent),
        (ToastKind::Ok, true) => ("ok", t.good),
        (ToastKind::Error, true) => ("x", t.bad),
    };
    let mark_style =
        if t.mono { Style::default().add_modifier(Modifier::BOLD) } else { Style::default().fg(color).add_modifier(Modifier::BOLD) };
    let text_style = if toast.kind == ToastKind::Error { t.text().add_modifier(Modifier::BOLD) } else { t.text() };
    vec![Span::styled(format!("{mark} "), mark_style), Span::styled(trunc(&toast.text, width.saturating_sub(2)), text_style)]
}

/// The toast alone on a row (before the main screen exists).
fn draw_toast_row(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let Some(toast) = s.toasts.last() else { return };
    if area.height == 0 {
        return;
    }
    let spans = toast_spans(toast, area.width as usize, t);
    let w = spans_width(&spans) as u16;
    let r = Rect { x: area.right().saturating_sub(w), width: w.min(area.width), ..area };
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(Line::from(spans)), r);
}

/// The key hints for what has focus, the help key last.
fn status_hints(s: &State) -> (Vec<String>, String) {
    let k = &s.keymap;
    let h = |ctx, a, label: &str| format!("{} {label}", k.hint(ctx, a));
    use Action::*;
    use Context as C;
    if let Some(crate::app::state::Overlay::History { .. }) = &s.overlay {
        return (vec!["type to search".into(), "↑↓ select".into(), "enter use".into()], "esc close".into());
    }
    if let Some(crate::app::state::Overlay::Palette { .. }) = &s.overlay {
        return (vec!["type to filter".into(), "↑↓ select".into(), "enter run".into()], "esc close".into());
    }
    if let Some(crate::app::state::Overlay::Messages { .. }) = &s.overlay {
        return (vec!["↑↓ scroll".into(), "g/G newest/oldest".into()], "esc close".into());
    }
    if let Some(crate::app::state::Overlay::Help { searching, .. }) = &s.overlay {
        if *searching {
            return (vec!["type to search".into(), "↑↓ scroll".into(), "enter keep".into()], "esc clear".into());
        }
        return (
            vec![
                "/ search".into(),
                "↑↓ scroll".into(),
                format!("{}/{} page", k.hint(C::Nav, PageUp), k.hint(C::Nav, PageDown)),
                "g/G top/end".into(),
            ],
            "esc close".into(),
        );
    }
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
        ],
        (View::People, _) => vec![
            "↑↓ people".into(),
            h(C::People, ChangeRole, "role"),
            h(C::People, RemoveMember, "remove"),
            h(C::People, Invite, "invite"),
            h(C::People, AdmitBox, "admit box"),
        ],
        (View::Agents, _) => vec![
            "↑↓ agents".into(),
            h(C::Agents, AddAgent, "add agent"),
            h(C::Agents, ChangeRuntime, "runtime"),
            h(C::Agents, ToggleAccess, "who can use"),
        ],
        (View::Notifications, _) => vec!["↑↓ select".into(), h(C::Nav, Select, "open"), h(C::Notifications, MarkAllRead, "mark all read")],
        (View::Settings, _) => vec!["↑↓ select".into(), h(C::Settings, Toggle, "toggle")],
        (View::Hosts, _) => vec!["↑↓ hosts".into(), h(C::Hosts, BindCheckout, "bind checkout")],
        (View::Workflows, _) => vec![
            "↑↓ select".into(),
            h(C::Workflows, StartRun, "start"),
            h(C::Workflows, ApproveGate, "approve gate"),
            h(C::Workflows, RejectGate, "reject"),
            h(C::Workflows, CancelRun, "cancel"),
        ],
        (View::Memory, _) => vec!["↑↓ task state and memory".into()],
    };
    // While a text field has the keyboard only a command chord (f1) helps.
    let help = if s.typing_text() {
        k.keys(C::Global, Help).iter().find(|c| c.is_command()).map(|c| format!("{c} help"))
    } else {
        Some(h(C::Global, Help, "help"))
    };
    (hints, help.unwrap_or_default())
}

/// Three zones: key hints on the left, cut from the right (the help key
/// always stays); the toast, if there is one, in the centre; the sync
/// state on the right, at least two columns from either. A toast takes
/// room from the hints before the right zone gives any up, and is cut
/// itself only when nothing else is left to give.
fn draw_status(f: &mut Frame, area: Rect, s: &State, t: &Theme) {
    let width = area.width as usize;
    let (hints, help) = status_hints(s);
    let rights = right_zones(s, t);
    let left_text = |n: usize| {
        let mut parts: Vec<&str> = hints.iter().take(n).map(String::as_str).collect();
        if n < hints.len() {
            parts.push("…");
        }
        if !help.is_empty() {
            parts.push(&help);
        }
        parts.join("  ")
    };
    let render = |f: &mut Frame, left: String, middle: Vec<Span<'static>>, right: &[Span<'static>]| {
        let mut spans = vec![Span::styled(left.clone(), t.dim())];
        let used = left.width() + spans_width(&middle) + spans_width(right);
        let pad = width.saturating_sub(used);
        if middle.is_empty() {
            spans.push(Span::raw(" ".repeat(pad)));
        } else {
            let before = if left.is_empty() { 0 } else { 2.min(pad) };
            spans.push(Span::raw(" ".repeat(before)));
            spans.extend(middle);
            spans.push(Span::raw(" ".repeat(pad - before)));
        }
        spans.extend(right.iter().cloned());
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    };
    let gap = |w: usize| if w == 0 { 0 } else { 2 };
    if let Some(toast) = s.toasts.last() {
        let full = toast_spans(toast, usize::MAX, t);
        let tw = spans_width(&full);
        for right in &rights {
            let rw = spans_width(right);
            for n in (0..=hints.len()).rev() {
                let left = left_text(n);
                if left.width() + 2 + tw + gap(rw) + rw <= width {
                    render(f, left, full, right);
                    return;
                }
            }
        }
        // Nothing left to give: the help key and the toast, cut to fit.
        let left = left_text(0);
        let room = width.saturating_sub(left.width() + 2);
        render(f, left, toast_spans(toast, room, t), &[]);
        return;
    }
    for n in (0..=hints.len()).rev() {
        let left = left_text(n);
        for right in &rights {
            let rw = spans_width(right);
            if left.width() + gap(rw) + rw <= width {
                render(f, left, vec![], right);
                return;
            }
        }
    }
    f.render_widget(Paragraph::new(Line::styled(trunc(&left_text(0), width), t.dim())), area);
}

/// A session state's glyph; working spins one frame per tick (one
/// spinner cell per running item, nothing else moves).
pub(crate) fn state_glyph(s: &State, st: crate::backend::model::SessionState, t: &Theme) -> &'static str {
    const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    const SPIN_ASCII: [&str; 4] = ["|", "/", "-", "\\"];
    if st == crate::backend::model::SessionState::Working {
        let frame = (s.now / 250).rem_euclid(10) as usize;
        return if t.ascii { SPIN_ASCII[frame % 4] } else { SPIN[frame] };
    }
    st.glyph(t.ascii)
}

/// `--ascii` borders: `+`, `-` and `|`.
const ASCII_BORDER: ratatui::symbols::border::Set = ratatui::symbols::border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// With `--ascii`, the symbols lt draws itself, as ASCII. Text people
/// wrote passes through, apart from typographic quotes and dashes.
fn ascii_for(c: char) -> Option<char> {
    Some(match c {
        '▍' | '▏' => '_',
        '─' | '━' | '═' | '┄' | '╌' => '-',
        '│' | '┃' | '║' | '┆' | '╎' => '|',
        '\u{2500}'..='\u{257f}' => '+',
        '\u{2580}'..='\u{259f}' => '#',
        '\u{2800}'..='\u{28ff}' => '*',
        '…' => '.',
        '•' | '●' | '◆' | '⚒' | '★' => '*',
        '✓' | '✔' => 'v',
        '✗' | '✕' | '×' => 'x',
        '⚠' => '!',
        '⇅' | '⧗' => '~',
        '‖' | '⊘' => '=',
        '‹' | '←' => '<',
        '›' | '→' | '▶' | '▸' => '>',
        '▾' | '↓' => 'v',
        '↑' => '^',
        '⏎' => '/',
        '·' => '.',
        '“' | '”' => '"',
        '‘' | '’' => '\'',
        '–' | '—' => '-',
        _ => return None,
    })
}

/// Replace the symbols in a drawn frame with ASCII (`--ascii`).
fn asciify(buf: &mut ratatui::buffer::Buffer) {
    for cell in buf.content.iter_mut() {
        let mut chars = cell.symbol().chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && !c.is_ascii()
            && let Some(a) = ascii_for(c)
        {
            cell.set_char(a);
        }
    }
}

/// A bordered pane with a title; the border lights up when focused.
pub(crate) fn pane<'a>(title: &'a str, focused: bool, t: &Theme) -> Block<'a> {
    if t.ascii {
        return Block::default()
            .borders(Borders::ALL)
            .border_set(ASCII_BORDER)
            .border_style(t.border(focused))
            .title(Span::styled(format!(" {title} "), if focused { t.accent() } else { t.text() }));
    }
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
/// Hint parts joined with " · ", as many from the front as fit in `width`;
/// the first is shortened with "…" when even it doesn't fit.
pub(crate) fn fit_parts(parts: &[&str], width: usize) -> String {
    let mut out = String::new();
    for p in parts {
        let next = if out.is_empty() { p.to_string() } else { format!("{out} · {p}") };
        if next.width() > width {
            break;
        }
        out = next;
    }
    if out.is_empty() { parts.first().map(|p| trunc(p, width)).unwrap_or_default() } else { out }
}

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
