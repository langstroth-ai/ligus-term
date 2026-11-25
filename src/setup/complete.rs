//! Folder completion for a setup path field (`tab`).
//!
//! It lists one directory's subfolders and nothing else: it never reads a
//! file, never descends past the folder typed, and drops names a terminal
//! would act on (control and bidi characters), so a crafted folder name
//! can't paint over the screen. Hidden folders show only when the typed
//! name starts with a dot. The folder itself is checked later by the
//! engine (a git checkout, not a linked worktree, not bound elsewhere).

use std::path::{Path, PathBuf};

/// The most entries read from one directory, and the most offered.
const READ_CAP: usize = 4096;
const OFFER_CAP: usize = 200;

/// A completion: the text to put in the field and the folders that match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub candidates: Vec<String>,
}

fn shown_ok(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
}

/// `~/` expanded with `home`.
pub fn expand(input: &str, home: Option<&Path>) -> PathBuf {
    match (input.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => h.join(rest),
        _ if input == "~" => home.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(input)),
        _ => PathBuf::from(input),
    }
}

/// Complete `input` (as typed, `~/` kept) to the subfolders of the folder
/// it names.
pub fn complete(input: &str, home: Option<&Path>) -> Completion {
    let none = || Completion { text: input.to_string(), candidates: vec![] };
    let (dir_typed, prefix) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    };
    let dir = if dir_typed.is_empty() { PathBuf::from(".") } else { expand(dir_typed, home) };
    let Ok(rd) = std::fs::read_dir(&dir) else { return none() };
    let mut names: Vec<String> = rd
        .take(READ_CAP)
        .filter_map(|e| e.ok())
        // A link to a folder counts (metadata follows it); its target is
        // only listed, never read.
        .filter(|e| std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| shown_ok(n) && n.starts_with(prefix) && (prefix.starts_with('.') || !n.starts_with('.')))
        .collect();
    names.sort();
    names.truncate(OFFER_CAP);
    match names.as_slice() {
        [] => none(),
        [one] => Completion { text: format!("{dir_typed}{one}/"), candidates: names.clone() },
        many => {
            let common = many
                .iter()
                .skip(1)
                .fold(many[0].clone(), |acc, n| acc.chars().zip(n.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a).collect());
            let text = if common.len() > prefix.len() { format!("{dir_typed}{common}") } else { input.to_string() };
            Completion { text, candidates: names }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_folders_only() {
        let d = tempfile::tempdir().unwrap();
        for n in ["code", "configs", ".hidden"] {
            std::fs::create_dir(d.path().join(n)).unwrap();
        }
        std::fs::write(d.path().join("copy.txt"), "x").unwrap();
        let base = format!("{}/", d.path().display());
        let c = complete(&format!("{base}co"), None);
        assert_eq!(c.candidates, vec!["code", "configs"], "files and hidden folders are left out");
        assert_eq!(c.text, format!("{base}co"));
        let c = complete(&format!("{base}cod"), None);
        assert_eq!(c.text, format!("{base}code/"));
        assert_eq!(complete(&format!("{base}.h"), None).candidates, vec![".hidden"]);
        assert!(complete(&format!("{base}nothing/x"), None).candidates.is_empty());
    }

    #[test]
    fn tilde_is_home() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("work")).unwrap();
        let c = complete("~/wo", Some(d.path()));
        assert_eq!(c.text, "~/work/");
        assert_eq!(expand("~/work", Some(d.path())), d.path().join("work"));
    }

    #[cfg(unix)]
    #[test]
    fn names_a_terminal_would_act_on_are_dropped() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("ok")).unwrap();
        std::fs::create_dir(d.path().join("o\x1b]0;pwned\x07")).unwrap();
        let c = complete(&format!("{}/o", d.path().display()), None);
        assert_eq!(c.candidates, vec!["ok"]);
    }
}
