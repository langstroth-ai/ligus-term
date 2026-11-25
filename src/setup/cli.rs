//! `lt setup`: the guided flows on the command line, the same as `ligus
//! setup` (flags are the engine's field ids, one `ligus.checks.v1`
//! document with `--json`, the shared exit codes).
//!
//! * `lt setup` lists the flows and what is done on this device.
//! * `lt setup FLOW [--FIELD VALUE …] [--json] [--dry-run]` runs one. A
//!   secret field takes `-` (stdin) or `@FILE` (owner-only), never its value
//!   on the command line; a value given there is refused without being
//!   echoed.
//! * Exit 4 is a person's step: approving a box, pairing a phone with
//!   `ligus pair` in your own terminal, creating an invite where its code is
//!   shown. `next` says what to do.
//!
//! In interactive plain mode (`lt --plain`, then `setup …`) the same
//! commands print records instead ([`plain`]); secrets come only from
//! `@FILE` there, since stdin carries the commands.

use std::collections::BTreeMap;
use std::sync::Arc;

use ligus_runtime::setup::{FlowId, SetupOps, Surface};
use zeroize::Zeroizing;

use super::{CancelFlag, FieldType, FlowRow, LtFlow, Progress, SetupForm, SetupInputs, SetupOutcome, SetupSecrets, exit};
use crate::backend::Backend;
use crate::backend::model::WorkspaceId;

/// What a command line asked for.
#[derive(Debug)]
pub struct Parsed {
    pub flow: Option<LtFlow>,
    pub json: bool,
    pub dry_run: bool,
    pub help: bool,
    pub inputs: SetupInputs,
    pub secrets: SetupSecrets,
}

/// A usage problem (exit 2). Never carries a secret.
#[derive(Debug, PartialEq, Eq)]
pub struct Usage(pub String);

/// Where a secret flag's value comes from.
pub enum SecretSource<'a> {
    /// One-shot: `-` reads this (stdin, once); `@FILE` an owner-only file.
    Stdin(&'a mut dyn FnMut() -> std::io::Result<Zeroizing<String>>),
    /// Interactive plain mode: only `@FILE`.
    FileOnly,
}

/// An argument as an error may show it: a short plain word, else nothing
/// (it may be a code pasted where a flag was meant).
fn shown(a: &str) -> String {
    if a.len() <= 24 && a.chars().all(|c| c.is_ascii_lowercase() || c == '-') { a.to_string() } else { "(not shown)".to_string() }
}

/// The flow named first, if any (`lt setup agent …`).
pub fn flow_arg(args: &[String]) -> Result<Option<LtFlow>, Usage> {
    // The first word that isn't a flag or a flag's value.
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with("--") {
            let bare = matches!(a.as_str(), "--json" | "--dry-run" | "--help") || a.contains('=');
            i += if bare { 1 } else { 2 };
            continue;
        }
        if a == "-h" {
            i += 1;
            continue;
        }
        // Never echoed: it may be a secret pasted where a flag was meant.
        return LtFlow::parse(a).map(Some).ok_or_else(|| Usage("unknown flow (not shown): lt setup lists them".into()));
    }
    Ok(None)
}

/// Parse `args` (everything after `setup`) against the flow's fields.
pub fn parse(args: &[String], form: Option<&SetupForm>, mut secret_src: SecretSource) -> Result<Parsed, Usage> {
    let flow = flow_arg(args)?;
    let mut p = Parsed { flow, json: false, dry_run: false, help: false, inputs: BTreeMap::new(), secrets: SetupSecrets::new() };
    let fname = flow.map(|f| f.id()).unwrap_or("");
    let mut seen_flow = false;
    let mut i = 0;
    let mut stdin_used = false;
    while i < args.len() {
        let a = args[i].as_str();
        i += 1;
        match a {
            "--json" => p.json = true,
            "--dry-run" => p.dry_run = true,
            "--help" | "-h" => p.help = true,
            _ if !a.starts_with("--") => {
                if !seen_flow && a == fname {
                    seen_flow = true;
                    continue;
                }
                return Err(Usage(format!(
                    "unexpected argument (not shown) for lt setup {fname}: flags are --FIELD VALUE; secrets take - or @FILE"
                )));
            }
            _ => {
                let Some(form) = form else { return Err(Usage(format!("{}: name a flow first (lt setup FLOW --FIELD …)", shown(a)))) };
                let (name, inline) = match a[2..].split_once('=') {
                    Some((n, v)) => (n, Some(v.to_string())),
                    None => (&a[2..], None),
                };
                let Some(f) = form.fields().find(|f| f.id == name) else {
                    return Err(Usage(format!("lt setup {fname} has no flag --{}: lt setup {fname} --help lists them", shown(name))));
                };
                match f.kind {
                    FieldType::Toggle => {
                        p.inputs.insert(f.id.clone(), inline.unwrap_or_else(|| "true".into()));
                    }
                    FieldType::Secret => {
                        let v = match inline {
                            Some(v) => Some(v),
                            None => {
                                i += 1;
                                args.get(i - 1).cloned()
                            }
                        };
                        let flag = format!("--{}", f.id);
                        let value = match v.as_deref() {
                            Some("-") => match &mut secret_src {
                                SecretSource::Stdin(read) if !stdin_used => {
                                    stdin_used = true;
                                    Zeroizing::new(read().map_err(|e| Usage(format!("{flag} -: reading stdin: {e}")))?.trim().to_string())
                                }
                                SecretSource::Stdin(_) => return Err(Usage(format!("{flag} -: stdin is already used by another secret"))),
                                SecretSource::FileOnly => {
                                    return Err(Usage(format!("{flag} takes @FILE here (stdin carries the commands)")));
                                }
                            },
                            Some(path) if path.starts_with('@') => {
                                let raw = Zeroizing::new(
                                    crate::backend::presence::read_owner_only(std::path::Path::new(&path[1..]), 64 * 1024)
                                        // Not the path: it may be a secret typed after @.
                                        .map_err(|e| {
                                            Usage(format!("{flag} @FILE: {}", e.rsplit(": ").next().unwrap_or("can't read it")))
                                        })?,
                                );
                                Zeroizing::new(raw.trim().to_string())
                            }
                            Some(_) => {
                                return Err(Usage(format!(
                                    "{flag} takes - (stdin) or @FILE (owner-only), never the secret itself on the command line; if you typed one, rotate it"
                                )));
                            }
                            None => return Err(Usage(format!("{flag} needs - (stdin) or @FILE"))),
                        };
                        p.secrets.set(&f.id, value);
                    }
                    _ => {
                        let v = match inline {
                            Some(v) => v,
                            None => {
                                i += 1;
                                args.get(i - 1).cloned().ok_or_else(|| Usage(format!("--{} needs a value", f.id)))?
                            }
                        };
                        let v = if f.kind == FieldType::Path {
                            super::complete::expand(&v, dirs::home_dir().as_deref()).display().to_string()
                        } else {
                            v
                        };
                        p.inputs.insert(f.id.clone(), v);
                    }
                }
            }
        }
    }
    Ok(p)
}

/// Text for a terminal: each line through plain mode's cleaner (no
/// escape sequences or bidi controls from what a peer chose).
fn clean_text(s: &str) -> String {
    s.lines().map(|l| crate::plain::clean(l) + "\n").collect()
}

/// The word for a row's progress.
pub fn progress_word(p: Progress) -> &'static str {
    match p {
        Progress::Done => "done",
        Progress::Waiting => "waiting",
        Progress::Todo => "todo",
        Progress::Unavailable => "unavailable",
    }
}

/// `lt setup` in text.
pub fn list_text(rows: &[FlowRow]) -> String {
    let mut s = String::new();
    for r in rows {
        let mark = match r.progress {
            Progress::Done => "done ",
            Progress::Waiting => "wait ",
            _ => "     ",
        };
        s.push_str(&format!("{mark} {:<7} {:<18} {}\n", r.flow.id(), r.flow.title(), r.detail));
    }
    s.push_str("\nRun one: lt setup FLOW --help (its flags), lt setup FLOW --dry-run (checks only). Every flow is safe to run again.\n");
    s
}

/// A run in text: the checks, then one line.
pub fn outcome_text(o: &SetupOutcome) -> String {
    let mut s: String = o.checks.iter().map(|c| c.text()).collect();
    let head = if o.done && o.exit == 0 {
        if o.dry_run { "dry run: nothing was written".to_string() } else { "done".to_string() }
    } else if o.person() {
        "waiting for a person".to_string()
    } else {
        let fails = o.checks.iter().filter(|c| c.status == super::CheckStatus::Fail).count();
        format!("stopped ({fails} failed)")
    };
    let next = if o.next.is_empty() { String::new() } else { format!(" · next: {}", o.next.join(" ; ")) };
    s.push_str(&format!("{head}{next}\n"));
    s
}

/// A flow's flags, from its form.
pub fn flow_help(form: &SetupForm) -> String {
    let f = form.flow;
    let mut s = format!("lt setup {} [--FIELD VALUE …] [--json] [--dry-run]\n\n  {}: {}.\n", f.id(), f.title(), f.summary());
    for l in crate::ui::wrap(f.why(), 76) {
        s.push_str(&format!("  {l}\n"));
    }
    s.push('\n');
    for st in &form.steps {
        for fd in &st.fields {
            let value = match &fd.kind {
                FieldType::Toggle => String::new(),
                FieldType::Secret => " -|@FILE".into(),
                FieldType::Choice(o) => format!(" {}", o.iter().map(|c| c.value.as_str()).collect::<Vec<_>>().join("|")),
                FieldType::Multi(_) => " NAME,NAME".into(),
                FieldType::Path => " PATH".into(),
                _ => " VALUE".into(),
            };
            let mut notes = vec![];
            if fd.required {
                notes.push("required".to_string());
            }
            if let Some(d) = &fd.default {
                notes.push(format!("default {d}"));
            }
            if let Some((k, v)) = &fd.when {
                notes.push(format!("with --{k} {}", v.join("|")));
            }
            let extra = if notes.is_empty() { String::new() } else { format!(" ({})", notes.join(", ")) };
            s.push_str(&format!("  --{}{value}\n", fd.id));
            for l in crate::ui::wrap(&format!("{}{extra}. {}", fd.label, fd.help), 72) {
                s.push_str(&format!("      {l}\n"));
            }
        }
        if st.kind == super::StepKind::Person && !st.where_.is_empty() {
            s.push_str(&format!("  A person's step, {}: {}\n", st.title.to_lowercase(), st.where_));
        }
    }
    s.push_str(
        "\n  Exit: 0 done, 1 a check or write failed, 2 usage, 3 something to install or fix first,\n  \
         4 a person must act (next says what), 6 refused by your role or lockdown, 7 offline.\n",
    );
    s
}

/// The engine's forms with nothing known, for `lt help setup`.
struct Offline;
impl SetupOps for Offline {
    fn surface(&self) -> Surface {
        Surface::Term
    }
    fn sandbox(&self) -> ligus_runtime::setup::SandboxFact {
        ligus_runtime::setup::SandboxFact { tier: "seatbelt".into(), summary: String::new() }
    }
    fn inspect_folder(&self, path: &str) -> ligus_runtime::setup::FolderFact {
        ligus_runtime::setup::FolderFact { path: path.into(), ..Default::default() }
    }
}

/// Every flow's flags (`lt help setup`).
pub fn all_help() -> String {
    let mut s = String::from(
        "Guided setup (lt setup)\n\n  \
         lt setup                 the flows and what is done on this device (--json: ligus.checks.v1)\n  \
         lt setup FLOW [flags]    run one; --dry-run checks and writes nothing; --json prints one document\n\n  \
         In the full-screen app u (or the palette: Set up an agent, Connect a tool, ...) opens the list;\n  \
         enter runs a flow one step per screen; esc cancels (a running flow undoes its own writes);\n  \
         d toggles a dry run. Secrets (a token, an admission code) are masked, kept only in memory and\n  \
         never printed or logged; on the command line they take - (stdin) or @FILE (owner-only).\n\n",
    );
    for f in FlowId::ALL {
        s.push_str(&flow_help(&super::form(f, &Offline, &BTreeMap::new())));
        s.push('\n');
    }
    s.push_str(&flow_help(&super::device_form()));
    s.push('\n');
    s.push_str(
        "Agents run on Claude Code, Codex, Copilot, Antigravity or a local model (Aider, Pi and gateways:\n\
         the desktop or ligus setup agent). Tools reach the agents whose turns run the tool loop here (a\n\
         model API or a local model); coding CLIs keep their own MCP config.\n",
    );
    s
}

/// The workspace a command means: `--workspace` (a name, id or number),
/// else the first.
pub async fn workspace(b: &Arc<dyn Backend>, want: Option<&str>) -> Result<Option<WorkspaceId>, String> {
    let st = b.startup().await.map_err(|e| e.0)?;
    match want {
        None => Ok(st.workspaces.first().map(|w| w.id.clone())),
        Some(w) => {
            let by_n = w.parse::<usize>().ok().and_then(|n| st.workspaces.get(n.wrapping_sub(1)));
            by_n.or_else(|| st.workspaces.iter().find(|x| x.id.as_str() == w || x.name == w))
                .map(|x| Some(x.id.clone()))
                .ok_or_else(|| format!("no workspace {w}"))
        }
    }
}

fn visible(b: &Arc<dyn Backend>, rows: Vec<FlowRow>) -> Vec<FlowRow> {
    rows.into_iter().filter(|r| r.flow != LtFlow::Device || b.links_devices()).collect()
}

/// `lt setup …` one-shot: prints to `out`, problems to stderr, and
/// returns the exit code.
pub async fn run(
    b: Arc<dyn Backend>,
    args: &[String],
    json: bool,
    want_ws: Option<&str>,
    stdin: &mut dyn FnMut() -> std::io::Result<Zeroizing<String>>,
    out: &mut dyn std::io::Write,
) -> i32 {
    let usage = |m: String| {
        eprintln!("lt: {m}");
        exit::USAGE
    };
    let flow = match flow_arg(args) {
        Ok(f) => f,
        Err(Usage(m)) => return usage(m),
    };
    let ws = match workspace(&b, want_ws).await {
        Ok(w) => w,
        Err(e) => {
            eprintln!("lt: {e}");
            return exit::NOT_FOUND;
        }
    };
    let Some(flow) = flow else {
        let p = match parse(args, None, SecretSource::FileOnly) {
            Ok(p) => p,
            Err(Usage(m)) => return usage(m),
        };
        if p.help {
            let _ = write!(out, "{}", all_help());
            return 0;
        }
        return match b.setup_status(ws.as_ref()).await {
            Ok(rows) => {
                let rows = visible(&b, rows);
                let _ = if json || p.json {
                    writeln!(out, "{}", super::list_json(&rows))
                } else {
                    write!(out, "{}", clean_text(&list_text(&rows)))
                };
                0
            }
            Err(e) => {
                eprintln!("lt: {e}");
                exit::FAILED
            }
        };
    };
    if flow == LtFlow::Device && !b.links_devices() {
        return usage("lt setup device: linking this computer isn't in this build of lt; use the Ligus desktop (My devices)".into());
    }
    let form = match b.setup_form(ws.as_ref(), flow, BTreeMap::new()).await {
        Ok(f) => f,
        Err(e) => {
            eprintln!("lt: {e}");
            return exit::FAILED;
        }
    };
    let p = match parse(args, Some(&form), SecretSource::Stdin(stdin)) {
        Ok(p) => p,
        Err(Usage(m)) => return usage(m),
    };
    if p.help {
        let _ = write!(out, "{}", flow_help(&form));
        return 0;
    }
    match b.setup_run(ws.as_ref(), flow, p.inputs, p.secrets, p.dry_run, CancelFlag::default()).await {
        Ok(o) => {
            let _ = if json || p.json { writeln!(out, "{}", o.json) } else { write!(out, "{}", clean_text(&outcome_text(&o))) };
            o.exit
        }
        Err(e) => {
            eprintln!("lt: {e}");
            exit::FAILED
        }
    }
}

/// Interactive plain mode's `setup [FLOW flags…]`: the records to print
/// (plain mode escapes each one). Words are split as a shell would
/// (`--command "npx -y server"`).
pub async fn plain(b: &Arc<dyn Backend>, ws: Option<&WorkspaceId>, arg: &str) -> Vec<String> {
    let Some(args) = ligus_runtime::setup::validate::split_command(arg) else { return vec!["error\tusage: a quote isn't closed".into()] };
    let flow = match flow_arg(&args) {
        Ok(f) => f,
        Err(Usage(m)) => return vec![format!("error\tusage: {m}")],
    };
    let Some(flow) = flow else {
        return match b.setup_status(ws).await {
            Ok(rows) => visible(b, rows)
                .into_iter()
                .map(|r| format!("setup\t{}\t{}\t{}\t{}", r.flow.id(), progress_word(r.progress), r.flow.title(), r.detail))
                .collect(),
            Err(e) => vec![format!("error\t{e}")],
        };
    };
    if flow == LtFlow::Device && !b.links_devices() {
        return vec!["error\tusage: linking this computer isn't in this build of lt".into()];
    }
    let form = match b.setup_form(ws, flow, BTreeMap::new()).await {
        Ok(f) => f,
        Err(e) => return vec![format!("error\t{e}")],
    };
    let p = match parse(&args, Some(&form), SecretSource::FileOnly) {
        Ok(p) => p,
        Err(Usage(m)) => return vec![format!("error\tusage: {m}")],
    };
    if p.help {
        return flow_help(&form).lines().map(|l| format!("help\t{l}")).collect();
    }
    match b.setup_run(ws, flow, p.inputs, p.secrets, p.dry_run, CancelFlag::default()).await {
        Ok(o) => {
            let mut v: Vec<String> = o
                .checks
                .iter()
                .map(|c| {
                    format!("check\t{}\t{}\t{}\t{}\t{}", c.id, c.status.as_str(), c.title, c.detail, c.fix.clone().unwrap_or_default())
                })
                .collect();
            v.extend(o.next.iter().map(|n| format!("next\t{n}")));
            let state = if o.done && o.exit == 0 {
                "done"
            } else if o.person() {
                "waiting"
            } else {
                "stopped"
            };
            v.push(format!("setup-done\t{}\t{}\t{state}", flow.id(), o.exit));
            v
        }
        Err(e) => vec![format!("error\t{e}")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> SetupForm {
        super::super::form(FlowId::Tool, &Offline, &BTreeMap::new())
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_are_field_ids() {
        let mut no_stdin = || -> std::io::Result<Zeroizing<String>> { panic!("stdin read") };
        let p = parse(
            &args(&["tool", "--id", "fs", "--command", "npx -y srv", "--dry-run"]),
            Some(&form()),
            SecretSource::Stdin(&mut no_stdin),
        )
        .unwrap();
        assert_eq!(p.inputs.get("id").map(String::as_str), Some("fs"));
        assert!(p.dry_run);
        assert!(parse(&args(&["tool", "--nope", "x"]), Some(&form()), SecretSource::FileOnly).is_err());
        assert_eq!(flow_arg(&args(&["--json", "agent"])).unwrap(), Some(LtFlow::Engine(FlowId::Agent)));
    }

    #[test]
    fn a_secret_on_the_command_line_is_refused_and_not_echoed() {
        let mut no_stdin = || -> std::io::Result<Zeroizing<String>> { panic!("stdin read") };
        let e = parse(&args(&["tool", "--token", "hrt1-canary-value"]), Some(&form()), SecretSource::Stdin(&mut no_stdin)).unwrap_err();
        assert!(!e.0.contains("hrt1-canary-value"), "{}", e.0);
        let e = parse(&args(&["tool", "--token=hrt1-canary-value"]), Some(&form()), SecretSource::FileOnly).unwrap_err();
        assert!(!e.0.contains("hrt1-canary-value"), "{}", e.0);
        let e = parse(&args(&["tool", "hrt1-canary-value-pasted"]), Some(&form()), SecretSource::FileOnly).unwrap_err();
        assert!(!e.0.contains("canary"), "{}", e.0);
        let e = flow_arg(&args(&["abc123tok"])).unwrap_err();
        assert!(!e.0.contains("abc123tok"), "{}", e.0);
        let e = parse(&args(&["tool", "--token", "@hrt1-canary-xyz"]), Some(&form()), SecretSource::FileOnly).unwrap_err();
        assert!(!e.0.contains("canary"), "{}", e.0);
        let mut stdin = || Ok(Zeroizing::new("  tok-canary \n".to_string()));
        let p = parse(&args(&["tool", "--token", "-"]), Some(&form()), SecretSource::Stdin(&mut stdin)).unwrap();
        assert_eq!(p.secrets.ids(), vec!["token".to_string()]);
        assert!(!format!("{p:?}").contains("tok-canary"), "Debug redacts");
        assert!(parse(&args(&["tool", "--token", "-"]), Some(&form()), SecretSource::FileOnly).is_err(), "plain mode: no stdin");
    }

    #[test]
    fn every_flow_has_help() {
        let h = all_help();
        for f in FlowId::ALL {
            assert!(h.contains(&format!("lt setup {}", f.as_str())), "{f:?}");
        }
        assert!(h.contains("--token -|@FILE"));
    }
}
