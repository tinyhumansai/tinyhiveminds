//! Every outbound operation intersects captured membership and session authority.
use super::support::*;
use crate::{
    SendAuthorizer, SendRequest, TurnHooks,
    deploy::{BuildOptions, hooks::Hooks},
};
fn requests() -> [SendRequest; 4] {
    [
        SendRequest::Agent {
            agent_id: "bob".into(),
            body: "work".into(),
        },
        SendRequest::Hive {
            hive_id: "a".into(),
            body: "work".into(),
            thread: None,
            only_for: Vec::new(),
        },
        SendRequest::Ask {
            episode_id: "episode".into(),
            agents: vec!["bob".into()],
            body: "work".into(),
        },
        SendRequest::Broadcast {
            episode_id: "episode".into(),
            body: "work".into(),
        },
    ]
}
#[test]
fn sends_intersect_all_operations_destinations_and_authenticated_sessions() -> anyhow::Result<()> {
    let options = BuildOptions::default();
    let mut config = manifest()?;
    config.permission_profiles[0].send_operations = Some(Vec::new());
    let hooks = Hooks::new(permissions(config.clone(), &options), &options);
    hooks.context(&scope("a"));
    for request in requests() {
        assert!(
            hooks
                .authorize_in_session("alice", &request, Some("session"))
                .is_ok()
        );
        assert!(hooks.authorize("alice", &request).is_err());
        assert!(
            hooks
                .authorize_in_session("alice", &request, Some("forged"))
                .is_err()
        );
        assert!(
            hooks
                .authorize_in_session("unknown", &request, Some("session"))
                .is_err()
        );
    }
    hooks.context(&scope("b"));
    for request in requests() {
        assert!(
            hooks
                .authorize_in_session("alice", &request, Some("session"))
                .is_err()
        );
    }
    config.permission_profiles[0].send_operations = None;
    config.permission_profiles[0].send_destinations = Some(vec!["other".into()]);
    let hooks = Hooks::new(permissions(config, &options), &options);
    hooks.context(&scope("b"));
    for request in requests() {
        assert!(
            hooks
                .authorize_in_session("alice", &request, Some("session"))
                .is_err()
        );
    }
    let hooks = Hooks::new(permissions(manifest()?, &options), &options);
    for request in requests().into_iter().skip(2) {
        assert!(hooks.authorize("alice", &request).is_err());
    }
    let mut forged = scope("a");
    forged
        .episode
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("fixture has no episode"))?
        .episode_id = "another".into();
    hooks.context(&forged);
    for request in requests().into_iter().skip(2) {
        assert!(
            hooks
                .authorize_in_session("alice", &request, Some("session"))
                .is_err()
        );
    }
    Ok(())
}
