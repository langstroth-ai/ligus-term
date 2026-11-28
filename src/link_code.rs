//! Reading a desktop's code (`ligusgrant1:…`) out of whatever was pasted or
//! copied: the code alone, the code with the whitespace and line breaks a
//! terminal or a text box adds, or a whole command line someone copied
//! around it. Shared by the link form and `lt link-redeem`.
//!
//! Also the person's clock (`HH:MM`) for the "Linking waiting…" line.

/// The prefix of a desktop's grant.
pub const GRANT_PREFIX: &str = "ligusgrant1:";
/// The prefix of this computer's own request.
pub const REQUEST_PREFIX: &str = "liguslink1:";
/// The longest text read for a code (a paste, the clipboard, stdin).
pub const MAX_PASTE: usize = 64 * 1024;

/// What a paste holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pasted {
    /// A desktop's code, cut out of what surrounded it.
    Grant(String),
    /// This computer's own request (`liguslink1:`): it goes to the desktop.
    Request,
    /// The `lt --plain link-redeem` command the desktop copies, without a
    /// code in it: the code is copied separately (Copy for lt).
    Command,
    /// Nothing that looks like either.
    Nothing,
}

fn b64url(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// The desktop's code in `text`. The code is base64url after its prefix;
/// it runs on across a line break (a terminal wraps long lines) when the
/// next line goes on with code characters, and stops at a space, a quote,
/// or a flag (`--…`) after a break.
pub fn grant_in(text: &str) -> Pasted {
    let text = if text.len() > MAX_PASTE { &text[..floor_char(text, MAX_PASTE)] } else { text };
    let Some(at) = text.find(GRANT_PREFIX) else {
        if text.contains(REQUEST_PREFIX) {
            return Pasted::Request;
        }
        if text.contains("link-redeem") || text.contains("devices redeem") {
            return Pasted::Command;
        }
        return Pasted::Nothing;
    };
    let rest: Vec<char> = text[at + GRANT_PREFIX.len()..].chars().collect();
    let mut body = String::new();
    let mut i = 0;
    while i < rest.len() {
        let c = rest[i];
        if b64url(c) {
            body.push(c);
            i += 1;
            continue;
        }
        if c == '\n' || c == '\r' {
            // Skip the break and the next line's indent.
            let mut j = i;
            while j < rest.len() && matches!(rest[j], '\n' | '\r' | ' ' | '\t') {
                j += 1;
            }
            let flag = rest.get(j) == Some(&'-') && rest.get(j + 1) == Some(&'-');
            if j < rest.len() && b64url(rest[j]) && !flag && !body.is_empty() {
                i = j;
                continue;
            }
        }
        break;
    }
    if body.is_empty() {
        return Pasted::Nothing;
    }
    Pasted::Grant(format!("{GRANT_PREFIX}{body}"))
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// What to say when a paste holds no desktop code.
pub fn not_a_grant(p: &Pasted, from_clipboard: bool) -> String {
    match p {
        Pasted::Request => "that's this computer's request (liguslink1:…): it goes to your desktop; paste back the code the \
                            desktop shows after you approve"
            .into(),
        Pasted::Command if from_clipboard => {
            "the clipboard holds the command, not the code: on your desktop press Copy for lt, then press Enter here".into()
        }
        Pasted::Command => "that's the command, not the code: on your desktop press Copy for lt, then paste here".into(),
        _ if from_clipboard => {
            "the clipboard doesn't hold a desktop code (ligusgrant1:…): on your desktop press Copy for lt, then try again".into()
        }
        _ => "not a Ligus device grant (expected ligusgrant1:...): paste the code your desktop shows".into(),
    }
}

/// Whether two fingerprints are the same, however they were written
/// (`2C41-88E0-…`, `2c41 88e0 …`).
pub fn same_fingerprint(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect::<String>();
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && a == b
}

/// This computer's offset from UTC now, in seconds (0 when unknown).
pub fn local_offset_secs() -> i32 {
    #[cfg(unix)]
    {
        // SAFETY: localtime_r writes only into `tm`.
        unsafe {
            let now = libc::time(std::ptr::null_mut());
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&now, &mut tm).is_null() {
                return 0;
            }
            tm.tm_gmtoff as i32
        }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// `ms` (Unix ms) as `HH:MM` on a clock `offset_secs` from UTC.
pub fn hh_mm(ms: i64, offset_secs: i32) -> String {
    let secs = ms.div_euclid(1000) + i64::from(offset_secs);
    let day = secs.rem_euclid(86_400);
    format!("{:02}:{:02}", day / 3600, (day % 3600) / 60)
}

/// A short code the desktop shows (`K7Q-4MX`, Add a computer), typed
/// alone or with the relay after it for a self-hosted relay
/// (`K7Q-4MX https://relay.example.org`). The code as typed (`K7Q4MX`) and
/// the relay, or `None` when the text isn't one.
pub fn short_code_in(text: &str) -> Option<(String, Option<String>)> {
    let text = text.trim();
    let (code, relay) = match text.find("http://").or_else(|| text.find("https://")) {
        Some(at) => (&text[..at], Some(text[at..].trim().trim_end_matches('/').to_string())),
        None => (text, None),
    };
    if relay.as_deref().is_some_and(|r| r.contains(char::is_whitespace)) {
        return None;
    }
    let code = ligus_core::link_code::ShortCode::parse(code)?;
    Some((code.as_str().to_string(), relay))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_short_code_is_read_alone_or_with_its_relay() {
        assert_eq!(super::short_code_in(" k7q-4mx "), Some(("K7Q4MX".into(), None)));
        assert_eq!(
            super::short_code_in("K7Q 4MX https://relay.example.org/"),
            Some(("K7Q4MX".into(), Some("https://relay.example.org".into())))
        );
        assert_eq!(super::short_code_in("K7Q4MX http://127.0.0.1:9"), Some(("K7Q4MX".into(), Some("http://127.0.0.1:9".into()))));
        for t in ["", "K7Q4M", "ligusaddr1:abc", "ligusgrant1:abc", "K7Q4MX https://a b", "https://relay.example.org"] {
            assert_eq!(super::short_code_in(t), None, "{t:?}");
        }
    }

    use super::*;

    const G: &str = "ligusgrant1:eyJ2IjoxLCJh_Zm9v-YmFy";

    #[test]
    fn the_code_alone_or_with_whitespace() {
        assert_eq!(grant_in(G), Pasted::Grant(G.into()));
        assert_eq!(grant_in(&format!("\n  {G}  \n\n")), Pasted::Grant(G.into()));
        assert_eq!(grant_in(&format!("{G}\r\n")), Pasted::Grant(G.into()));
    }

    #[test]
    fn a_code_a_terminal_wrapped_is_joined() {
        let wrapped = "ligusgrant1:eyJ2IjoxLCJh\n  _Zm9v-YmFy\n";
        assert_eq!(grant_in(wrapped), Pasted::Grant(G.into()));
    }

    #[test]
    fn a_whole_command_gives_just_the_code() {
        for cmd in [
            format!("lt --plain link-redeem '{G}' --approver-fingerprint 2C41-88E0-1B9D-04AF-6E73"),
            format!("lt --plain link-redeem \"{G}\"\n--approver-fingerprint 2C41"),
            format!("$ lt --plain link-redeem {G} --approver-fingerprint 2C41"),
            format!("ligus devices redeem {G}"),
        ] {
            assert_eq!(grant_in(&cmd), Pasted::Grant(G.into()), "{cmd}");
        }
    }

    #[test]
    fn a_request_or_the_bare_command_is_named() {
        assert_eq!(grant_in("liguslink1:abc"), Pasted::Request);
        assert_eq!(grant_in("lt --plain link-redeem --approver-fingerprint 2C41-88E0"), Pasted::Command);
        assert_eq!(grant_in("hello"), Pasted::Nothing);
        assert_eq!(grant_in("ligusgrant1:"), Pasted::Nothing);
        assert!(not_a_grant(&Pasted::Command, false).contains("Copy for lt"));
        assert!(not_a_grant(&Pasted::Nothing, true).contains("clipboard"));
    }

    #[test]
    fn fingerprints_match_however_written() {
        assert!(same_fingerprint("2C41-88E0-1B9D", "2c41 88e0 1b9d"));
        assert!(!same_fingerprint("2C41-88E0", "2C41-88E1"));
        assert!(!same_fingerprint("", "--"));
    }

    #[test]
    fn clock_times() {
        // 2026-11-25 14:02:30 UTC.
        let ms = 1_795_615_350_000;
        assert_eq!(hh_mm(ms, 0), "14:02");
        assert_eq!(hh_mm(ms, -5 * 3600), "09:02");
        assert_eq!(hh_mm(ms, 11 * 3600), "01:02");
    }
}
