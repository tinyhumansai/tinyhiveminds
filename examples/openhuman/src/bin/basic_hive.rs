//! Two host-created OpenHuman agents join a hive and keep their conversations after leaving.
//!
//! Run from the repository root with:
//! `cargo run --manifest-path examples/openhuman/Cargo.toml --bin basic_hive`.
//! Pass `--live` and `OPENROUTER_API_KEY` to use a real OpenRouter model.

use anyhow::{Context, Result, ensure};
use openhuman_embed::Provider;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tinyhivemind_hives::{Destination, MemoryStorage, SendMessage, Storage};
use tinyhivemind_openhuman::deploy::BuildOptions;
use tinyhivemind_openhuman_example::{deploy, manifest};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// A local model stub that calls the real attached completion tool for hive work.
#[derive(Clone, Default)]
struct ScriptedModel {
    next_call: Arc<AtomicUsize>,
}

enum Mode {
    Offline,
    Live { api_key: String, model: String },
}

impl Respond for ScriptedModel {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let last = body["messages"].as_array().and_then(|rows| rows.last());
        let assignment = last
            .filter(|row| row["role"] == "user")
            .and_then(|row| row["content"].as_str())
            .and_then(|text| text.find('{').map(|start| &text[start..]))
            .and_then(|json| serde_json::from_str::<Value>(json).ok());
        let episode_id = assignment
            .as_ref()
            .and_then(|turn| turn["episode"]["episode_id"].as_str());
        let message = if let Some(episode_id) = episode_id {
            let agent_id = assignment
                .as_ref()
                .and_then(|turn| turn["agent_id"].as_str());
            let call_id = self.next_call.fetch_add(1, Ordering::Relaxed);
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": format!("complete-{call_id}"),
                    "type": "function",
                    "function": {
                        "name": "hivemind_complete",
                        "arguments": json!({
                            "episode_id": episode_id,
                            "body": format!("{} finished the hive task", agent_id.unwrap_or("agent")),
                        }).to_string(),
                    },
                }],
            })
        } else {
            json!({"role": "assistant", "content": "Offline OpenHuman agent reply."})
        };
        let finish_reason = if episode_id.is_some() {
            "tool_calls"
        } else {
            "stop"
        };
        ResponseTemplate::new(200).set_body_json(json!({
            "id": "basic-hive-completion",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "fixture",
            "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12},
        }))
    }
}

fn main() -> Result<()> {
    let mode = match std::env::args().nth(1).as_deref() {
        None => Mode::Offline,
        Some("--live") => Mode::Live {
            api_key: std::env::var("OPENROUTER_API_KEY")
                .context("OPENROUTER_API_KEY is required for --live")?,
            model: std::env::var("OPENROUTER_MODEL")
                .unwrap_or_else(|_| "openai/gpt-oss-120b:nitro".into()),
        },
        Some(other) => anyhow::bail!("unknown argument: {other}; use --live or no argument"),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()?;
    runtime.block_on(async { tokio::spawn(run(mode)).await })??;
    Ok(())
}

async fn run(mode: Mode) -> Result<()> {
    let backend = tinyhivemind_openhuman::offline::backend().await;
    let (provider, route) = match mode {
        Mode::Offline => {
            let provider = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ScriptedModel::default())
                .mount(&provider)
                .await;
            let route = Provider::openai_compatible(format!("{}/v1", provider.uri()), "fixture")
                .model("fixture");
            (Some(provider), route)
        }
        Mode::Live { api_key, model } => {
            println!("Using OpenRouter model {model}.");
            let route =
                Provider::openai_compatible("https://openrouter.ai/api/v1", api_key).model(model);
            (None, route)
        }
    };
    let storage = Arc::new(MemoryStorage::new());
    let deployment = deploy(
        manifest("basic_hive")?,
        BuildOptions {
            config: Some(tinyhivemind_openhuman::offline::config()),
            backend_url: Some(backend.uri()),
            provider: Some(route),
            coordinator_storage: Some(storage.clone()),
            ..Default::default()
        },
    )
    .await?;
    let alice = deployment.seats()["alice"].clone();
    let bob = deployment.seats()["bob"].clone();
    let alice_session = alice
        .turn("ALICE_BEFORE_HIVE: plan a release")
        .session("alice-session")
        .send()
        .await?
        .session_id;
    let bob_session = bob
        .turn("BOB_BEFORE_HIVE: review a release")
        .session("bob-session")
        .send()
        .await?
        .session_id;
    println!("Before the hive: Alice and Bob each have a host conversation.");

    let coordinator = deployment.host().coordinator();
    coordinator.bind_session("alice", &alice_session).await?;
    coordinator.bind_session("bob", &bob_session).await?;
    println!("In the hive: {:?}", coordinator.list_hives()?[0].members);

    // Each private task reaches its named member through the real adapter.
    for (agent_id, task) in [
        ("alice", "Plan the rollout"),
        ("bob", "Review the rollback"),
    ] {
        let receipt = coordinator
            .send_as_host(SendMessage {
                message_id: format!("task-{agent_id}"),
                sender: String::new(),
                destination: Destination::Hive("release".into()),
                body: task.into(),
                thread: None,
                only_for: vec![agent_id.into()],
                starters: Vec::new(),
            })
            .await?;
        let report = coordinator.run_until_idle().await?;
        ensure!(
            report.completed > 0 && report.failed == 0,
            "hive turn failed: {report:?}"
        );
        ensure!(
            storage.load().await?.episodes.iter().any(|episode| {
                episode.opened_at == receipt.sequence
                    && episode.finished
                    && episode.failure.is_none()
            }),
            "{agent_id}'s hive episode did not finish successfully"
        );
        println!("{agent_id} completed a hive turn.");
    }
    ensure!(
        coordinator
            .read_hive("alice", "release", None, None)?
            .iter()
            .any(|message| message.body == "Plan the rollout"),
        "Alice's task is missing"
    );
    ensure!(
        !coordinator
            .read_hive("bob", "release", None, None)?
            .iter()
            .any(|message| message.body == "Plan the rollout"),
        "Alice's private task leaked to Bob"
    );

    coordinator.leave_hive("release", "bob").await?;
    ensure!(
        coordinator.read_hive("bob", "release", None, None).is_err(),
        "Bob retained hive access after leaving"
    );
    bob.turn("BOB_AFTER_HIVE: continue the host conversation")
        .session(&bob_session)
        .send()
        .await?;
    println!("Outside the hive: Bob left and continued his original session.");

    // The captured provider request proves the ordinary conversation continued.
    let Some(provider) = provider else {
        println!(
            "Live OpenRouter turns completed; Bob's host session remained usable after leaving."
        );
        return Ok(());
    };
    let requests = provider
        .received_requests()
        .await
        .context("provider captures unavailable")?;
    let continued = requests
        .iter()
        .filter_map(|request| serde_json::from_slice::<Value>(&request.body).ok())
        .find(|body| {
            body["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|row| {
                    row["role"] == "user"
                        && row["content"]
                            .as_str()
                            .is_some_and(|text| text.contains("BOB_AFTER_HIVE"))
                })
            })
        })
        .context("no post-hive provider request")?;
    let history = continued["messages"]
        .as_array()
        .context("no request messages")?;
    ensure!(
        history.iter().any(|row| {
            row["role"] == "user"
                && row["content"]
                    .as_str()
                    .is_some_and(|text| text.contains("BOB_BEFORE_HIVE"))
        }),
        "Bob's original host conversation was lost"
    );
    ensure!(
        history.iter().any(|row| {
            row["role"] == "user"
                && row["content"]
                    .as_str()
                    .is_some_and(|text| text.contains("Incoming attributed Hivemind context"))
        }),
        "Bob's hive turn was not in the continuing conversation"
    );
    ensure!(
        continued["tools"].as_array().is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool["function"]["name"] == "hivemind_list_hives")
        }),
        "the host turn lost Hivemind's permanent tools"
    );
    println!("Verified Bob's host, hive, and post-hive turns share one OpenHuman session.");
    Ok(())
}
