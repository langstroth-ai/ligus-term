//! Asking the terminal what its background is (OSC 11), so `theme = auto`
//! picks light or dark without `COLORFGBG`, which few terminals set.
//!
//! lt writes `ESC ] 11 ; ? ESC \` and then a primary device attributes
//! query (`ESC [ c`), which every terminal answers. The background reply,
//! if the terminal has one, arrives before that answer, so lt reads until
//! the device attributes come back or 100 ms pass, whichever is first,
//! and never waits on a terminal that ignores the question.

use std::time::Duration;

/// How long start-up waits for the terminal's answers at most.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(100);

/// Whether the background is light (`Some(true)`), dark, or unknown.
pub fn background_is_light(timeout: Duration) -> Option<bool> {
    let reply = query(timeout)?;
    parse_osc11(&reply)
}

/// The background colour in an OSC 11 reply (`ESC ] 11 ; rgb:RRRR/GGGG/BBBB`
/// ended by BEL or ST), as light or dark by its luminance.
pub fn parse_osc11(reply: &[u8]) -> Option<bool> {
    let text = String::from_utf8_lossy(reply);
    let start = text.find("]11;")? + 4;
    let rest = &text[start..];
    let end = rest.find(['\u{7}', '\u{1b}']).unwrap_or(rest.len());
    let spec = rest[..end].trim();
    let hex = spec.strip_prefix("rgb:").or_else(|| spec.strip_prefix("rgba:"))?;
    let parts: Vec<f64> = hex
        .split('/')
        .take(3)
        .map(|p| {
            let p = p.trim();
            if p.is_empty() || p.len() > 4 {
                return None;
            }
            let v = u32::from_str_radix(p, 16).ok()?;
            let max = (1u32 << (4 * p.len())) - 1;
            Some(f64::from(v) / f64::from(max))
        })
        .collect::<Option<Vec<_>>>()?;
    let [r, g, b] = parts.as_slice() else { return None };
    // Relative luminance (sRGB coefficients on the encoded values is close
    // enough to split light from dark).
    Some(0.2126 * r + 0.7152 * g + 0.0722 * b > 0.5)
}

#[cfg(unix)]
fn query(timeout: Duration) -> Option<Vec<u8>> {
    use std::io::{IsTerminal, Write};
    use std::time::Instant;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return None;
    }
    if std::env::var("TERM").is_ok_and(|t| t == "dumb") {
        return None;
    }
    crossterm::terminal::enable_raw_mode().ok()?;
    let mut out = std::io::stdout();
    let sent = out.write_all(b"\x1b]11;?\x1b\\\x1b[c").and_then(|_| out.flush()).is_ok();
    let mut buf: Vec<u8> = vec![];
    let deadline = Instant::now() + timeout;
    loop {
        if !sent {
            break;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut pfd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd for stdin, which outlives the call.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().clamp(1, 1000) as libc::c_int) };
        if ready <= 0 {
            break;
        }
        let mut chunk = [0u8; 256];
        // SAFETY: reading at most chunk.len() bytes into chunk.
        let n = unsafe { libc::read(0, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n <= 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n as usize]);
        if da1_done(&buf) || buf.len() > 4096 {
            break;
        }
    }
    let _ = crossterm::terminal::disable_raw_mode();
    Some(buf)
}

#[cfg(not(unix))]
fn query(_timeout: Duration) -> Option<Vec<u8>> {
    None
}

/// Whether the device attributes answer (`ESC [ ? … c`) has arrived.
fn da1_done(buf: &[u8]) -> bool {
    let Some(i) = buf.windows(3).position(|w| w == b"\x1b[?") else { return false };
    buf[i + 3..].contains(&b'c')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_background_from_a_reply() {
        assert_eq!(parse_osc11(b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\\x1b[?62;22c"), Some(true));
        assert_eq!(parse_osc11(b"\x1b]11;rgb:1e1e/1e1e/2e2e\x07"), Some(false));
        assert_eq!(parse_osc11(b"\x1b]11;rgb:fd/f6/e3\x07"), Some(true), "two hex digits per channel");
        assert_eq!(parse_osc11(b"\x1b]11;rgba:0000/0000/0000/ffff\x07"), Some(false));
        assert_eq!(parse_osc11(b"\x1b[?62;22c"), None, "no answer: unknown");
        assert_eq!(parse_osc11(b"\x1b]11;rgb:zz/00/00\x07"), None);
        assert!(da1_done(b"\x1b]11;rgb:0/0/0\x07\x1b[?1;2c"));
        assert!(!da1_done(b"\x1b]11;rgb:0/0/0\x07"));
    }
}
