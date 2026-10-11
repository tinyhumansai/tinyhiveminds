//! Registration, settings, runtime and attachment ownership contracts.
// Test assertions deliberately panic on invalid fixture construction.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::RUNTIME_LOCK;
use crate::offline;
use openhuman_embed::{AgentSpec, Runtime, Workspace};
use tinyhivemind_hives::{CoordinatorOptions, MemoryStorage};
fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .unwrap()
}
async fn fixture() -> (Runtime, wiremock::MockServer, OpenHumanHost) {
    let backend = offline::backend().await;
    let runtime = Box::pin(
        Runtime::builder()
            .config(offline::config())
            .workspace(Workspace::Ephemeral)
            .backend_url(backend.uri())
            .build(),
    )
    .await
    .unwrap();
    let coordinator = Coordinator::new(
        runtime.runtime_id().into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    let host = OpenHumanHost::new(runtime.runtime_id().into(), coordinator).unwrap();
    (runtime, backend, host)
}
struct Factory;
impl AgentFactory for Factory {
    fn create(&self, _: String, _: serde_json::Value) -> AgentFuture {
        Box::pin(async { Err(Error::Unauthorized("factory failed".into())) })
    }
}
struct ConfiguredFactory(Agent);
impl AgentFactory for ConfiguredFactory {
    fn create(&self, template: String, _: serde_json::Value) -> AgentFuture {
        let agent = self.0.clone();
        Box::pin(async move {
            if template == "configured" {
                Ok(agent)
            } else {
                Err(Error::Unauthorized("factory failed".into()))
            }
        })
    }
}
struct Allow;
impl ManagementAuthorizer for Allow {
    fn authorize(&self, _: &str, _: &ManagementRequest) -> Result<()> {
        Ok(())
    }
}
#[test]
fn supplied_clones_attach_once_and_drop_services_without_cycle() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {
        tokio::spawn(async {
            let (runtime, _backend, host) = Box::pin(fixture()).await;
            let agent = runtime.agent(AgentSpec::new("agent")).unwrap();
            let bound = RegisteredAgent(agent.clone());
            assert_eq!(
                tinyhivemind_core::driver::BoundAgent::runtime_id(&bound),
                "agent"
            );
            assert!(format!("{bound:?}").contains("agent"));
            let scope = TurnScope::from_request(&tinyhivemind_hives::TurnRequest {
                turn_id: String::new(),
                scheduled_job_id: None,
                teammates: Vec::new(),
                agent_id: "agent".into(),
                session_id: None,
                messages: vec![],
                memberships: vec![],
                episode: None,
                resumption: None,
            });
            assert!(DefaultHooks.progress(&scope).is_none());
            assert_eq!(DefaultHooks.prepare(&scope), TurnOptions::default());
            assert_eq!(
                DefaultHooks.after_turn(&scope, None).unwrap(),
                tinyhivemind_hives::TurnDisposition::Completed
            );
            let pass = DefaultHooks
                .wrap_turn(
                    &scope,
                    Box::pin(async {
                        Ok(openhuman_embed::TurnOutcome {
                            reply: "passthrough".into(),
                            session_id: "s".into(),
                            usage: None,
                            structured: None,
                            finish_reason: None,
                            answered_model: None,
                        })
                    }),
                )
                .await
                .unwrap();
            assert_eq!(pass.reply, "passthrough");
            let weak = Arc::downgrade(&host.inner);
            host.register_agent_in_session(agent.clone(), "existing")
                .await
                .unwrap();
            host.register_agent(agent.clone()).await.unwrap();
            assert_eq!(host.coordinator().list_agents().unwrap(), vec!["agent"]);
            assert!(
                host.register_agent_in_session(agent.clone(), "other")
                    .await
                    .is_err()
            );
            let clone = host.clone();
            assert!(host.with_hooks(Arc::new(DefaultHooks)).is_err());
            assert!(
                clone
                    .clone()
                    .with_management(Arc::new(Factory), Arc::new(Allow))
                    .is_err()
            );
            drop(clone);
            assert!(weak.upgrade().is_none());
            assert_eq!(agent.id(), "agent");
        })
        .await
        .unwrap();
    });
}
async fn foreign_host() -> OpenHumanHost {
    OpenHumanHost::new(
        "foreign".into(),
        Coordinator::new(
            "foreign".into(),
            Arc::new(MemoryStorage::new()),
            CoordinatorOptions::default(),
        )
        .await
        .unwrap(),
    )
    .unwrap()
}
#[test]
fn rejects_other_runtime_and_authorizes_before_factory() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {
        tokio::spawn(async {
            let (runtime, _backend, host) = Box::pin(fixture()).await;
            let foreign = foreign_host().await;
            assert!(matches!(
                foreign
                    .register_agent(runtime.agent(AgentSpec::new("foreign")).unwrap())
                    .await,
                Err(Error::RuntimeMismatch)
            ));
            assert!(matches!(
                host.manage(
                    "a",
                    ManagementRequest::CreateAgent {
                        template: "x".into(),
                        memory: None,
                        config: serde_json::json!({})
                    }
                )
                .await,
                Err(Error::ManagementDisabled)
            ));
            let host = host
                .with_hooks(Arc::new(DefaultHooks))
                .unwrap()
                .with_management(
                    Arc::new(ConfiguredFactory(
                        runtime.agent(AgentSpec::new("generated")).unwrap(),
                    )),
                    Arc::new(Allow),
                )
                .unwrap();
            assert!(
                host.manage(
                    "a",
                    ManagementRequest::CreateAgent {
                        template: "x".into(),
                        memory: None,
                        config: serde_json::json!({})
                    }
                )
                .await
                .is_err()
            );
            assert_eq!(host.coordinator().list_agents().unwrap().len(), 0);
            host.register_agent(runtime.agent(AgentSpec::new("a")).unwrap())
                .await
                .unwrap();
            let generated = host
                .manage(
                    "a",
                    ManagementRequest::CreateAgent {
                        template: "configured".into(),
                        memory: None,
                        config: serde_json::json!({}),
                    },
                )
                .await
                .unwrap();
            assert_eq!(generated["agent_id"], "generated");
            assert_eq!(
                host.coordinator().list_agents().unwrap(),
                vec!["a", "generated"]
            );
            let mut hive = tinyhivemind_hives::HiveInfo {
                hive_id: "h".into(),
                name: "Hive".into(),
                description: None,
                members: vec![],
            };
            host.manage("a", ManagementRequest::CreateHive(hive.clone()))
                .await
                .unwrap();
            host.manage(
                "a",
                ManagementRequest::JoinHive {
                    hive_id: "h".into(),
                    agent_id: "a".into(),
                },
            )
            .await
            .unwrap();
            hive.members.push("a".into());
            assert_eq!(host.coordinator().list_hives().unwrap(), vec![hive]);
            host.manage(
                "a",
                ManagementRequest::LeaveHive {
                    hive_id: "h".into(),
                    agent_id: "a".into(),
                },
            )
            .await
            .unwrap();
        })
        .await
        .unwrap();
    });
}
#[derive(Default)]
struct Hooks {
    mode: std::sync::atomic::AtomicU8,
    finalized: std::sync::atomic::AtomicUsize,
    wrapped: std::sync::atomic::AtomicUsize,
}
impl TurnHooks for Hooks {
    fn progress(&self, _: &TurnScope) -> Option<TurnProgressSink> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        Some(tx)
    }
    fn wrap_turn<'a>(&'a self, _: &'a TurnScope, turn: HostedTurn<'a>) -> HostedTurn<'a> {
        self.wrapped
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.mode.load(std::sync::atomic::Ordering::SeqCst) == 3 {
            Box::pin(async { Err(Error::Unauthorized("wrapper failed".into())) })
        } else {
            turn
        }
    }
    fn after_turn(
        &self,
        _: &TurnScope,
        usage: Option<&openhuman_core::agent::tinyagents::host::LastTurnUsage>,
    ) -> Result<tinyhivemind_hives::TurnDisposition> {
        use std::sync::atomic::Ordering;
        self.finalized.fetch_add(1, Ordering::SeqCst);
        match self.mode.load(Ordering::SeqCst) {
            1 => Ok(tinyhivemind_hives::TurnDisposition::Parked),
            2 => Err(Error::Unauthorized("finalizer failed".into())),
            3 => {
                assert!(
                    usage.is_none(),
                    "an unstarted failed turn must not reuse previous usage"
                );
                Ok(tinyhivemind_hives::TurnDisposition::Completed)
            }
            _ => Ok(tinyhivemind_hives::TurnDisposition::Completed),
        }
    }
}
#[test]
fn continuing_runner_preserves_history_and_finalizes_all_outcomes() {
    use std::sync::atomic::Ordering;
    use tinyhivemind_hives::{AgentRunner, TurnDisposition, TurnRequest};
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {tokio::spawn(async {
        let (runtime,_backend,host) = Box::pin(fixture()).await;
        let provider = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id":"fixture","object":"chat.completion","created":0,"model":"fixture",
                "choices":[{"index":0,"message":{"role":"assistant","content":"HOST_REPLY"},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}
            }))).mount(&provider).await;
        let hooks = Arc::new(Hooks::default());
        let host = host.with_hooks(hooks.clone()).unwrap();
        let agent = runtime.agent(AgentSpec::new("metered")
            .provider(openhuman_embed::Provider::openai_compatible(format!("{}/v1",provider.uri()),"fixture").model("fixture"))
            .system_prompt("HOST_CONFIGURED_PROMPT")).unwrap();
        host.register_agent(agent).await.unwrap();
        let runner = host.inner.agents.lock().unwrap()["metered"].runner.clone();
        let mut request = TurnRequest { turn_id: String::new(), scheduled_job_id: None, teammates: Vec::new(), agent_id:"metered".into(),session_id:None,messages:vec![],memberships:vec![],episode:None,resumption:None };
        let first = runner.run(request.clone()).await.unwrap();
        assert_eq!(first.reply.as_deref(),Some("HOST_REPLY"));
        request.session_id = Some(first.session_id.clone());
        hooks.mode.store(1,Ordering::SeqCst);
        let second = runner.run(request.clone()).await.unwrap();
        assert_eq!(first.session_id,second.session_id);
        assert_eq!(second.disposition,TurnDisposition::Parked);
        hooks.mode.store(2,Ordering::SeqCst);
        let failed = runner.run(request.clone()).await.unwrap();
        assert_eq!(failed.session_id, first.session_id);
        assert!(matches!(failed.disposition, TurnDisposition::Failed(_)));
        hooks.mode.store(3,Ordering::SeqCst);
        assert!(runner.run(request).await.is_err());
        assert_eq!(hooks.finalized.load(Ordering::SeqCst),4);
        assert_eq!(hooks.wrapped.load(Ordering::SeqCst),4);
        let requests:Vec<_> = provider.received_requests().await.unwrap().into_iter().filter(|request| request.method==wiremock::http::Method::POST && request.url.path()=="/v1/chat/completions").collect();
        assert_eq!(requests.len(),3);
        let last:serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
        let messages = last["messages"].as_array().unwrap();
        assert!(messages.iter().filter(|message| message["role"]=="assistant").count()>=2);
        assert!(messages.iter().any(|message| message["content"].as_str().is_some_and(|text|text.contains("HOST_CONFIGURED_PROMPT"))));
    }).await.unwrap();});
}
struct RegistrationStorage {
    reject: std::sync::atomic::AtomicBool,
    memory: MemoryStorage,
}
impl tinyhivemind_hives::Storage for RegistrationStorage {
    fn load(&self) -> tinyhivemind_hives::StorageFuture<'_, tinyhivemind_hives::StoredState> {
        tinyhivemind_hives::Storage::load(&self.memory)
    }
    fn commit<'a>(
        &'a self,
        commit: tinyhivemind_hives::Commit<'a>,
    ) -> tinyhivemind_hives::StorageFuture<'a, ()> {
        if self.reject.load(std::sync::atomic::Ordering::SeqCst) {
            Box::pin(async {
                Err(tinyhivemind_hives::Error::InvalidState(
                    "registration storage unavailable".into(),
                ))
            })
        } else {
            tinyhivemind_hives::Storage::commit(&self.memory, commit)
        }
    }
}
#[test]
fn failed_registration_cannot_use_tools_and_retries_the_identical_attachment() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {
        tokio::spawn(async {
            let (runtime, _backend, _) = Box::pin(fixture()).await;
            let storage = Arc::new(RegistrationStorage {
                reject: std::sync::atomic::AtomicBool::new(false),
                memory: MemoryStorage::new(),
            });
            // Construction commits the writer claim, so reject only afterwards.
            let coordinator = Coordinator::new(
                runtime.runtime_id().into(),
                storage.clone(),
                CoordinatorOptions::default(),
            )
            .await
            .unwrap();
            storage
                .reject
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let host = OpenHumanHost::new(runtime.runtime_id().into(), coordinator).unwrap();
            let agent = runtime.agent(AgentSpec::new("pending")).unwrap();
            assert!(host.register_agent(agent.clone()).await.is_err());
            assert_eq!(host.coordinator().list_agents().unwrap().len(), 0);
            let source = host.inner.agents.lock().unwrap()["pending"].source.clone();
            let tools = source(openhuman_embed::TurnContext::new("pending", None));
            assert_eq!(tools.tools.len(), 9);
            let tool = tools
                .tools
                .into_iter()
                .find(|tool| tool.name() == "hivemind_list_agents")
                .unwrap();
            assert!(
                tool.execute(serde_json::json!({})).await.unwrap().is_error,
                "a rejected registration must not expose working capabilities"
            );
            storage
                .reject
                .store(false, std::sync::atomic::Ordering::SeqCst);
            host.register_agent(agent).await.unwrap();
            assert!(Arc::ptr_eq(
                &source,
                &host.inner.agents.lock().unwrap()["pending"].source
            ));
            assert!(!tool.execute(serde_json::json!({})).await.unwrap().is_error);
        })
        .await
        .unwrap();
    });
}

#[path = "continuity_test.rs"]
mod continuity;
#[path = "hooks_test.rs"]
mod hooks;
#[path = "language_test.rs"]
mod language;
#[path = "memory_test.rs"]
mod memory;
#[path = "replace_test.rs"]
mod replace;
