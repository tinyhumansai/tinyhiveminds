//! Bound read tool exposes durable direct replies without return deliveries.
#![allow(clippy::unwrap_used)]
use super::*;
use std::sync::Arc;
use tinyhivemind_hives::*;
struct Reply;
impl AgentRunner for Reply {
    fn run(&self, request: TurnRequest) -> TurnFuture {
        Box::pin(async move {
            Ok(TurnOutcome {
                session_id: format!("session:{}", request.agent_id),
                reply: Some("answer".into()),
                disposition: TurnDisposition::Completed,
            })
        })
    }
}
#[tokio::test]
async fn read_tool_observes_direct_replies_as_its_bound_caller() {
    let c = Coordinator::new(
        "r".into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    for id in ["a", "b", "outsider"] {
        c.register_agent(AgentRegistration {
            agent_id: id.into(),
            runtime_id: "r".into(),
            runner: Arc::new(Reply),
        })
        .await
        .unwrap();
    }
    let host = OpenHumanHost::new("r".into(), c).unwrap();
    let activation = Arc::new(Activation::default());
    activation.activate();
    let tool = |actor: &str, kind| HiveTool {
        session: None,
        actor: actor.into(),
        host: Arc::downgrade(&host.inner),
        kind,
        activation: activation.clone(),
    };
    let receipt = tool("a", Kind::SendAgent)
        .execute(serde_json::json!({"agent_id":"b","message_id":"q","body":"question"}))
        .await
        .unwrap();
    let receipt: Value = serde_json::from_str(&receipt.text()).unwrap();
    assert_eq!(
        host.coordinator().run_until_idle().await.unwrap().completed,
        1
    );
    let rows = tool("a", Kind::Read)
        .execute(serde_json::json!({"agent_id":"b","after":receipt["sequence"]}))
        .await
        .unwrap();
    assert!(!rows.is_error, "{}", rows.text());
    let rows: Value = serde_json::from_str(&rows.text()).unwrap();
    assert_eq!(rows[0]["body"], "answer");
    assert_eq!(rows[0]["sender"], "b");
    let outsider = tool("outsider", Kind::Read)
        .execute(serde_json::json!({"agent_id":"b"}))
        .await
        .unwrap();
    assert_eq!(outsider.text(), "[]");
    assert_eq!(
        host.coordinator().run_until_idle().await.unwrap().completed,
        0
    );
}
#[test]
fn read_schema_and_validator_require_exactly_one_destination() {
    let schema = Kind::Read.schema();
    assert_eq!(schema["oneOf"].as_array().unwrap().len(), 2);
    for args in [
        serde_json::json!({}),
        serde_json::json!({"hive_id":"h","agent_id":"b"}),
        serde_json::json!({"agent_id":"b","thread":1}),
    ] {
        assert!(Kind::Read.validate(&args).is_err());
    }
    assert!(
        Kind::Read
            .validate(&serde_json::json!({"agent_id":"b","after":0}))
            .is_ok()
    );
    assert!(
        Kind::Read
            .validate(&serde_json::json!({"hive_id":"h","thread":1}))
            .is_ok()
    );
}
