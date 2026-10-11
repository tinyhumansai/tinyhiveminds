//! Shared storage revision, incremental transcript and atomicity contract.
// Panicking assertions are confined to deterministic test fixtures.
#![allow(clippy::unwrap_used, clippy::panic)]
use super::*;
use crate::{Destination, Message, SendMessage};

pub(super) fn row(sequence: u64, accepted: bool) -> TranscriptRow {
    let message = Message {
        scheduled_job_id: None,
        message_id: format!("m{sequence}"),
        sequence,
        sender: "a".into(),
        destination: Destination::Agent("b".into()),
        body: format!("body {sequence}"),
        thread: None,
        episode_id: None,
        only_for: Vec::new(),
    };
    TranscriptRow {
        accepted: accepted.then(|| SendMessage {
            message_id: message.message_id.clone(),
            sender: "a".into(),
            destination: message.destination.clone(),
            body: message.body.clone(),
            thread: None,
            only_for: Vec::new(),
            starters: Vec::new(),
        }),
        message,
    }
}
pub(super) fn state_at(revision: u64) -> StoredState {
    StoredState {
        revision,
        ..StoredState::default()
    }
}
pub(super) async fn commit(
    storage: &dyn Storage,
    expected_revision: u64,
    state: &StoredState,
    appended: &[TranscriptRow],
) -> crate::Result<()> {
    storage
        .commit(Commit {
            expected_revision,
            state,
            appended,
        })
        .await
}
pub(super) async fn contract(storage: &dyn Storage) {
    let mut state = storage.load().await.unwrap();
    let start = state.revision;
    state.revision += 1;
    state.agents.insert(
        "a".into(),
        AgentRecord {
            session_id: Some("session".into()),
            ..AgentRecord::default()
        },
    );
    commit(storage, start, &state, &[row(0, true), row(1, false)])
        .await
        .unwrap();
    assert!(matches!(
        commit(storage, start, &state, &[]).await,
        Err(Error::RevisionConflict { .. })
    ));
    let committed = storage.load().await.unwrap();
    assert_eq!(committed.agents["a"].session_id.as_deref(), Some("session"));
    assert_eq!(committed.messages.len(), 2);
    assert_eq!(committed.accepted.len(), 1);
    assert!(committed.accepted.contains_key("m0"));
    // Only the newly appended row travels; prior rows are never rewritten.
    let mut next = committed.clone();
    next.revision += 1;
    commit(storage, committed.revision, &next, &[row(2, true)])
        .await
        .unwrap();
    let appended = storage.load().await.unwrap();
    assert_eq!(
        appended
            .messages
            .iter()
            .map(|m| m.sequence)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(matches!(
        commit(
            storage,
            appended.revision,
            &state_at(appended.revision + 1),
            &[row(2, false)]
        )
        .await,
        Err(Error::TranscriptOutOfOrder(2))
    ));
    let mut skipped = appended.clone();
    skipped.revision += 2;
    assert!(matches!(
        commit(storage, appended.revision, &skipped, &[]).await,
        Err(Error::InvalidRevision)
    ));
    let unchanged = storage.load().await.unwrap();
    assert_eq!(unchanged.revision, appended.revision);
    assert_eq!(unchanged.messages.len(), 3);
}
#[tokio::test]
async fn memory_obeys_shared_incremental_contract() {
    contract(&MemoryStorage::new()).await;
}
#[tokio::test]
async fn memory_commits_exactly_one_revision_and_rejects_stale_writers() {
    let storage = MemoryStorage::new();
    let mut state = storage.load().await.unwrap();
    assert_eq!(state.revision, 0);
    state.revision = 1;
    commit(&storage, 0, &state, &[]).await.unwrap();
    assert!(commit(&storage, 0, &state, &[]).await.is_err());
    assert_eq!(storage.load().await.unwrap().revision, 1);
    state.revision = 3;
    assert!(commit(&storage, 1, &state, &[]).await.is_err());
    assert_eq!(storage.load().await.unwrap().revision, 1);
}
#[test]
fn the_state_row_never_carries_the_transcript() {
    let mut state = state_at(1);
    let row = row(0, true);
    state.append(row.clone());
    assert_eq!(state.messages, std::slice::from_ref(&row.message));
    let json = serde_json::to_value(&state).unwrap();
    assert!(json.get("messages").is_none());
    assert!(json.get("accepted").is_none());
    let bounded = state.without_transcript();
    assert!(bounded.messages.is_empty() && bounded.accepted.is_empty());
    assert_eq!(state.rows_since(0), [row]);
    assert_eq!(state.rows_since(1).len(), 0);
}
#[test]
fn retention_keeps_recent_settled_episodes_and_deliveries_only() {
    let mut state = StoredState::default();
    for (index, finished) in [true, true, false, true].into_iter().enumerate() {
        state.episodes.push(EpisodeRecord {
            settings: None,
            scheduled_job_id: None,
            episode_id: format!("e{index}"),
            hive: crate::HiveInfo {
                hive_id: "h".into(),
                name: "h".into(),
                description: None,
                members: Vec::new(),
            },
            opened_at: index as u64,
            thread: None,
            starters: Vec::new(),
            conductor: None,
            pending: Vec::new(),
            wave_open: false,
            waiting: false,
            finished,
            failure: None,
        });
    }
    for (sequence, status) in [
        DeliveryStatus::Delivered,
        DeliveryStatus::Interrupted,
        DeliveryStatus::Delivered,
        DeliveryStatus::Pending,
        DeliveryStatus::Delivered,
    ]
    .into_iter()
    .enumerate()
    {
        state.deliveries.push(Delivery {
            sequence: sequence as u64,
            agent_id: "a".into(),
            status,
        });
    }
    let mut unbounded = state.clone();
    RetentionPolicy::default().apply(&mut unbounded);
    assert_eq!(unbounded.episodes.len(), 4);
    assert_eq!(unbounded.deliveries.len(), 5);
    RetentionPolicy {
        settled_episodes: Some(1),
        delivered: Some(1),
        interrupted: None,
        pending_per_agent: None,
    }
    .apply(&mut state);
    assert_eq!(
        state
            .episodes
            .iter()
            .map(|e| e.episode_id.as_str())
            .collect::<Vec<_>>(),
        ["e2", "e3"]
    );
    assert_eq!(
        state
            .deliveries
            .iter()
            .map(|d| d.sequence)
            .collect::<Vec<_>>(),
        [1, 3, 4]
    );
}
#[test]
fn retention_keeps_a_settled_episode_a_running_turn_still_reports_to() {
    let mut state = StoredState::default();
    for id in ["running", "idle"] {
        state.episodes.push(EpisodeRecord {
            settings: None,
            scheduled_job_id: None,
            episode_id: id.into(),
            hive: crate::HiveInfo {
                hive_id: id.into(),
                name: id.into(),
                description: None,
                members: Vec::new(),
            },
            opened_at: 0,
            thread: None,
            starters: Vec::new(),
            conductor: None,
            pending: Vec::new(),
            wave_open: false,
            waiting: false,
            finished: true,
            failure: None,
        });
    }
    state.running.insert(
        "a".into(),
        RunningTurn {
            request: crate::TurnRequest {
                turn_id: String::new(),
                scheduled_job_id: None,
                teammates: Vec::new(),
                agent_id: "a".into(),
                session_id: None,
                messages: Vec::new(),
                memberships: Vec::new(),
                episode: Some(crate::EpisodeContext {
                    episode_id: "running".into(),
                    hive_id: "running".into(),
                    thread: None,
                    brief: String::new(),
                }),
                resumption: None,
            },
            turn: None,
            actions: Vec::new(),
            delivery_sequence: None,
        },
    );
    RetentionPolicy {
        settled_episodes: Some(0),
        delivered: None,
        interrupted: None,
        pending_per_agent: None,
    }
    .apply(&mut state);
    assert_eq!(state.episodes.len(), 1);
    assert_eq!(state.episodes[0].episode_id, "running");
}

#[test]
fn retention_policy_wire_form_and_omitted_defaults_are_stable() {
    let value: crate::RetentionPolicy = serde_json::from_str("{}").unwrap();
    assert_eq!(value, crate::RetentionPolicy::default());
    assert_eq!(
        serde_json::to_value(value).unwrap(),
        serde_json::json!({"settled_episodes":null,"delivered":null,"interrupted":null,"pending_per_agent":null})
    );
}
