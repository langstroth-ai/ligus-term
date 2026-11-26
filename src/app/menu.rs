//! The row menu (`.`, `shift-f10`): the selected row's actions, then the
//! view's, then the destructive ones (each asks first when chosen). Rows
//! show only who or what they are and one status; what you can do to one
//! lives here. Pure, like the reducer: built from [`State`], and choosing
//! an entry runs its action as its key would.

use crate::app::state::{MenuItem, MenuSection, State, View, WorkflowItem};
use crate::backend::model::{Allowed, ApplyState, ProposalStatus, Role, Shelf, StageStatus};
use crate::keymap::{Action, Context};

fn item(label: impl Into<String>, action: Action, context: Context, allowed: Allowed, section: MenuSection) -> MenuItem {
    MenuItem { label: label.into(), action, context, allowed, section }
}

/// The menu for what is selected: its title (the row's name, or the view's
/// when nothing is selected) and its entries in section order. Empty
/// entries mean there is nothing to do here.
pub fn row_menu(s: &State) -> (String, Vec<MenuItem>) {
    use Action::*;
    use MenuSection::{Danger, Pane, Row};
    let mut items = vec![];
    let mut title = s.view.title().to_string();
    match s.view {
        View::Chats => {
            let ctx = Context::Chat;
            items.push(item("New chat…", NewChat, ctx, Allowed::Yes, Pane));
            if let Some(c) = s.open_chat_summary() {
                title = c.title.clone();
                items.push(item("Write a message", Compose, ctx, Allowed::Yes, Row));
                let tools = if s.tools_expanded { "Hide what tool calls returned" } else { "Show what tool calls returned" };
                items.push(item(tools, ToggleTools, ctx, Allowed::Yes, Row));
                items.push(item("Transcript in $PAGER", DumpTranscript, ctx, Allowed::Yes, Row));
                items.push(item("Repository…", ChatRepo, ctx, Allowed::Yes, Row));
                if c.repo.as_ref().is_some_and(|r| r.mine) {
                    items.push(item("Keep the scratch repo as a repo…", KeepScratch, ctx, Allowed::Yes, Row));
                }
                match &c.lockdown {
                    None => items.push(item("Lock this chat down", LockChat, ctx, Allowed::Yes, Row)),
                    Some(l) => items.push(item("Clear the lockdown", ClearLockdown, ctx, l.clear.clone(), Row)),
                }
            }
        }
        View::Proposals => {
            let ctx = Context::Review;
            let shelf_label = if s.show_archived { "Back to proposals" } else { "Archived proposals" };
            items.push(item(shelf_label, ShowArchived, ctx, Allowed::Yes, Pane));
            if let Some(p) = s.selected_proposal().filter(|p| !p.in_inbox()) {
                // Archived or hidden: only bringing it back (or hiding it).
                title = p.title.clone();
                if s.focus != crate::app::state::Focus::Detail {
                    items.push(item("View the diff", Select, Context::Nav, Allowed::Yes, Row));
                }
                match p.shelf {
                    Shelf::Hidden => items.push(item("Show again", HideProposal, ctx, Allowed::Yes, Row)),
                    _ => {
                        items.push(item("Restore", ArchiveProposal, ctx, p.can_archive.clone(), Row));
                        items.push(item("Hide for me", HideProposal, ctx, Allowed::Yes, Row));
                    }
                }
            } else if let Some(p) = s.selected_proposal() {
                title = p.title.clone();
                if s.focus != crate::app::state::Focus::Detail {
                    items.push(item("View the diff", Select, Context::Nav, Allowed::Yes, Row));
                }
                let vote = if p.status == ProposalStatus::Open {
                    p.can_vote.clone()
                } else {
                    Allowed::No(format!("already {}", p.status.label()))
                };
                items.push(item("Approve", Approve, ctx, vote.clone(), Row));
                let apply_ok = match (&p.status, &p.apply) {
                    (_, ApplyState::Waiting { .. }) => Allowed::No("one applier per proposal".into()),
                    (ProposalStatus::Approved, _) => Allowed::Yes,
                    (ProposalStatus::Applied, _) => Allowed::No("already applied".into()),
                    _ => Allowed::No("needs approval first".into()),
                };
                let host = s
                    .default_apply_host()
                    .map(|h| if h.this_device { "this device".to_string() } else { h.label.clone() })
                    .unwrap_or_else(|| "a host".into());
                let more = if s.hosts.len() > 1 { "…" } else { "" };
                items.push(item(format!("Apply on {host}{more}"), ApplyOnHost, ctx, apply_ok, Row));
                if matches!(p.apply, ApplyState::Waiting { .. }) {
                    items.push(item("Withdraw the apply request", WithdrawApply, ctx, Allowed::Yes, Row));
                }
                // Only a settled proposal can be archived for everyone;
                // anyone can hide one for themselves.
                if !p.waiting {
                    items.push(item("Archive", ArchiveProposal, ctx, p.can_archive.clone(), Row));
                }
                items.push(item("Hide for me", HideProposal, ctx, Allowed::Yes, Row));
                items.push(item("Reject…", Reject, ctx, vote, Danger));
            }
        }
        View::Agents => {
            let ctx = Context::Agents;
            if let Some(a) = s.agents.get(s.agent_sel.index) {
                title = format!("@{}", a.handle);
                let manage = if a.can_manage {
                    Allowed::Yes
                } else {
                    Allowed::No(if a.shared { "only an Admin".into() } else { "only its owner".into() })
                };
                items.push(item("Change its runtime…", ChangeRuntime, ctx, Allowed::Yes, Row));
                let access = if a.access == "everyone" { "Only me can use it" } else { "Let everyone use it" };
                items.push(item(access, ToggleAccess, ctx, manage, Row));
            }
            items.push(item("Add an agent…", AddAgent, ctx, Allowed::Yes, Pane));
        }
        View::Hosts => {
            let label = if s.checkout.is_some() { "Change this device's checkout…" } else { "Bind this device's checkout…" };
            items.push(item(label, BindCheckout, Context::Hosts, Allowed::Yes, Pane));
            if let Some(h) = s.hosts.get(s.host_sel.index) {
                title = h.label.clone();
            }
        }
        View::People => {
            let ctx = Context::People;
            if let Some(m) = s.selected_member() {
                title = m.name.clone();
                let role = if m.can_set_role.iter().any(|(_, a)| a.is_yes()) {
                    Allowed::Yes
                } else {
                    Allowed::No(
                        m.can_set_role.iter().find(|(r, _)| *r != m.role).and_then(|(_, a)| a.reason()).unwrap_or("not allowed").into(),
                    )
                };
                items.push(item("Change role…", ChangeRole, ctx, role, Row));
                let remove = if m.you { "Leave the workspace…" } else { "Remove from the workspace…" };
                items.push(item(remove, RemoveMember, ctx, m.can_remove.clone(), Danger));
            }
            let invite = s.active_workspace().map(|w| w.can_invite.clone()).unwrap_or(Allowed::Yes);
            items.push(item("Create an invite", Invite, ctx, invite, Pane));
            let admit = if s.active_workspace().is_some_and(|w| w.your_role >= Role::Admin) {
                Allowed::Yes
            } else {
                Allowed::No("only an Admin or Owner".into())
            };
            items.push(item("Admit a box…", AdmitBox, ctx, admit, Pane));
            items.push(item("Approval requests…", Approvals, Context::Global, Allowed::Yes, Pane));
            if s.own_member_unlinked() {
                items.push(item("Link to my devices…", LinkDevice, ctx, Allowed::Yes, Pane));
            }
        }
        View::Notifications => {
            if let Some(n) = s.notifications.get(s.notification_sel.index) {
                title = crate::ui::trunc(&n.text, 40);
                items.push(item("Open", Select, Context::Nav, Allowed::Yes, Row));
            }
            if s.notifications.iter().any(|n| !n.read) {
                items.push(item("Mark all read", MarkAllRead, Context::Notifications, Allowed::Yes, Pane));
            }
        }
        View::Workflows => {
            let ctx = Context::Workflows;
            match s.selected_workflow_item() {
                Some(WorkflowItem::Def(d)) => {
                    title = d.name.clone();
                    items.push(item("Start a run…", StartRun, ctx, d.can_start.clone(), Row));
                }
                Some(WorkflowItem::Run(r)) => {
                    title = r.name.clone();
                    let gate = r.stages.iter().any(|st| st.status == StageStatus::AwaitingApproval);
                    if gate {
                        items.push(item("Approve the gate", ApproveGate, ctx, Allowed::Yes, Row));
                    }
                    items.push(item("Open its chat", Select, Context::Nav, Allowed::Yes, Row));
                    if let Some(d) = s.workflows.iter().find(|d| d.id == r.workflow) {
                        items.push(item("Start another run…", StartRun, ctx, d.can_start.clone(), Row));
                    }
                    if gate {
                        items.push(item("Reject the gate…", RejectGate, ctx, Allowed::Yes, Danger));
                    }
                    if !r.status.is_terminal() {
                        items.push(item("Cancel the run…", CancelRun, ctx, Allowed::Yes, Danger));
                    }
                }
                None => {}
            }
        }
        View::Settings => {
            let ctx = Context::Settings;
            if s.settings.as_ref().is_some_and(|st| st.linked_to.is_none()) && s.active_workspace().is_some() {
                items.push(item("Link to my devices…", LinkDevice, ctx, Allowed::Yes, Pane));
            }
            if s.settings.as_ref().is_some_and(|st| st.link_unconfirmed.is_some()) {
                items.push(item("Retry confirming the link", ConfirmLink, ctx, Allowed::Yes, Pane));
            }
            if !s.device.safety_code.is_empty() {
                items.push(item("Copy your safety code", CopySafetyCode, ctx, Allowed::Yes, Pane));
            }
        }
        View::Memory => {}
    }
    items.sort_by_key(|i| i.section);
    (title, items)
}
