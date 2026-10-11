//! Native schemas and bound service lifetime behavior.
// Test assertions deliberately panic on invalid fixture construction.
#![allow(clippy::unwrap_used)]
use super::*;
#[tokio::test]
async fn stable_vocabulary_has_one_schema_and_rejects_impersonation() {
    assert_eq!(Kind::all(false).len(), 9);
    let tools = belt("actor", &Weak::new(), &active(), true);
    assert_eq!(tools.len(), 13);
    for tool in tools {
        let schema = tool.parameters_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert!(
            !schema["properties"]
                .as_object()
                .unwrap()
                .contains_key("sender")
        );
        assert_ne!(tool.name(), "");
        assert_ne!(tool.description(), "");
        let _ = tool.permission_level();
        assert!(
            tool.execute(serde_json::json!({"sender":"victim"}))
                .await
                .unwrap()
                .text()
                .contains("unknown argument")
        );
    }
    assert!(
        Kind::SendAgent
            .validate(&serde_json::json!({"agent_id":"a"}))
            .is_err()
    );
    assert!(
        Kind::Read
            .validate(&serde_json::json!({"hive_id":"h","thread":-1}))
            .is_err()
    );
    assert!(
        Kind::Ask
            .validate(&serde_json::json!({"episode_id":"e","body":"b","agents":[1]}))
            .is_err()
    );
    assert!(Kind::ListAgents.validate(&serde_json::json!([])).is_err());
    let tool = HiveTool {
        session: None,
        actor: "a".into(),
        host: Weak::new(),
        kind: Kind::ListAgents,
        activation: active(),
    };
    assert!(
        tool.execute(serde_json::json!({}))
            .await
            .unwrap()
            .text()
            .contains("unavailable")
    );
}
struct NoTurn;
impl tinyhivemind_hives::AgentRunner for NoTurn {
    fn run(&self, _: tinyhivemind_hives::TurnRequest) -> tinyhivemind_hives::TurnFuture {
        Box::pin(async { unreachable!("this test never drains work") })
    }
}
struct Factory;
impl crate::AgentFactory for Factory {
    fn create(&self, _: String, _: Value) -> crate::AgentFuture {
        Box::pin(async { Err(Error::Unauthorized("factory unavailable".into())) })
    }
}
struct Authorize;
impl crate::ManagementAuthorizer for Authorize {
    fn authorize(&self, actor: &str, _: &ManagementRequest) -> Result<()> {
        if actor == "a" {
            Ok(())
        } else {
            Err(Error::Unauthorized("actor".into()))
        }
    }
}
#[tokio::test]
async fn bound_native_calls_validate_destinations_and_manage_membership() {
    use std::sync::Arc;
    let coor = registered_coordinator().await;
    let host = OpenHumanHost::new("r".into(), coor)
        .unwrap()
        .with_management(Arc::new(Factory), Arc::new(Authorize))
        .unwrap();
    let call = |kind| HiveTool {
        session: None,
        actor: "a".into(),
        host: Arc::downgrade(&host.inner),
        kind,
        activation: active(),
    };
    let execute = |kind, args| async move {
        let result = call(kind).execute(args).await.unwrap();
        assert!(!result.is_error, "{}", result.text());
        serde_json::from_str::<Value>(&result.text()).unwrap()
    };
    execute(Kind::CreateHive,serde_json::json!({"hive_id":"h","name":"Hive","members":["a","b"],"description":"purpose"})).await;
    let hives = execute(Kind::ListHives, serde_json::json!({})).await;
    assert_eq!(hives.as_array().unwrap().len(), 1);
    assert_eq!(
        execute(Kind::ListAgents, serde_json::json!({})).await,
        serde_json::json!(["a", "b"])
    );
    let receipt = execute(
        Kind::SendHive,
        serde_json::json!({"hive_id":"h","message_id":"m","body":"visible","only_for":["b"]}),
    )
    .await;
    assert_eq!(receipt["message_id"], "m");
    let rows = execute(Kind::Read, serde_json::json!({"hive_id":"h"})).await;
    assert_eq!(rows[0]["sender"], "a");
    execute(
        Kind::SendAgent,
        serde_json::json!({"agent_id":"b","message_id":"dm","body":"direct"}),
    )
    .await;
    for kind in [Kind::Post, Kind::Ask, Kind::Broadcast, Kind::Complete] {
        let args = if matches!(kind, Kind::Ask) {
            serde_json::json!({"episode_id":"stale","body":"b","agents":["b"]})
        } else {
            serde_json::json!({"episode_id":"stale","body":"b"})
        };
        assert!(call(kind).execute(args).await.unwrap().is_error);
    }
    assert!(
        call(Kind::CreateAgent)
            .execute(serde_json::json!({"template":"missing","config":{}}))
            .await
            .unwrap()
            .is_error
    );
    execute(
        Kind::LeaveHive,
        serde_json::json!({"hive_id":"h","agent_id":"a"}),
    )
    .await;
    assert_eq!(
        execute(Kind::ListHives, serde_json::json!({})).await,
        serde_json::json!([])
    );
    assert!(
        call(Kind::Read)
            .execute(serde_json::json!({"hive_id":"h"}))
            .await
            .unwrap()
            .is_error
    );
    execute(
        Kind::JoinHive,
        serde_json::json!({"hive_id":"h","agent_id":"a"}),
    )
    .await;
    let denied = HiveTool {
        session: None,
        actor: "b".into(),
        host: Arc::downgrade(&host.inner),
        kind: Kind::CreateHive,
        activation: active(),
    };
    assert!(
        denied
            .execute(serde_json::json!({"hive_id":"forbidden","name":"Forbidden"}))
            .await
            .unwrap()
            .is_error
    );
    assert_eq!(host.coordinator().list_hives().unwrap().len(), 1);
}
struct ActiveTools(std::sync::Arc<std::sync::Mutex<Weak<Inner>>>);
impl tinyhivemind_hives::AgentRunner for ActiveTools {
    fn run(&self, request: tinyhivemind_hives::TurnRequest) -> tinyhivemind_hives::TurnFuture {
        let weak = self.0.lock().unwrap().clone();
        Box::pin(async move {
            let episode = request.episode.unwrap();
            for kind in [Kind::Post, Kind::Complete] {
                let tool = HiveTool {
                    session: None,
                    actor: request.agent_id.clone(),
                    host: weak.clone(),
                    kind,
                    activation: active(),
                };
                let result=tool.execute(serde_json::json!({"episode_id":episode.episode_id,"body":"native active action"})).await.unwrap();
                assert!(!result.is_error, "{}", result.text());
            }
            Ok(tinyhivemind_hives::TurnOutcome {
                session_id: "stable".into(),
                reply: None,
                disposition: tinyhivemind_hives::TurnDisposition::Completed,
            })
        })
    }
}
#[tokio::test]
async fn explicit_episode_actions_execute_only_during_the_bound_assignment() {
    use std::sync::{Arc, Mutex};
    use tinyhivemind_hives::{AgentRegistration, Coordinator, CoordinatorOptions, MemoryStorage};
    let coor = Coordinator::new(
        "r".into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    let service = Arc::new(Mutex::new(Weak::new()));
    coor.register_agent(AgentRegistration {
        agent_id: "a".into(),
        runtime_id: "r".into(),
        runner: Arc::new(ActiveTools(service.clone())),
    })
    .await
    .unwrap();
    let host = OpenHumanHost::new("r".into(), coor).unwrap();
    *service.lock().unwrap() = Arc::downgrade(&host.inner);
    host.coordinator()
        .create_hive(HiveInfo {
            hive_id: "h".into(),
            name: "Hive".into(),
            description: None,
            members: vec!["a".into()],
        })
        .await
        .unwrap();
    host.coordinator()
        .send_as_host(SendMessage {
            message_id: "start".into(),
            sender: String::new(),
            destination: Destination::Hive("h".into()),
            body: "work".into(),
            thread: None,
            only_for: vec![],
            starters: Vec::new(),
        })
        .await
        .unwrap();
    let report = host.coordinator().run_until_idle().await.unwrap();
    assert_eq!(report.completed, 1);
    let rows = host.coordinator().read_hive("a", "h", None, None).unwrap();
    assert!(
        rows.iter()
            .any(|message| message.sender == "a" && message.body == "native active action")
    );
}

pub(super) fn active() -> std::sync::Arc<Activation> {
    let activation = std::sync::Arc::new(Activation::default());
    activation.activate();
    activation
}

pub(super) async fn registered_coordinator() -> tinyhivemind_hives::Coordinator {
    use std::sync::Arc;
    use tinyhivemind_hives::{AgentRegistration, Coordinator, CoordinatorOptions, MemoryStorage};
    let coor = Coordinator::new(
        "r".into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    for id in ["a", "b"] {
        coor.register_agent(AgentRegistration {
            agent_id: id.into(),
            runtime_id: "r".into(),
            runner: Arc::new(NoTurn),
        })
        .await
        .unwrap();
    }
    coor
}
