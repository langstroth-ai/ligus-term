//! Shell completions (`lt completions bash|zsh|fish`) and the man page
//! (`lt man`). Neither needs a device: they are answered before any data
//! dir is opened, so a package manager can run them at install time.
//!
//! The scripts complete flags, commands, help topics and theme names from
//! lt's own tables, and workspace names (after `--workspace` and `use`) by
//! calling `lt __complete workspaces`, which reads only the data dir's
//! workspace list (`workspaces.json`, names and relays, no secrets). It
//! never opens the database or the secret store, so tab completion can't
//! cause a keychain prompt; chats, proposals and agents live behind the
//! device's keys and aren't completed.

use std::fmt::Write as _;
use std::path::Path;

use crate::help::{COMMANDS, ENV, EXIT_CODES, TOPICS};

/// The shells `lt completions` writes a script for.
pub const SHELLS: &[&str] = &["bash", "zsh", "fish"];

/// The commands lt answers without a device, besides the plain ones.
pub const DEVICELESS: &[&str] = &["help", "completions", "man", "doctor"];

/// What the scripts ask `lt __complete` for.
pub fn complete(kind: &str, data_dir: Option<&Path>) -> Vec<String> {
    match kind {
        "commands" => {
            let mut out: Vec<String> = COMMANDS.iter().map(|c| c.name.to_string()).collect();
            out.extend(DEVICELESS.iter().filter(|d| !out.iter().any(|c| c == *d)).map(|s| s.to_string()).collect::<Vec<_>>());
            out
        }
        "topics" => TOPICS.iter().map(|(t, _)| t.to_string()).chain(COMMANDS.iter().map(|c| c.name.to_string())).collect(),
        "flags" => {
            crate::cli::flags().into_iter().map(|(long, ..)| format!("--{long}")).chain(["--help".into(), "--version".into()]).collect()
        }
        "themes" => ["auto", "dark", "light", "mono"].iter().map(|s| s.to_string()).collect(),
        "shells" => SHELLS.iter().map(|s| s.to_string()).collect(),
        "workspaces" => data_dir.map(workspace_names).unwrap_or_default(),
        _ => vec![],
    }
}

/// The workspace names in a data dir's list, read as plain JSON (no secret
/// store, no database).
pub fn workspace_names(data_dir: &Path) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct List {
        #[serde(default)]
        workspaces: Vec<Saved>,
    }
    #[derive(serde::Deserialize)]
    struct Saved {
        #[serde(default)]
        name: String,
    }
    let Ok(bytes) = std::fs::read(data_dir.join("workspaces.json")) else { return vec![] };
    let Ok(list) = serde_json::from_slice::<List>(&bytes) else { return vec![] };
    // Only printable names: a completion is pasted onto a command line.
    list.workspaces.into_iter().map(|w| w.name).filter(|n| !n.is_empty() && !n.chars().any(char::is_control)).collect()
}

/// The completion script for `shell`.
pub fn script(shell: &str) -> Option<String> {
    Some(match shell {
        "bash" => BASH.to_string(),
        "zsh" => ZSH.to_string(),
        "fish" => FISH.to_string(),
        _ => return None,
    })
}

const BASH: &str = r#"# bash completion for lt (lt completions bash)
_lt() {
    local cur prev words cword
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    local IFS=$'\n'
    case "$prev" in
        --theme) COMPREPLY=($(compgen -W "$(lt __complete themes)" -- "$cur")); return ;;
        --workspace|use) COMPREPLY=($(compgen -W "$(lt __complete workspaces)" -- "$cur")); return ;;
        --data-dir|--config|--recovery-words-file) COMPREPLY=($(compgen -f -- "$cur")); return ;;
        help) COMPREPLY=($(compgen -W "$(lt __complete topics)" -- "$cur")); return ;;
        completions) COMPREPLY=($(compgen -W "$(lt __complete shells)" -- "$cur")); return ;;
    esac
    if [[ "$cur" == -* ]]; then
        COMPREPLY=($(compgen -W "$(lt __complete flags)" -- "$cur"))
        return
    fi
    local i
    for ((i = 1; i < COMP_CWORD; i++)); do
        case "${COMP_WORDS[i]}" in
            --data-dir|--config|--theme|--name|--workspace|--recovery-words-file) ((i++)) ;;
            -*) ;;
            *) return ;;
        esac
    done
    COMPREPLY=($(compgen -W "$(lt __complete commands)" -- "$cur"))
}
complete -F _lt lt
"#;

const ZSH: &str = r#"#compdef lt
# zsh completion for lt (lt completions zsh)
_lt() {
    local -a words_
    case "${words[CURRENT-1]}" in
        --theme) words_=(${(f)"$(lt __complete themes)"}) ;;
        --workspace|use) words_=(${(f)"$(lt __complete workspaces)"}) ;;
        --data-dir|--config|--recovery-words-file) _files; return ;;
        help) words_=(${(f)"$(lt __complete topics)"}) ;;
        completions) words_=(${(f)"$(lt __complete shells)"}) ;;
        *)
            if [[ "${words[CURRENT]}" == -* ]]; then
                words_=(${(f)"$(lt __complete flags)"})
            elif (( CURRENT == 2 )) || [[ "${words[CURRENT-1]}" == --* ]]; then
                words_=(${(f)"$(lt __complete commands)"})
            else
                return
            fi
            ;;
    esac
    compadd -a words_
}
if [[ "${funcstack[1]}" == "_lt" ]]; then _lt "$@"; else compdef _lt lt; fi
"#;

const FISH: &str = r#"# fish completion for lt (lt completions fish)
complete -c lt -f
complete -c lt -n '__fish_use_subcommand' -a '(lt __complete commands)'
complete -c lt -n '__fish_seen_subcommand_from help' -a '(lt __complete topics)'
complete -c lt -n '__fish_seen_subcommand_from completions' -a '(lt __complete shells)'
complete -c lt -n '__fish_seen_subcommand_from use' -a '(lt __complete workspaces)'
complete -c lt -l theme -x -a '(lt __complete themes)'
complete -c lt -l workspace -x -a '(lt __complete workspaces)'
complete -c lt -l data-dir -r -F
complete -c lt -l config -r -F
complete -c lt -l recovery-words-file -r -F
complete -c lt -l name -x
for f in (lt __complete flags | string replace -- '--' '')
    complete -c lt -l $f
end
"#;

/// Escape text for roff: backslashes, and a leading dot or quote.
fn roff(s: &str) -> String {
    let s = s.replace('\\', "\\e").replace('-', "\\-");
    if s.starts_with('.') || s.starts_with('\'') { format!("\\&{s}") } else { s }
}

/// `lt man`: the man page (section 1), from the same tables as `lt --help`.
pub fn man_page() -> String {
    let mut m = String::new();
    let v = env!("CARGO_PKG_VERSION");
    let _ = writeln!(m, ".TH LT 1 \"\" \"lt {v}\" \"User Commands\"");
    m.push_str(".SH NAME\nlt \\- a full Ligus client in the terminal\n");
    m.push_str(".SH SYNOPSIS\n.B lt\n[\\fIFLAGS\\fR] [\\fICOMMAND\\fR [\\fIARGS\\fR]]\n");
    m.push_str(".SH DESCRIPTION\n");
    m.push_str(&roff(
        "With no command, lt opens the full-screen app. lt --plain is a line mode for screen readers and scripts, \
         and lt COMMAND runs one command and exits. lt help TOPIC explains the rest. \
         Ligus is short for ligustica, the Italian honeybee.",
    ));
    m.push_str("\n.SH OPTIONS\n");
    for (long, value, help) in crate::cli::flags() {
        let _ = writeln!(m, ".TP\n\\fB\\-\\-{}\\fR{}", roff(&long), value.map(|v| format!(" \\fI{}\\fR", roff(&v))).unwrap_or_default());
        let _ = writeln!(m, "{}", roff(&help.replace('\n', " ")));
    }
    m.push_str(".TP\n\\fB\\-h\\fR, \\fB\\-\\-help\\fR\nPrint help (\\-\\-help: everything).\n");
    m.push_str(".TP\n\\fB\\-V\\fR, \\fB\\-\\-version\\fR\nPrint the version.\n");
    m.push_str(".SH COMMANDS\n");
    for c in COMMANDS {
        let _ = writeln!(m, ".TP\n\\fB{}\\fR\n{}", roff(c.usage), roff(c.about));
    }
    m.push_str(
        ".TP\n\\fBhelp\\fR [\\fITOPIC\\fR|\\fICOMMAND\\fR|\\fI\\-\\-FLAG\\fR]\nExplain a topic, a command or a flag; needs no device.\n",
    );
    m.push_str(".TP\n\\fBcompletions\\fR \\fIbash\\fR|\\fIzsh\\fR|\\fIfish\\fR\nPrint a shell completion script.\n");
    m.push_str(".TP\n\\fBman\\fR\nPrint this page.\n");
    m.push_str(".SH HELP TOPICS\n");
    for (t, about) in TOPICS {
        let _ = writeln!(m, ".TP\n\\fBlt help {}\\fR\n{}", roff(t), roff(about));
    }
    m.push_str(".SH ENVIRONMENT\n");
    for e in ENV {
        let _ = writeln!(m, ".TP\n\\fB{}\\fR (default: {})\n{}", roff(e.name), roff(e.default), roff(e.meaning));
    }
    m.push_str(".SH EXIT STATUS\n.nf\n");
    m.push_str(&roff(EXIT_CODES));
    m.push_str("\n.fi\n.SH SEE ALSO\n\\fBligus\\fR(1), \\fBlangstroth\\-relay\\fR(1); https://github.com/langstroth\\-ai/ligus\\-term\n");
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_names_come_from_the_list_alone() {
        let dir = tempfile::tempdir().unwrap();
        assert!(workspace_names(dir.path()).is_empty(), "no list: nothing");
        std::fs::write(
            dir.path().join("workspaces.json"),
            r#"{"workspaces":[{"name":"Northwind API","room":"r1","relay_url":"https://relay.example"},{"name":"bad\u001b[2J","room":"r2"},{"room":"r3"}]}"#,
        )
        .unwrap();
        assert_eq!(workspace_names(dir.path()), ["Northwind API"]);
        std::fs::write(dir.path().join("workspaces.json"), "not json").unwrap();
        assert!(workspace_names(dir.path()).is_empty());
    }

    #[test]
    fn every_shell_has_a_script_and_the_page_names_every_command() {
        for sh in SHELLS {
            let s = script(sh).unwrap();
            assert!(s.contains("lt __complete commands") && s.contains("lt __complete workspaces"), "{sh}");
        }
        assert!(script("tcsh").is_none());
        let page = man_page();
        assert!(page.starts_with(".TH LT 1"));
        for c in COMMANDS {
            assert!(page.contains(&roff(c.name)), "{}", c.name);
        }
        for e in ENV {
            assert!(page.contains(&roff(e.name)), "{}", e.name);
        }
        assert!(complete("commands", None).contains(&"completions".to_string()));
        assert!(complete("flags", None).contains(&"--ascii".to_string()));
    }
}
