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
use crate::app::state::{FormField, FormMode, LiveStage, State, WorkspaceForm};
use crate::backend::model::{Created, InviteTrust, LinkRequestView, MatchCodeView, Millis};
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
    // Add to my devices: as many rows as its section has.
    let link_w = 72.min(area.width).saturating_sub(22) as usize;
    let link_rows = if link {
        let mut scratch = vec![];
        draw_link(&mut scratch, form, false, link_w, s.now, t);
        scratch.len() as u16
    } else {
        0
    };
    let h = match (first_run, connect, link) {
        (true, _, true) => 15 + link_rows,
        (false, _, true) => 10 + link_rows,
        (true, true, _) => 26,
        (true, false, _) => 24,
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
        lines.push(Line::from(vec![
            Span::styled("Once in: ", t.dim()),
            Span::styled(s.keymap.hint(crate::keymap::Context::Global, crate::keymap::Action::Palette), t.accent()),
            Span::styled(" opens every command, ", t.dim()),
            Span::styled(s.keymap.hint(crate::keymap::Context::Global, crate::keymap::Action::Help), t.accent()),
            Span::styled(" shows every key.", t.dim()),
        ]));
        lines.push(Line::raw(""));
        if !link {
            lines.push(field_line("Your name", &form.name, form.field == FormField::Name, w, "how others see you", t));
            lines.push(Line::raw(""));
        }
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
        FormMode::Link => draw_link(&mut lines, form, vf, w, s.now, t),
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
    let hints = if link { link_hints(form, esc) } else { format!("tab next · ←→ choose · enter continue · esc {esc}") };
    lines.push(Line::styled(hints, t.dim()));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Add to my devices. Before anything is sent: three steps (the desktop's
/// approval address is pasted here), and the request itself for an older
/// desktop. Then the live request: waiting, the code to compare, the
/// verified person to confirm, or how it ended. The longer explanation is
/// behind `?`. Lines are broken by hand to keep their indent.
fn draw_link<'a>(lines: &mut Vec<Line<'a>>, form: &'a WorkspaceForm, focused: bool, w: usize, now: Millis, t: &Theme) {
    let checked = form.link_checked.as_ref().filter(|(g, _)| *g == form.grant.text.trim());
    let stage = form.live.as_ref().map(|l| &l.stage);
    let Some(r) = &form.link else {
        lines.push(Line::styled("  Making this computer's request…", t.dim()));
        lines.push(Line::raw(""));
        lines.push(field_line("From desktop", &form.grant, focused, w, "the approval address it shows (ligusaddr1:…)", t));
        return;
    };
    match stage {
        None => {
            lines.push(Line::styled("  Make this computer one of your devices.", t.text()));
            if form.link_about {
                for l in [
                    "  An invite makes a second member with your name; this makes it you.",
                    "  It acts as you without your account key: votes ask for your",
                    "  passkey; devices, phones and keys stay on the desktop.",
                ] {
                    lines.push(Line::styled(l, t.dim()));
                }
                if r.new_device {
                    lines.push(Line::styled("  In use already: it asks with a new device key, and the desktop", t.dim()));
                    lines.push(Line::styled("  can then remove the old member (their messages keep their name).", t.dim()));
                }
            }
            lines.push(Line::raw(""));
            for (n, l) in [
                ("1", "Desktop: My devices > Add a computer (palette: link)"),
                ("2", "Copy its approval address and paste it below"),
                ("3", "Check both screens show the same code"),
            ] {
                lines.push(Line::from(vec![Span::styled(format!("  {n} "), t.accent()), Span::styled(l, t.text())]));
            }
            lines.push(Line::raw(""));
            if form.live_unavailable {
                lines.push(Line::styled("  That relay can't pass requests along: paste this request on your", t.text()));
                lines.push(Line::styled("  desktop instead (c copies it), then the code it shows below.", t.text()));
            } else {
                lines.push(Line::styled("  Older desktop? Paste this request there (c copies it):", t.dim()));
            }
            paste_text(lines, r, w, t);
        }
        Some(LiveStage::Sending) => {
            lines.push(Line::styled("  Sending the request to your desktop…", t.text()));
            lines.push(Line::raw(""));
        }
        Some(LiveStage::Waiting) => {
            lines.push(Line::styled("  Waiting for your desktop…", t.text()));
            lines.push(Line::styled("  It shows up in My devices there by itself.", t.dim()));
            lines.push(Line::raw(""));
            lines.push(Line::styled("  Older desktop? Paste this request there (c copies it):", t.dim()));
            paste_text(lines, r, w, t);
        }
        Some(LiveStage::Code { code, yes }) => {
            code_frame(lines, code, t);
            lines.push(Line::styled("  Check your desktop shows the same code.", t.text()));
            lines.push(Line::styled(
                format!("  Says it's {} (checked after approval) · {} left", code.claimed, countdown(code.expires_at, now)),
                t.dim(),
            ));
            lines.push(Line::raw(""));
            lines.push(buttons("Yes, they match", "No", *yes, t));
            return;
        }
        Some(LiveStage::Matched { code, acknowledged }) => {
            code_frame(lines, code, t);
            lines.push(Line::styled("  The codes match.", t.fg(t.good)));
            let next = if *acknowledged {
                "  Waiting for your desktop's passkey…"
            } else {
                "  Waiting for your desktop: confirm the code there too."
            };
            lines.push(Line::styled(next, t.text()));
            return;
        }
        Some(LiveStage::Checking) => {
            lines.push(Line::styled("  Your desktop approved it.", t.text()));
            lines.push(Line::styled("  Checking the approval on the workspace log…", t.text()));
            lines.push(Line::styled("  (it can take up to 2 minutes to arrive)", t.dim()));
            return;
        }
        Some(LiveStage::Confirm { person, workspace, yes, busy }) => {
            lines.push(Line::from(vec![
                Span::styled("  Link this computer as ", t.text()),
                Span::styled(person.clone(), t.accent().add_modifier(Modifier::BOLD)),
                Span::styled(" (verified on the workspace log)?", t.text()),
            ]));
            lines.push(Line::styled(format!("  In “{workspace}”, it acts as {person} from then on."), t.dim()));
            lines.push(Line::raw(""));
            if *busy {
                lines.push(Line::styled("  Linking…", t.fg(t.warn)));
            } else {
                lines.push(buttons("Yes, link", "No", *yes, t));
            }
            return;
        }
        Some(LiveStage::Ended(e)) => {
            lines.push(Line::styled(format!("  ✗ {}", e.describe()), t.fg(t.bad).add_modifier(Modifier::BOLD)));
            for l in super::wrap(e.next(), 66) {
                lines.push(Line::styled(format!("  {l}"), t.dim()));
            }
            lines.push(Line::raw(""));
        }
    }
    lines.push(field_line("From desktop", &form.grant, focused, w, "the approval address it shows (ligusaddr1:…)", t));
    if let Some((_, c)) = checked {
        lines.push(Line::from(vec![
            Span::styled(format!("  Approved by {}'s desktop ", c.person_name), t.text()),
            Span::styled(c.approver_fingerprint.clone(), t.accent()),
        ]));
        lines.push(Line::styled(format!("  for “{}”. Is that you?", c.workspace_name), t.text()));
    }
}

/// The request's fingerprint and paste text, for a desktop without the
/// approval mailbox.
fn paste_text<'a>(lines: &mut Vec<Line<'a>>, r: &'a LinkRequestView, w: usize, t: &Theme) {
    lines.push(Line::from(vec![
        Span::styled("  Fingerprint    ", t.dim()),
        Span::styled(r.fingerprint.clone(), t.accent().add_modifier(Modifier::BOLD)),
    ]));
    // The code in pieces (the desktop ignores the breaks when it is pasted).
    // Selecting it here takes the border too: c copies it, o shows it alone
    // on the screen.
    let chars: Vec<char> = r.code.chars().collect();
    for piece in chars.chunks(w.saturating_add(16).clamp(20, 66)) {
        lines.push(Line::styled(format!("  {}", piece.iter().collect::<String>()), t.dim()));
    }
    lines.push(Line::raw(""));
}

/// The code on one framed line, large as a terminal allows: bold, two
/// groups. Not a secret, and not for copying.
fn code_frame<'a>(lines: &mut Vec<Line<'a>>, code: &MatchCodeView, t: &Theme) {
    let inner = code.shown.chars().count() + 4;
    lines.push(Line::styled(format!("  ┌{}┐", "─".repeat(inner)), t.dim()));
    lines.push(Line::from(vec![
        Span::styled("  │  ", t.dim()),
        Span::styled(code.shown.clone(), t.accent().add_modifier(Modifier::BOLD)),
        Span::styled("  │", t.dim()),
    ]));
    lines.push(Line::styled(format!("  └{}┘", "─".repeat(inner)), t.dim()));
}

/// How long a code has left, as `4:59` (quiet: no colour change).
fn countdown(expires_at: Millis, now: Millis) -> String {
    let secs = (expires_at - now).max(0) / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Two answers; the selected one is marked in words (`›`) and in colour.
fn buttons<'a>(yes_label: &'a str, no_label: &'a str, yes: bool, t: &Theme) -> Line<'a> {
    let b = |label: &'a str, on: bool| {
        let style = if on { t.selected().add_modifier(Modifier::BOLD) } else { t.text() };
        Span::styled(format!("{}[ {label} ]", if on { "›" } else { " " }), style)
    };
    Line::from(vec![Span::raw("  "), b(yes_label, yes), Span::raw("   "), b(no_label, !yes)])
}

/// The key hints under Add to my devices.
fn link_hints(form: &WorkspaceForm, esc: &str) -> String {
    match form.live.as_ref().map(|l| &l.stage) {
        Some(LiveStage::Code { .. }) | Some(LiveStage::Confirm { busy: false, .. }) => {
            return "y yes · n no · ←→ choose · enter answer · esc end the request".into();
        }
        Some(LiveStage::Confirm { .. }) => return String::new(),
        Some(LiveStage::Sending | LiveStage::Waiting) => return "c copy the request · o full screen · esc end the request".into(),
        Some(LiveStage::Matched { .. } | LiveStage::Checking) => return "esc end the request".into(),
        Some(LiveStage::Ended(_)) => return format!("enter start again · esc {esc}"),
        None => {}
    }
    let checked = form.link_checked.as_ref().is_some_and(|(g, _)| *g == form.grant.text.trim());
    if checked {
        "enter link · esc no".into()
    } else if form.link.is_some() && form.grant.text.is_empty() {
        format!("c copy · o full screen · enter continue · ? more · esc {esc}")
    } else if form.link.is_some() {
        format!("ctrl-y copy · enter continue · ? more · esc {esc}")
    } else {
        format!("enter continue · ? more · esc {esc}")
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
