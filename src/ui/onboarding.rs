//! First run: your name, then create a workspace or join one with an
//! invite; then the recovery key, shown once. The same form is the "+"
//! overlay later (without the name).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::{center, pane};
use crate::app::input::TextBuf;
use crate::app::state::{FormField, FormMode, State, WorkspaceForm};
use crate::backend::model::Created;
use crate::theme::Theme;

fn field_line<'a>(label: &'a str, buf: &TextBuf, focused: bool, width: usize, placeholder: &'a str, t: &Theme) -> Line<'a> {
    let marker = if focused { "› " } else { "  " };
    let shown = if buf.text.is_empty() && !focused {
        Span::styled(placeholder.to_string(), t.dim())
    } else {
        let mut text = buf.text.clone();
        if focused {
            text.insert(buf.cursor, '▏');
        }
        Span::styled(super::trunc(&text, width), if focused { t.accent() } else { t.text() })
    };
    Line::from(vec![Span::styled(marker, t.accent()), Span::styled(format!("{label:<15}"), t.text()), shown])
}

pub fn draw_form(f: &mut Frame, area: Rect, s: &State, form: &WorkspaceForm, t: &Theme, first_run: bool) {
    let h = if first_run { 20 } else { 14 };
    let r = center(area, 72, h);
    f.render_widget(Clear, r);
    let title = if first_run { "Welcome to Ligus" } else { "Add a workspace" };
    let block = pane(title, true, t);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let w = inner.width.saturating_sub(20) as usize;
    let mut lines: Vec<Line> = vec![];
    if first_run {
        lines.push(Line::styled("Ligus is short for ligustica, the Italian honeybee.", t.dim()));
        lines.push(Line::styled("This terminal is its own Ligus device, with its own keys.", t.text()));
        lines.push(Line::from(vec![
            Span::styled("Safety code    ", t.dim()),
            Span::styled(s.device.safety_code.clone(), t.accent()),
            Span::styled("  (compare it with people you invite)", t.dim()),
        ]));
        lines.push(Line::raw(""));
        lines.push(field_line("Your name", &form.name, form.field == FormField::Name, w, "how others see you", t));
        lines.push(Line::raw(""));
    }
    let mode_focus = form.field == FormField::Mode;
    let opt = |label: &'static str, on: bool| {
        let mark = if on { "(•) " } else { "( ) " };
        let style = if on { t.accent() } else { t.text() };
        Span::styled(format!("{mark}{label}"), style)
    };
    lines.push(Line::from(vec![
        Span::styled(if mode_focus { "› " } else { "  " }, t.accent()),
        opt("Create a workspace", form.mode == FormMode::Create),
        Span::raw("    "),
        opt("Join with an invite", form.mode == FormMode::Join),
    ]));
    lines.push(Line::raw(""));
    let vf = form.field == FormField::Value;
    match form.mode {
        FormMode::Create => {
            lines.push(field_line("Workspace name", &form.ws_name, vf, w, "e.g. Apiary Tracker", t));
            lines.push(Line::styled("  No server needed: it lives on this device until you invite someone.", t.dim()));
        }
        FormMode::Join => {
            lines.push(field_line("Invite code", &form.invite, vf, w, "paste the code you were sent", t));
            lines.push(Line::styled("  It carries what this device needs to sync the workspace.", t.dim()));
        }
    }
    lines.push(Line::raw(""));
    if form.busy {
        let msg = match form.mode {
            FormMode::Create => "Creating… confirm the passkey prompt",
            FormMode::Join => "Joining…",
        };
        lines.push(Line::styled(format!("  {msg}"), t.fg(t.warn)));
    } else if let Some(e) = &form.error {
        lines.push(Line::styled(format!("  ✗ {e}"), t.fg(t.bad).add_modifier(Modifier::BOLD)));
    } else {
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw(""));
    let esc = if first_run { "quit" } else { "close" };
    lines.push(Line::styled(format!("tab next · ←→ create/join · enter continue · esc {esc}"), t.dim()));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn draw_recovery(f: &mut Frame, area: Rect, c: &Created, t: &Theme) {
    let r = center(area, 76, 22);
    f.render_widget(Clear, r);
    let title = format!("“{}” is ready", c.workspace.name);
    let block = pane(&title, true, t);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let mut lines = vec![
        Line::styled("Write down your recovery key.", t.accent()),
        Line::styled(
            "It stands in for a lost passkey: without it, a sole owner who loses their passkeys can't govern the workspace again. It is shown once.",
            t.text(),
        ),
        Line::raw(""),
    ];
    let words = &c.recovery_words;
    let rows = words.len().div_ceil(4);
    for row in 0..rows {
        let mut spans = vec![];
        for col in 0..4 {
            let i = col * rows + row;
            if let Some(w) = words.get(i) {
                spans.push(Span::styled(format!("{:>3} ", i + 1), t.dim()));
                spans.push(Span::styled(format!("{w:<13}"), t.accent()));
            }
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "To confirm from a terminal without a browser later, keep the words in a file only you can read (chmod 600) and give lt its path.",
        t.dim(),
    ));
    if let Some(inv) = &c.invite {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled("Invite code  ", t.dim()), Span::styled(inv.clone(), t.text())]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Press enter when you've written them down.", t.accent()));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
