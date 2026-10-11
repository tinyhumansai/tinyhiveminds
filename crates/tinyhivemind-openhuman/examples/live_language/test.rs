//! Regression checks for live completion and private audience evidence.
use super::{Result, evidence};
use tinyhivemind_core::{
    hive::{CompletionEpisodeState, apply_completion},
    runtime::{Conversation, Sequence},
};
use tinyhivemind_hives::{Destination, Message};

fn message(sequence: u64, sender: &str, episode: &str, body: &str) -> Message {
    Message {
        message_id: format!("message-{sequence}"),
        sequence,
        sender: sender.into(),
        destination: Destination::Hive("invoices".into()),
        body: body.into(),
        thread: None,
        episode_id: Some(episode.into()),
        scheduled_job_id: None,
        only_for: vec![sender.into()],
    }
}
fn finished() -> Result<CompletionEpisodeState> {
    let state = CompletionEpisodeState::opened(
        Conversation {
            desk_id: "invoices".into(),
            desk_name: "Invoices".into(),
            thread_root: None,
        },
        Sequence(10),
        ["solver"],
    )?;
    Ok(apply_completion(&state, "solver", Sequence(12))?)
}
#[test]
fn rejects_a_correct_post_when_native_completion_is_wrong() -> Result<()> {
    let messages = vec![
        message(11, "solver", "episode:10", "33"),
        message(12, "solver", "episode:10", "999"),
    ];
    assert!(!evidence::completion_matches(
        &finished()?,
        Sequence(10),
        &messages,
        "episode:10",
        "solver",
        "33"
    ));
    Ok(())
}
#[test]
fn accepts_only_the_completed_assignment_row_and_actor() -> Result<()> {
    let state = finished()?;
    assert!(evidence::completion_matches(
        &state,
        Sequence(10),
        &[message(12, "solver", "episode:10", "33")],
        "episode:10",
        "solver",
        "33"
    ));
    for wrong in [
        message(13, "solver", "episode:10", "33"),
        message(12, "auditor", "episode:10", "33"),
        message(12, "solver", "episode:9", "33"),
    ] {
        assert!(!evidence::completion_matches(
            &state,
            Sequence(10),
            &[wrong],
            "episode:10",
            "solver",
            "33"
        ));
    }
    Ok(())
}
#[test]
fn rejects_private_tasks_leaking_in_either_direction_by_sequence() {
    let tasks = vec![("solver".into(), 10), ("auditor".into(), 20)];
    for reader in ["solver", "auditor"] {
        let leaked = if reader == "solver" {
            message(20, "host", "episode:20", "not a numeric marker")
        } else {
            message(10, "host", "episode:10", "not a numeric marker")
        };
        let result = evidence::private_tasks_hidden(&tasks, |id| {
            Ok(if id == reader {
                vec![leaked.clone()]
            } else {
                vec![]
            })
        });
        assert!(result.is_err());
    }
}
#[test]
fn permits_views_without_the_other_private_task() -> Result<()> {
    evidence::private_tasks_hidden(&[("solver".into(), 10), ("auditor".into(), 20)], |_| {
        Ok(vec![])
    })
}

#[test]
fn rejects_a_seat_named_after_the_privileged_host() {
    let seats = [tinyhivemind_lang::Seat {
        id: "host".into(),
        ..tinyhivemind_lang::Seat::default()
    }];
    assert!(evidence::validate_seat_actors(&seats).is_err());
}
