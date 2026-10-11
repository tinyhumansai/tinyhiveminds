//! Hooks see the turn's scope; `prepare` and the turn timeout shape the turn.
use super::*;
use std::time::Duration;
use tinyhivemind_hives::{Destination, HOST_ID, SendMessage};

/// Records every scope each hook receives; `prepare` asks for a directory.
#[derive(Default)]
struct Scoped {
    seen: Mutex<Vec<(&'static str, TurnScope)>>,
    cwd: Option<std::path::PathBuf>,
}
impl Scoped {
    fn record(&self, hook: &'static str, scope: &TurnScope) {
        self.seen.lock().unwrap().push((hook, scope.clone()));
    }
}
impl TurnHooks for Scoped {
    fn prepare(&self, scope: &TurnScope) -> TurnOptions {
        self.record("prepare", scope);
        TurnOptions {
            cwd: self.cwd.clone(),
        }
    }
    fn progress(&self, scope: &TurnScope) -> Option<TurnProgressSink> {
        self.record("progress", scope);
        None
    }
    fn wrap_turn<'a>(&'a self, scope: &'a TurnScope, turn: HostedTurn<'a>) -> HostedTurn<'a> {
        self.record("wrap_turn", scope);
        turn
    }
    fn after_turn(
        &self,
        scope: &TurnScope,
        _: Option<&openhuman_core::agent::tinyagents::host::LastTurnUsage>,
    ) -> Result<tinyhivemind_hives::TurnDisposition> {
        self.record("after_turn", scope);
        Ok(tinyhivemind_hives::TurnDisposition::Completed)
    }
}
async fn provider(delay: Duration) -> wiremock::MockServer {
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_delay(delay)
                .set_body_json(serde_json::json!({
                    "id":"fixture","object":"chat.completion","created":0,"model":"fixture",
                    "choices":[{"index":0,"message":{"role":"assistant","content":"SCOPED"},"finish_reason":"stop"}],
                    "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
                })),
        )
        .mount(&provider)
        .await;
    provider
}
fn spec(id: &str, provider: &wiremock::MockServer) -> AgentSpec {
    AgentSpec::new(id).provider(
        openhuman_embed::Provider::openai_compatible(format!("{}/v1", provider.uri()), "fixture")
            .model("fixture"),
    )
}
#[test]
fn every_hook_receives_the_turn_scope() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {
        tokio::spawn(async {
            let (runtime, _backend, host) = Box::pin(fixture()).await;
            let provider = provider(Duration::ZERO).await;
            let directory = tempfile::tempdir().unwrap();
            let hooks = Arc::new(Scoped {
                cwd: Some(directory.path().to_owned()),
                ..Scoped::default()
            });
            let host = host.with_hooks(hooks.clone()).unwrap();
            host.register_agent(runtime.agent(spec("scoped", &provider)).unwrap())
                .await
                .unwrap();
            host.coordinator()
                .send_as_host(SendMessage {
                    message_id: "ask".into(),
                    sender: String::new(),
                    destination: Destination::Agent("scoped".into()),
                    body: "hello".into(),
                    thread: None,
                    only_for: vec![],
                    starters: vec![],
                })
                .await
                .unwrap();
            assert_eq!(
                host.coordinator().run_until_idle().await.unwrap().completed,
                1
            );
            let seen = hooks.seen.lock().unwrap();
            let hooks_called: Vec<_> = seen.iter().map(|(hook, _)| *hook).collect();
            assert_eq!(
                hooks_called,
                ["prepare", "progress", "wrap_turn", "after_turn"]
            );
            for (_, scope) in seen.iter() {
                assert_eq!(scope.agent_id, "scoped");
                assert_eq!(scope.message_ids, ["ask"]);
                assert_eq!(scope.senders, [HOST_ID]);
                assert_eq!(scope.destination, Destination::Agent("scoped".into()));
                assert_eq!(scope.episode, None);
            }
        })
        .await
        .unwrap();
    });
}
#[test]
fn prepared_options_reach_the_turn_builder() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {
        tokio::spawn(async {
            let (runtime, _backend, _host) = Box::pin(fixture()).await;
            let agent = runtime.agent(AgentSpec::new("cwd")).unwrap();
            let directory = tempfile::tempdir().unwrap();
            let turn = runner::configure(
                agent.turn("x"),
                &TurnOptions {
                    cwd: Some(directory.path().to_owned()),
                },
            );
            assert_eq!(
                turn.request().cwd.as_deref(),
                Some(directory.path().to_string_lossy().as_ref())
            );
            let plain = runner::configure(agent.turn("x"), &TurnOptions::default());
            assert_eq!(plain.request().cwd, None);
        })
        .await
        .unwrap();
    });
}
#[test]
fn the_turn_timeout_is_configurable_and_nonzero() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {
        tokio::spawn(async {
            let (runtime, _backend, host) = Box::pin(fixture()).await;
            assert_eq!(host.turn_timeout(), TURN_TIMEOUT);
            assert!(matches!(
                host.clone().with_turn_timeout(Duration::ZERO),
                Err(Error::InvalidTurnTimeout)
            ));
            let host = host.with_turn_timeout(Duration::from_millis(20)).unwrap();
            assert_eq!(host.turn_timeout(), Duration::from_millis(20));
            let provider = provider(Duration::from_secs(60)).await;
            host.register_agent(runtime.agent(spec("slow", &provider)).unwrap())
                .await
                .unwrap();
            let runner = host.inner.agents.lock().unwrap()["slow"].runner.clone();
            let failed = tinyhivemind_hives::AgentRunner::run(
                runner.as_ref(),
                tinyhivemind_hives::TurnRequest {
                    turn_id: String::new(),
                    scheduled_job_id: None,
                    teammates: Vec::new(),
                    agent_id: "slow".into(),
                    session_id: None,
                    messages: vec![],
                    memberships: vec![],
                    episode: None,
                    resumption: None,
                },
            )
            .await;
            assert!(failed.unwrap_err().to_string().contains("timed out"));
        })
        .await
        .unwrap();
    });
}
