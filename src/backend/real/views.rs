//! From the crates' projections to `lt`'s view models.
//!
//! Nothing here decides authorization: refusals come from
//! `ligus_core::authz::member_action_refusal` and the lockdown rule from
//! `ligus_runtime::lockdown_view`.

use ligus_core::authz::{MemberAction, RosterState, member_action_refusal};
use ligus_core::workspace_repo::ApplyOutcome;
use ligus_core::{ActionProposal, ActorKind, ChatMessage, ChatSession, MessageRole, ModelProviderKind, Timestamp, WorkspaceRole};
use uuid::Uuid;

use crate::backend::model::*;

pub fn ms(t: &Timestamp) -> Millis {
    (t.inner().unix_timestamp_nanos() / 1_000_000) as Millis
}

pub fn role(r: WorkspaceRole) -> Role {
    match r {
        WorkspaceRole::Owner => Role::Owner,
        WorkspaceRole::Admin => Role::Admin,
        WorkspaceRole::Contributor => Role::Contributor,
        _ => Role::Viewer,
    }
}

pub fn workspace_role(r: Role) -> WorkspaceRole {
    match r {
        Role::Owner => WorkspaceRole::Owner,
        Role::Admin => WorkspaceRole::Admin,
        Role::Contributor => WorkspaceRole::Contributor,
        Role::Viewer => WorkspaceRole::Viewer,
    }
}

pub fn allowed(refusal: Option<String>) -> Allowed {
    match refusal {
        None => Allowed::Yes,
        Some(r) => Allowed::No(r),
    }
}

/// A message, with its author as the transcript shows it.
pub fn message(s: &ChatSession, m: &ChatMessage) -> Message {
    let author = match m.role {
        MessageRole::System => Author::System,
        MessageRole::User => {
            let id = m.actor_identity.as_ref().map(|a| a.id.clone()).unwrap_or_default();
            let name = s.members.iter().find(|x| x.id == id).map(|x| x.actor.display_name.clone()).unwrap_or_else(|| m.author.clone());
            Author::Member { id: MemberId::new(id), name }
        }
        _ => {
            let agent = m.responder_id.and_then(|r| s.workspace_agents.iter().find(|a| a.id == r));
            Author::Agent {
                id: AgentId::new(m.responder_id.map(|r| r.to_string()).unwrap_or_default()),
                handle: agent.map(|a| a.name.clone()).unwrap_or_else(|| m.author.clone()),
                runtime: m.runtime_id.clone().unwrap_or_default(),
            }
        }
    };
    Message {
        id: MessageId::new(m.id.to_string()),
        chat: ChatId::new(s.id.to_string()),
        author,
        text: m.body.clone(),
        at: ms(&m.created_at),
        streaming: m.is_streaming,
        tools: m.tool_calls.iter().map(|c| ToolCall { name: c.name.clone(), preview: preview(&c.input_json), ok: true }).collect(),
    }
}

fn preview(json: &str) -> String {
    let one: String = json.chars().filter(|c| *c != '\n').take(80).collect();
    one
}

/// The transcript: chat rows only (proposal and tool records are their own
/// views).
pub fn transcript(s: &ChatSession) -> Vec<Message> {
    s.messages.iter().map(|m| message(s, m)).collect()
}

/// A proposal for `me`. `can_vote` is the backend's: an open proposal you
/// haven't voted on, at or above its role floor.
pub fn proposal(s: &ChatSession, p: &ActionProposal, me: &str, hosts: &[(String, String)]) -> Proposal {
    let host_label = |id: &Uuid| {
        let id = id.to_string();
        hosts.iter().find(|(h, _)| *h == id).map(|(_, l)| l.clone()).unwrap_or(id)
    };
    let request = s.apply_requests.values().filter(|r| r.proposal_id == p.id).max_by_key(|r| r.id);
    let apply = match request {
        None => ApplyState::NotRequested,
        Some(r) => match s.apply_outcomes.get(&r.id) {
            None => ApplyState::Waiting { host: host_label(&r.host_device_id) },
            Some(ApplyOutcome::Applied { commit }) => {
                ApplyState::Applied { host: host_label(&r.host_device_id), commit: commit.clone().unwrap_or_default() }
            }
            Some(ApplyOutcome::Failed { reason }) => ApplyState::Failed { host: host_label(&r.host_device_id), reason: reason.clone() },
            Some(ApplyOutcome::Withdrawn) => ApplyState::Withdrawn,
        },
    };
    let my_role = s.role_of(me);
    let voted = p.approvals.iter().any(|a| a.actor_id == me);
    let can_vote = if p.status != ligus_core::ProposalStatus::Open {
        Allowed::No("this proposal is closed".into())
    } else if voted {
        Allowed::No("you already voted".into())
    } else if !s.members.iter().any(|m| m.id == me) {
        Allowed::No("you aren't a member of this workspace".into())
    } else if my_role.rank() < p.approval_role_floor.rank() {
        Allowed::No(format!("votes here need {} or above", role(p.approval_role_floor).label()))
    } else {
        Allowed::Yes
    };
    let name_of = |id: &str| s.members.iter().find(|m| m.id == id).map(|m| m.actor.display_name.clone());
    let author = s
        .workspace_agents
        .iter()
        .find(|a| Some(a.id) == p.producing_agent_id)
        .map(|a| format!("@{}", a.name))
        .or_else(|| name_of(&p.author_actor_id))
        .unwrap_or_else(|| p.author_actor_id.clone());
    Proposal {
        id: ProposalId::new(p.id.to_string()),
        chat: Some(ChatId::new(s.id.to_string())),
        title: p.title.clone(),
        author,
        status: match p.status {
            ligus_core::ProposalStatus::Open => ProposalStatus::Open,
            ligus_core::ProposalStatus::Approved => ProposalStatus::Approved,
            ligus_core::ProposalStatus::Rejected => ProposalStatus::Rejected,
            ligus_core::ProposalStatus::Applied => ProposalStatus::Applied,
        },
        files_changed: p.changed_files.len(),
        approvals: p.approvals.iter().filter(|a| a.approved).count(),
        quorum: p.required_approvals as usize,
        votes: p
            .approvals
            .iter()
            .map(|a| Vote { member: name_of(&a.actor_id).unwrap_or_else(|| a.actor_id.clone()), role: role(a.role), approve: a.approved })
            .collect(),
        created_at: ms(&p.created_at),
        needs_presence: p.workflow.is_some(),
        apply,
        can_vote,
    }
}

/// A unified diff, split into files and hunks.
pub fn parse_diff(text: &str) -> Diff {
    let mut files: Vec<FileDiff> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest.split(" b/").nth(1).unwrap_or(rest).to_string();
            files.push(FileDiff { path, added: 0, removed: 0, hunks: Vec::new() });
            continue;
        }
        if let Some(p) = line.strip_prefix("+++ ") {
            let p = p.trim();
            if let Some(f) = files.last_mut() {
                if p != "/dev/null" {
                    f.path = p.strip_prefix("b/").unwrap_or(p).to_string();
                }
            } else {
                files.push(FileDiff { path: p.strip_prefix("b/").unwrap_or(p).into(), added: 0, removed: 0, hunks: Vec::new() });
            }
            continue;
        }
        if line.starts_with("--- ") || line.starts_with("index ") || line.starts_with("new file") || line.starts_with("deleted file") {
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if line.starts_with("@@") {
            f.hunks.push(Hunk { header: line.to_string(), lines: Vec::new() });
            continue;
        }
        let Some(h) = f.hunks.last_mut() else { continue };
        if let Some(a) = line.strip_prefix('+') {
            f.added += 1;
            h.lines.push(DiffLine::Added(a.to_string()));
        } else if let Some(r) = line.strip_prefix('-') {
            f.removed += 1;
            h.lines.push(DiffLine::Removed(r.to_string()));
        } else if let Some(c) = line.strip_prefix(' ') {
            h.lines.push(DiffLine::Context(c.to_string()));
        } else if line.is_empty() {
            h.lines.push(DiffLine::Context(String::new()));
        }
    }
    Diff { files }
}

/// The workspace's members for `me`, with what `me` may do to each.
pub fn members(config: &ChatSession, roster: &RosterState, me: &str) -> Vec<Member> {
    config
        .members
        .iter()
        .map(|m| {
            let you = m.id == me;
            let can_remove = if you {
                Allowed::Yes
            } else {
                allowed(member_action_refusal(roster, config, me, &MemberAction::Remove { member_id: m.id.clone() }))
            };
            let can_set_role = Role::ALL
                .iter()
                .map(|&r| {
                    let wr = workspace_role(r);
                    let a = if wr == m.role {
                        Allowed::No("already that role".into())
                    } else {
                        allowed(member_action_refusal(roster, config, me, &MemberAction::SetRole { member_id: m.id.clone(), role: wr }))
                    };
                    (r, a)
                })
                .collect();
            Member {
                id: MemberId::new(m.id.clone()),
                name: m.actor.display_name.clone(),
                role: role(m.role),
                founder: !roster.founder.is_empty() && roster.founder == m.id,
                service: m.actor.kind == ActorKind::Service,
                you,
                can_remove,
                can_set_role,
            }
        })
        .collect()
}

pub fn runtime_kind(p: ModelProviderKind) -> RuntimeKind {
    match p {
        ModelProviderKind::ClaudeCode => RuntimeKind::ClaudeCode,
        ModelProviderKind::Codex => RuntimeKind::Codex,
        ModelProviderKind::Antigravity => RuntimeKind::Antigravity,
        ModelProviderKind::Copilot => RuntimeKind::Copilot,
        ModelProviderKind::Pi => RuntimeKind::Pi,
        ModelProviderKind::Aider => RuntimeKind::Aider,
        ModelProviderKind::Ollama => RuntimeKind::Ollama,
        ModelProviderKind::Anthropic => RuntimeKind::Anthropic,
        ModelProviderKind::OpenAI | ModelProviderKind::Azure => RuntimeKind::OpenAi,
        ModelProviderKind::OpenRouter => RuntimeKind::OpenRouter,
        _ => RuntimeKind::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_unified_diff() {
        let d = parse_diff(
            "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\ndiff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n",
        );
        assert_eq!(d.files.len(), 2);
        assert_eq!((d.files[0].path.as_str(), d.files[0].added, d.files[0].removed), ("new.txt", 2, 0));
        assert_eq!((d.files[1].path.as_str(), d.files[1].added, d.files[1].removed), ("a.rs", 1, 1));
        assert_eq!(
            d.files[1].hunks[0].lines,
            vec![DiffLine::Context("ctx".into()), DiffLine::Removed("old".into()), DiffLine::Added("new".into())]
        );
    }

    #[test]
    fn roles_map_both_ways() {
        for r in Role::ALL {
            assert_eq!(role(workspace_role(r)), r);
        }
    }
}
