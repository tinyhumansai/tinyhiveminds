//! Request attribution and failure mapping.
// Test assertions deliberately panic on invalid fixture construction.
#![allow(clippy::unwrap_used)]
use super::*;
#[test]
fn renders_attribution_and_explicit_episode_without_reseeding() {
    let request = TurnRequest {
        turn_id: String::new(),
        scheduled_job_id: None,
        teammates: Vec::new(),
        agent_id: "a".into(),
        session_id: Some("history".into()),
        messages: vec![],
        memberships: vec![],
        episode: None,
        resumption: None,
    };
    let prompt = render(&request).unwrap();
    assert!(prompt.contains("\"agent_id\":\"a\""));
    assert!(prompt.contains("\"session_id\":\"history\""));
    assert!(
        map_error(&Error::TimedOut)
            .to_string()
            .contains("timed out")
    );
    assert!(!prompt.contains("Host resumption note"));
}
#[test]
fn renders_a_release_note_ahead_of_the_attributed_context() {
    let request = TurnRequest {
        turn_id: String::new(),
        scheduled_job_id: None,
        teammates: Vec::new(),
        agent_id: "a".into(),
        session_id: None,
        messages: vec![],
        memberships: vec![],
        episode: None,
        resumption: Some("approved: staging only".into()),
    };
    let prompt = render(&request).unwrap();
    let note = prompt
        .find("Host resumption note: approved: staging only")
        .unwrap();
    assert!(note < prompt.find("Incoming attributed Hivemind context").unwrap());
}
