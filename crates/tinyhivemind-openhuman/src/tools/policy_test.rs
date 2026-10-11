//! The host send policy gates the four outbound tools before they execute.
#![allow(clippy::unwrap_used)]
use super::test::{active, registered_coordinator};
use super::*;
use crate::{SendAuthorizer, SendRequest};
use std::sync::{Arc, Mutex};

/// Refuses any body mentioning "secret"; records every request it sees.
#[derive(Default)]
struct NoSecrets(Mutex<Vec<(String, SendRequest)>>);
impl SendAuthorizer for NoSecrets {
    fn authorize(&self, actor: &str, request: &SendRequest) -> Result<()> {
        self.0.lock().unwrap().push((actor.into(), request.clone()));
        let body = match request {
            SendRequest::Agent { body, .. }
            | SendRequest::Hive { body, .. }
            | SendRequest::Ask { body, .. }
            | SendRequest::Broadcast { body, .. } => body,
        };
        if body.contains("secret") {
            Err(Error::SendDenied("no secrets leave this agent".into()))
        } else {
            Ok(())
        }
    }
}
async fn policed() -> (OpenHumanHost, Arc<NoSecrets>) {
    let coordinator = registered_coordinator().await;
    coordinator
        .create_hive(HiveInfo {
            hive_id: "h".into(),
            name: "Hive".into(),
            description: None,
            members: vec!["a".into(), "b".into()],
        })
        .await
        .unwrap();
    let policy = Arc::new(NoSecrets::default());
    let host = OpenHumanHost::new("r".into(), coordinator)
        .unwrap()
        .with_send_policy(policy.clone())
        .unwrap();
    (host, policy)
}
fn tool(host: &OpenHumanHost, kind: Kind) -> HiveTool {
    HiveTool {
        session: None,
        actor: "a".into(),
        host: Arc::downgrade(&host.inner),
        kind,
        activation: active(),
    }
}
#[tokio::test]
async fn refused_sends_are_tool_errors_and_enqueue_nothing() {
    let (host, policy) = policed().await;
    let cases = [
        (
            Kind::SendAgent,
            serde_json::json!({"agent_id":"b","message_id":"m1","body":"the secret"}),
        ),
        (
            Kind::SendHive,
            serde_json::json!({"hive_id":"h","message_id":"m2","body":"secret plan","only_for":["b"]}),
        ),
        (
            Kind::Ask,
            serde_json::json!({"episode_id":"e","agents":["b"],"body":"secret?"}),
        ),
        (
            Kind::Broadcast,
            serde_json::json!({"episode_id":"e","body":"secret work"}),
        ),
    ];
    for (kind, args) in cases {
        let result = tool(&host, kind).execute(args).await.unwrap();
        assert!(result.is_error, "{kind:?} must be refused");
        assert!(
            result
                .text()
                .contains("send denied: no secrets leave this agent")
        );
    }
    assert_eq!(host.coordinator().read_transcript(None).unwrap().len(), 0);
    let seen = policy.0.lock().unwrap();
    assert!(seen.iter().all(|(actor, _)| actor == "a"));
    assert_eq!(
        seen.iter()
            .map(|(_, request)| request.clone())
            .collect::<Vec<_>>(),
        [
            SendRequest::Agent {
                agent_id: "b".into(),
                body: "the secret".into()
            },
            SendRequest::Hive {
                hive_id: "h".into(),
                body: "secret plan".into(),
                thread: None,
                only_for: vec!["b".into()]
            },
            SendRequest::Ask {
                episode_id: "e".into(),
                agents: vec!["b".into()],
                body: "secret?".into()
            },
            SendRequest::Broadcast {
                episode_id: "e".into(),
                body: "secret work".into()
            },
        ]
    );
}
#[tokio::test]
async fn admitted_sends_execute_and_other_tools_are_not_consulted() {
    let (host, policy) = policed().await;
    let sent = tool(&host, Kind::SendAgent)
        .execute(serde_json::json!({"agent_id":"b","message_id":"ok","body":"hello"}))
        .await
        .unwrap();
    assert!(!sent.is_error, "{}", sent.text());
    assert_eq!(host.coordinator().read_transcript(None).unwrap().len(), 1);
    // Posting and completing are not sends; reads and listings neither.
    for (kind, args) in [
        (
            Kind::Post,
            serde_json::json!({"episode_id":"e","body":"secret"}),
        ),
        (
            Kind::Complete,
            serde_json::json!({"episode_id":"e","body":"secret"}),
        ),
        (Kind::ListAgents, serde_json::json!({})),
    ] {
        let result = tool(&host, kind).execute(args).await.unwrap();
        assert!(!result.text().contains("send denied"));
    }
    assert_eq!(policy.0.lock().unwrap().len(), 1);
    assert!(matches!(
        host.clone().with_send_policy(policy.clone()),
        Err(Error::ManagementAlreadyStarted)
    ));
}
#[test]
fn send_request_wire_shape_is_explicit() {
    let request = SendRequest::Hive {
        hive_id: "h".into(),
        body: "b".into(),
        thread: Some(3),
        only_for: vec![],
    };
    let wire = serde_json::json!({"Hive":{"hive_id":"h","body":"b","thread":3,"only_for":[]}});
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<SendRequest>(wire).unwrap(),
        request
    );
}
