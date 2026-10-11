//! Management request serialization remains explicit and round trips.
#[test]
fn management_request_wire_shape_keeps_host_template_and_identity_fields() {
    use super::ManagementRequest;
    use serde_json::json;
    let cases = [
        (
            ManagementRequest::CreateAgent {
                template: "research".into(),
                memory: None,
                config: json!({"memory_ref":"private-1"}),
            },
            json!({"CreateAgent":{"template":"research","config":{"memory_ref":"private-1"}}}),
        ),
        (
            ManagementRequest::JoinHive {
                hive_id: "h".into(),
                agent_id: "a".into(),
            },
            json!({"JoinHive":{"hive_id":"h","agent_id":"a"}}),
        ),
        (
            ManagementRequest::LeaveHive {
                hive_id: "h".into(),
                agent_id: "a".into(),
            },
            json!({"LeaveHive":{"hive_id":"h","agent_id":"a"}}),
        ),
        (
            ManagementRequest::CreateHive(tinyhivemind_hives::HiveInfo {
                hive_id: "h".into(),
                name: "Hive".into(),
                description: None,
                members: vec![],
            }),
            json!({"CreateHive":{"hive_id":"h","name":"Hive","description":null,"members":[]}}),
        ),
    ];
    for (request, wire) in cases {
        assert_eq!(serde_json::to_value(&request).ok(), Some(wire.clone()));
        assert_eq!(
            serde_json::from_value::<ManagementRequest>(wire).ok(),
            Some(request)
        );
    }
}
#[test]
fn turn_scope_names_the_episode_its_thread_and_distinct_senders() {
    use super::TurnScope;
    use tinyhivemind_hives::{Destination, EpisodeContext, Message, TurnRequest};
    let row = |id: &str, sender: &str, thread| Message {
        scheduled_job_id: None,
        message_id: id.into(),
        sequence: 0,
        sender: sender.into(),
        destination: Destination::Hive("work".into()),
        body: String::new(),
        thread,
        episode_id: Some("episode:0".into()),
        only_for: vec![],
    };
    let episode = EpisodeContext {
        episode_id: "episode:0".into(),
        hive_id: "work".into(),
        thread: Some(7),
        brief: "brief".into(),
    };
    let request = TurnRequest {
        turn_id: String::new(),
        scheduled_job_id: None,
        teammates: Vec::new(),
        agent_id: "a".into(),
        session_id: None,
        messages: vec![
            row("m1", "b", None),
            row("m2", "c", Some(7)),
            row("m3", "b", None),
        ],
        memberships: vec![],
        episode: Some(episode.clone()),
        resumption: None,
    };
    let scope = TurnScope::from_request(&request);
    assert_eq!(scope.agent_id, "a");
    assert_eq!(scope.episode, Some(episode));
    assert_eq!(scope.message_ids, ["m1", "m2", "m3"]);
    assert_eq!(scope.senders, ["b", "c"]);
    assert_eq!(scope.destination, Destination::Hive("work".into()));
    assert_eq!(scope.thread, Some(7));
    let mut direct = request;
    direct.episode = None;
    direct.messages = vec![Message {
        scheduled_job_id: None,
        destination: Destination::Agent("a".into()),
        thread: None,
        ..row("d1", tinyhivemind_hives::HOST_ID, None)
    }];
    let scope = TurnScope::from_request(&direct);
    assert_eq!(scope.destination, Destination::Agent("a".into()));
    assert_eq!(scope.thread, None);
    assert_eq!(scope.senders, [tinyhivemind_hives::HOST_ID]);
    direct.messages.clear();
    assert_eq!(
        TurnScope::from_request(&direct).destination,
        Destination::Agent("a".into())
    );
}

#[test]
fn create_agent_binding_wire_is_optional_and_preserves_the_contract()
-> Result<(), serde_json::Error> {
    use super::ManagementRequest;
    let legacy = serde_json::json!({"CreateAgent":{"template":"research","config":{}}});
    let decoded: ManagementRequest = serde_json::from_value(legacy.clone())?;
    assert!(matches!(
        &decoded,
        ManagementRequest::CreateAgent { memory: None, .. }
    ));
    assert_eq!(serde_json::to_value(decoded)?, legacy);
    let portable = serde_json::json!({
        "agent_id":"shared", "root":"team:hive", "namespace":"team:hive/agent:shared", "kind":"agent",
        "identity":{"id":"memory","namespace":"team:hive/agent:shared","kind":"agent","lifecycle":"hive","read_only":true,"fork_parent":null},
        "settings":{"seat":"scout","identity":"memory","reads":[],"recall_at":["compaction"],"budget_chars":128,"remember":["note"]}
    });
    let wire =
        serde_json::json!({"CreateAgent":{"template":"research","config":{},"memory":portable}});
    let decoded: ManagementRequest = serde_json::from_value(wire.clone())?;
    assert_eq!(serde_json::to_value(decoded)?, wire);
    Ok(())
}
