//! Two hives share one continuing seat and narrow its authority independently.
//! Offline by default; `--live` resolves `OPENROUTER_API_KEY` only at deployment.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tinyhivemind_hives::{Destination, MemoryStorage, SendMessage, Storage};
use tinyhivemind_openhuman::deploy::BuildOptions;
use tinyhivemind_openhuman_example::{deploy, manifest};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

// This host-owned tool writes only the proof file, and declares its native effect.
struct ProofWrite(std::path::PathBuf);
#[async_trait::async_trait]
impl tinytools::Tool for ProofWrite {
    fn name(&self) -> &'static str {
        "file_write"
    }
    fn description(&self) -> &'static str {
        "Write content to the host's isolated proof.txt"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string","enum":["proof.txt"]},"content":{"type":"string"}},"required":["path","content"]})
    }
    async fn execute(&self, arguments: Value) -> Result<tinytools::ToolResult> {
        ensure!(
            arguments["path"] == "proof.txt",
            "only proof.txt is writable"
        );
        let content = arguments["content"]
            .as_str()
            .context("content is required")?;
        std::fs::write(&self.0, content)?;
        Ok(tinytools::ToolResult::success("written"))
    }
    fn policy(&self) -> tinytools::ToolPolicy {
        tinytools::ToolPolicy::classified().with_side_effects(tinytools::ToolSideEffects {
            writes_files: true,
            ..Default::default()
        })
    }
}

#[derive(Clone, Default)]
struct Model {
    calls: Arc<AtomicUsize>,
    overlays: Arc<Mutex<Vec<String>>>,
}
impl Respond for Model {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let text = body["messages"]
            .as_array()
            .and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "user")
            })
            .and_then(|message| message["content"].as_str());
        let turn = text
            .and_then(|text| text.find('{').map(|start| &text[start..]))
            .and_then(|text| serde_json::from_str::<Value>(text).ok());
        let mut calls = Vec::new();
        if let (Some(text), Some(turn)) = (text, turn) {
            self.overlays.lock().unwrap().push(text.into());
            if let Some(episode) = turn["episode"]["episode_id"].as_str() {
                let completed = body["messages"]
                    .as_array()
                    .and_then(|messages| messages.last())
                    .is_some_and(|message| {
                        message["role"] == "tool" && message["name"] == "hivemind_complete"
                    });
                let initial = body["messages"]
                    .as_array()
                    .and_then(|messages| messages.last())
                    .is_some_and(|message| message["role"] == "user");
                if initial && turn["resumption"].is_null() {
                    let value = if turn["episode"]["hive_id"] == "a" {
                        "allowed in A"
                    } else {
                        "forbidden in B"
                    };
                    calls.push(("file_write", json!({"path":"proof.txt", "content":value})));
                } else if !completed {
                    calls.push((
                        "hivemind_complete",
                        json!({"episode_id":episode, "body":"Finished safely."}),
                    ));
                }
            }
        }
        let calls: Vec<_> = calls.into_iter().map(|(name, arguments)| json!({"id":format!("proof-{}", self.calls.fetch_add(1, Ordering::Relaxed)), "type":"function", "function":{"name":name,"arguments":arguments.to_string()}})).collect();
        let called = !calls.is_empty();
        ResponseTemplate::new(200).set_body_json(json!({"id":"multi", "object":"chat.completion", "created":0, "model":"fixture", "choices":[{"index":0,"message":if called { json!({"role":"assistant","content":null,"tool_calls":calls}) } else { json!({"role":"assistant","content":"done"}) }, "finish_reason":if called { "tool_calls" } else { "stop" }}], "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))
    }
}
fn message(id: &str, hive: &str, body: &str) -> SendMessage {
    SendMessage {
        message_id: id.into(),
        sender: String::new(),
        destination: Destination::Hive(hive.into()),
        body: body.into(),
        thread: None,
        only_for: vec!["shared".into()],
        starters: Vec::new(),
    }
}
fn main() -> Result<()> {
    let live = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--live") => true,
        Some(value) => anyhow::bail!("unknown argument {value}; use --live or no argument"),
    };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()?
        .block_on(async move { tokio::spawn(run(live)).await })??;
    Ok(())
}
async fn run(live: bool) -> Result<()> {
    let backend = tinyhivemind_openhuman::offline::backend().await;
    let provider = MockServer::start().await;
    let script = Model::default();
    if !live {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(script.clone())
            .mount(&provider)
            .await;
    }
    let mut config = manifest("multi_hive")?;
    if !live {
        config.runtime.provider = Default::default();
        config.runtime.model.name = Some("fixture".into());
    } else {
        config.runtime.model.name = Some(
            std::env::var("OPENROUTER_MODEL")
                .unwrap_or_else(|_| "openai/gpt-oss-120b:nitro".into()),
        );
    }
    let storage = Arc::new(MemoryStorage::new());
    let proof_dir = tempfile::tempdir()?;
    let proof = proof_dir.path().join("proof.txt");
    let tool_path = proof.clone();
    let tools: openhuman_embed::HostTools = Arc::new(move |_| {
        openhuman_embed::HostTurnTools::advertised(vec![Box::new(ProofWrite(tool_path.clone()))])
    });
    let deployment = deploy(
        config,
        BuildOptions {
            config: Some(tinyhivemind_openhuman::offline::config()),
            provider: (!live).then(|| {
                openhuman_embed::Provider::openai_compatible(
                    format!("{}/v1", provider.uri()),
                    "fixture",
                )
                .model("fixture")
            }),
            backend_url: Some(backend.uri()),
            coordinator_storage: Some(storage.clone()),
            tools: Some(tools),
            ..Default::default()
        },
    )
    .await?;
    let coordinator = deployment.host().coordinator();
    ensure!(
        deployment.seats().len() == 4,
        "expected four persistent seats"
    );

    coordinator
        .send_as_host(message(
            "write-a",
            "a",
            "Use file_write to write proof.txt with content allowed in A, then hivemind_complete.",
        ))
        .await?;
    let a = coordinator.run_until_idle().await?;
    ensure!(a.failed == 0 && a.completed > 0, "hive A failed: {a:?}");
    ensure!(
        std::fs::read_to_string(&proof)? == "allowed in A",
        "hive A did not perform the effect"
    );
    let session = storage.load().await?.agents["shared"]
        .session_id
        .clone()
        .context("shared session missing")?;
    coordinator.send_as_host(message("write-b", "b", "Attempt file_write to overwrite proof.txt with forbidden in B, then hivemind_complete.")).await?;
    let b = coordinator.run_until_idle().await?;
    ensure!(b.failed == 1, "hive B must audit its refused write: {b:?}");
    ensure!(
        std::fs::read_to_string(&proof)? == "allowed in A",
        "hive B executed a refused effect"
    );
    coordinator.release_with("shared", Some("The write was refused. Complete the new assignment safely, without replaying any write.".into())).await?;
    coordinator
        .send_as_host(message(
            "safe-b",
            "b",
            "Finish safely with hivemind_complete; do not write.",
        ))
        .await?;
    let resumed = coordinator.run_until_idle().await?;
    ensure!(
        resumed.failed == 0 && resumed.completed > 0,
        "safe completion failed: {resumed:?}"
    );
    let state = storage.load().await?;
    ensure!(
        state.episodes.iter().all(|episode| episode.finished),
        "unfinished work"
    );
    for hive in ["a", "b"] {
        ensure!(
            state
                .episodes
                .iter()
                .rev()
                .find(|episode| episode.hive.hive_id == hive)
                .is_some_and(|episode| episode.failure.is_none()),
            "{hive} has no successful completion"
        );
    }
    ensure!(
        state.agents["shared"].session_id.as_deref() == Some(&session),
        "shared seat changed sessions"
    );
    ensure!(
        std::fs::read_to_string(&proof)? == "allowed in A",
        "safe completion replayed the write"
    );
    if !live {
        let overlays = script.overlays.lock().unwrap();
        ensure!(
            overlays
                .iter()
                .any(|text| text.contains("@shared: implementer")),
            "hive A role missing"
        );
        ensure!(
            overlays
                .iter()
                .any(|text| text.contains("@shared: observer")),
            "hive B role missing"
        );
    }
    println!(
        "One runtime, three profiles, four seats, two hives; shared seat kept session {session}."
    );
    println!(
        "A wrote proof.txt; B refused the same tool, retained its failed-turn audit, and completed a fresh safe assignment after explicit host release."
    );
    Ok(())
}
