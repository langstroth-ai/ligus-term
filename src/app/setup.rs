//! The Set up list and a flow's step-by-step popup: their state and keys.
//! Pure, like the reducer: keys change the state and return effects; the
//! engine's form decides the steps and fields, and only a run writes.
//!
//! A flow's popup shows one step per screen (the engine's input steps,
//! and a person's step as where it happens), then a review, then the run's
//! checks with their fix lines. Nothing is written before the review's
//! enter; esc before it closes with nothing done, esc during the run asks
//! the engine to stop between steps (it undoes its own writes).
//!
//! Secret fields are typed into [`SecretBuf`]s (zeroized, redacted in
//! `Debug`, drawn as dots) and move into the run's [`SetupSecrets`] when it
//! starts; the popup keeps none afterwards.

use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::input::TextBuf;
use crate::app::state::{Overlay, State, ToastKind};
use crate::app::update::Effect;
use crate::backend::model::{Allowed, Role};
use crate::setup::complete::Completion;
use crate::setup::{CancelFlag, FieldType, LtFlow, SecretBuf, SetupField, SetupForm, SetupInputs, SetupOutcome, SetupSecrets, StepKind};

/// Where a flow's popup is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Waiting for the form.
    Loading,
    /// Filling in a step (`Wizard::screen`).
    Fill,
    /// Everything entered: what will run, the dry-run switch.
    Review,
    Running,
    Done(SetupOutcome),
    /// The form couldn't be had (one line).
    Failed(String),
}

/// One flow's popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wizard {
    pub flow: LtFlow,
    pub form: Option<SetupForm>,
    pub stage: Stage,
    /// Index into [`Wizard::screens`].
    pub screen: usize,
    /// The focused field on the screen (index into its fields).
    pub field: usize,
    pub texts: BTreeMap<String, TextBuf>,
    /// The cursor in a choice, multi or suggest field's options.
    pub cursor: BTreeMap<String, usize>,
    pub multi: BTreeMap<String, BTreeSet<String>>,
    pub toggles: BTreeSet<String>,
    pub secrets: BTreeMap<String, SecretBuf>,
    pub dry_run: bool,
    /// The last completion's folders (a path field).
    pub completions: Vec<String>,
    /// A problem with this screen's input (a required field left empty).
    pub error: Option<String>,
    /// Opened from the Set up list (esc goes back to it).
    pub from_list: bool,
    pub cancel: CancelFlag,
    /// Asked to stop.
    pub stopping: bool,
    /// Fields whose defaults were filled in once.
    pub seen: BTreeSet<String>,
    /// Where each secret was typed for (the origin it would go to): one
    /// typed for one server or relay never goes to another.
    pub secret_for: BTreeMap<String, String>,
    /// While running: the passkey confirmation being asked for ("Confirm 1
    /// of 2: Add @coder"), when the run asks for more than one.
    pub confirming: Option<String>,
}

impl Wizard {
    pub fn new(flow: LtFlow, from_list: bool) -> Self {
        Wizard {
            flow,
            form: None,
            stage: Stage::Loading,
            screen: 0,
            field: 0,
            texts: BTreeMap::new(),
            cursor: BTreeMap::new(),
            multi: BTreeMap::new(),
            toggles: BTreeSet::new(),
            secrets: BTreeMap::new(),
            dry_run: false,
            completions: vec![],
            error: None,
            from_list,
            cancel: CancelFlag::default(),
            stopping: false,
            seen: BTreeSet::new(),
            secret_for: BTreeMap::new(),
            confirming: None,
        }
    }

    /// A form arrived (the first, or a refresh with the values so far):
    /// fill in defaults for fields not seen yet, keep what was entered.
    pub fn set_form(&mut self, form: SetupForm) {
        // A choice whose options changed (Runs on, after another source was
        // picked) starts again from its default.
        let before: BTreeMap<String, Vec<String>> = self
            .form
            .iter()
            .flat_map(|f| f.fields())
            .map(|f| (f.id.clone(), f.kind.options().iter().map(|o| o.value.clone()).collect()))
            .collect();
        let old_default: BTreeMap<String, String> =
            self.form.iter().flat_map(|f| f.fields()).map(|f| (f.id.clone(), f.default.clone().unwrap_or_default())).collect();
        for f in form.fields() {
            match &f.kind {
                FieldType::Choice(opts) => {
                    let now: Vec<String> = opts.iter().map(|o| o.value.clone()).collect();
                    if before.get(&f.id).is_some_and(|b| *b != now) {
                        self.cursor.remove(&f.id);
                    }
                }
                // A suggestion left at its old default (a model, when another
                // source was picked) follows the new one.
                FieldType::Suggest(_) | FieldType::Text => {
                    let new = f.default.clone().unwrap_or_default();
                    if let (Some(old), Some(buf)) = (old_default.get(&f.id), self.texts.get_mut(&f.id))
                        && *old != new
                        && buf.text.trim() == old.trim()
                    {
                        *buf = TextBuf::with(&new);
                        self.cursor.remove(&f.id);
                    }
                }
                _ => {}
            }
        }
        for f in form.fields() {
            match &f.kind {
                FieldType::Text | FieldType::Path | FieldType::Suggest(_) => {
                    self.texts.entry(f.id.clone()).or_insert_with(|| TextBuf::with(f.default.as_deref().unwrap_or("")));
                }
                FieldType::Choice(opts) => {
                    let first = opts.iter().position(|o| !o.disabled).unwrap_or(0);
                    let at = f.default.as_ref().and_then(|d| opts.iter().position(|o| &o.value == d)).unwrap_or(first);
                    let c = self.cursor.entry(f.id.clone()).or_insert(at);
                    *c = (*c).min(opts.len().saturating_sub(1));
                }
                FieldType::Multi(opts) => {
                    self.cursor.entry(f.id.clone()).or_insert(0);
                    self.multi.entry(f.id.clone()).or_insert_with(|| {
                        f.default
                            .as_deref()
                            .unwrap_or("")
                            .split(',')
                            .map(str::trim)
                            .filter(|v| opts.iter().any(|o| o.value == *v))
                            .map(String::from)
                            .collect()
                    });
                }
                FieldType::Toggle => {
                    if f.default.as_deref() == Some("true") && !self.seen.contains(&f.id) {
                        self.toggles.insert(f.id.clone());
                    }
                }
                FieldType::Secret => {
                    self.secrets.entry(f.id.clone()).or_default();
                }
            }
        }
        self.seen.extend(form.fields().map(|f| f.id.clone()));
        if self.stage == Stage::Loading {
            self.stage = Stage::Fill;
        }
        self.form = Some(form);
        self.screen = self.screen.min(self.screens().len().saturating_sub(1));
    }

    /// The form's steps that get a screen: input steps with a field that
    /// applies, and a person's steps (where they happen).
    pub fn screens(&self) -> Vec<usize> {
        let Some(form) = &self.form else { return vec![] };
        if form.blocked {
            return vec![];
        }
        let inputs = self.inputs();
        form.steps
            .iter()
            .enumerate()
            .filter(|(_, st)| match st.kind {
                StepKind::Input => st.fields.iter().any(|f| f.applies(&inputs)),
                StepKind::Person => !st.where_.is_empty(),
                StepKind::Verify => false,
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The fields on the current screen that apply.
    pub fn fields(&self) -> Vec<&SetupField> {
        let (Some(form), Some(&i)) = (&self.form, self.screens().get(self.screen)) else { return vec![] };
        let inputs = self.inputs();
        form.steps[i].fields.iter().filter(|f| f.applies(&inputs)).collect()
    }

    pub fn focused(&self) -> Option<&SetupField> {
        let f = self.fields();
        f.get(self.field.min(f.len().saturating_sub(1))).copied()
    }

    /// A field's value as the engine takes it (`None`: unset).
    fn value(&self, f: &SetupField) -> Option<String> {
        let v = match &f.kind {
            FieldType::Text | FieldType::Path | FieldType::Suggest(_) => {
                let t = self.texts.get(&f.id)?.text.trim().to_string();
                if f.kind == FieldType::Path {
                    crate::setup::complete::expand(&t, dirs::home_dir().as_deref()).display().to_string()
                } else {
                    t
                }
            }
            FieldType::Choice(opts) => opts.get(*self.cursor.get(&f.id)?)?.value.clone(),
            FieldType::Multi(opts) => {
                let picked = self.multi.get(&f.id)?;
                opts.iter().filter(|o| picked.contains(&o.value)).map(|o| o.value.clone()).collect::<Vec<_>>().join(",")
            }
            FieldType::Toggle => (if self.toggles.contains(&f.id) { "true" } else { "" }).to_string(),
            FieldType::Secret => return None,
        };
        (!v.is_empty()).then_some(v)
    }

    /// Every non-secret value, for the fields that apply.
    pub fn inputs(&self) -> SetupInputs {
        let Some(form) = &self.form else { return SetupInputs::new() };
        // Choices first: the other fields' `when` reads them.
        let mut out = SetupInputs::new();
        for f in form.fields().filter(|f| f.when.is_none()) {
            if let Some(v) = self.value(f) {
                out.insert(f.id.clone(), v);
            }
        }
        for f in form.fields().filter(|f| f.when.is_some()) {
            if f.applies(&out)
                && let Some(v) = self.value(f)
            {
                out.insert(f.id.clone(), v);
            }
        }
        out
    }

    /// Where a secret entered now would be sent: the URL's origin (a tool
    /// or your own relay), the community relay's, or nothing to say.
    pub fn destination(&self) -> String {
        let i = self.inputs();
        match (i.get("kind").map(String::as_str), i.get("url")) {
            (Some("community"), _) => ligus_runtime::setup::validate::origin(ligus_core::COMMUNITY_RELAY_URL),
            (Some("url" | "own"), Some(u)) => ligus_runtime::setup::validate::origin(u),
            _ => String::new(),
        }
    }

    /// Clear the secrets typed for somewhere else than where they would go
    /// now (the URL or the relay changed after the token was typed).
    pub fn forget_moved_secrets(&mut self) {
        let dest = self.destination();
        for (id, buf) in self.secrets.iter_mut() {
            if !buf.is_empty() && self.secret_for.get(id).is_some_and(|d| *d != dest) {
                buf.clear();
                self.secret_for.remove(id);
                self.error = Some("the destination changed, so the secret typed for it was cleared: type it again".into());
            }
        }
    }

    /// The secrets for the run, taken out of the popup (its buffers are
    /// cleared). Only fields that apply, and only for where they were typed.
    pub fn take_secrets(&mut self) -> SetupSecrets {
        self.forget_moved_secrets();
        let inputs = self.inputs();
        let applies: BTreeSet<String> = self
            .form
            .iter()
            .flat_map(|f| f.fields())
            .filter(|f| f.kind == FieldType::Secret && f.applies(&inputs))
            .map(|f| f.id.clone())
            .collect();
        let mut out = SetupSecrets::new();
        for (id, buf) in self.secrets.iter_mut() {
            if applies.contains(id) && !buf.is_empty() {
                out.set(id, buf.value());
            }
            buf.clear();
        }
        out
    }

    /// Why this screen can't be left yet.
    fn missing(&self) -> Option<String> {
        for f in self.fields() {
            if let FieldType::Choice(opts) = &f.kind
                && let Some(o) = self.cursor.get(&f.id).and_then(|i| opts.get(*i))
                && o.disabled
            {
                return Some(format!("{}: {}", o.label, o.hint.clone().unwrap_or_else(|| "not available here".into())));
            }
            if f.required && f.kind != FieldType::Secret && self.value(f).is_none() {
                return Some(format!("{} is needed", f.label));
            }
        }
        None
    }

    /// Whether a text field (or a secret) has the keyboard.
    pub fn typing(&self) -> bool {
        self.stage == Stage::Fill
            && self
                .focused()
                .is_some_and(|f| matches!(f.kind, FieldType::Text | FieldType::Path | FieldType::Suggest(_) | FieldType::Secret))
    }
}

fn ws(s: &State) -> Option<crate::backend::model::WorkspaceId> {
    s.active.clone()
}

/// Open the Set up list.
pub fn open_list(s: &mut State) -> Vec<Effect> {
    s.overlay = Some(Overlay::Setup { sel: 0 });
    vec![Effect::LoadSetup(ws(s)), Effect::LoadPendingPresence]
}

/// Open a flow's popup. The device flow is lt's Add to my devices screens.
pub fn open_flow(s: &mut State, flow: LtFlow, from_list: bool) -> Vec<Effect> {
    if flow == LtFlow::Device {
        let mut f = crate::app::state::WorkspaceForm::linking(false);
        f.link_asked = true;
        s.overlay = Some(Overlay::AddWorkspace(f));
        return vec![Effect::LinkRequest];
    }
    s.overlay = Some(Overlay::SetupFlow(Box::new(Wizard::new(flow, from_list))));
    vec![Effect::SetupForm(ws(s), flow, SetupInputs::new())]
}

/// Close a flow's popup: back to the list when it came from there.
fn close(s: &mut State, from_list: bool) -> Vec<Effect> {
    if from_list {
        return open_list(s);
    }
    s.overlay = None;
    vec![]
}

/// Keys in the Set up list.
pub fn list_key(s: &mut State, k: &KeyEvent, sel: usize) -> Vec<Effect> {
    // The passkey hand-offs left waiting come first, then the flows.
    let waiting: Vec<String> = s.pending_presence.iter().map(|p| p.rid.clone()).collect();
    let rows: Vec<LtFlow> = s.setup_rows.iter().flatten().map(|r| r.flow).collect();
    let n = waiting.len() + rows.len();
    if let Some(rid) = waiting.get(sel) {
        match k.code {
            KeyCode::Enter | KeyCode::Char('w') => {
                s.overlay = None;
                return vec![Effect::ResumePresence(rid.clone())];
            }
            KeyCode::Char('c') => {
                s.pending_presence.retain(|p| &p.rid != rid);
                return vec![Effect::CancelPendingPresence(rid.clone())];
            }
            _ => {}
        }
    }
    let flow_at = |i: usize| i.checked_sub(waiting.len()).and_then(|i| rows.get(i)).copied();
    let set = |s: &mut State, i: usize| s.overlay = Some(Overlay::Setup { sel: i });
    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            s.overlay = None;
            vec![]
        }
        KeyCode::Up | KeyCode::Char('k') => {
            set(s, sel.saturating_sub(1));
            vec![]
        }
        KeyCode::Down | KeyCode::Char('j') => {
            set(s, (sel + 1).min(n.saturating_sub(1)));
            vec![]
        }
        KeyCode::Home | KeyCode::Char('g') => {
            set(s, 0);
            vec![]
        }
        KeyCode::End | KeyCode::Char('G') => {
            set(s, n.saturating_sub(1));
            vec![]
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => match flow_at(sel) {
            Some(f) => open_flow(s, f, true),
            None => vec![],
        },
        _ => vec![],
    }
}

/// lt's own screen for a person's step it can do here: creating an invite
/// (the code is shown only in this window) or admitting a box (your
/// passkey). `None` when it isn't yours to do.
pub fn here_action(s: &State, flow: LtFlow) -> Option<(char, &'static str)> {
    let w = s.active_workspace()?;
    match flow {
        LtFlow::Engine(ligus_runtime::setup::FlowId::Invite) if w.can_invite == Allowed::Yes => Some(('i', "create an invite here")),
        LtFlow::Engine(ligus_runtime::setup::FlowId::Box) if w.your_role >= Role::Admin => Some(('a', "admit a box from here")),
        _ => None,
    }
}

fn run_here(s: &mut State, flow: LtFlow) -> Vec<Effect> {
    match flow {
        LtFlow::Engine(ligus_runtime::setup::FlowId::Invite) => {
            s.overlay = Some(Overlay::Invite { code: None });
            ws(s).map(|w| vec![Effect::CreateInvite(w, Role::Contributor)]).unwrap_or_default()
        }
        _ => {
            s.overlay = Some(Overlay::AdmitBox { request: TextBuf::single(), busy: false, checked: None, result: None });
            vec![]
        }
    }
}

/// Keys in a flow's popup.
pub fn flow_key(s: &mut State, k: &KeyEvent) -> Vec<Effect> {
    let here = match &s.overlay {
        Some(Overlay::SetupFlow(w)) => here_action(s, w.flow),
        _ => return vec![],
    };
    let wsid = ws(s);
    let Some(Overlay::SetupFlow(w)) = s.overlay.as_mut() else { return vec![] };
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match w.stage.clone() {
        Stage::Loading | Stage::Failed(_) => {
            if k.code == KeyCode::Esc || matches!(w.stage, Stage::Failed(_)) {
                let from = w.from_list;
                return close(s, from);
            }
            vec![]
        }
        Stage::Running => {
            if k.code == KeyCode::Esc && !w.stopping {
                w.stopping = true;
                return vec![Effect::CancelSetup(w.cancel.clone())];
            }
            vec![]
        }
        Stage::Done(o) => {
            let flow = w.flow;
            match k.code {
                KeyCode::Char('r') if o.exit != 0 && !o.person() => {
                    w.stage = Stage::Fill;
                    w.screen = 0;
                    w.field = 0;
                    w.error = None;
                    w.stopping = false;
                    w.cancel = CancelFlag::default();
                    let inputs = w.inputs();
                    vec![Effect::SetupForm(wsid, flow, inputs)]
                }
                KeyCode::Char(c) if here.is_some_and(|(h, _)| h == c) && o.person() => run_here(s, flow),
                KeyCode::Enter | KeyCode::Esc => {
                    let from = w.from_list;
                    close(s, from)
                }
                _ => vec![],
            }
        }
        Stage::Review => match k.code {
            KeyCode::Esc => {
                let from = w.from_list;
                close(s, from)
            }
            KeyCode::Char('d') => {
                w.dry_run = !w.dry_run;
                vec![]
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Backspace => {
                w.stage = Stage::Fill;
                w.screen = w.screens().len().saturating_sub(1);
                w.field = 0;
                vec![]
            }
            KeyCode::Enter => {
                w.stage = Stage::Running;
                w.confirming = None;
                w.cancel = CancelFlag::default();
                let inputs = w.inputs();
                let secrets = w.take_secrets();
                vec![Effect::SetupRun { workspace: wsid, flow: w.flow, inputs, secrets, dry_run: w.dry_run, cancel: w.cancel.clone() }]
            }
            _ => vec![],
        },
        Stage::Fill => {
            let blocked = w.form.as_ref().is_some_and(|f| f.blocked);
            if blocked {
                return match k.code {
                    KeyCode::Char(c) if here.is_some_and(|(h, _)| h == c) => {
                        let flow = w.flow;
                        run_here(s, flow)
                    }
                    _ => {
                        let from = w.from_list;
                        close(s, from)
                    }
                };
            }
            let mut fx = fill_key(w, k, ctrl, wsid);
            fx.extend(settle(s));
            fx
        }
    }
}

fn fill_key(w: &mut Wizard, k: &KeyEvent, ctrl: bool, wsid: Option<crate::backend::model::WorkspaceId>) -> Vec<Effect> {
    let n_fields = w.fields().len();
    let Some(f) = w.focused().cloned() else {
        // A person's step: enter goes on, shift-tab back.
        return match k.code {
            KeyCode::Esc => {
                w.form = None;
                vec![]
            }
            KeyCode::Enter => next_screen(w, wsid),
            KeyCode::BackTab => {
                prev_screen(w);
                vec![]
            }
            _ => vec![],
        };
    };
    if k.code == KeyCode::Esc {
        // Nothing was written: close (the caller's close goes back to the
        // list when the popup came from there).
        w.form = None;
        return vec![];
    }
    match k.code {
        KeyCode::Enter => return next_screen(w, wsid),
        KeyCode::BackTab => {
            if w.field == 0 {
                prev_screen(w);
            } else {
                w.field -= 1;
            }
            w.completions.clear();
            return vec![];
        }
        KeyCode::Tab if f.kind == FieldType::Path => {
            let text = w.texts.get(&f.id).map(|t| t.text.clone()).unwrap_or_default();
            return vec![Effect::CompletePath(f.id.clone(), text)];
        }
        KeyCode::Tab => {
            w.field = (w.field + 1) % n_fields.max(1);
            w.completions.clear();
            return vec![];
        }
        _ => {}
    }
    w.error = None;
    let opts_len = f.kind.options().len();
    match &f.kind {
        FieldType::Choice(_) | FieldType::Multi(_) => match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let c = w.cursor.entry(f.id.clone()).or_insert(0);
                *c = c.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let c = w.cursor.entry(f.id.clone()).or_insert(0);
                *c = (*c + 1).min(opts_len.saturating_sub(1));
            }
            KeyCode::Char(' ') if matches!(f.kind, FieldType::Multi(_)) => {
                let at = *w.cursor.get(&f.id).unwrap_or(&0);
                if let Some(o) = f.kind.options().get(at) {
                    let set = w.multi.entry(f.id.clone()).or_default();
                    if !set.remove(&o.value) {
                        // "all" and named agents exclude each other.
                        if o.value == "all" {
                            set.clear();
                        } else {
                            set.remove("all");
                        }
                        set.insert(o.value.clone());
                    }
                }
            }
            _ => {}
        },
        FieldType::Toggle => match k.code {
            KeyCode::Up if w.field > 0 => w.field -= 1,
            KeyCode::Down => w.field = (w.field + 1).min(n_fields.saturating_sub(1)),
            KeyCode::Char(' ') | KeyCode::Char('t') => {
                if !w.toggles.remove(&f.id) {
                    w.toggles.insert(f.id.clone());
                }
                // What else is needed may follow it ("Start with a scratch
                // repo" needs no folder): the form again, values kept.
                w.error = None;
                return vec![Effect::SetupForm(wsid, w.flow, w.inputs())];
            }
            _ => {}
        },
        FieldType::Secret => {
            let dest = w.destination();
            let buf = w.secrets.entry(f.id.clone()).or_default();
            match k.code {
                KeyCode::Char('u') if ctrl => buf.clear(),
                KeyCode::Char(c) if !ctrl => buf.push(c),
                KeyCode::Backspace => buf.pop(),
                _ => {}
            }
            w.secret_for.insert(f.id.clone(), dest);
        }
        FieldType::Suggest(opts) => match k.code {
            KeyCode::Up | KeyCode::Down if !opts.is_empty() => {
                let at = w.cursor.get(&f.id).copied();
                let next = match (k.code, at) {
                    (KeyCode::Down, None) => 0,
                    (KeyCode::Down, Some(i)) => (i + 1).min(opts.len() - 1),
                    (_, None) | (_, Some(0)) => 0,
                    (_, Some(i)) => i - 1,
                };
                w.cursor.insert(f.id.clone(), next);
                w.texts.entry(f.id.clone()).or_insert_with(TextBuf::single).set(&opts[next].value);
            }
            _ => {
                w.texts.entry(f.id.clone()).or_insert_with(TextBuf::single).handle(k);
            }
        },
        FieldType::Text | FieldType::Path => match k.code {
            KeyCode::Up if w.field > 0 => w.field -= 1,
            KeyCode::Down => w.field = (w.field + 1).min(n_fields.saturating_sub(1)),
            _ => {
                w.texts.entry(f.id.clone()).or_insert_with(TextBuf::single).handle(k);
                w.completions.clear();
            }
        },
    }
    vec![]
}

fn next_screen(w: &mut Wizard, wsid: Option<crate::backend::model::WorkspaceId>) -> Vec<Effect> {
    w.error = None;
    w.forget_moved_secrets();
    if w.error.is_some() {
        return vec![];
    }
    if let Some(e) = w.missing() {
        w.error = Some(e);
        return vec![];
    }
    w.error = None;
    w.completions.clear();
    let inputs = w.inputs();
    if w.screen + 1 >= w.screens().len() {
        w.stage = Stage::Review;
        return vec![];
    }
    w.screen += 1;
    w.field = 0;
    // The next screens may depend on what was chosen (a harness's models).
    vec![Effect::SetupForm(wsid, w.flow, inputs)]
}

fn prev_screen(w: &mut Wizard) {
    w.screen = w.screen.saturating_sub(1);
    w.field = 0;
    w.error = None;
}

/// A paste into the focused field (a secret goes into its buffer only).
pub fn paste(w: &mut Wizard, text: &str) {
    if w.stage != Stage::Fill {
        return;
    }
    let Some(f) = w.focused().cloned() else { return };
    match f.kind {
        FieldType::Secret => {
            let dest = w.destination();
            w.secrets.entry(f.id.clone()).or_default().push_str(text);
            w.secret_for.insert(f.id, dest);
        }
        FieldType::Text | FieldType::Path | FieldType::Suggest(_) => {
            let one: String = text.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).collect();
            w.texts.entry(f.id).or_insert_with(TextBuf::single).insert_str(&one);
        }
        _ => {}
    }
}

/// A path completion came back.
pub fn completed(s: &mut State, field: &str, c: Completion) {
    if let Some(Overlay::SetupFlow(w)) = s.overlay.as_mut()
        && w.focused().is_some_and(|f| f.id == field)
    {
        w.texts.entry(field.to_string()).or_insert_with(TextBuf::single).set(&c.text);
        w.completions = if c.candidates.len() > 1 { c.candidates } else { vec![] };
    }
}

/// A run is about to ask for the passkey: confirmation `step` of `of`.
pub fn confirming(s: &mut State, step: usize, of: usize, what: &str) {
    if let Some(Overlay::SetupFlow(w)) = s.overlay.as_mut()
        && w.stage == Stage::Running
        && of > 1
    {
        w.confirming = Some(crate::setup::shown(&format!("Confirm {step} of {of}: {what}")));
    }
}

/// A run finished.
pub fn ran(s: &mut State, flow: LtFlow, r: crate::backend::Result<SetupOutcome>) -> Vec<Effect> {
    let Some(Overlay::SetupFlow(w)) = s.overlay.as_mut() else { return vec![] };
    if w.flow != flow {
        return vec![];
    }
    match r {
        Ok(o) => {
            let wrote = o.exit == 0 && !o.dry_run;
            w.confirming = None;
            w.stage = Stage::Done(o);
            if wrote && let Some(ws) = s.active.clone() {
                return vec![
                    Effect::LoadAgents(ws.clone()),
                    Effect::LoadRuntimes(ws.clone()),
                    Effect::LoadHosts(ws.clone()),
                    Effect::LoadMembers(ws.clone()),
                    Effect::LoadCheckout(ws),
                    Effect::LoadWorkspaces,
                ];
            }
        }
        Err(e) => {
            w.stage = Stage::Failed(e.0.clone());
            s.toast(ToastKind::Error, e.0);
        }
    }
    vec![]
}

/// The form came back.
pub fn formed(s: &mut State, flow: LtFlow, r: crate::backend::Result<SetupForm>) {
    let Some(Overlay::SetupFlow(w)) = s.overlay.as_mut() else { return };
    if w.flow != flow {
        return;
    }
    match r {
        Ok(f) => w.set_form(f),
        Err(e) => w.stage = Stage::Failed(e.0),
    }
}

/// The device's facts changed (a sign-in check finished): a form being
/// filled in asks again, values kept.
pub fn refresh(s: &mut State) -> Vec<Effect> {
    match &s.overlay {
        Some(Overlay::SetupFlow(w)) if w.stage == Stage::Fill && w.form.is_some() => vec![Effect::SetupForm(ws(s), w.flow, w.inputs())],
        _ => vec![],
    }
}

/// After a key: a popup whose form was dropped by esc closes.
pub fn settle(s: &mut State) -> Vec<Effect> {
    if let Some(Overlay::SetupFlow(w)) = &s.overlay
        && w.form.is_none()
        && w.stage == Stage::Fill
    {
        let from = w.from_list;
        return close(s, from);
    }
    vec![]
}
