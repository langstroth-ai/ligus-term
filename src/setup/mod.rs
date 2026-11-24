//! Guided setup flows in `lt`, on the shared engine
//! (`ligus_runtime::setup`, docs/design/setup-flows.md in the ligus repo).
//!
//! The engine owns the flows, their steps and fields, validation, the check
//! ids and fix lines, idempotency, rollback and the secret rules. `lt`
//! implements the engine's `SetupOps` in each backend (the real one over
//! the crates, the demo's in memory) and renders what the engine returns:
//!
//! * the full-screen app: the Set up list (`u`, the palette, empty states)
//!   and a step-by-step popup per flow ([`crate::ui::setup`]);
//! * `lt setup [FLOW] [--FIELD VALUE …] [--json] [--dry-run]`, the same
//!   flags as `ligus setup` ([`cli`]).
//!
//! This module holds what both share: lt's flow ids (the engine's seven
//! plus the `device` hook), the view models the backends return, the
//! conversions from the engine's types, and the secret and cancel types
//! the reducer carries. Every run is a person's ([`Driver::Person`]): the
//! engine refuses agent-driven runs, and lt never claims to be one.

pub mod cli;
pub mod complete;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ligus_runtime::checks::{Check, Report};
use ligus_runtime::setup::{self as engine, Driver, FieldKind, FlowId, Inputs, RunOpts, SetupOps};
use zeroize::Zeroizing;

pub use ligus_runtime::checks::Status as CheckStatus;
pub use ligus_runtime::setup::{Progress, StepKind, exit};

/// The harnesses an lt device runs agents on (`local:…` runtimes). The
/// engine offers Aider, Pi and model gateways too; lt shows them disabled.
pub const LT_HARNESSES: &[&str] = &["claude-code", "codex", "copilot", "antigravity", "local"];

/// Why a harness isn't offered here.
pub const NOT_IN_LT: &str = "not in lt yet";

/// A flow as lt lists it: one of the engine's, or linking this computer
/// to your devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LtFlow {
    Engine(FlowId),
    /// `lt setup device`: link this computer as one of your devices. A
    /// hook: the flow is listed only when the backend can link
    /// ([`crate::backend::Backend::links_devices`]); its steps are the
    /// backend's link screens (`computer_link::setup` in the crates lists
    /// them), not an engine form.
    Device,
}

/// The device flow's words (`computer_link::setup` in the crates).
pub mod device {
    pub const FLOW_ID: &str = "device";
    pub const TITLE: &str = "Add a computer";
    pub const SUMMARY: &str = "Link this computer to your devices, so it acts as you (not a separate member).";
    pub const WHY: &str = "Joining with an invite makes a new member. Linking makes this computer one of your devices: your desktop signs it with your passkey, and the workspace key is sealed to this computer.";
}

impl LtFlow {
    /// The engine's flows, in order.
    pub fn engine_flows() -> impl Iterator<Item = LtFlow> {
        FlowId::ALL.into_iter().map(LtFlow::Engine)
    }

    /// The id on the command line (`lt setup <id>`).
    pub fn id(self) -> &'static str {
        match self {
            LtFlow::Engine(f) => f.as_str(),
            LtFlow::Device => device::FLOW_ID,
        }
    }

    pub fn parse(s: &str) -> Option<LtFlow> {
        let s = s.trim();
        if s == device::FLOW_ID {
            return Some(LtFlow::Device);
        }
        FlowId::parse(s).map(LtFlow::Engine)
    }

    /// The palette and popup title ("Set up an agent").
    pub fn title(self) -> &'static str {
        match self {
            LtFlow::Engine(f) => f.def().title,
            LtFlow::Device => device::TITLE,
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            LtFlow::Engine(f) => f.def().summary,
            LtFlow::Device => device::SUMMARY,
        }
    }

    pub fn why(self) -> &'static str {
        match self {
            LtFlow::Engine(f) => f.def().why,
            LtFlow::Device => device::WHY,
        }
    }

    /// Words the palette also matches.
    pub fn keywords(self) -> &'static str {
        match self {
            LtFlow::Engine(f) => f.def().keywords,
            LtFlow::Device => "device computer link my devices",
        }
    }
}

// ── View models ─────────────────────────────────────────────────────────────

/// One row of the Set up list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRow {
    pub flow: LtFlow,
    pub progress: Progress,
    /// One line ("2 agents", "no relay yet").
    pub detail: String,
}

/// One check, as the popup and `lt setup` show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupCheck {
    pub id: String,
    pub status: CheckStatus,
    pub title: String,
    pub detail: String,
    pub fix: Option<String>,
    pub notes: Vec<String>,
}

impl SetupCheck {
    fn of(c: &Check) -> Self {
        SetupCheck {
            id: c.id.clone(),
            status: c.status,
            title: c.title.clone(),
            detail: c.detail.clone(),
            fix: c.fix.clone(),
            notes: c.notes.clone(),
        }
    }

    /// The `ligus.checks.v1` text form: the status, the title, what was
    /// found, then `fix:` under anything that isn't ok.
    pub fn text(&self) -> String {
        let mut s = format!("{:<5} {:<15} {}\n", self.status.as_str(), self.title, self.detail);
        for n in &self.notes {
            s.push_str(&format!("      {n}\n"));
        }
        if let (Some(f), false) = (&self.fix, self.status == CheckStatus::Ok) {
            s.push_str(&format!("      fix: {f}\n"));
        }
        s
    }
}

/// One option of a choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupChoice {
    pub value: String,
    pub label: String,
    pub hint: Option<String>,
    pub disabled: bool,
}

/// How a field is entered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    Text,
    Choice(Vec<SetupChoice>),
    /// Several, comma-separated on the command line.
    Multi(Vec<SetupChoice>),
    /// Free text with suggestions.
    Suggest(Vec<SetupChoice>),
    /// Masked; held only in [`SetupSecrets`].
    Secret,
    /// A folder on this device.
    Path,
    Toggle,
}

impl FieldType {
    pub fn options(&self) -> &[SetupChoice] {
        match self {
            FieldType::Choice(o) | FieldType::Multi(o) | FieldType::Suggest(o) => o,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupField {
    /// Also the command-line flag: `--<id>`.
    pub id: String,
    pub label: String,
    pub kind: FieldType,
    pub required: bool,
    pub default: Option<String>,
    pub help: String,
    /// Shown only when another field has one of these values.
    pub when: Option<(String, Vec<String>)>,
}

impl SetupField {
    /// Whether it applies given the other values.
    pub fn applies(&self, inputs: &SetupInputs) -> bool {
        match &self.when {
            None => true,
            Some((f, vals)) => inputs.get(f).map(|v| v.trim()).is_some_and(|v| vals.iter().any(|x| x == v)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupStep {
    pub id: String,
    pub title: String,
    pub kind: StepKind,
    pub fields: Vec<SetupField>,
    /// For a person's step: where it happens.
    pub where_: String,
}

/// A flow's form: preflight checks, then its steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupForm {
    pub flow: LtFlow,
    pub preflight: Vec<SetupCheck>,
    /// A preflight check failed: the flow can't run here as things are.
    pub blocked: bool,
    pub steps: Vec<SetupStep>,
}

impl SetupForm {
    pub fn fields(&self) -> impl Iterator<Item = &SetupField> {
        self.steps.iter().flat_map(|s| s.fields.iter())
    }
}

/// What a run did. `json` is the `ligus.checks.v1` document `lt setup
/// FLOW --json` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupOutcome {
    pub flow: LtFlow,
    pub checks: Vec<SetupCheck>,
    pub next: Vec<String>,
    pub exit: i32,
    pub done: bool,
    pub dry_run: bool,
    pub results: BTreeMap<String, String>,
    pub json: serde_json::Value,
}

impl SetupOutcome {
    /// A person must act (exit 4).
    pub fn person(&self) -> bool {
        self.exit == exit::PERSON
    }
}

/// Non-secret field values by field id.
pub type SetupInputs = BTreeMap<String, String>;

/// Secret field values by field id: zeroized on drop, redacted in `Debug`,
/// not serializable. They go from the popup's [`SecretBuf`] (or `lt setup`'s
/// stdin or owner-only file) straight to the engine.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SetupSecrets(BTreeMap<String, Zeroizing<String>>);

impl SetupSecrets {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, id: &str, v: Zeroizing<String>) {
        self.0.insert(id.to_string(), v);
    }
    pub fn is_empty(&self) -> bool {
        self.0.values().all(|v| v.trim().is_empty())
    }
    /// The ids that hold a value (never the values).
    pub fn ids(&self) -> Vec<String> {
        self.0.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, _)| k.clone()).collect()
    }
    fn into_engine(self) -> engine::Secrets {
        let mut s = engine::Secrets::new();
        for (k, v) in self.0 {
            s.set(&k, v);
        }
        s
    }
}

impl std::fmt::Debug for SetupSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.0.keys().map(|k| (k, "***"))).finish()
    }
}

/// A secret field being typed: zeroized on drop, redacted in `Debug`,
/// drawn as a fixed row of dots (not even its length shows).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretBuf(Zeroizing<String>);

impl SecretBuf {
    pub fn push(&mut self, c: char) {
        if !c.is_control() {
            self.0.push(c);
        }
    }
    /// A paste: one line, control characters dropped.
    pub fn push_str(&mut self, s: &str) {
        for c in s.trim().chars() {
            self.push(c);
        }
    }
    pub fn pop(&mut self) {
        self.0.pop();
    }
    pub fn clear(&mut self) {
        // Zeroizing clears the old buffer when it is replaced.
        self.0 = Zeroizing::new(String::new());
    }
    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
    /// The value, for the run (copied into another zeroizing buffer).
    pub fn value(&self) -> Zeroizing<String> {
        Zeroizing::new(self.0.trim().to_string())
    }
    /// How it is drawn.
    pub fn masked(&self) -> &'static str {
        if self.is_empty() { "" } else { "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}" }
    }
}

impl std::fmt::Debug for SecretBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_empty() { "SecretBuf(empty)" } else { "SecretBuf(***)" })
    }
}

/// Stops a run between steps (the engine undoes the run's own writes).
/// Shared between the popup and the running effect; equal when it is the
/// same flag.
#[derive(Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn arc(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

impl PartialEq for CancelFlag {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.0, &o.0)
    }
}

impl Eq for CancelFlag {}

impl std::fmt::Debug for CancelFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CancelFlag({})", self.is_cancelled())
    }
}

// ── On the engine ───────────────────────────────────────────────────────────

/// The Set up list: the engine's flows with what is done (cheap, no
/// network), then the device row when the backend links devices.
pub fn rows(ops: &dyn SetupOps, device: Option<FlowRow>) -> Vec<FlowRow> {
    let mut out: Vec<FlowRow> = engine::status(ops)
        .into_iter()
        .map(|s| FlowRow { flow: LtFlow::Engine(s.flow.id), progress: s.progress, detail: s.detail })
        .collect();
    out.extend(device);
    out
}

fn choices(o: &[engine::Choice]) -> Vec<SetupChoice> {
    o.iter().map(|c| SetupChoice { value: c.value.clone(), label: c.label.clone(), hint: c.hint.clone(), disabled: c.disabled }).collect()
}

fn field(f: &engine::Field) -> SetupField {
    let kind = match &f.kind {
        FieldKind::Text => FieldType::Text,
        FieldKind::Choice { options } => FieldType::Choice(choices(options)),
        FieldKind::Multi { options } => FieldType::Multi(choices(options)),
        FieldKind::Suggest { options } => FieldType::Suggest(choices(options)),
        FieldKind::Secret => FieldType::Secret,
        FieldKind::Path => FieldType::Path,
        FieldKind::Toggle => FieldType::Toggle,
    };
    SetupField {
        id: f.id.to_string(),
        label: f.label.to_string(),
        kind,
        required: f.required,
        default: f.default.clone(),
        help: f.help.to_string(),
        when: f.when.as_ref().map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect())),
    }
}

fn engine_inputs(inputs: &SetupInputs) -> Inputs {
    Inputs(inputs.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// The engine's text for lt: where it names `ligus setup FLOW` to run on
/// this device, lt's own command (a box's `ligus setup box` stays: it runs on
/// the box).
fn for_lt(flow: FlowId, s: &str) -> String {
    if flow == FlowId::Box {
        return s.to_string();
    }
    let f = flow.as_str();
    s.replace(&format!("`ligus setup {f}"), &format!("`lt setup {f}"))
}

fn check(flow: FlowId, c: &Check) -> SetupCheck {
    let mut c = SetupCheck::of(c);
    c.fix = c.fix.map(|f| for_lt(flow, &f));
    c
}

/// A flow's form, with the harnesses lt can't run disabled.
pub fn form(flow: FlowId, ops: &dyn SetupOps, inputs: &SetupInputs) -> SetupForm {
    let f = engine::form(flow, ops, &engine_inputs(inputs));
    let mut steps: Vec<SetupStep> = f
        .steps
        .iter()
        .map(|s| SetupStep {
            id: s.id.to_string(),
            title: s.title.to_string(),
            kind: s.kind,
            fields: s.fields.iter().map(field).collect(),
            where_: s.where_.to_string(),
        })
        .collect();
    if flow == FlowId::Agent {
        for fd in steps.iter_mut().flat_map(|s| s.fields.iter_mut()).filter(|f| f.id == "harness") {
            if let FieldType::Choice(opts) = &mut fd.kind {
                for o in opts.iter_mut().filter(|o| !LT_HARNESSES.contains(&o.value.as_str())) {
                    o.disabled = true;
                    o.hint = Some(NOT_IN_LT.into());
                }
            }
        }
    }
    SetupForm { flow: LtFlow::Engine(flow), preflight: f.preflight.iter().map(|c| check(flow, c)).collect(), blocked: f.blocked, steps }
}

/// lt's document: the engine's, named as lt's.
fn lt_json(mut d: serde_json::Value) -> serde_json::Value {
    d["tool"] = "lt".into();
    d["version"] = env!("CARGO_PKG_VERSION").into();
    d
}

/// Run a flow as a person: gate, preflight, validation, the steps, verify.
pub fn run(
    flow: FlowId,
    ops: &mut dyn SetupOps,
    inputs: &SetupInputs,
    secrets: SetupSecrets,
    dry_run: bool,
    cancel: &CancelFlag,
) -> SetupOutcome {
    // A harness lt can't run is a usage error here, before anything.
    if flow == FlowId::Agent
        && let Some(h) = inputs.get("harness").map(|h| h.trim()).filter(|h| !h.is_empty() && !LT_HARNESSES.contains(h))
    {
        let mut r = Report::new("setup agent");
        r.add(
            Check::fail("agent.input.harness", "harness", format!("lt doesn't run {h} agents yet"))
                .fix(format!("pick one of {} here, or run ligus setup agent (or the desktop) for {h}", LT_HARNESSES.join(", "))),
        );
        return outcome_of(LtFlow::Engine(flow), &r, exit::USAGE, false, dry_run, BTreeMap::new(), flow);
    }
    let opts = RunOpts { dry_run, driver: Driver::Person, cancel: Some(cancel.arc()) };
    let o = engine::run(flow, ops, &engine_inputs(inputs), &secrets.into_engine(), &opts);
    let json = lt_json(o.to_json(dry_run));
    SetupOutcome {
        flow: LtFlow::Engine(flow),
        checks: o.report.checks.iter().map(|c| check(flow, c)).collect(),
        next: o.report.next.iter().map(|n| for_lt(flow, n)).collect(),
        exit: o.exit,
        done: o.done,
        dry_run,
        results: o.results,
        json,
    }
}

fn outcome_of(
    flow: LtFlow,
    r: &Report,
    exit: i32,
    done: bool,
    dry_run: bool,
    results: BTreeMap<String, String>,
    f: FlowId,
) -> SetupOutcome {
    let mut d = r.to_json(exit);
    d["flow"] = flow.id().into();
    d["done"] = done.into();
    d["dryRun"] = dry_run.into();
    d["results"] = serde_json::to_value(&results).unwrap_or_default();
    SetupOutcome {
        flow,
        checks: r.checks.iter().map(|c| check(f, c)).collect(),
        next: r.next.clone(),
        exit,
        done,
        dry_run,
        results,
        json: lt_json(d),
    }
}

/// `lt setup --json`: the flows and what is done, as checks
/// (`<flow>.status`), the same document as `ligus setup --json`.
pub fn list_json(rows: &[FlowRow]) -> serde_json::Value {
    let mut r = Report::new("setup");
    for row in rows {
        let id = format!("{}.status", row.flow.id());
        let fix = format!("lt setup {}", row.flow.id());
        let c = match row.progress {
            Progress::Done => Check::ok(&id, row.flow.title(), row.detail.clone()),
            Progress::Waiting => Check::warn(&id, row.flow.title(), row.detail.clone()).fix(fix),
            Progress::Todo => Check::skip(&id, row.flow.title(), row.detail.clone()).fix(fix),
            Progress::Unavailable => Check::skip(&id, row.flow.title(), row.detail.clone()),
        };
        if row.progress == Progress::Todo && r.next.len() < 2 {
            r.next(format!("lt setup {}", row.flow.id()));
        }
        r.add(c);
    }
    lt_json(r.to_json(0))
}

/// A device-flow outcome the backend builds (the hook's run).
pub fn device_outcome(checks: Vec<Check>, next: Vec<String>, exit: i32, done: bool) -> SetupOutcome {
    let mut r = Report::new("setup device");
    for c in checks {
        r.add(c);
    }
    for n in next {
        r.next(n);
    }
    outcome_of(LtFlow::Device, &r, exit, done, false, BTreeMap::new(), FlowId::Agent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_ids_round_trip_and_device_is_its_own() {
        for f in LtFlow::engine_flows() {
            assert_eq!(LtFlow::parse(f.id()), Some(f));
        }
        assert_eq!(LtFlow::parse("device"), Some(LtFlow::Device));
        assert_eq!(LtFlow::parse("nope"), None);
        assert_eq!(LtFlow::Engine(FlowId::Tool).title(), "Connect a tool");
    }

    #[test]
    fn secrets_never_print() {
        let mut s = SetupSecrets::new();
        s.set("token", Zeroizing::new("hrt1-canary".into()));
        assert_eq!(format!("{s:?}"), r#"{"token": "***"}"#);
        let mut b = SecretBuf::default();
        b.push_str("  hrt1-canary\n");
        assert_eq!(format!("{b:?}"), "SecretBuf(***)");
        assert!(!b.masked().contains("hrt1"));
        assert_eq!(b.value().as_str(), "hrt1-canary");
        b.clear();
        assert!(b.is_empty());
    }

    #[test]
    fn a_cancel_flag_is_shared() {
        let a = CancelFlag::default();
        let b = a.clone();
        assert_eq!(a, b);
        assert_ne!(a, CancelFlag::default());
        b.cancel();
        assert!(a.is_cancelled());
    }

    #[test]
    fn fixes_name_lt_for_this_device_but_not_for_a_box() {
        assert_eq!(for_lt(FlowId::Tool, "see `ligus setup tool --help`"), "see `lt setup tool --help`");
        assert_eq!(for_lt(FlowId::Box, "run `ligus setup box` on the box itself"), "run `ligus setup box` on the box itself");
    }
}
