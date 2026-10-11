//! Real native host-tool calls obey captured membership and approval boundaries.
use super::support::*;
use crate::{config::*, deploy::*};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicUsize, Ordering},
};
use tinyhivemind_hives::{Destination, SendMessage};
use wiremock::{
    Mock, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};
#[derive(Clone, Default)]
pub(super) struct Script {
    pub calls: Arc<AtomicUsize>,
    pub seen: Arc<Mutex<Vec<String>>>,
    pub action: Option<String>,
    pub arguments: Option<Value>,
    pub child: bool,
    pub parents: Arc<Mutex<BTreeSet<String>>>,
}
impl Respond for Script {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let last = body["messages"].as_array().and_then(|rows| rows.last());
        let text = last
            .filter(|row| row["role"] == "user")
            .and_then(|row| row["content"].as_str());
        let turn = text
            .and_then(|text| text.find('{').map(|start| &text[start..]))
            .and_then(|text| serde_json::from_str::<Value>(text).ok());
        let episode = turn
            .as_ref()
            .and_then(|turn| turn["episode"]["episode_id"].as_str());
        let mut calls = Vec::new();
        if let Some(text) = text {
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(text.into());
        }
        if let Some(episode) = episode {
            if turn
                .as_ref()
                .is_some_and(|turn| turn["resumption"].is_null())
            {
                if let Some(name) = &self.action
                    && !(self.child
                        && turn
                            .as_ref()
                            .is_some_and(|turn| turn["agent_id"] == "alice"))
                {
                    calls.push((
                        name.clone(),
                        self.arguments.clone().unwrap_or_else(|| json!({})),
                    ));
                }
                if self.child
                    && turn
                        .as_ref()
                        .is_some_and(|turn| turn["agent_id"] == "alice")
                    && self
                        .parents
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(episode.into())
                {
                    calls.push((
                        "hivemind_ask".into(),
                        json!({"episode_id":episode,"agents":["bob"],"body":"private child work"}),
                    ));
                }
            }
            calls.push((
                "hivemind_complete".into(),
                json!({"episode_id":episode,"body":"done"}),
            ));
        } else if text.is_some()
            && let Some(name) = &self.action
        {
            calls.push((
                name.clone(),
                self.arguments.clone().unwrap_or_else(|| json!({})),
            ));
        }
        let tools: Vec<_> = calls.into_iter().map(|(name,args)| json!({"id":format!("call-{}",self.calls.fetch_add(1,Ordering::Relaxed)),"type":"function","function":{"name":name,"arguments":args.to_string()}})).collect();
        let message = if tools.is_empty() {
            json!({"role":"assistant","content":"done"})
        } else {
            json!({"role":"assistant","content":null,"tool_calls":tools})
        };
        ResponseTemplate::new(200).set_body_json(json!({"id":"fixture","object":"chat.completion","created":0,"model":"fixture","choices":[{"index":0,"message":message,"finish_reason":if episode.is_some() || text.is_some() && self.action.is_some() { "tool_calls" } else { "stop" }}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))
    }
}
pub(super) async fn mount(server: &wiremock::MockServer, script: Script) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(script)
        .mount(server)
        .await;
}
pub(super) struct WriteTool(pub Arc<AtomicUsize>);
#[async_trait::async_trait]
impl tinytools::Tool for WriteTool {
    fn name(&self) -> &'static str {
        "write"
    }
    fn description(&self) -> &'static str {
        "A classified write fixture"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    async fn execute(&self, _: Value) -> anyhow::Result<tinytools::ToolResult> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(tinytools::ToolResult::success("written"))
    }
    fn policy(&self) -> tinytools::ToolPolicy {
        tinytools::ToolPolicy::classified().with_side_effects(tinytools::ToolSideEffects {
            writes_files: true,
            ..Default::default()
        })
    }
}
pub(super) fn source(count: Arc<AtomicUsize>) -> openhuman_embed::HostTools {
    Arc::new(move |_| {
        openhuman_embed::HostTurnTools::advertised(vec![Box::new(WriteTool(count.clone()))])
    })
}
pub(super) fn message(id: &str, hive: &str) -> SendMessage {
    SendMessage {
        message_id: id.into(),
        sender: String::new(),
        destination: Destination::Hive(hive.into()),
        body: "work".into(),
        thread: None,
        only_for: Vec::new(),
        starters: vec!["alice".into()],
    }
}
#[test]
fn configured_dollar_budget_refuses_reservation_before_provider_dispatch() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (mut options, _backend, model) = build_options().await;
        let script = Script::default();
        mount(&model, script.clone()).await;
        options.call_budget = Some(openhuman_embed::budget::CallBudget {
            input_tokens: 100_000,
            output_tokens: 20,
            cost_micros: 2_000_000,
        });
        let mut config = manifest()?;
        config.profiles[0].limits.budget_usd = Some(1.0);
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        deployment
            .host()
            .coordinator()
            .send_as_host(message("budget", "a"))
            .await?;
        let report = deployment.host().coordinator().run_until_idle().await?;
        assert_eq!(report.failed, 1, "{report:?}");
        assert!(
            model
                .received_requests()
                .await
                .ok_or_else(|| anyhow::anyhow!("request recording disabled"))?
                .iter()
                .all(|request| request.method.as_str() != "POST"
                    || !request.url.path().ends_with("/chat/completions")),
            "provider received an unreserved inference request"
        );
        Ok(())
    })
}
#[test]
fn one_continuing_seat_uses_two_roles_and_tool_restrictions() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (mut options, _backend, model) = build_options().await;
        let script = Script {
            action: Some("write".into()),
            ..Default::default()
        };
        mount(&model, script.clone()).await;
        let count = Arc::new(AtomicUsize::new(0));
        options.tools = Some(source(count.clone()));
        let mut config = manifest()?;
        config
            .contexts
            .insert("note".into(), "Only hive B receives this context".into());
        config.hives[1].members[0].context.push("note".into());
        let storage = Arc::new(tinyhivemind_hives::MemoryStorage::new());
        options.coordinator_storage = Some(storage.clone());
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        for hive in ["a", "b"] {
            deployment
                .host()
                .coordinator()
                .send_as_host(message(hive, hive))
                .await?;
            let report = deployment.host().coordinator().run_until_idle().await?;
            assert_eq!(report.failed, usize::from(hive == "b"), "{report:?}");
            if hive == "b" {
                deployment
                    .host()
                    .coordinator()
                    .release_with(
                        "alice",
                        Some(
                            "The write was refused. Finish the hive safely without replaying it."
                                .into(),
                        ),
                    )
                    .await?;
                deployment
                    .host()
                    .coordinator()
                    .send_as_host(message("b-safe", "b"))
                    .await?;
                let resumed = deployment.host().coordinator().run_until_idle().await?;
                assert_eq!(resumed.failed, 0, "{resumed:?}");
                assert!(resumed.completed > 0);
            }
        }
        assert_eq!(count.load(Ordering::Relaxed), 1);
        let seen = script
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert!(
            seen.iter()
                .any(|text| text.contains("@alice: lead") && !text.contains("Only hive B"))
        );
        assert!(
            seen.iter()
                .any(|text| text.contains("@alice: observer") && text.contains("Only hive B"))
        );
        let state = tinyhivemind_hives::Storage::load(storage.as_ref()).await?;
        assert!(state.episodes.iter().all(|episode| episode.finished));
        for hive in ["a", "b"] {
            let last = state
                .episodes
                .iter()
                .rev()
                .find(|episode| episode.hive.hive_id == hive)
                .ok_or_else(|| anyhow::anyhow!("missing episode"))?;
            assert!(last.failure.is_none(), "safe completion must succeed");
        }
        assert!(
            state
                .agents
                .get("alice")
                .and_then(|agent| agent.session_id.as_ref())
                .is_some()
        );
        Ok::<_, anyhow::Error>(())
    })
}
struct AllowHandler;
struct CleanupGate {
    entered: Arc<tokio::sync::Notify>,
    finish: Arc<tokio::sync::Notify>,
}
impl crate::TurnHooks for CleanupGate {
    fn wrap_turn<'a>(
        &'a self,
        _: &'a crate::TurnScope,
        turn: crate::HostedTurn<'a>,
    ) -> crate::HostedTurn<'a> {
        Box::pin(async move {
            let result = turn.await;
            self.entered.notify_one();
            self.finish.notified().await;
            result
        })
    }
}
#[async_trait::async_trait]
impl openhuman_embed::ApprovalHandler for AllowHandler {
    async fn decide(
        &self,
        _: &openhuman_embed::PendingApproval,
    ) -> openhuman_embed::ApprovalDecision {
        openhuman_embed::ApprovalDecision::ApproveOnce
    }
}
#[test]
fn ask_parks_then_explicit_release_continues_without_replaying_effect() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (mut options, _backend, model) = build_options().await;
        mount(
            &model,
            Script {
                action: Some("write".into()),
                ..Default::default()
            },
        )
        .await;
        let count = Arc::new(AtomicUsize::new(0));
        options.tools = Some(source(count.clone()));
        options.approval_handler = Some(Arc::new(AllowHandler));
        let entered = Arc::new(tokio::sync::Notify::new());
        let finish = Arc::new(tokio::sync::Notify::new());
        options.hooks = Some(Arc::new(CleanupGate {
            entered: entered.clone(),
            finish: finish.clone(),
        }));
        options
            .approval
            .people
            .push(tinyhivemind_core::roster::Person {
                id: "human".into(),
                label: "Human".into(),
            });
        let mut config = manifest()?;
        let policy: tinyhivemind_core::approval::ApprovalPolicy = serde_json::from_value(
            json!({"enabled":true,"default":"deny","rules":[{"effect":"read_only","verb":null,"target":null,"verdict":"allow"},{"effect":"mutating","verb":null,"target":null,"verdict":"ask"}],"approver":{"kind":"person","id":"human"},"allow_grants":false,"max_grant_ttl":null}),
        )?;
        config.permission_profiles.push(PermissionProfile {
            id: "ask".into(),
            access: Access::Autonomous,
            approval: Some(policy),
            ..Default::default()
        });
        config.runtime.permission_profile = Some("ask".into());
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        deployment
            .host()
            .coordinator()
            .send_as_host(message("question", "a"))
            .await?;
        let coordinator = deployment.host().coordinator().clone();
        let drain = tokio::spawn(async move { coordinator.run_until_idle().await });
        entered.notified().await;
        assert_eq!(count.load(Ordering::Relaxed), 0);
        let request = deployment
            .pending()
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing approval"))?;
        let answer = deployment.wait_decision(&request.request.request_id).await;
        assert_eq!(answer, Some(openhuman_embed::ApprovalDecision::ApproveOnce));
        assert!(
            deployment
                .release(&request.request.request_id)
                .await
                .is_err(),
            "running turn was released before finalization"
        );
        assert_eq!(deployment.pending().len(), 1);
        finish.notify_one();
        let report = drain.await??;
        assert_eq!(report.parked, 1, "{report:?}");
        deployment.release(&request.request.request_id).await?;
        let coordinator = deployment.host().coordinator().clone();
        let drain = tokio::spawn(async move { coordinator.run_until_idle().await });
        entered.notified().await;
        finish.notify_one();
        let report = drain.await??;
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(count.load(Ordering::Relaxed), 0);
        assert!(deployment.pending().is_empty());
        assert!(deployment.release("unknown").await.is_err());
        Ok::<_, anyhow::Error>(())
    })
}
