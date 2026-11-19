//! Workflow runs this device starts and drives.
//!
//! The crates hold the records and the scheduling rules
//! (`ligus_core::workflow`: `new_run`, `ready_nodes`, `apply_gate_outcome`,
//! `propagate_skips`, `derive_run_status`); the driver loop itself lives in
//! each client, as the desktop's does (`app/src/workflows.rs`). `lt`'s
//! follows the desktop's:
//!
//! 1. Start: validate, `new_run`, `upsert_workflow_run`, then claim the
//!    run's coordinator lease (`LeaseScope::WorkflowRun`, epoch 1). Every
//!    later write carries that epoch (the fencing token); the presence loop
//!    publishes the lease's heartbeat so no other device takes the run over.
//! 2. Each round: stop if the run settled elsewhere or the lease moved; fold
//!    each gate's proposal into the run; skip what can't run; open a gate
//!    proposal for each ready gate; post each ready agent stage's prompt.
//! 3. A stage whose agent runs on this device is a local turn (the same
//!    runner and sandbox policy as any reply). Any other stage @mentions its
//!    agent, so its host (a `ligus worker`, the owner's desktop) answers it
//!    like any mention; the driver waits for the reply that answers the
//!    prompt (`reply_to_message_id`).
//! 4. Gates are proposals: anyone may vote (`decide_gate` votes for you);
//!    the driver folds the outcome.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ligus_core::lease::{DEFAULT_LEASE_MS, LeaseClaimReason, LeaseScope};
use ligus_core::workflow::{self as wf, NodeRunStatus, WorkflowNodeKind, WorkflowRun, WorkflowRunStatus};
use ligus_core::{ActionProposal, ChatSession, MessageRole, ProposalKind, ProposalStatus, Timestamp};
use ligus_runtime::turn_dispatch::{host_binding, runs_here_linked};
use tokio::sync::Notify;
use uuid::Uuid;

use super::{Inner, lock, turns, views};
use crate::backend::model::{Run, WorkspaceId};
use crate::backend::{BackendError, Event, Result, err};

/// How much of a stage's output the run record keeps.
const EXCERPT_CHARS: usize = 400;
/// How often a waiting driver looks again (a vote or a reply also wakes it).
const POLL: Duration = Duration::from_millis(500);

/// A run this device drives.
pub struct Driving {
    pub ws: Uuid,
    pub sid: Uuid,
    pub epoch: u64,
    pub beat: u64,
    pub wake: Arc<Notify>,
    pub cancel: bool,
}

fn load_run(i: &Inner, ws: Uuid, sid: Uuid, run: Uuid) -> Result<(WorkflowRun, ChatSession)> {
    let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
    let r = s.workflow_runs.iter().find(|r| r.id == run).cloned().ok_or_else(|| BackendError("no such run".into()))?;
    Ok((r, s))
}

/// Find a run (or a definition) across the workspace's chats.
pub fn find_run(i: &Inner, ws: Uuid, run: Uuid) -> Result<(WorkflowRun, ChatSession)> {
    for s in i.chat_sessions(ws)? {
        if let Some(r) = s.workflow_runs.iter().find(|r| r.id == run) {
            return Ok((r.clone(), s.clone()));
        }
    }
    err("no such run")
}

impl Inner {
    /// A run as the UI shows it, with the label of the device driving it.
    pub(crate) fn run_view(&self, s: &ChatSession, r: &WorkflowRun) -> Run {
        let lease = s.guards.leases.get(&ligus_core::lease::lease_key(LeaseScope::WorkflowRun, &r.id.to_string()));
        let driver = lease.filter(|l| !l.released).map(|l| {
            let id = l.current.holder.to_string();
            self.host_labels(s.workspace_id).into_iter().find(|(h, _)| *h == id).map(|(_, label)| label).unwrap_or(id)
        });
        views::run(s, r, driver)
    }

    /// Persist `run` under this device's lease epoch.
    fn persist(&self, ws: Uuid, sid: Uuid, run: &mut WorkflowRun) -> Result<()> {
        run.updated_at = Timestamp::now();
        run.lease_epoch = lock(&self.drivers).get(&run.id).map(|d| d.epoch);
        let r = run.clone();
        self.write(ws, |svc| svc.upsert_workflow_run(sid, ws, r))?;
        // Straight to the UI: the change feed may fold this write into the
        // next one on the chat.
        if let Ok(Some(s)) = self.read(ws, |svc| svc.load(sid))
            && let Some(r) = s.workflow_runs.iter().find(|r| r.id == run.id)
        {
            self.emit(Event::RunChanged { workspace: WorkspaceId::new(ws.to_string()), run: self.run_view(&s, r) });
        }
        Ok(())
    }

    /// Whether this device still holds `run`'s lease at its epoch.
    fn still_holds(&self, ws: Uuid, sid: Uuid, run: Uuid) -> bool {
        let Some(epoch) = lock(&self.drivers).get(&run).map(|d| d.epoch) else { return false };
        let Ok(core) = self.core() else { return false };
        let me = core.identity.device.id;
        self.read(ws, |svc| svc.lease(sid, LeaseScope::WorkflowRun, &run.to_string()))
            .ok()
            .flatten()
            .is_some_and(|l| l.current.holder == me && l.current.epoch == epoch && !l.released)
    }
}

/// Start `def` (by id) with `input`: write the run, claim its lease, drive it.
pub fn start(inner: &Arc<Inner>, ws: Uuid, def: Uuid, input: &str) -> Result<Run> {
    let me = inner.me();
    let (s, d) = inner
        .chat_sessions(ws)?
        .into_iter()
        .find_map(|s| s.workflow_definitions.iter().find(|d| d.id == def).cloned().map(|d| (s, d)))
        .ok_or_else(|| BackendError("no such workflow".into()))?;
    if let crate::backend::model::Allowed::No(r) = views::workflow(&s, &d, &me).can_start {
        return Err(BackendError(r));
    }
    let run = wf::new_run(&d, input.trim(), me);
    let (sid, rid) = (s.id, run.id);
    inner.write(ws, |svc| svc.upsert_workflow_run(sid, ws, run))?;
    let device = inner.core()?.identity.device.id;
    let claimed = inner.write(ws, |svc| {
        svc.claim_lease(sid, ws, LeaseScope::WorkflowRun, &rid.to_string(), 1, DEFAULT_LEASE_MS, LeaseClaimReason::Initial)
    })?;
    if !claimed.is_some_and(|l| l.current.holder == device && l.current.epoch == 1) {
        return err("couldn't take the run's coordinator lease");
    }
    lock(&inner.drivers).insert(rid, Driving { ws, sid, epoch: 1, beat: 0, wake: Arc::new(Notify::new()), cancel: false });
    spawn(inner.clone(), ws, sid, rid);
    let (r, s) = load_run(inner, ws, sid, rid)?;
    Ok(inner.run_view(&s, &r))
}

/// Cancel a run this device drives.
pub fn cancel(inner: &Inner, ws: Uuid, run: Uuid) -> Result<()> {
    let (r, _) = find_run(inner, ws, run)?;
    if matches!(
        r.status,
        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Halted | WorkflowRunStatus::Canceled
    ) {
        return err(format!("the run already {}", views::run_status(r.status).label()));
    }
    let mut drivers = lock(&inner.drivers);
    let Some(d) = drivers.get_mut(&run) else {
        return err("another device drives this run: cancel it there");
    };
    d.cancel = true;
    d.wake.notify_one();
    Ok(())
}

/// Wake the driver of the run whose gate is `proposal` (after a vote).
pub fn wake_all(inner: &Inner, ws: Uuid) {
    for d in lock(&inner.drivers).values().filter(|d| d.ws == ws) {
        d.wake.notify_one();
    }
}

fn spawn(inner: Arc<Inner>, ws: Uuid, sid: Uuid, run: Uuid) {
    tokio::spawn(async move {
        if let Err(e) = drive(&inner, ws, sid, run).await {
            let failed = blocking(&inner, move |i| {
                let (mut r, _) = load_run(i, ws, sid, run)?;
                if !matches!(r.status, WorkflowRunStatus::Running | WorkflowRunStatus::AwaitingGate) {
                    return Ok(());
                }
                for n in r.nodes.iter_mut().filter(|n| matches!(n.status, NodeRunStatus::Running | NodeRunStatus::Pending)) {
                    n.status = NodeRunStatus::Failed;
                    n.error = e.0.clone();
                }
                r.status = WorkflowRunStatus::Failed;
                i.persist(ws, sid, &mut r)
            })
            .await;
            if let Err(f) = failed {
                inner.emit(Event::Sync { workspace: WorkspaceId::new(ws.to_string()), status: format!("workflow: {f}") });
            }
        }
        lock(&inner.drivers).remove(&run);
    });
}

async fn blocking<T: Send + 'static>(inner: &Arc<Inner>, f: impl FnOnce(&Inner) -> Result<T> + Send + 'static) -> Result<T> {
    let i = inner.clone();
    tokio::task::spawn_blocking(move || f(&i)).await.map_err(super::one_line)?
}

fn terminal(st: WorkflowRunStatus) -> bool {
    matches!(st, WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Halted | WorkflowRunStatus::Canceled)
}

/// Full outputs of the succeeded stages, from the transcript.
fn outputs(run: &WorkflowRun, s: &ChatSession) -> HashMap<String, String> {
    run.nodes
        .iter()
        .filter(|n| n.status == NodeRunStatus::Succeeded)
        .filter_map(|n| {
            let m = s.messages.iter().find(|m| Some(m.id) == n.message_id)?;
            Some((n.node_id.clone(), m.body.clone()))
        })
        .collect()
}

/// Close gate proposals a settled or reset run no longer waits on.
fn close_orphaned_gates(i: &Inner, ws: Uuid, sid: Uuid, run: &WorkflowRun, s: &ChatSession) -> Result<()> {
    for n in &run.nodes {
        let Some(pid) = n.proposal_id else { continue };
        if n.status == NodeRunStatus::AwaitingApproval {
            continue;
        }
        if let Some(p) = s.proposals.iter().find(|p| p.id == pid && p.status == ProposalStatus::Open && !p.dismissed) {
            let mut p = p.clone();
            p.dismissed = true;
            i.write(ws, |svc| svc.upsert_proposal(sid, ws, p))?;
        }
    }
    Ok(())
}

/// A stage prepared to run.
enum Stage {
    Local(Box<turns::Plan>),
    Remote { prompt: Uuid, author: String },
}

async fn drive(inner: &Arc<Inner>, ws: Uuid, sid: Uuid, run_id: Uuid) -> Result<()> {
    let wake = lock(&inner.drivers).get(&run_id).map(|d| d.wake.clone()).ok_or_else(|| BackendError("not driving".into()))?;
    loop {
        let round = blocking(inner, move |i| round(i, ws, sid, run_id)).await?;
        match round {
            Round::Done => return Ok(()),
            Round::Wait => {
                tokio::select! {
                    _ = wake.notified() => {}
                    _ = tokio::time::sleep(POLL) => {}
                }
            }
            Round::Stages(stages) => {
                let futs = stages.into_iter().map(|(node, stage)| {
                    let inner = inner.clone();
                    let wake = wake.clone();
                    async move {
                        let r = match stage {
                            Stage::Local(plan) => turns::run_turn(&inner, &plan).await,
                            Stage::Remote { prompt, author } => await_reply(&inner, ws, sid, run_id, prompt, &author, &wake).await,
                        };
                        (node, r)
                    }
                });
                let results = futures::future::join_all(futs).await;
                blocking(inner, move |i| {
                    let (mut run, _) = load_run(i, ws, sid, run_id)?;
                    for (node, r) in results {
                        let Some(n) = run.node_state_mut(&node) else { continue };
                        match r {
                            Ok((mid, body)) if !body.starts_with("[error] ") => {
                                n.status = NodeRunStatus::Succeeded;
                                n.message_id = Some(mid);
                                n.output_excerpt = body.chars().take(EXCERPT_CHARS).collect();
                            }
                            Ok((_, body)) => {
                                n.status = NodeRunStatus::Failed;
                                n.error = body.trim_start_matches("[error] ").lines().next().unwrap_or_default().to_string();
                            }
                            Err(e) => {
                                n.status = NodeRunStatus::Failed;
                                n.error = e.0;
                            }
                        }
                    }
                    run.status = wf::derive_run_status(&run);
                    i.persist(ws, sid, &mut run)
                })
                .await?;
            }
        }
    }
}

enum Round {
    Done,
    Wait,
    Stages(Vec<(String, Stage)>),
}

/// One scheduling round under the service locks (no awaits).
fn round(i: &Inner, ws: Uuid, sid: Uuid, run_id: Uuid) -> Result<Round> {
    if !i.still_holds(ws, sid, run_id) {
        return Ok(Round::Done);
    }
    let (mut run, s) = load_run(i, ws, sid, run_id)?;
    if terminal(run.status) {
        close_orphaned_gates(i, ws, sid, &run, &s)?;
        return Ok(Round::Done);
    }
    let cancel = lock(&i.drivers).get(&run_id).is_some_and(|d| d.cancel);
    if cancel {
        for n in run.nodes.iter_mut().filter(|n| !n.status.is_terminal()) {
            n.status = NodeRunStatus::Skipped;
        }
        run.status = WorkflowRunStatus::Canceled;
        i.persist(ws, sid, &mut run)?;
        close_orphaned_gates(i, ws, sid, &run, &s)?;
        return Ok(Round::Done);
    }
    // Fold settled gates.
    let before = format!("{:?}", run.nodes.iter().map(|n| n.status).collect::<Vec<_>>());
    for k in 0..run.nodes.len() {
        if run.nodes[k].status != NodeRunStatus::AwaitingApproval {
            continue;
        }
        let Some(pid) = run.nodes[k].proposal_id else { continue };
        let node = run.nodes[k].node_id.clone();
        match s.proposals.iter().find(|p| p.id == pid).map(|p| p.status) {
            Some(ProposalStatus::Approved | ProposalStatus::Applied) => wf::apply_gate_outcome(&mut run, &node, true),
            Some(ProposalStatus::Rejected) | None => wf::apply_gate_outcome(&mut run, &node, false),
            Some(ProposalStatus::Open) => {}
        }
    }
    wf::propagate_skips(&mut run);
    run.status = wf::derive_run_status(&run);
    close_orphaned_gates(i, ws, sid, &run, &s)?;
    if terminal(run.status) {
        i.persist(ws, sid, &mut run)?;
        return Ok(Round::Done);
    }
    let ready: Vec<wf::WorkflowNode> = wf::ready_nodes(&run).into_iter().cloned().collect();
    if ready.is_empty() {
        if !run.nodes.iter().any(|n| matches!(n.status, NodeRunStatus::AwaitingApproval | NodeRunStatus::Running)) {
            return err("no runnable stages left");
        }
        let after = format!("{:?}", run.nodes.iter().map(|n| n.status).collect::<Vec<_>>());
        if after != before {
            i.persist(ws, sid, &mut run)?;
        }
        return Ok(Round::Wait);
    }
    let outs = outputs(&run, &s);
    let mut agents_now = std::collections::HashSet::new();
    let mut agent_nodes = Vec::new();
    for node in &ready {
        match &node.kind {
            WorkflowNodeKind::Gate { title_template, body_template, required_approvals, .. } => {
                let title: String = wf::render_template(title_template, &run.input, &outs).chars().take(160).collect();
                // A gate has no human author, so no one is excluded from it.
                let mut p = ActionProposal::new(title, ProposalKind::Decision, "");
                p.body = wf::render_template(body_template, &run.input, &outs);
                p.required_approvals = (*required_approvals).max(1);
                let pid = p.id;
                i.write(ws, |svc| svc.upsert_proposal(sid, ws, p))?;
                if let Some(n) = run.node_state_mut(&node.id) {
                    n.status = NodeRunStatus::AwaitingApproval;
                    n.proposal_id = Some(pid);
                }
            }
            WorkflowNodeKind::Agent { agent_id, .. } => {
                // One stage per agent at a time: replies are matched per prompt,
                // but an agent answers one mention at a time.
                if agents_now.insert(*agent_id)
                    && let Some(n) = run.node_state_mut(&node.id)
                {
                    n.status = NodeRunStatus::Running;
                    n.attempts += 1;
                    agent_nodes.push(node.clone());
                }
            }
        }
    }
    run.status = wf::derive_run_status(&run);
    i.persist(ws, sid, &mut run)?;
    if agent_nodes.is_empty() {
        return Ok(Round::Wait);
    }
    let mut stages = Vec::new();
    for node in agent_nodes {
        match prepare(i, ws, sid, &run, &node) {
            Ok(st) => stages.push((node.id.clone(), st)),
            Err(e) => {
                let (mut r, _) = load_run(i, ws, sid, run_id)?;
                if let Some(n) = r.node_state_mut(&node.id) {
                    n.status = NodeRunStatus::Failed;
                    n.error = e.0;
                }
                r.status = wf::derive_run_status(&r);
                i.persist(ws, sid, &mut r)?;
            }
        }
    }
    Ok(Round::Stages(stages))
}

/// Post a stage's prompt; decide whether it runs here or on its agent's host.
fn prepare(i: &Inner, ws: Uuid, sid: Uuid, run: &WorkflowRun, node: &wf::WorkflowNode) -> Result<Stage> {
    let WorkflowNodeKind::Agent { agent_id, prompt_template } = &node.kind else { return err("not an agent stage") };
    let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
    let rendered = wf::render_template(prompt_template, &run.input, &outputs(run, &s));
    let agent = match agent_id {
        Some(id) => Some(
            s.workspace_agents
                .iter()
                .find(|a| a.id == *id)
                .cloned()
                .ok_or_else(|| BackendError(format!("stage '{}': its agent is no longer in the roster", node.name)))?,
        ),
        None => None,
    };
    let core = i.core()?;
    let local = core.identity.actor().id;
    let device = core.identity.device.id.to_string();
    let (owner, host, runtime_id, author) = match &agent {
        Some(a) => (a.owner_actor_id.clone(), a.host_id.clone(), a.runtime_id.clone(), a.name.clone()),
        None => (s.creator_actor_id.clone(), String::new(), s.runtime_id.clone(), "Ligus".to_string()),
    };
    let links = i.read(ws, |svc| svc.device_links(ws))?;
    let local_only = i.local_only(ws);
    let here = runs_here_linked(local_only, &links, &s, &local, &owner, host_binding(&host, &device));
    let tag = format!("**[Workflow · {} → {}]**", run.definition.name, node.name);
    if here {
        let posted = i.write(ws, |svc| svc.post_workflow_stage_prompt(sid, ws, format!("{tag}\n\n{rendered}"), run.id))?;
        // Claimed, so no other device answers the stage's prompt.
        i.write(ws, |svc| svc.claim_turn(sid, ws, posted.id))?;
        return Ok(Stage::Local(Box::new(turns::Plan { ws, sid, trigger: posted.id, agent, runtime_id })));
    }
    let mention = agent.as_ref().map(|a| format!("@{}", a.name)).unwrap_or_else(|| "@primary".into());
    let posted = i.write(ws, |svc| svc.post_workflow_stage_prompt(sid, ws, format!("{mention} {tag}\n\n{rendered}"), run.id))?;
    Ok(Stage::Remote { prompt: posted.id, author })
}

/// Wait for the reply to a remote stage's prompt.
async fn await_reply(
    inner: &Arc<Inner>,
    ws: Uuid,
    sid: Uuid,
    run: Uuid,
    prompt: Uuid,
    author: &str,
    wake: &Notify,
) -> Result<(Uuid, String)> {
    let deadline = Instant::now() + inner.cfg.stage_timeout;
    loop {
        let by = author.to_string();
        let found = blocking(inner, move |i| {
            let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
            let Some(start) = s.messages.iter().position(|m| m.id == prompt) else { return Ok(None) };
            Ok(s.messages[start + 1..]
                .iter()
                .find(|m| {
                    matches!(m.role, MessageRole::Assistant | MessageRole::Agent)
                        && !m.is_streaming
                        && !m.body.is_empty()
                        && match m.reply_to_message_id {
                            Some(answers) => answers == prompt,
                            None => m.author == by,
                        }
                })
                .map(|m| (m.id, m.body.clone())))
        })
        .await?;
        if let Some(r) = found {
            return Ok(r);
        }
        if lock(&inner.drivers).get(&run).is_some_and(|d| d.cancel) {
            return err("canceled");
        }
        if Instant::now() >= deadline {
            return err(format!(
                "no reply from {author} synced back within {}s: is its host (a worker, or its owner's device) online?",
                inner.cfg.stage_timeout.as_secs()
            ));
        }
        tokio::select! {
            _ = wake.notified() => {}
            _ = tokio::time::sleep(POLL) => {}
        }
    }
}
