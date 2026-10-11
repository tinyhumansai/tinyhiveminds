//! Hive memory binds every registered seat, and refuses seats built without it.
use super::*;
use std::num::NonZeroU32;

fn hive(id: &str) -> HiveMemory {
    HiveMemory::for_hive(id).unwrap()
}

fn bound(agent: &Agent) -> (Option<&str>, Option<&str>) {
    let memory = &agent.config().memory;
    (memory.agent_id.as_deref(), memory.root.as_deref())
}

async fn runtime_with(config: openhuman_embed::RuntimeConfig) -> (Runtime, wiremock::MockServer) {
    let backend = offline::backend().await;
    let runtime = Box::pin(
        Runtime::builder()
            .config(config)
            .workspace(Workspace::Ephemeral)
            .backend_url(backend.uri())
            .build(),
    )
    .await
    .unwrap();
    (runtime, backend)
}

async fn host_on(runtime: &Runtime) -> OpenHumanHost {
    let coordinator = Coordinator::new(
        runtime.runtime_id().into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    OpenHumanHost::new(runtime.runtime_id().into(), coordinator).unwrap()
}

fn on_runtime<F, Fut>(test: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async { tokio::spawn(test()).await.unwrap() });
}

#[test]
fn every_seat_registered_with_hive_memory_is_bound_under_the_hive_root() {
    on_runtime(|| async {
        let (runtime, _backend, host) = Box::pin(fixture()).await;
        let host = host.with_hive_memory(hive("hive-1")).unwrap();
        assert_eq!(
            host.hive_memory().map(HiveMemory::root),
            Some("team:hive-1")
        );
        for seat in ["scout", "reviewer-seat", "writer"] {
            let agent = host
                .register_spec(&runtime, AgentSpec::new(seat))
                .await
                .unwrap();
            assert_eq!(bound(&agent), (Some(seat), Some("team:hive-1")));
        }
        let registered: Vec<_> = host.inner.agents.lock().unwrap().keys().cloned().collect();
        assert_eq!(registered, ["reviewer-seat", "scout", "writer"]);
    });
}

#[test]
fn seats_carry_no_binding_when_hive_memory_is_off() {
    on_runtime(|| async {
        let (runtime, _backend, host) = Box::pin(fixture()).await;
        assert!(host.hive_memory().is_none());
        let agent = host
            .register_spec(&runtime, AgentSpec::new("scout"))
            .await
            .unwrap();
        assert_eq!(bound(&agent), (None, None));
        // An unbound agent built elsewhere registers as before.
        let other = runtime.agent(AgentSpec::new("reviewer-seat")).unwrap();
        host.register_agent(other).await.unwrap();
    });
}

#[test]
fn a_seat_bound_by_the_host_itself_registers() {
    on_runtime(|| async {
        let (runtime, _backend, host) = Box::pin(fixture()).await;
        let memory = hive("hive-1");
        let host = host.with_hive_memory(memory.clone()).unwrap();
        let spec = memory.bind(AgentSpec::new("scout")).unwrap();
        let agent = runtime.agent(spec).unwrap();
        host.register_agent_in_session(agent, "session-1")
            .await
            .unwrap();
    });
}

#[test]
fn rejects_a_seat_built_without_the_hive_binding() {
    on_runtime(|| async {
        let (runtime, _backend, host) = Box::pin(fixture()).await;
        let host = host.with_hive_memory(hive("hive-1")).unwrap();
        let unbound = runtime.agent(AgentSpec::new("scout")).unwrap();
        let error = host.register_agent(unbound).await.unwrap_err();
        assert!(
            matches!(&error, Error::UnboundSeat { seat, .. } if seat == "scout"),
            "{error}"
        );
        let elsewhere = hive("hive-2")
            .bind(AgentSpec::new("reviewer-seat"))
            .unwrap();
        let error = host
            .register_agent(runtime.agent(elsewhere).unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("team:hive-2"), "{error}");
        let renamed = AgentSpec::new("writer")
            .memory(openhuman_embed::MemoryBinding::new("someone-else").root("team:hive-1"));
        let error = host
            .register_agent(runtime.agent(renamed).unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("someone-else"), "{error}");
        assert!(host.inner.agents.lock().unwrap().is_empty());
    });
}

#[test]
fn an_unusable_seat_id_fails_before_the_runtime_sees_it() {
    on_runtime(|| async {
        let (runtime, _backend, host) = Box::pin(fixture()).await;
        let host = host.with_hive_memory(hive("hive-1")).unwrap();
        assert!(matches!(
            host.register_spec(&runtime, AgentSpec::new("Bad Seat"))
                .await,
            Err(Error::InvalidMemoryAgentId { .. })
        ));
        let plain = host_on(&runtime).await;
        assert!(matches!(
            plain
                .register_spec(&runtime, AgentSpec::new("Bad Seat"))
                .await,
            Err(Error::Agent(_))
        ));
    });
}

#[test]
fn the_recall_budget_reaches_seats_through_the_runtime_config() {
    on_runtime(|| async {
        let memory = hive("hive-1").recall_budget_tokens(NonZeroU32::new(640).unwrap());
        let mut config = offline::config();
        memory.configure(&mut config);
        let (runtime, _backend) = Box::pin(runtime_with(config)).await;
        let host = host_on(&runtime)
            .await
            .with_hive_memory(memory.clone())
            .unwrap();
        let agent = host
            .register_spec(&runtime, AgentSpec::new("scout"))
            .await
            .unwrap();
        assert_eq!(agent.config().memory.recall.budget_tokens, 640);
        // One runtime per process: release this one before the next.
        drop((agent, host, runtime));

        let (unconfigured, _backend) = Box::pin(runtime_with(offline::config())).await;
        let host = host_on(&unconfigured)
            .await
            .with_hive_memory(memory)
            .unwrap();
        let error = host
            .register_spec(&unconfigured, AgentSpec::new("scout"))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::UnboundSeat { reason, .. } if reason.contains("640")),
            "{error}"
        );
    });
}

#[test]
fn hive_memory_cannot_change_after_registration() {
    on_runtime(|| async {
        let (runtime, _backend, host) = Box::pin(fixture()).await;
        host.register_spec(&runtime, AgentSpec::new("scout"))
            .await
            .unwrap();
        assert!(matches!(
            host.with_hive_memory(hive("hive-1")),
            Err(Error::ManagementAlreadyStarted)
        ));
    });
}
