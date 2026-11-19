//! An agent turn in `lt` through the shared turn engine: a stub model calls
//! Ligus's built-in tools, the edit lands in a per-turn worktree of the
//! checkout bound on this device (never the checkout itself), and the diff
//! comes back as a proposal, with the tool calls shown live. One `lt` device
//! (temporary data dir, file secrets) syncing through the in-process test
//! relay; nothing touches the login keychain, a real relay or a real model.

mod support;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::post;
use ligus_core::{ModelProviderKind, WorkspaceAgent, WorkspaceRuntime, workspace_config_session_id};
use ligus_term::backend::model::*;
use ligus_term::backend::{Backend, Event};
use serde_json::{Value, json};
use support::*;
use uuid::Uuid;

/// An OpenAI-compatible model that answers each request with the next
/// scripted round (the last one repeats), and keeps what it was sent.
async fn scripted_model(rounds: Vec<Vec<Value>>) -> (String, Arc<Mutex<Vec<Value>>>) {
    let bodies: Arc<Mutex<Vec<Value>>> = Arc::default();
    let (b, rounds) = (bodies.clone(), Arc::new(rounds));
    let router = Router::new().route(
        "/v1/chat/completions",
        post(move |body: String| {
            let (b, rounds) = (b.clone(), rounds.clone());
            async move {
                let n = {
                    let mut b = b.lock().unwrap();
                    b.push(serde_json::from_str::<Value>(&body).unwrap_or(Value::Null));
                    b.len() - 1
                };
                let deltas = rounds.get(n).or_else(|| rounds.last()).cloned().unwrap_or_default();
                let mut out: String =
                    deltas.iter().map(|d| format!("data: {}\n\n", json!({"choices": [{"index": 0, "delta": d}]}))).collect();
                out.push_str("data: [DONE]\n\n");
                axum::response::Response::builder().header("content-type", "text/event-stream").body(axum::body::Body::from(out)).unwrap()
            }
        }),
    );
    (format!("{}/v1/chat/completions", serve(router).await), bodies)
}

fn tool_call(id: &str, name: &str, args: Value) -> Vec<Value> {
    vec![json!({"tool_calls": [{"index": 0, "id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}]})]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_turn_uses_tools_edits_in_its_worktree_and_files_a_proposal() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve(ligus_runtime::test_relay::router()).await;
    let (model, bodies) = scripted_model(vec![
        tool_call("c1", "read_file", json!({"path": "README.md"})),
        tool_call("c2", "write_file", json!({"path": "greeting.txt", "content": "hello from the hive\n"})),
        vec![json!({"content": "Added greeting.txt."})],
    ])
    .await;
    let a = device(&tmp.path().join("ana"), "lt Ana", &relay);
    let created = a.b.create_workspace("Ana", "Bee Yard").await.unwrap();
    let ws = created.workspace.id.clone();
    let wsu = Uuid::parse_str(ws.as_str()).unwrap();

    // A workspace runtime on the stub model, and an agent Ana owns.
    let svc = a.b.service().unwrap();
    tokio::task::spawn_blocking(move || {
        let mut svc = svc.lock().unwrap();
        let mut rt = WorkspaceRuntime::new("gw", "Lab gateway", ModelProviderKind::Custom, "test-model");
        rt.endpoint = model;
        svc.add_workspace_runtime(wsu, rt).unwrap();
        let mut agent = WorkspaceAgent::new("coder", "gw");
        agent.owner_actor_id = svc.author().id.clone();
        svc.add_agent(workspace_config_session_id(wsu), wsu, agent).unwrap();
    })
    .await
    .unwrap();

    // The checkout this device works in.
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "# Bee Yard\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["-c", "user.email=t@example.com", "-c", "user.name=t", "commit", "-qm", "init"]);
    a.b.bind_checkout(&ws, &repo).unwrap();

    let chat = a.b.create_chat(&ws, "Greeting").await.unwrap();
    a.b.send_message(&ws, &chat.id, "@coder add a greeting file").await.unwrap();
    let reply = until("the agent's reply", || async {
        a.b.transcript(&ws, &chat.id)
            .await
            .unwrap()
            .into_iter()
            .find(|m| matches!(&m.author, Author::Agent { handle, .. } if handle == "coder") && !m.streaming)
    })
    .await;
    assert_eq!(reply.text, "Added greeting.txt.");

    // The model was offered the tools, and each result went back to it.
    {
        let b = bodies.lock().unwrap();
        assert_eq!(b.len(), 3, "two tool rounds, then the answer");
        let tools: Vec<&str> = b[0]["tools"].as_array().unwrap().iter().filter_map(|t| t["function"]["name"].as_str()).collect();
        for t in ["read_file", "write_file", "ligus_propose"] {
            assert!(tools.contains(&t), "{t} offered: {tools:?}");
        }
        assert!(!tools.contains(&"run_command"), "no shell unless the setting and the Owner's rule allow it");
        assert!(b[1].to_string().contains("# Bee Yard"), "the file's contents reached the model");
    }

    // The tool calls were shown live.
    let calls: Vec<ToolCall> = a
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            Event::ToolActivity { call, .. } => Some(call.clone()),
            _ => None,
        })
        .collect();
    assert!(calls.iter().any(|c| c.name == "read_file" && c.ok && c.preview.contains("README.md")), "{calls:?}");
    assert!(calls.iter().any(|c| c.name == "write_file" && c.ok), "{calls:?}");

    // The edit is a proposal, not a change to the checkout.
    assert!(!repo.join("greeting.txt").exists(), "never written in place");
    let proposal = until("the turn's proposal", || async {
        a.b.proposals(&ws).await.unwrap().into_iter().find(|p| p.title.contains("proposed changes to 1 file"))
    })
    .await;
    let diff = a.b.proposal_diff(&ws, &proposal.id).await.unwrap();
    let text = format!("{diff:?}");
    assert!(text.contains("greeting.txt") && text.contains("hello from the hive"), "{text}");
}
