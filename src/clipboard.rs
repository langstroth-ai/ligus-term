//! Copy a code to the system clipboard, only when the person asks (`c`):
//! the terminal's OSC 52 (works over SSH and in tmux with passthrough), and
//! the platform's own tool when there is one (`pbcopy`, `wl-copy`, `xclip`,
//! `xsel`, `clip.exe`). Nothing is copied on its own, and a copied code is
//! never logged: [`Clip`]'s `Debug` shows only its length.

use std::io::Write;
use std::process::{Command, Stdio};

/// What to copy, and what to suggest when no clipboard is reachable.
#[derive(Clone, PartialEq, Eq)]
pub struct Clip {
    pub text: String,
    /// What to do instead, after "Couldn't reach the clipboard — " ("run
    /// lt --plain link-request").
    pub fallback: &'static str,
}

impl std::fmt::Debug for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Clip(‹{} chars›)", self.text.chars().count())
    }
}

impl Clip {
    pub fn new(text: impl Into<String>, fallback: &'static str) -> Clip {
        Clip { text: text.into(), fallback }
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The OSC 52 sequence that sets the clipboard to `text` (inside tmux,
/// through its passthrough).
pub fn osc52(text: &str, tmux: bool) -> Vec<u8> {
    let seq = format!("\x1b]52;c;{}\x07", base64(text.as_bytes()));
    if tmux { format!("\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b")).into_bytes() } else { seq.into_bytes() }
}

/// The platform tools to try, in order.
fn tools() -> Vec<(&'static str, &'static [&'static str])> {
    let mut v: Vec<(&'static str, &'static [&'static str])> = vec![];
    if cfg!(target_os = "macos") {
        v.push(("pbcopy", &[]));
    }
    if cfg!(windows) {
        v.push(("clip.exe", &[]));
    }
    if cfg!(all(unix, not(target_os = "macos"))) {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            v.push(("wl-copy", &[]));
        }
        if std::env::var_os("DISPLAY").is_some() {
            v.push(("xclip", &["-selection", "clipboard"]));
            v.push(("xsel", &["--clipboard", "--input"]));
        }
        // WSL reaches the Windows clipboard.
        if std::env::var_os("WSL_DISTRO_NAME").is_some() {
            v.push(("clip.exe", &[]));
        }
    }
    v
}

/// Copy with the platform's tool: its name, or `None` when none worked.
/// Blocking (it waits for the tool).
pub fn copy_native(text: &str) -> Option<&'static str> {
    for (tool, args) in tools() {
        let Ok(mut child) = Command::new(tool).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
            continue;
        };
        let wrote = child.stdin.take().is_some_and(|mut i| i.write_all(text.as_bytes()).is_ok());
        if wrote && child.wait().is_ok_and(|s| s.success()) {
            return Some(tool);
        }
    }
    None
}

/// What the toast says: `Ok` when a copy reached a clipboard (or the
/// terminal was asked to), else the one-line fallback.
pub fn outcome(native: Option<&str>, sent_osc52: bool, fallback: &str) -> Result<String, String> {
    match (native, sent_osc52) {
        (Some(_), _) => Ok("Copied".into()),
        (None, true) => Ok("Copied (through the terminal)".into()),
        (None, false) => Err(format!("Couldn't reach the clipboard — {fallback}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (i, o) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn osc52_sets_the_clipboard_and_passes_through_tmux() {
        assert_eq!(osc52("hi", false), b"\x1b]52;c;aGk=\x07".to_vec());
        assert_eq!(osc52("hi", true), b"\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\".to_vec());
    }

    #[test]
    fn a_clip_never_prints_its_text() {
        let c = Clip::new("hivews1:secret", "run lt --plain invite");
        assert_eq!(format!("{c:?}"), "Clip(‹14 chars›)");
    }

    #[test]
    fn the_toast_says_copied_or_how_to_get_the_code() {
        assert_eq!(outcome(Some("pbcopy"), true, "x"), Ok("Copied".into()));
        assert_eq!(outcome(None, true, "x"), Ok("Copied (through the terminal)".into()));
        assert_eq!(
            outcome(None, false, "run lt --plain link-request"),
            Err("Couldn't reach the clipboard — run lt --plain link-request".into())
        );
    }
}
