//! First run: your name, then create a workspace or join one with an
//! invite; then the recovery key, shown once. The same form is the "+"
//! overlay later (without the name).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use super::{center, pane, splash};
use crate::app::input::TextBuf;
use crate::app::state::{FormField, FormMode, State, WorkspaceForm};
use crate::backend::model::{Created, InviteTrust};
use crate::theme::Theme;

fn field_line<'a>(label: &'a str, buf: &TextBuf, focused: bool, width: usize, placeholder: &'a str, t: &Theme) -> Line<'a> {
    field_line_as(label, buf, focused, width, placeholder, t, false)
}

/// A field; a `secret` one (a workspace key, a relay token) shows a dot per
/// character, never the value.
fn field_line_as<'a>(
    label: &'a str,
    buf: &TextBuf,
    focused: bool,
    width: usize,
    placeholder: &'a str,
    t: &Theme,
    secret: bool,
) -> Line<'a> {
    let marker = if focused { "› " } else { "  " };
    let shown = if buf.text.is_empty() && !focused {
        Span::styled(placeholder.to_string(), t.dim())
    } else {
        let mut text = if secret { buf.text.char_indices().map(|_| '•').collect::<String>() } else { buf.text.clone() };
        if focused {
            // The cursor is a byte offset into the value; in the dots it
            // sits after as many dots as the value has chars before it.
            let at = if secret { buf.text[..buf.cursor].chars().count() * '•'.len_utf8() } else { buf.cursor };
            text.insert(at, '▏');
        }
        Span::styled(super::trunc(&text, width), if focused { t.accent() } else { t.text() })
    };
    Line::from(vec![Span::styled(marker, t.accent()), Span::styled(format!("{label:<15}"), t.text()), shown])
}

/// The form's rect, centred; on first run the mark goes beside it (or above
/// it) when the terminal has room, and is left out when it doesn't.
fn with_splash(f: &mut Frame, area: Rect, w: u16, h: u16, first_run: bool, t: &Theme) -> Rect {
    const GAP: u16 = 4;
    let r = center(area, w, h);
    if !first_run {
        return r;
    }
    if area.width >= splash::WIDTH + GAP + w + 2 && area.height >= h {
        let x = area.x + (area.width - (splash::WIDTH + GAP + w)) / 2;
        splash::draw(f, Rect { x, y: r.y, width: splash::WIDTH, height: r.height }, t);
        return Rect { x: x + splash::WIDTH + GAP, ..r };
    }
    if area.width >= w && area.height >= splash::HEIGHT + 1 + h + 2 {
        let y = area.y + (area.height - (splash::HEIGHT + 1 + h)) / 2;
        splash::draw(f, Rect { x: r.x, y, width: r.width, height: splash::HEIGHT }, t);
        return Rect { y: y + splash::HEIGHT + 1, ..r };
    }
    r
}

pub fn draw_form(f: &mut Frame, area: Rect, s: &State, form: &WorkspaceForm, t: &Theme, first_run: bool) {
    let connect = form.mode == FormMode::Connect;
    let link = form.mode == FormMode::Link;
    let h = match (first_run, connect, link) {
        (true, _, true) => 30,
        (false, _, true) => 25,
        (true, true, _) => 25,
        (true, false, _) => 23,
        (false, true, _) => 19,
        (false, false, _) => 17,
    };
    let r = with_splash(f, area, 72, h, first_run, t);
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
        if !link {
            lines.push(field_line("Your name", &form.name, form.field == FormField::Name, w, "how others see you", t));
        }
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
        opt("Create", form.mode == FormMode::Create),
        Span::raw("   "),
        opt("Join with an invite", form.mode == FormMode::Join),
        Span::raw("   "),
        opt("Connect a deployment", form.mode == FormMode::Connect),
    ]));
    let mut second = vec![Span::raw("  "), opt("Add to my devices", link)];
    if first_run {
        second.push(Span::raw("   "));
        second.push(opt("Look around first (the demo)", form.mode == FormMode::Demo));
    }
    lines.push(Line::from(second));
    lines.push(Line::raw(""));
    let vf = form.field == FormField::Value;
    match form.mode {
        FormMode::Create => {
            lines.push(field_line("Workspace name", &form.ws_name, vf, w, "e.g. Northwind API", t));
            lines.push(Line::styled("  No server needed: it lives on this device until you invite someone.", t.dim()));
        }
        FormMode::Join => {
            lines.push(field_line("Invite code", &form.invite, vf, w, "paste the code you were sent", t));
            match form.checked.as_ref().filter(|(i, _)| *i == form.invite.text.trim()) {
                Some((_, c)) => {
                    let who = c.founder_name.clone().unwrap_or_else(|| "its founder".into());
                    let code = c.safety_code.clone().unwrap_or_else(|| "none".into());
                    let style = match c.trust {
                        InviteTrust::Known => t.fg(t.good),
                        InviteTrust::Mismatch => t.fg(t.bad),
                        _ => t.fg(t.warn),
                    };
                    lines.push(Line::styled(format!("  “{}”, founded by {who}", c.workspace_name), t.text()));
                    lines.push(Line::styled(format!("  {}", c.trust.describe()), style));
                    lines.push(Line::styled(format!("  Founder's safety code {code}"), t.dim()));
                    lines.push(Line::styled(format!("  Relay {}", c.relay_url), t.dim()));
                }
                None => lines.push(Line::styled("  Enter checks who it's from (its founder's signature); enter again joins.", t.dim())),
            }
        }
        FormMode::Link => draw_link(&mut lines, form, vf, w, t),
        FormMode::Connect => {
            lines.push(field_line("Relay URL", &form.relay, form.field == FormField::Relay, w, "http://127.0.0.1:18443", t));
            lines.push(field_line("Room", &form.room, form.field == FormField::Room, w, "LIGUS_WORKSPACE / Secret key room", t));
            lines.push(field_line_as("Key", &form.key, form.field == FormField::Key, w, "the workspace key / Secret key key", t, true));
            lines.push(field_line_as(
                "Relay token",
                &form.token,
                form.field == FormField::Token,
                w,
                "only for a gated relay (hrt1…)",
                t,
                true,
            ));
            lines.push(field_line("Workspace name", &form.ws_name, vf, w, "defaults to the room", t));
            lines.push(Line::styled("  Founds the workspace a ligus-local or Kubernetes deployment serves;", t.dim()));
            lines.push(Line::styled("  then admit its boxes from People (B).", t.dim()));
        }
        FormMode::Demo => {
            lines.push(Line::styled("  Opens lt --demo: sample workspaces, chats, agents and proposals in memory.", t.text()));
            lines.push(Line::styled("  Nothing is saved and nothing leaves this terminal. Quit it and run lt", t.dim()));
            lines.push(Line::styled("  again to set up for real.", t.dim()));
        }
    }
    lines.push(Line::raw(""));
    if form.busy {
        let msg = match form.mode {
            FormMode::Create | FormMode::Connect | FormMode::Demo => "Creating… confirm the passkey prompt",
            FormMode::Join if form.checked.is_none() => "Checking the invite…",
            FormMode::Join => "Joining…",
            FormMode::Link if form.link_checked.is_none() => "Opening the code…",
            FormMode::Link => "Linking: connecting and checking your desktop's approval…",
        };
        lines.push(Line::styled(format!("  {msg}"), t.fg(t.warn)));
    } else if let Some(e) = &form.error {
        lines.push(Line::styled(format!("  ✗ {e}"), t.fg(t.bad).add_modifier(Modifier::BOLD)));
    } else {
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw(""));
    let esc = if first_run { "quit" } else { "close" };
    lines.push(Line::styled(format!("tab next · ←→ choose · enter continue · esc {esc}"), t.dim()));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Add to my devices: this computer's request, then the desktop's code,
/// then who approved it. Lines are broken by hand to keep their indent.
fn draw_link<'a>(lines: &mut Vec<Line<'a>>, form: &'a WorkspaceForm, focused: bool, w: usize, t: &Theme) {
    match &form.link {
        None => lines.push(Line::styled("  Making this computer's request…", t.dim())),
        Some(r) => {
            for l in [
                "  Already in the workspace on your desktop? Make this computer one",
                "  of your devices instead of a second member with your name.",
            ] {
                lines.push(Line::styled(l, t.text()));
            }
            lines.push(Line::raw(""));
            for l in [
                "  On your desktop: Settings > Profile > My devices > Add a computer.",
                "  Paste this code, check the fingerprint, confirm with your passkey.",
                "  Then paste the code it shows here.",
            ] {
                lines.push(Line::styled(l, t.dim()));
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("  Fingerprint    ", t.dim()),
                Span::styled(r.fingerprint.clone(), t.accent().add_modifier(Modifier::BOLD)),
            ]));
            // The code in pieces (the desktop ignores the breaks when it is
            // selected and pasted).
            let chars: Vec<char> = r.code.chars().collect();
            for piece in chars.chunks(w.saturating_add(16).clamp(20, 66)) {
                lines.push(Line::styled(format!("  {}", piece.iter().collect::<String>()), t.text()));
            }
            if r.new_device {
                lines.push(Line::styled("  This install was already in use: it asks with a new device key,", t.dim()));
                lines.push(Line::styled("  and your desktop can then remove the old member.", t.dim()));
            }
            lines.push(Line::raw(""));
        }
    }
    lines.push(field_line("Desktop's code", &form.grant, focused, w, "the code your desktop shows (ligusgrant1:…)", t));
    match form.link_checked.as_ref().filter(|(g, _)| *g == form.grant.text.trim()) {
        Some((_, c)) => {
            lines.push(Line::from(vec![
                Span::styled(format!("  Approved by {}'s desktop ", c.person_name), t.text()),
                Span::styled(c.approver_fingerprint.clone(), t.accent()),
            ]));
            lines.push(Line::styled(format!("  for “{}”. Is that you and your desktop?", c.workspace_name), t.text()));
            lines.push(Line::styled("  Enter: yes, link · change the code or esc: no", t.fg(t.warn)));
        }
        None => lines.push(Line::styled("  Enter shows who approved it; enter again links.", t.dim())),
    }
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
