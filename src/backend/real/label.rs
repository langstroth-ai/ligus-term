//! This computer's name in Ligus ("lt on Michael’s MacBook Air"): chosen
//! once, the first time lt opens a data dir, from the most stable name the
//! OS has, and kept in the data dir. Later launches read it back and never
//! re-derive it: a Mac's network host name changes with the network
//! ("Michaels-Air-3" one day, "Michaels-MacBook-Air-3" the next), and a
//! device name that follows it would show as a different computer each day.
//! `lt device-rename` is the only thing that changes it afterwards.

use std::path::Path;

/// The file under the data dir that holds the chosen name.
pub const LABEL_FILE: &str = "device-label";
/// What every name lt picks for itself starts with.
pub const AUTO_PREFIX: &str = "lt on ";
/// The longest device name Ligus takes (`ligus_core::profile::MAX_LABEL_CHARS`).
const MAX_CHARS: usize = 48;

/// The name to give this computer, from what the OS says now: on macOS the
/// Sharing name (`scutil --get ComputerName`), then its `LocalHostName`; on
/// Linux `/etc/hostname`; then the host name.
pub fn choose() -> String {
    from_sources(&os_names())
}

/// `lt on ‹the first usable name›`, or `lt on this machine` when none is.
pub(crate) fn from_sources(names: &[Option<String>]) -> String {
    let host = names.iter().flatten().find_map(|n| plain(n)).unwrap_or_else(|| "this machine".into());
    fit(&format!("{AUTO_PREFIX}{host}"))
}

/// Whether `label` looks like one lt picked itself (not a person's rename).
pub fn is_auto(label: &str) -> bool {
    label.starts_with(AUTO_PREFIX)
}

/// The stored name, if this data dir has one.
pub fn load(data_dir: &Path) -> Option<String> {
    let s = std::fs::read_to_string(data_dir.join(LABEL_FILE)).ok()?;
    let s = fit(&s);
    (!s.is_empty()).then_some(s)
}

/// Keep `label` as this data dir's name (written whole, then moved in).
pub fn store(data_dir: &Path, label: &str) -> std::io::Result<()> {
    let tmp = data_dir.join(format!("{LABEL_FILE}.tmp"));
    std::fs::write(&tmp, format!("{}\n", fit(label)))?;
    std::fs::rename(&tmp, data_dir.join(LABEL_FILE))
}

/// One line, single spaces, no control characters, at most 48 characters.
pub(crate) fn fit(label: &str) -> String {
    let clean: String = label.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    clean.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_CHARS).collect::<String>().trim_end().to_string()
}

/// A name with any network part dropped ("mini.local", "mini.lan",
/// "mini.example.com" are all "mini"); none for an empty one or `localhost`.
fn plain(name: &str) -> Option<String> {
    let n = name.trim();
    // A dotted name is a network host name; a Sharing name keeps its dots
    // only when it has spaces too ("Ana's Mac v2.1"), which a host name never does.
    let n = if n.contains(' ') { n } else { n.split('.').next().unwrap_or("") };
    let n = fit(n);
    (!n.is_empty() && !n.eq_ignore_ascii_case("localhost")).then_some(n)
}

#[cfg(target_os = "macos")]
fn os_names() -> Vec<Option<String>> {
    let scutil = |key: &str| {
        let out = std::process::Command::new("/usr/sbin/scutil").args(["--get", key]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    vec![scutil("ComputerName"), scutil("LocalHostName"), ligus_runtime::machine_hostname()]
}

#[cfg(not(target_os = "macos"))]
fn os_names() -> Vec<Option<String>> {
    vec![std::fs::read_to_string("/etc/hostname").ok(), ligus_runtime::machine_hostname()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn prefers_the_computer_name() {
        let names = [s("Michael’s MacBook Air\n"), s("Michaels-MacBook-Air-3"), s("Michaels-Air-3.local")];
        assert_eq!(from_sources(&names), "lt on Michael’s MacBook Air");
    }

    #[test]
    fn falls_back_in_order() {
        assert_eq!(from_sources(&[None, s("Michaels-MacBook-Air"), s("x.local")]), "lt on Michaels-MacBook-Air");
        assert_eq!(from_sources(&[s("  "), None, s("mini.local")]), "lt on mini");
        assert_eq!(from_sources(&[None, s("localhost"), s("box.example.com")]), "lt on box");
        assert_eq!(from_sources(&[None, None]), "lt on this machine");
    }

    #[test]
    fn fits_ligus_limits() {
        let long = "A".repeat(80);
        let l = from_sources(&[s(&long)]);
        assert_eq!(l.chars().count(), 48);
        assert!(l.starts_with(AUTO_PREFIX));
        assert_eq!(fit("a\tb\n  c "), "a b c");
    }

    #[test]
    fn stored_name_survives_and_wins() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load(tmp.path()), None);
        store(tmp.path(), "lt on Michael’s MacBook Air").unwrap();
        assert_eq!(load(tmp.path()).as_deref(), Some("lt on Michael’s MacBook Air"));
        store(tmp.path(), "Work laptop").unwrap();
        assert_eq!(load(tmp.path()).as_deref(), Some("Work laptop"));
        assert!(!tmp.path().join(format!("{LABEL_FILE}.tmp")).exists());
    }

    #[test]
    fn auto_names_are_told_apart_from_renames() {
        assert!(is_auto("lt on Michaels-Air-3"));
        assert!(!is_auto("Work laptop"));
    }

    #[test]
    fn chooses_something_here() {
        let l = choose();
        assert!(l.starts_with(AUTO_PREFIX) && l.chars().count() <= 48 && !l.ends_with(".local"), "{l}");
    }
}
