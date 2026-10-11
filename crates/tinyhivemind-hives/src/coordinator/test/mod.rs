//! Registration and durable message acceptance contracts.
// Panicking assertions are confined to deterministic test fixtures.
#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use crate::MemoryStorage;
use std::sync::Arc;
#[tokio::test]
async fn creates_empty_hives_but_rejects_delivery_without_members() {
    let c = Coordinator::new(
        "runtime".into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    let hive = HiveInfo {
        hive_id: "work".into(),
        name: "Work".into(),
        description: None,
        members: vec![],
    };
    c.create_hive(hive.clone()).await.unwrap();
    c.create_hive(hive).await.unwrap();
    assert_eq!(c.list_hives().unwrap().len(), 1);
    assert!(
        c.send_as_host(SendMessage {
            message_id: "m".into(),
            sender: "ignored".into(),
            destination: Destination::Hive("work".into()),
            body: "task".into(),
            thread: None,
            only_for: vec![],
            starters: Vec::new(),
        })
        .await
        .is_err()
    );
}

struct Script(Arc<dyn Fn(TurnRequest) -> TurnFuture + Send + Sync>);
impl AgentRunner for Script {
    fn run(&self, request: TurnRequest) -> TurnFuture {
        (self.0)(request)
    }
}
async fn setup() -> Coordinator {
    Coordinator::new(
        "runtime".into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap()
}
async fn add(
    c: &Coordinator,
    id: &str,
    run: impl Fn(TurnRequest) -> TurnFuture + Send + Sync + 'static,
) -> Arc<dyn AgentRunner> {
    let runner: Arc<dyn AgentRunner> = Arc::new(Script(Arc::new(run)));
    c.register_agent(AgentRegistration {
        agent_id: id.into(),
        runtime_id: "runtime".into(),
        runner: runner.clone(),
    })
    .await
    .unwrap();
    runner
}
async fn hive(c: &Coordinator, id: &str, ids: &[&str]) {
    c.create_hive(HiveInfo {
        hive_id: id.into(),
        name: id.into(),
        description: None,
        members: ids.iter().map(|id| (*id).into()).collect(),
    })
    .await
    .unwrap();
}
fn message(id: &str, target: Destination) -> SendMessage {
    SendMessage {
        message_id: id.into(),
        sender: "a".into(),
        destination: target,
        body: id.into(),
        thread: None,
        only_for: Vec::new(),
        starters: Vec::new(),
    }
}
fn done(request: &TurnRequest) -> TurnOutcome {
    TurnOutcome {
        session_id: request
            .session_id
            .clone()
            .unwrap_or_else(|| format!("session:{}", request.agent_id)),
        reply: None,
        disposition: TurnDisposition::Completed,
    }
}

#[tokio::test]
async fn registration_checks_runtime_handle_identity_and_ids() {
    let c = setup().await;
    let runner = add(&c, "a", |request| {
        Box::pin(async move { Ok(done(&request)) })
    })
    .await;
    c.register_agent(AgentRegistration {
        agent_id: "a".into(),
        runtime_id: "runtime".into(),
        runner: runner.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(
        c.register_agent(AgentRegistration {
            agent_id: "a".into(),
            runtime_id: "other".into(),
            runner: runner.clone()
        })
        .await,
        Err(Error::RuntimeMismatch)
    ));
    let other = Arc::new(Script(Arc::new(|request| {
        Box::pin(async move { Ok(done(&request)) })
    })));
    assert!(matches!(
        c.register_agent(AgentRegistration {
            agent_id: "a".into(),
            runtime_id: "runtime".into(),
            runner: other
        })
        .await,
        Err(Error::AgentConflict(_))
    ));
    assert!(
        c.register_agent(AgentRegistration {
            agent_id: HOST_ID.into(),
            runtime_id: "runtime".into(),
            runner
        })
        .await
        .is_err()
    );
}
#[tokio::test]
async fn acceptance_deduplicates_exact_payload_and_enforces_private_visibility() {
    let c = setup().await;
    for id in ["a", "b", "c"] {
        add(&c, id, |request| {
            Box::pin(async move { Ok(done(&request)) })
        })
        .await;
    }
    hive(&c, "work", &["a", "b"]).await;
    let mut input = message("secret", Destination::Hive("work".into()));
    input.only_for = vec!["b".into()];
    let receipt = c.send(input.clone()).await.unwrap();
    assert_eq!(c.send(input.clone()).await.unwrap(), receipt);
    input.body = "different".into();
    assert!(matches!(
        c.send(input).await,
        Err(Error::MessageConflict(_))
    ));
    assert_eq!(c.read_hive("b", "work", None, None).unwrap().len(), 1);
    assert!(c.read_hive("c", "work", None, None).is_err());
    c.join_hive("work", "c").await.unwrap();
    assert_eq!(
        c.read_hive("c", "work", None, None).unwrap(),
        Vec::<Message>::new()
    );
    let mut unauthorized = message("bad", Destination::Hive("work".into()));
    unauthorized.sender = "unknown".into();
    assert!(c.send(unauthorized).await.is_err());
    let mut bad_thread = message("thread", Destination::Hive("work".into()));
    bad_thread.thread = Some(999);
    assert!(c.send(bad_thread).await.is_err());
    c.leave_hive("work", "c").await.unwrap();
    c.leave_hive("work", "c").await.unwrap();
    assert!(c.read_hive("c", "work", None, None).is_err());
}
#[tokio::test]
async fn one_agent_continues_one_session_across_three_hives() {
    let c = setup().await;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let actions = c.clone();
    add(&c, "a", move |request| {
        let c = actions.clone();
        let recorded = recorded.clone();
        Box::pin(async move {
            recorded.lock().unwrap().push(request.clone());
            let ep = request.episode.as_ref().unwrap();
            c.submit_action(
                &request.agent_id,
                &ep.episode_id,
                EpisodeAction::Complete {
                    body: "done".into(),
                },
            )
            .await?;
            Ok(done(&request))
        })
    })
    .await;
    for id in ["one", "two", "three"] {
        hive(&c, id, &["a"]).await;
        c.send_as_host(message(id, Destination::Hive(id.into())))
            .await
            .unwrap();
    }
    assert_eq!(c.run_until_idle().await.unwrap().completed, 3);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].session_id, None);
    assert_eq!(seen[1].session_id.as_deref(), Some("session:a"));
    assert_eq!(seen[2].session_id.as_deref(), Some("session:a"));
    assert_eq!(
        seen.iter()
            .map(|r| r.episode.as_ref().unwrap().hive_id.as_str())
            .collect::<Vec<_>>(),
        ["one", "two", "three"]
    );
}
#[tokio::test]
async fn conductor_opens_child_ask_and_delivers_its_conclusion() {
    let c = setup().await;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    for id in ["a", "b"] {
        let c2 = c.clone();
        let seen = seen.clone();
        add(&c, id, move |request| {
            let c = c2.clone();
            let seen = seen.clone();
            Box::pin(async move {
                seen.lock().unwrap().push(request.clone());
                let ep = request.episode.as_ref().unwrap();
                if request.agent_id == "a" && request.session_id.is_none() {
                    c.submit_action(
                        "a",
                        &ep.episode_id,
                        EpisodeAction::Ask {
                            agents: vec!["b".into()],
                            body: "question".into(),
                        },
                    )
                    .await?;
                } else {
                    c.submit_action(
                        &request.agent_id,
                        &ep.episode_id,
                        EpisodeAction::Complete {
                            body: if ep.thread.is_some() {
                                "answer".into()
                            } else {
                                "done".into()
                            },
                        },
                    )
                    .await?;
                }
                Ok(done(&request))
            })
        })
        .await;
    }
    hive(&c, "work", &["a", "b"]).await;
    let mut input = message("task", Destination::Hive("work".into()));
    input.only_for = vec!["a".into()];
    c.send_as_host(input).await.unwrap();
    c.run_until_idle().await.unwrap();
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter()
            .any(|r| r.agent_id == "b" && r.episode.as_ref().unwrap().thread.is_some())
    );
    let last_a = seen.iter().rfind(|r| r.agent_id == "a").unwrap();
    assert!(last_a.episode.as_ref().unwrap().brief.contains("answer"));
    assert!(c.lock().unwrap().durable.episodes[0].finished);
}
#[tokio::test]
async fn membership_removed_before_claim_prevents_later_delivery() {
    let c = setup().await;
    add(&c, "a", |_| {
        Box::pin(async { panic!("removed agent must not run") })
    })
    .await;
    hive(&c, "work", &["a"]).await;
    c.send_as_host(message("task", Destination::Hive("work".into())))
        .await
        .unwrap();
    c.advance().await.unwrap();
    c.leave_hive("work", "a").await.unwrap();
    assert_eq!(c.run_until_idle().await.unwrap().completed, 0);
}
mod failures;
mod finalization;
mod lifecycle;
mod observation;
mod privacy;
mod registration;
mod release;
mod review_regressions;
mod scheduling;
mod starters;
mod transactions;

mod options;
mod settings;
