//! Decision cards (agent questions, Ligus 0.4) from the shared card view
//! (`ligus_runtime::ask_view`): the backend's half of `Backend::asks` and
//! `Backend::answer_ask`. Who may answer and what an answer does are the
//! fold's; this only shapes the card and says who it waits on.

use ligus_core::ChatSession;
use ligus_runtime::ask_view::{AskView, views};
use uuid::Uuid;

use super::Inner;
use crate::backend::model::*;

/// The cards in `s` for `me`, oldest first.
pub(super) fn cards(inner: &Inner, ws: Uuid, s: &ChatSession, me: &str, now: Millis) -> Vec<Ask> {
    let hosts = inner.host_labels(ws);
    let label = |d: Uuid| hosts.iter().find(|(id, _)| *id == d.to_string()).map(|(_, l)| l.clone());
    let mut out: Vec<Ask> = views(s, me, now, 50, &label)
        .into_iter()
        .map(|v| {
            let turn = Uuid::parse_str(&v.ask_id)
                .ok()
                .and_then(|id| s.flow.asks.get(&id))
                .map(|st| st.ask.turn_id)
                .filter(|t| !t.is_nil())
                .map(|t| MessageId::new(t.to_string()));
            card(v, turn, me)
        })
        .collect();
    out.sort_by_key(|a| a.at);
    out
}

fn names(list: &[String]) -> String {
    match list {
        [] => "nobody".into(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// One card from the shared view.
pub fn card(v: AskView, turn: Option<MessageId>, me: &str) -> Ask {
    let audience = match v.audience.kind {
        "people" if v.audience.member_ids.len() == 1 && v.audience.member_ids[0] == me => "you".to_string(),
        "people" => names(&v.audience.names),
        _ => match v.audience.min_role {
            Some("admin") => "Admins".into(),
            Some("owner") => "Owners".into(),
            _ => "the room".into(),
        },
    };
    let waiting_on = if v.audience.kind == "people" { names(&v.audience.names) } else { audience.clone() };
    let can_answer = if v.eligible {
        Allowed::Yes
    } else if v.quorum > 1 {
        let most = v.tally.iter().copied().max().unwrap_or(0);
        Allowed::No(format!("Waiting on {waiting_on} · {most} of {}", v.quorum))
    } else {
        Allowed::No(format!("Waiting on {waiting_on}"))
    };
    let status = if v.declined {
        AskStatus::Declined { by: v.answered_by.first().map(|b| b.name.clone()).unwrap_or_default() }
    } else if let Some(answer) = v.answer.clone() {
        AskStatus::Answered {
            by: names(&v.answered_by.iter().map(|b| b.name.clone()).collect::<Vec<_>>()),
            answer,
            text: v.answer_text.clone(),
            at: v.answered_by.last().map(|b| b.at_ms).unwrap_or(v.at_ms),
        }
    } else if v.live {
        AskStatus::Open
    } else {
        AskStatus::Stopped
    };
    let origin = v.origin.as_ref().map(|o| match o.via.as_str() {
        "subagent" => "via sub-agent".to_string(),
        "outside" => "outside Ligus".to_string(),
        _ if !o.label.is_empty() => format!("from {}", o.label),
        _ => "from another chat".to_string(),
    });
    let kind = match v.kind {
        "permission" => AskKind::Permission,
        "secret" => AskKind::Secret,
        _ => AskKind::Question,
    };
    Ask {
        id: AskId::new(v.ask_id),
        chat: ChatId::new(v.chat_id),
        turn,
        agent: v.agent_name,
        kind,
        v2: v.v >= 2,
        question: v.question,
        options: v.options,
        allow_text: v.allow_text,
        audience,
        quorum: v.quorum,
        tally: v.tally,
        can_answer,
        effect: v.effect.map(|e| AskEffect {
            tool: e.tool,
            title: e.title,
            detail: e.detail,
            cwd: e.cwd,
            host: e.host_label,
            presence_required: e.presence_required,
            standing: e.standing,
            too_large: e.too_large,
        }),
        origin,
        status,
        at: v.at_ms,
        expires_at: v.expires_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ligus_core::flow::{ASK_V2, AskAudience, AskState, AskVote, HumanAnswered, HumanAsk};
    use ligus_core::{ActorIdentity, ActorKind, WorkspaceRole};

    fn session() -> ChatSession {
        let mut s = ChatSession::new("t", Uuid::nil(), "rt");
        for (id, name, role) in
            [("me", "Mara", WorkspaceRole::Owner), ("ada", "Ada", WorkspaceRole::Admin), ("lin", "Lin", WorkspaceRole::Contributor)]
        {
            s.members.push(ligus_core::identity::WorkspaceMember {
                id: id.into(),
                actor: ActorIdentity::new(id, name, ActorKind::Human),
                role,
                title: String::new(),
                index: 0,
                joined_at: Default::default(),
            });
        }
        s
    }

    fn st(audience: AskAudience, quorum: Option<u8>) -> AskState {
        AskState {
            ask: HumanAsk {
                ask_id: Uuid::from_u128(1),
                turn_id: Uuid::from_u128(2),
                question: "Ship it?".into(),
                options: vec!["Ship".into(), "Hold".into()],
                v: ASK_V2,
                audience: Some(audience),
                quorum,
                expires_at_ms: 10_000,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn of(s: &ChatSession, a: &AskState, me: &str, now: Millis) -> Ask {
        card(ligus_runtime::ask_view::view(s, a, me, now, &|_| None), None, me)
    }

    /// Who a card is put to and who it waits on, as the header and the
    /// read-only line say it.
    #[test]
    fn audience_and_waiting_on() {
        let s = session();
        let mine = st(AskAudience::people(vec!["me".into()]), None);
        let c = of(&s, &mine, "me", 0);
        assert_eq!((c.audience.as_str(), c.can_answer.clone()), ("you", Allowed::Yes));
        assert_eq!(c.status, AskStatus::Open);
        let c = of(&s, &mine, "lin", 0);
        assert_eq!(c.can_answer, Allowed::No("Waiting on Mara".into()));
        let mut admins = st(AskAudience::room(WorkspaceRole::Admin), Some(2));
        admins.counted = vec![AskVote { member_id: "ada".into(), option_index: Some(0), ..Default::default() }];
        let c = of(&s, &admins, "lin", 0);
        assert_eq!(c.audience, "Admins");
        assert_eq!(c.can_answer, Allowed::No("Waiting on Admins · 1 of 2".into()));
        // Past its expiry it stopped waiting; settled, it names who answered.
        assert_eq!(of(&s, &mine, "me", 20_000).status, AskStatus::Stopped);
        let mut done = st(AskAudience::people(vec!["ada".into()]), None);
        done.answer = Some(HumanAnswered { option_index: Some(1), answer: "Hold".into(), ..Default::default() });
        done.answered_by = vec![AskVote { member_id: "ada".into(), name: "Not Ada".into(), at_ms: 5, ..Default::default() }];
        match of(&s, &done, "me", 0).status {
            AskStatus::Answered { by, answer, .. } => assert_eq!((by.as_str(), answer.as_str()), ("Ada", "Hold"), "roster names"),
            other => panic!("{other:?}"),
        }
    }
}
