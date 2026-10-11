//! Frozen hive policy and durable scheduled authority regressions.
use super::*;

#[tokio::test]
async fn settings_are_frozen_per_episode_and_survive_restart() {
    let store = Arc::new(MemoryStorage::new());
    let global = CoordinatorOptions {
        round_width: 3,
        ..Default::default()
    };
    let c = Coordinator::new("runtime".into(), store.clone(), global.clone())
        .await
        .unwrap();
    for id in ["a", "b"] {
        add(&c, id, |r| Box::pin(async move { Ok(done(&r)) })).await;
    }
    hive(&c, "one", &["a", "b"]).await;
    hive(&c, "two", &["a", "b"]).await;
    let mut settings = HiveSettings::default();
    settings.roles.insert("b".into(), "reviewer".into());
    settings.options.conduct_policy.turn_wall = 2;
    settings.routing.choice_option_limit = 3;
    c.configure_hive("one", settings.clone()).await.unwrap();
    c.send_as_host(message("first", Destination::Hive("one".into())))
        .await
        .unwrap();
    settings.roles.insert("b".into(), "solver".into());
    settings.options.round_width = 2;
    settings.routing.round_width = 2;
    c.configure_hive("one", settings.clone()).await.unwrap();
    c.configure_hive("two", settings.clone()).await.unwrap();
    c.send_as_host(message("second", Destination::Hive("two".into())))
        .await
        .unwrap();
    let state = c.lock().unwrap().durable.clone();
    assert_eq!(
        state.episodes[0].settings.as_ref().unwrap().roles["b"],
        "reviewer"
    );
    assert_eq!(
        state.episodes[0]
            .settings
            .as_ref()
            .unwrap()
            .options
            .round_width,
        1
    );
    assert_eq!(
        state.episodes[1]
            .settings
            .as_ref()
            .unwrap()
            .options
            .round_width,
        2
    );
    c.leave_hive("one", "b").await.unwrap();
    c.join_hive("one", "b").await.unwrap();
    let restarted = Coordinator::new("runtime".into(), store, global)
        .await
        .unwrap();
    assert_eq!(
        restarted.hive_settings("one").unwrap().unwrap().roles["b"],
        "solver"
    );
    settings.options.round_width = 4;
    assert!(matches!(
        restarted.configure_hive("one", settings).await,
        Err(Error::InvalidOptions)
    ));
}

#[tokio::test]
async fn scheduled_identity_survives_queueing_and_rejects_origin_changes() {
    let store = Arc::new(MemoryStorage::new());
    let c = Coordinator::new(
        "runtime".into(),
        store.clone(),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    add(&c, "a", |r| Box::pin(async move { Ok(done(&r)) })).await;
    hive(&c, "one", &["a"]).await;
    let input = message("scheduled", Destination::Hive("one".into()));
    let receipt = c
        .send_scheduled_as_host("job", input.clone())
        .await
        .unwrap();
    assert_eq!(
        c.send_scheduled_as_host("job", input.clone())
            .await
            .unwrap(),
        receipt
    );
    assert!(matches!(
        c.send_as_host(input.clone()).await,
        Err(Error::MessageConflict(_))
    ));
    assert!(matches!(
        c.send_scheduled_as_host("other", input).await,
        Err(Error::MessageConflict(_))
    ));
    let restarted = Coordinator::new("runtime".into(), store, CoordinatorOptions::default())
        .await
        .unwrap();
    add(&restarted, "a", |r| {
        Box::pin(async move {
            assert_eq!(r.scheduled_job_id.as_deref(), Some("job"));
            Ok(done(&r))
        })
    })
    .await;
    assert!(restarted.run_until_idle().await.unwrap().completed > 0);
}

#[tokio::test]
async fn per_hive_width_conduct_walls_and_roles_drive_actual_turns() {
    let c = Coordinator::new(
        "runtime".into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions {
            round_width: 3,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    for id in ["a", "b"] {
        let seen = seen.clone();
        add(&c, id, move |r| {
            seen.lock().unwrap().push(r.clone());
            Box::pin(async move { Ok(done(&r)) })
        })
        .await;
    }
    for (name, width, wall, role) in [("one", 1, 2, "reviewer"), ("two", 2, 4, "solver")] {
        hive(&c, name, &["a", "b"]).await;
        let mut settings = HiveSettings::default();
        settings.options.round_width = width;
        settings.options.conduct_policy.turn_wall = wall;
        settings.routing.round_width = width;
        settings.roles.insert("b".into(), role.into());
        c.configure_hive(name, settings).await.unwrap();
        c.send_as_host(message(name, Destination::Hive(name.into())))
            .await
            .unwrap();
    }
    c.advance().await.unwrap();
    let state = c.lock().unwrap().durable.clone();
    assert_eq!(state.episodes[0].pending.len(), 1);
    assert_eq!(state.episodes[1].pending.len(), 2);
    c.run_until_idle().await.unwrap();
    let seen = seen.lock().unwrap();
    for (name, count, role) in [("one", 2, "reviewer"), ("two", 4, "solver")] {
        let turns: Vec<_> = seen
            .iter()
            .filter(|r| r.episode.as_ref().unwrap().hive_id == name)
            .collect();
        assert_eq!(turns.len(), count);
        assert!(turns[0].episode.as_ref().unwrap().brief.contains(role));
        assert_eq!(
            turns
                .iter()
                .find(|turn| turn.agent_id == "a")
                .unwrap()
                .teammates
                .iter()
                .find(|t| t.id == "b")
                .unwrap()
                .role
                .as_deref(),
            Some(role)
        );
    }
}

#[tokio::test]
async fn scheduled_authority_is_inherited_by_private_children_and_generated_rows() {
    let c = setup().await;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    for id in ["a", "b"] {
        let c2 = c.clone();
        let seen = seen.clone();
        add(&c, id, move |r| {
            let c = c2.clone();
            let seen = seen.clone();
            Box::pin(async move {
                assert_eq!(r.scheduled_job_id.as_deref(), Some("cron"));
                seen.lock().unwrap().push(r.clone());
                let ep = r.episode.as_ref().unwrap();
                let action = if r.agent_id == "a" && ep.thread.is_none() && r.session_id.is_none() {
                    EpisodeAction::Ask {
                        agents: vec!["b".into()],
                        body: "review".into(),
                    }
                } else {
                    EpisodeAction::Complete {
                        body: "done".into(),
                    }
                };
                c.submit_action(&r.agent_id, &ep.episode_id, action).await?;
                Ok(done(&r))
            })
        })
        .await;
    }
    hive(&c, "one", &["a", "b"]).await;
    let mut input = message("work", Destination::Hive("one".into()));
    input.starters = vec!["a".into()];
    c.send_scheduled_as_host("cron", input).await.unwrap();
    c.run_until_idle().await.unwrap();
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|r| r.agent_id == "b" && r.episode.as_ref().unwrap().thread.is_some())
    );
    assert!(
        c.read_transcript(None)
            .unwrap()
            .iter()
            .all(|m| m.scheduled_job_id.as_deref() == Some("cron"))
    );
}

#[tokio::test]
async fn hive_settings_refuse_unsupported_retention_and_unknown_roles() {
    let c = setup().await;
    hive(&c, "one", &[]).await;
    let mut settings = HiveSettings::default();
    settings.options.retention.delivered = Some(1);
    assert!(matches!(
        c.configure_hive("one", settings).await,
        Err(Error::InvalidOptions)
    ));
    let mut settings = HiveSettings::default();
    settings.roles.insert("unknown".into(), "role".into());
    assert!(matches!(
        c.configure_hive("one", settings).await,
        Err(Error::UnknownAgent(_))
    ));
    assert!(matches!(
        c.hive_settings("missing"),
        Err(Error::UnknownHive(_))
    ));
}

#[tokio::test]
async fn configured_hives_share_the_global_concurrent_turn_cap() {
    let c = Coordinator::new(
        "runtime".into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions {
            round_width: 3,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let permits = Arc::new(tokio::sync::Semaphore::new(0));
    let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
    for id in ["a", "b", "c", "d"] {
        let c2 = c.clone();
        let permits = permits.clone();
        let started = started.clone();
        add(&c, id, move |r| {
            let c = c2.clone();
            let permits = permits.clone();
            let started = started.clone();
            Box::pin(async move {
                started.send(r.agent_id.clone()).unwrap();
                permits.acquire().await.unwrap().forget();
                c.submit_action(
                    &r.agent_id,
                    &r.episode.as_ref().unwrap().episode_id,
                    EpisodeAction::Complete {
                        body: "done".into(),
                    },
                )
                .await?;
                Ok(done(&r))
            })
        })
        .await;
    }
    for (name, members) in [("one", ["a", "b"]), ("two", ["c", "d"])] {
        hive(&c, name, &members).await;
        let mut settings = HiveSettings::default();
        settings.options.round_width = 2;
        settings.routing.round_width = 2;
        c.configure_hive(name, settings).await.unwrap();
        c.send_as_host(message(name, Destination::Hive(name.into())))
            .await
            .unwrap();
    }
    let runner = c.clone();
    let drain = tokio::spawn(async move { runner.run_until_idle().await });
    for _ in 0..3 {
        starts.recv().await.unwrap();
    }
    assert!(starts.try_recv().is_err());
    assert_eq!(c.lock().unwrap().durable.running.len(), 3);
    permits.add_permits(4);
    assert_eq!(drain.await.unwrap().unwrap().completed, 4);
}

#[test]
fn legacy_turns_and_messages_decode_without_new_metadata() {
    let request = TurnRequest {
        turn_id: String::new(),
        scheduled_job_id: None,
        teammates: Vec::new(),
        agent_id: "a".into(),
        session_id: None,
        messages: Vec::new(),
        memberships: Vec::new(),
        episode: None,
        resumption: None,
    };
    let encoded = serde_json::to_value(&request).unwrap();
    assert!(encoded.get("scheduled_job_id").is_none());
    assert!(encoded.get("teammates").is_none());
    assert_eq!(
        serde_json::from_value::<TurnRequest>(encoded).unwrap(),
        request
    );
    let row = Message {
        scheduled_job_id: None,
        message_id: "m".into(),
        sequence: 0,
        sender: "a".into(),
        destination: Destination::Agent("b".into()),
        body: "hi".into(),
        thread: None,
        episode_id: None,
        only_for: Vec::new(),
    };
    let encoded = serde_json::to_value(&row).unwrap();
    assert!(encoded.get("scheduled_job_id").is_none());
    assert_eq!(serde_json::from_value::<Message>(encoded).unwrap(), row);
}

#[tokio::test]
async fn bound_agent_sends_cannot_strip_scheduled_authority() {
    for destination in [
        Destination::Agent("b".into()),
        Destination::Hive("two".into()),
    ] {
        let c = setup().await;
        let writer = c.clone();
        add(&c, "a", move |r| {
            let c = writer.clone();
            let destination = destination.clone();
            Box::pin(async move {
                let mut forwarded = message("forwarded", destination);
                if matches!(forwarded.destination, Destination::Hive(_)) {
                    forwarded.starters = vec!["b".into()];
                }
                c.send(forwarded).await?;
                c.submit_action(
                    "a",
                    &r.episode.as_ref().unwrap().episode_id,
                    EpisodeAction::Complete {
                        body: "done".into(),
                    },
                )
                .await?;
                Ok(done(&r))
            })
        })
        .await;
        let consumer = c.clone();
        add(&c, "b", move |r| {
            let c = consumer.clone();
            Box::pin(async move {
                assert_eq!(r.scheduled_job_id.as_deref(), Some("cron"));
                if let Some(episode) = &r.episode {
                    c.submit_action(
                        "b",
                        &episode.episode_id,
                        EpisodeAction::Complete {
                            body: "done".into(),
                        },
                    )
                    .await?;
                }
                Ok(done(&r))
            })
        })
        .await;
        hive(&c, "one", &["a"]).await;
        hive(&c, "two", &["a", "b"]).await;
        c.send_scheduled_as_host("cron", message("work", Destination::Hive("one".into())))
            .await
            .unwrap();
        c.run_until_idle().await.unwrap();
        assert_eq!(
            c.read_transcript(None)
                .unwrap()
                .iter()
                .find(|m| m.message_id == "forwarded")
                .unwrap()
                .scheduled_job_id
                .as_deref(),
            Some("cron")
        );
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_reopen_retains_frozen_settings_and_scheduled_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hives.sqlite");
    let c = Coordinator::new(
        "runtime".into(),
        Arc::new(crate::SqliteStorage::open(&path).unwrap()),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    for id in ["a", "b"] {
        add(&c, id, |r| Box::pin(async move { Ok(done(&r)) })).await;
    }
    hive(&c, "one", &["a", "b"]).await;
    let mut settings = HiveSettings::default();
    settings.roles.insert("b".into(), "reviewer".into());
    settings.options.conduct_policy.turn_wall = 2;
    c.configure_hive("one", settings).await.unwrap();
    let mut input = message("work", Destination::Hive("one".into()));
    input.starters = vec!["a".into()];
    let receipt = c
        .send_scheduled_as_host("cron", input.clone())
        .await
        .unwrap();
    c.advance().await.unwrap();
    drop(c);
    let c = Coordinator::new(
        "runtime".into(),
        Arc::new(crate::SqliteStorage::open(&path).unwrap()),
        CoordinatorOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        c.hive_settings("one").unwrap().unwrap().roles["b"],
        "reviewer"
    );
    assert_eq!(
        c.send_scheduled_as_host("cron", input).await.unwrap(),
        receipt
    );
    add(&c, "a", |r| {
        Box::pin(async move {
            assert_eq!(r.scheduled_job_id.as_deref(), Some("cron"));
            assert!(r.episode.as_ref().unwrap().brief.contains("reviewer"));
            assert_eq!(r.teammates[0].role.as_deref(), Some("reviewer"));
            Ok(done(&r))
        })
    })
    .await;
    assert_eq!(c.run_until_idle().await.unwrap().completed, 2);
}

#[tokio::test]
async fn broadcast_uses_single_owner_fallback_within_the_configured_routing_ceiling() {
    let c = Coordinator::new(
        "runtime".into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions {
            round_width: 3,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    for id in ["a", "b", "c"] {
        let coordinator = c.clone();
        let seen = seen.clone();
        add(&c, id, move |r| {
            let c = coordinator.clone();
            let seen = seen.clone();
            Box::pin(async move {
                seen.lock().unwrap().push(r.agent_id.clone());
                let action = if r.agent_id == "a" && r.session_id.is_none() {
                    EpisodeAction::Broadcast {
                        body: "delegate this work".into(),
                    }
                } else {
                    EpisodeAction::Complete {
                        body: "done".into(),
                    }
                };
                c.submit_action(&r.agent_id, &r.episode.as_ref().unwrap().episode_id, action)
                    .await?;
                Ok(done(&r))
            })
        })
        .await;
    }
    hive(&c, "one", &["a", "b", "c"]).await;
    let mut settings = HiveSettings::default();
    settings.options.round_width = 2;
    settings.routing.round_width = 1;
    settings.routing.minimum_confidence = tinyhivemind_core::runtime::responder::Probability::ONE;
    c.configure_hive("one", settings).await.unwrap();
    let mut input = message("work", Destination::Hive("one".into()));
    input.starters = vec!["a".into()];
    c.send_as_host(input).await.unwrap();
    c.run_until_idle().await.unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen.contains(&"b".into()));
    assert!(!seen.contains(&"c".into()));
    assert!(c.lock().unwrap().durable.episodes[0].finished);
}
