//! The Set up list and a flow's popup (`crate::app::setup`): one-line
//! rows, one step per screen, the run's checks with their fix lines. A
//! secret field is a fixed row of dots, whatever it holds.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{center, pane, trunc, wrap};
use crate::app::setup::{Stage, Wizard, here_action};
use crate::app::state::State;
use crate::setup::{CheckStatus, FieldType, Progress, SetupCheck, StepKind};
use crate::theme::Theme;

const W: u16 = 76;

fn wrapped(lines: &mut Vec<Line<'static>>, text: &str, width: usize, style: Style, indent: &str) {
    for l in wrap(text, width.saturating_sub(indent.width()).max(8)) {
        lines.push(Line::styled(format!("{indent}{l}"), style));
    }
}

fn popup(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line<'static>>, t: &Theme) -> Rect {
    let r = center(area, W, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let inner_w = r.width.saturating_sub(2) as usize;
    let title = trunc(title, inner_w.saturating_sub(2));
    f.render_widget(Paragraph::new(lines).block(pane(&title, true, t)), r);
    r
}

/// The Set up list.
pub fn list(f: &mut Frame, area: Rect, s: &State, sel: usize, t: &Theme) {
    let inner = (W.min(area.width)).saturating_sub(2) as usize;
    let mut lines: Vec<Line<'static>> = vec![];
    match &s.setup_rows {
        None => lines.push(Line::styled("Looking at what is set up\u{2026}", t.dim())),
        Some(rows) => {
            let tw = rows.iter().map(|r| r.flow.title().width()).max().unwrap_or(0);
            for (i, r) in rows.iter().enumerate() {
                let on = i == sel;
                let mark = Span::styled(if on { "› " } else { "  " }, t.accent());
                let title = Span::styled(format!("{:<tw$}  ", r.flow.title()), if on { t.selected() } else { t.text() });
                let (glyph, style) = match r.progress {
                    Progress::Done => ("✓ ", t.fg(t.good)),
                    Progress::Waiting => ("⧗ ", t.fg(t.warn)),
                    Progress::Todo => ("  ", t.dim()),
                    Progress::Unavailable => ("  ", t.disabled()),
                };
                let room = inner.saturating_sub(2 + tw + 2 + 2);
                lines.push(Line::from(vec![mark, title, Span::styled(glyph, style), Span::styled(trunc(&r.detail, room), style)]));
            }
            if let Some(r) = rows.get(sel) {
                lines.push(Line::raw(""));
                wrapped(&mut lines, r.flow.why(), inner, t.dim(), "");
            }
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("enter start · esc close · every flow is safe to run again", t.dim()));
    popup(f, area, "Set up", lines, t);
}

/// The title column's width for a set of checks (at most 16).
fn title_w(cs: &[&SetupCheck]) -> usize {
    cs.iter().map(|c| c.title.width()).max().unwrap_or(0).min(16)
}

fn check_lines(lines: &mut Vec<Line<'static>>, c: &SetupCheck, tw: usize, width: usize, t: &Theme) {
    let (glyph, style) = match c.status {
        CheckStatus::Ok => ("✓", t.fg(t.good)),
        CheckStatus::Warn => ("!", t.fg(t.warn)),
        CheckStatus::Fail => ("✗", t.fg(t.bad)),
        CheckStatus::Skip => ("–", t.dim()),
    };
    let head = format!("{:<tw$}  ", trunc(&c.title, tw.max(1)));
    let room = width.saturating_sub(2 + head.width());
    let mut detail = wrap(&c.detail, room.max(12)).into_iter();
    lines.push(Line::from(vec![
        Span::styled(format!("{glyph} "), style),
        Span::styled(head.clone(), t.text().add_modifier(Modifier::BOLD)),
        Span::styled(detail.next().unwrap_or_default(), t.text()),
    ]));
    let pad = " ".repeat(2 + head.width());
    for l in detail {
        lines.push(Line::styled(format!("{pad}{l}"), t.text()));
    }
    for n in &c.notes {
        wrapped(lines, n, width, t.text(), "  ");
    }
    if let (Some(fix), false) = (&c.fix, c.status == CheckStatus::Ok) {
        wrapped(lines, &format!("fix: {fix}"), width, t.accent(), "  ");
    }
}

/// A flow's popup.
pub fn flow(f: &mut Frame, area: Rect, s: &State, w: &Wizard, t: &Theme) {
    let width = (W.min(area.width)).saturating_sub(2) as usize;
    let mut lines: Vec<Line<'static>> = vec![];
    let mut title = w.flow.title().to_string();
    let mut cursor: Option<(u16, u16)> = None;
    let here = here_action(s, w.flow);
    let footer: String = match &w.stage {
        Stage::Loading => {
            lines.push(Line::styled("Looking at this device\u{2026}", t.dim()));
            "esc cancel".into()
        }
        Stage::Failed(e) => {
            wrapped(&mut lines, e, width, t.fg(t.bad), "");
            "any key closes".into()
        }
        Stage::Fill if w.form.as_ref().is_some_and(|f| f.blocked) => {
            lines.push(Line::styled("Not from here:", t.text()));
            lines.push(Line::raw(""));
            let cs: Vec<&SetupCheck> = w.form.iter().flat_map(|f| f.preflight.iter()).filter(|c| c.status != CheckStatus::Ok).collect();
            let tw = title_w(&cs);
            for c in cs {
                check_lines(&mut lines, c, tw, width, t);
            }
            lines.push(Line::raw(""));
            match here {
                Some((k, what)) => format!("{k} {what} · any other key closes"),
                None => "any key closes".into(),
            }
        }
        Stage::Fill => {
            let screens = w.screens();
            let form = w.form.as_ref();
            let step = screens.get(w.screen).and_then(|i| form.map(|f| &f.steps[*i]));
            if screens.len() > 1 {
                title = format!("{title} · {} of {}", w.screen + 1, screens.len());
            }
            if w.screen == 0 {
                let cs: Vec<&SetupCheck> = form.iter().flat_map(|f| f.preflight.iter()).filter(|c| c.status != CheckStatus::Ok).collect();
                let tw = title_w(&cs);
                for c in cs {
                    check_lines(&mut lines, c, tw, width, t);
                }
                if lines.is_empty() {
                    wrapped(&mut lines, w.flow.why(), width, t.dim(), "");
                }
                lines.push(Line::raw(""));
            }
            let mut path = false;
            match step {
                Some(st) if st.kind == StepKind::Person => {
                    lines.push(Line::styled(st.title.clone(), t.text().add_modifier(Modifier::BOLD)));
                    wrapped(&mut lines, &format!("A person does this, in {}.", st.where_), width, t.text(), "");
                    lines.push(Line::styled("Nothing here completes it: the flow checks for it when it runs.", t.dim()));
                }
                Some(st) => {
                    lines.push(Line::styled(st.title.clone(), t.text().add_modifier(Modifier::BOLD)));
                    let focused = w.focused().map(|f| f.id.clone());
                    let fields = w.fields();
                    let alone = fields.len() == 1;
                    for fd in fields {
                        let on = focused.as_deref() == Some(fd.id.as_str());
                        // A step of one field is titled by it already.
                        if !(alone && fd.label == st.title) {
                            let optional = !fd.required
                                && fd.default.is_none()
                                && matches!(fd.kind, FieldType::Text | FieldType::Secret | FieldType::Suggest(_));
                            let label = format!("{}{}", fd.label, if optional { " (optional)" } else { "" });
                            lines.push(Line::styled(label, if on { t.accent() } else { t.text() }));
                        }
                        match &fd.kind {
                            FieldType::Text | FieldType::Path | FieldType::Suggest(_) => {
                                let buf = w.texts.get(&fd.id);
                                let text = buf.map(|b| b.text.clone()).unwrap_or_default();
                                let shown = trunc(&text, width.saturating_sub(4));
                                if on {
                                    let col = buf.map(|b| b.cursor_pos().1).unwrap_or(0).min(width.saturating_sub(4));
                                    cursor = Some((2 + col as u16, lines.len() as u16));
                                }
                                lines.push(Line::from(vec![Span::styled("› ", t.accent()), Span::styled(shown, t.text())]));
                                if let FieldType::Suggest(opts) = &fd.kind
                                    && on
                                {
                                    let at = w.cursor.get(&fd.id).copied();
                                    let names: Vec<String> = opts.iter().take(6).map(|o| o.value.clone()).collect();
                                    if !names.is_empty() {
                                        let spans: Vec<Span> = names
                                            .iter()
                                            .enumerate()
                                            .flat_map(|(i, n)| {
                                                let st = if at == Some(i) { t.selected() } else { t.dim() };
                                                [Span::styled(n.clone(), st), Span::styled("  ", t.dim())]
                                            })
                                            .collect();
                                        lines.push(Line::from([vec![Span::styled("  ↑↓ ", t.dim())], spans].concat()));
                                    }
                                }
                                if fd.kind == FieldType::Path && on {
                                    path = true;
                                    if !w.completions.is_empty() {
                                        let more =
                                            if w.completions.len() > 6 { format!("  +{}", w.completions.len() - 6) } else { String::new() };
                                        let list = w.completions.iter().take(6).cloned().collect::<Vec<_>>().join("  ");
                                        lines.push(Line::styled(trunc(&format!("  {list}{more}"), width), t.dim()));
                                    }
                                }
                            }
                            FieldType::Secret => {
                                let buf = w.secrets.get(&fd.id);
                                let dots = buf.map(|b| b.masked()).unwrap_or("");
                                let span = if dots.is_empty() { Span::styled("(empty)", t.dim()) } else { Span::styled(dots, t.text()) };
                                if on {
                                    cursor = Some((2 + dots.chars().count() as u16, lines.len() as u16));
                                }
                                lines.push(Line::from(vec![Span::styled("› ", t.accent()), span]));
                            }
                            FieldType::Choice(opts) | FieldType::Multi(opts) => {
                                let multi = matches!(fd.kind, FieldType::Multi(_));
                                let at = w.cursor.get(&fd.id).copied().unwrap_or(0);
                                let picked = w.multi.get(&fd.id);
                                let lw = opts.iter().map(|o| o.label.width()).max().unwrap_or(0);
                                for (i, o) in opts.iter().enumerate() {
                                    let mark = if on && i == at { "› " } else { "  " };
                                    let state = if multi {
                                        if picked.is_some_and(|p| p.contains(&o.value)) { "[x] " } else { "[ ] " }
                                    } else if i == at {
                                        "● "
                                    } else {
                                        "○ "
                                    };
                                    let base = if o.disabled { t.disabled() } else { t.text() };
                                    let style = if on && i == at { t.selected() } else { base };
                                    let hint = o.hint.clone().unwrap_or_default();
                                    let room = width.saturating_sub(2 + 4 + lw + 2);
                                    let hint_style = if o.disabled {
                                        t.dim()
                                    } else if hint.starts_with("signed in") || hint == "installed" {
                                        t.fg(t.good)
                                    } else if hint.contains("not ") {
                                        t.fg(t.warn)
                                    } else {
                                        t.dim()
                                    };
                                    lines.push(Line::from(vec![
                                        Span::styled(mark, t.accent()),
                                        Span::styled(state, if o.disabled { t.disabled() } else { t.accent() }),
                                        Span::styled(format!("{:<lw$}", o.label), style),
                                        Span::styled(format!("  {}", trunc(&hint, room)), hint_style),
                                    ]));
                                }
                            }
                            FieldType::Toggle => {
                                let x = if w.toggles.contains(&fd.id) { "[x] " } else { "[ ] " };
                                lines.push(Line::from(vec![
                                    Span::styled(if on { "› " } else { "  " }, t.accent()),
                                    Span::styled(x, t.accent()),
                                    Span::styled("space switches", t.dim()),
                                ]));
                            }
                        }
                        if on && !fd.help.is_empty() {
                            wrapped(&mut lines, &fd.help, width, t.dim(), "  ");
                        }
                    }
                }
                None => {}
            }
            if let Some(e) = &w.error {
                lines.push(Line::raw(""));
                wrapped(&mut lines, &format!("✗ {e}"), width, t.fg(t.bad), "");
            }
            lines.push(Line::raw(""));
            let tab = if path { "tab complete" } else { "tab next field" };
            format!("enter next · {tab} · shift-tab back · esc cancel")
        }
        Stage::Review => {
            lines.push(Line::styled("Ready to run", t.text().add_modifier(Modifier::BOLD)));
            let inputs = w.inputs();
            let applies: Vec<_> = w.form.iter().flat_map(|f| f.fields()).filter(|f| f.applies(&inputs)).collect();
            let lw = applies.iter().map(|f| f.label.width()).max().unwrap_or(0);
            for fd in applies {
                let v = match &fd.kind {
                    FieldType::Secret => {
                        if w.secrets.get(&fd.id).is_some_and(|b| !b.is_empty()) {
                            match w.destination() {
                                d if d.is_empty() => "•••••••• (set)".to_string(),
                                d => format!("•••••••• (only to {d})"),
                            }
                        } else {
                            "not set".to_string()
                        }
                    }
                    FieldType::Toggle => if inputs.contains_key(&fd.id) { "yes" } else { "no" }.to_string(),
                    FieldType::Choice(o) | FieldType::Multi(o) => {
                        let v = inputs.get(&fd.id).cloned().unwrap_or_default();
                        v.split(',')
                            .map(|x| o.iter().find(|c| c.value == x).map(|c| c.label.clone()).unwrap_or_else(|| x.to_string()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                    _ => inputs.get(&fd.id).cloned().unwrap_or_else(|| "(default)".into()),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<lw$}  ", fd.label), t.dim()),
                    Span::styled(trunc(&v, width.saturating_sub(lw + 4)), t.text()),
                ]));
            }
            lines.push(Line::raw(""));
            let x = if w.dry_run { "[x]" } else { "[ ]" };
            lines.push(Line::from(vec![
                Span::styled(format!("{x} dry run"), t.accent()),
                Span::styled("  checks and validates, writes nothing", t.dim()),
            ]));
            lines.push(Line::raw(""));
            let go = if w.dry_run { "enter check" } else { "enter run" };
            format!("{go} · d dry run · shift-tab back · esc cancel")
        }
        Stage::Running => {
            if w.stopping {
                lines.push(Line::styled("Stopping after this step; its own writes are undone\u{2026}", t.fg(t.warn)));
            } else {
                lines.push(Line::styled(if w.dry_run { "Checking\u{2026}" } else { "Running\u{2026}" }, t.text()));
            }
            "esc stop".into()
        }
        Stage::Done(o) => {
            let (head, style) = if o.done && o.exit == 0 {
                (if o.dry_run { "Dry run: nothing was written" } else { "Done" }, t.fg(t.good))
            } else if o.person() {
                ("Waiting for a person", t.fg(t.warn))
            } else {
                ("Stopped", t.fg(t.bad))
            };
            lines.push(Line::styled(head, style.add_modifier(Modifier::BOLD)));
            lines.push(Line::raw(""));
            let tw = title_w(&o.checks.iter().collect::<Vec<_>>());
            for c in &o.checks {
                check_lines(&mut lines, c, tw, width, t);
            }
            // A next step a fix line already says isn't said twice.
            let next: Vec<&String> = o.next.iter().filter(|n| !o.checks.iter().any(|c| c.fix.as_ref() == Some(*n))).collect();
            if !next.is_empty() {
                lines.push(Line::raw(""));
                for n in next {
                    wrapped(&mut lines, &format!("next: {n}"), width, t.accent(), "");
                }
            }
            lines.push(Line::raw(""));
            let mut keys = vec!["enter close".to_string()];
            if o.exit != 0 && !o.person() {
                keys.push("r try again".into());
            }
            if let (Some((k, what)), true) = (here, o.person()) {
                keys.push(format!("{k} {what}"));
            }
            keys.join(" · ")
        }
    };
    lines.push(Line::styled(footer, t.dim()));
    // Keep the footer: a long screen loses its middle, never its keys.
    let max = area.height.saturating_sub(2) as usize;
    if lines.len() > max && max > 2 {
        let foot = lines.pop();
        lines.truncate(max - 1);
        lines.extend(foot);
    }
    let r = popup(f, area, &title, lines, t);
    if let Some((x, y)) = cursor
        && y + 1 < r.height.saturating_sub(1)
    {
        f.set_cursor_position(Position::new(r.x + 1 + x, r.y + 1 + y));
    }
}
