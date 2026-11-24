//! The command palette (`:`, `ctrl-p`, `ctrl-k`): one fuzzy list over the
//! keymap's actions for what has focus, every view, and the workspace's
//! workspaces, chats, proposals, runs and agents. Each action shows its key,
//! so the palette teaches the keys. Built from `keymap::BINDINGS` and the
//! loaded lists, so it stays as complete as the help.

use crate::app::state::{State, View, WorkflowItem};
use crate::backend::model::*;
use crate::keymap::{Action, BINDINGS, Context};

/// What choosing a palette entry does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Run a keymap action, as its key would.
    Action(Action),
    SwitchWorkspace(WorkspaceId),
    OpenChat(ChatId),
    ShowProposal(ProposalId),
    ShowRun(RunId),
    ShowAgent(AgentId),
    /// Keep this theme for the session (shown while selected).
    Theme(String),
    /// Keep this appearance for the session (shown while selected).
    Appearance(crate::theme::Appearance),
}

/// One entry: its label, the key that does the same (if any), and where
/// it applies or what it is ("Proposals", "Chat", "Workspace").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub key: String,
    pub scope: String,
    pub target: Target,
}

/// Actions the palette leaves out: moving in a list, editing text, and
/// opening the palette itself.
fn hidden(a: Action) -> bool {
    use Action::*;
    matches!(
        a,
        Up | Down | PageUp | PageDown | Top | Bottom | Select | Back | Send | Newline | CancelCompose | Complete | Palette | FocusPrev
    )
}

/// What an action acts on, so its entry names it ("Approve “Fix the
/// flaky test”"). `Some(None)` for an action about nothing in particular;
/// `None` when it needs a subject and nothing is selected (left out).
fn subject(s: &State, a: Action) -> Option<Option<String>> {
    use Action::*;
    Some(match a {
        Approve | Reject | ApplyOnHost | WithdrawApply => Some(s.selected_proposal()?.title.clone()),
        ChangeRole | RemoveMember => Some(s.selected_member()?.name.clone()),
        ChangeRuntime | ToggleAccess => Some(format!("@{}", s.agents.get(s.agent_sel.index)?.handle)),
        StartRun => match s.selected_workflow_item()? {
            WorkflowItem::Def(d) => Some(d.name.clone()),
            WorkflowItem::Run(r) => Some(s.workflows.iter().find(|d| d.id == r.workflow)?.name.clone()),
        },
        ApproveGate | RejectGate | CancelRun => match s.selected_workflow_item()? {
            WorkflowItem::Run(r) => Some(r.name.clone()),
            WorkflowItem::Def(_) => return None,
        },
        LockChat | ClearLockdown | Compose => Some(s.open_chat_summary()?.title.clone()),
        _ => None,
    })
}

/// A binding's help without its trailing aside: "Reject (asks first)" is
/// "Reject" here (the palette asks just the same).
fn short(help: &str) -> &str {
    match help.rfind(" (") {
        Some(i) if help.ends_with(')') => &help[..i],
        _ => help,
    }
}

fn keys(s: &State, ctx: Context, a: Action) -> String {
    s.keymap.keys(ctx, a).iter().take(2).map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
}

/// Every entry, before filtering: the focused view's actions first, then
/// the actions available anywhere, the views, and the entities.
pub fn items(s: &State) -> Vec<Item> {
    let mut out = vec![];
    let mut contexts = s.view_contexts();
    // From the composer, the chat's own actions still apply.
    if s.view == View::Chats && !contexts.contains(&Context::Chat) {
        contexts.insert(0, Context::Chat);
    }
    for ctx in contexts.iter().copied().filter(|c| *c != Context::Global && *c != Context::Nav && *c != Context::Composer) {
        for b in BINDINGS.iter().filter(|b| b.context == ctx && !hidden(b.action)) {
            let Some(about) = subject(s, b.action) else { continue };
            let label = match &about {
                Some(x) => format!("{} \u{201c}{}\u{201d}", short(b.help), x),
                None => short(b.help).to_string(),
            };
            out.push(Item { label, key: keys(s, ctx, b.action), scope: ctx.title().into(), target: Target::Action(b.action) });
        }
    }
    let views = [
        Action::GoChats,
        Action::GoWorkflows,
        Action::GoProposals,
        Action::GoAgents,
        Action::GoHosts,
        Action::GoMemory,
        Action::GoPeople,
        Action::GoNotifications,
        Action::GoSettings,
    ];
    for b in BINDINGS.iter().filter(|b| b.context == Context::Global && !hidden(b.action)) {
        let label = if views.contains(&b.action) { format!("Go to {}", b.help) } else { short(b.help).to_string() };
        out.push(Item { label, key: keys(s, Context::Global, b.action), scope: "Anywhere".into(), target: Target::Action(b.action) });
    }
    for w in &s.workspaces {
        if s.active.as_ref() != Some(&w.id) {
            out.push(Item {
                label: format!("Switch to {}", w.name),
                key: String::new(),
                scope: "Workspace".into(),
                target: Target::SwitchWorkspace(w.id.clone()),
            });
        }
    }
    for c in &s.chats {
        out.push(Item { label: c.title.clone(), key: String::new(), scope: "Chat".into(), target: Target::OpenChat(c.id.clone()) });
    }
    for p in &s.proposals {
        out.push(Item {
            label: format!("{} ({})", p.title, p.status.label()),
            key: String::new(),
            scope: "Proposal".into(),
            target: Target::ShowProposal(p.id.clone()),
        });
    }
    for r in &s.runs {
        out.push(Item {
            label: format!("{} ({})", r.name, r.status.label()),
            key: String::new(),
            scope: "Run".into(),
            target: Target::ShowRun(r.id.clone()),
        });
    }
    for a in &s.agents {
        out.push(Item {
            label: format!("@{}", a.handle),
            key: String::new(),
            scope: "Agent".into(),
            target: Target::ShowAgent(a.id.clone()),
        });
    }
    for (id, label) in &s.themes {
        let now = if *id == s.theme_id { " (now)" } else { "" };
        out.push(Item {
            label: format!("Theme: {label}{now}"),
            key: String::new(),
            scope: "Theme".into(),
            target: Target::Theme(id.clone()),
        });
    }
    for a in crate::theme::Appearance::ALL {
        let now = if a == s.appearance { " (now)" } else { "" };
        out.push(Item {
            label: format!("Light or dark: {}{now}", a.name()),
            key: String::new(),
            scope: "Theme".into(),
            target: Target::Appearance(a),
        });
    }
    out
}

/// How well `query` matches `text`: every character in order (a
/// subsequence), scored higher for a prefix, then for matches at word
/// starts and next to each other. `None` when it doesn't match.
pub fn score(query: &str, text: &str) -> Option<i64> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0i64;
    let mut qi = 0;
    let mut prev: Option<usize> = None;
    for (i, c) in t.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            let word_start = i == 0 || !t[i - 1].is_alphanumeric();
            score += if word_start { 8 } else { 1 };
            if prev == Some(i.wrapping_sub(1)) {
                score += 4;
            }
            prev = Some(i);
            qi += 1;
        }
    }
    if qi < q.len() {
        return None;
    }
    if t.iter().collect::<String>().starts_with(&q.iter().collect::<String>()) {
        score += 50;
    }
    Some(score - (t.len() as i64) / 8)
}

/// The entries matching `query`, best first (ties keep the list's order).
pub fn filtered(s: &State, query: &str) -> Vec<Item> {
    let mut scored: Vec<(i64, usize, Item)> = items(s)
        .into_iter()
        .enumerate()
        .filter_map(|(i, it)| {
            let label = score(query, &it.label);
            let scope = score(query, &format!("{} {}", it.scope, it.label)).map(|x| x - 10);
            label.max(scope).map(|sc| (sc, i, it))
        })
        .collect();
    if !query.trim().is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    scored.into_iter().map(|(_, _, it)| it).collect()
}

/// The view an entity lives in.
pub fn view_of(t: &Target) -> Option<View> {
    match t {
        Target::OpenChat(_) => Some(View::Chats),
        Target::ShowProposal(_) => Some(View::Proposals),
        Target::ShowRun(_) => Some(View::Workflows),
        Target::ShowAgent(_) => Some(View::Agents),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_scores_prefix_and_word_starts_first() {
        assert!(score("appr", "Approve").unwrap() > score("appr", "Re-approve later").unwrap());
        assert!(score("gp", "Go to Proposals").is_some());
        assert!(score("xyz", "Go to Proposals").is_none());
        assert_eq!(score("", "anything"), Some(0));
        assert!(score("go prop", "Go to Proposals").is_some(), "spaces are ignored");
    }
}
