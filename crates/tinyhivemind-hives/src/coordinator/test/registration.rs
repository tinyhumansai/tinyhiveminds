//! Atomic handle publication with host-owned continuing sessions.
use super::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reattached_pending_work_observes_bound_session_while_scheduler_is_live() {
    let storage = Arc::new(MemoryStorage::new());
    let original = Coordinator::new(
        "runtime".into(),
        storage.clone(),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    add(&original, "a", completed_turn).await;
    original
        .send_as_host(message("pending", Destination::Agent("a".into())))
        .await
        .unwrap();
    drop(original);
    let restored = Coordinator::new("runtime".into(), storage, CoordinatorOptions::default())
        .await
        .unwrap();
    let (seen, mut received) = tokio::sync::mpsc::channel(1);
    let runner: Arc<dyn AgentRunner> = Arc::new(Script(Arc::new(move |request| {
        let seen = seen.clone();
        Box::pin(async move {
            seen.send(request.session_id.clone()).await.unwrap();
            completed_turn(request).await
        })
    })));
    let scheduler = restored.clone();
    let running = tokio::spawn(async move { scheduler.run().await });
    tokio::task::yield_now().await;
    restored
        .register_agent_in_session(
            AgentRegistration {
                agent_id: "a".into(),
                runtime_id: "runtime".into(),
                runner: runner.clone(),
            },
            "host-conversation",
        )
        .await
        .unwrap();
    assert_eq!(
        received.recv().await.unwrap().as_deref(),
        Some("host-conversation")
    );
    restored.shutdown();
    running.await.unwrap().unwrap();
    restored
        .register_agent_in_session(
            AgentRegistration {
                agent_id: "a".into(),
                runtime_id: "runtime".into(),
                runner: runner.clone(),
            },
            "host-conversation",
        )
        .await
        .unwrap();
    assert!(matches!(
        restored
            .register_agent_in_session(
                AgentRegistration {
                    agent_id: "a".into(),
                    runtime_id: "runtime".into(),
                    runner
                },
                "replacement"
            )
            .await,
        Err(Error::SessionConflict(_))
    ));
}
#[tokio::test]
async fn session_registration_validates_before_publishing_and_storage_failure_is_atomic() {
    let storage = Arc::new(super::transactions::Recording::default());
    let writer = Coordinator::new(
        "runtime".into(),
        storage.clone(),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    add(&writer, "a", completed_turn).await;
    hive(&writer, "revision", &[]).await;
    // One writer per store: the next coordinator takes the store over.
    let stale = Coordinator::new(
        "runtime".into(),
        storage.clone(),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    let runner: Arc<dyn AgentRunner> = Arc::new(Script(Arc::new(completed_turn)));
    let registration = AgentRegistration {
        agent_id: "a".into(),
        runtime_id: "runtime".into(),
        runner: runner.clone(),
    };
    storage.fail_next_commits(1);
    assert!(matches!(
        stale
            .register_agent_in_session(registration.clone(), "existing")
            .await,
        Err(Error::InvalidState(_))
    ));
    assert_eq!(stale.lock().unwrap().durable.agents["a"].session_id, None);
    assert!(!stale.lock().unwrap().runners.contains_key("a"));
    assert_eq!(storage.load().await.unwrap().agents["a"].session_id, None);
    let current = Coordinator::new(
        "runtime".into(),
        storage.clone(),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    assert!(
        current
            .register_agent_in_session(registration.clone(), "")
            .await
            .is_err()
    );
    let mut foreign = registration.clone();
    foreign.runtime_id = "foreign".into();
    assert!(matches!(
        current.register_agent_in_session(foreign, "existing").await,
        Err(Error::RuntimeMismatch)
    ));
    assert!(!current.lock().unwrap().runners.contains_key("a"));
    current.register_agent(registration.clone()).await.unwrap();
    current
        .register_agent_in_session(registration.clone(), "existing")
        .await
        .unwrap();
    let other = AgentRegistration {
        runner: Arc::new(Script(Arc::new(completed_turn))),
        ..registration
    };
    assert!(matches!(
        current
            .register_agent_in_session(other.clone(), "existing")
            .await,
        Err(Error::AgentConflict(_))
    ));
    assert_eq!(
        current.lock().unwrap().durable.agents["a"]
            .session_id
            .as_deref(),
        Some("existing")
    );
    let restored = Coordinator::new("runtime".into(), storage, CoordinatorOptions::default())
        .await
        .unwrap();
    assert!(matches!(
        restored.register_agent_in_session(other, "switched").await,
        Err(Error::SessionConflict(_))
    ));
    assert!(!restored.lock().unwrap().runners.contains_key("a"));
}

// One normal completion callback keeps all registration-only handles equivalent
// in behavior while their outer Arc identities remain deliberately distinct.
fn completed_turn(request: TurnRequest) -> TurnFuture {
    Box::pin(async move { Ok(done(&request)) })
}

#[test]
fn coordinator_options_wire_form_and_omitted_defaults_are_stable() {
    let value: crate::CoordinatorOptions = serde_json::from_str("{}").unwrap();
    assert_eq!(
        serde_json::to_value(value).unwrap(),
        serde_json::json!({"round_width":1,"conduct_policy":{"child_turn_wall":6,"turn_wall":60},"broadcast_budget":null,"retention":{"settled_episodes":null,"delivered":null,"interrupted":null,"pending_per_agent":null}})
    );
}
