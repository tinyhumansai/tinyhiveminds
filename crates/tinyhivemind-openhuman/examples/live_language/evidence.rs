//! Exact completion and audience evidence used by the live host.
use super::Result;
use tinyhivemind_core::{hive::CompletionEpisodeState, runtime::Sequence};
use tinyhivemind_hives::Message;

/// Match the exact assignment completion, episode and author, never another post.
pub(super) fn completion_matches(
    state: &CompletionEpisodeState,
    opened: Sequence,
    messages: &[Message],
    episode: &str,
    seat: &str,
    expected: &str,
) -> bool {
    let completed = state
        .participants
        .iter()
        .find(|p| p.agent_id == seat)
        .and_then(|p| p.assignments.iter().find(|a| a.assigned_at == opened))
        .and_then(|a| a.completed_at);
    completed.is_some_and(|at| {
        messages.iter().any(|m| {
            m.sequence == at.0
                && m.sender == seat
                && m.episode_id.as_deref() == Some(episode)
                && m.body.trim() == expected
        })
    })
}

/// Check both task identities against the other seat’s authorized view.
pub(super) fn private_tasks_hidden(
    tasks: &[(String, u64)],
    mut read: impl FnMut(&str) -> Result<Vec<Message>>,
) -> Result<()> {
    for (seat, sequence) in tasks {
        let other = if seat == "solver" {
            "auditor"
        } else {
            "solver"
        };
        if read(other)?.iter().any(|m| m.sequence == *sequence) {
            return Err(format!("private {seat} task leaked to {other}").into());
        }
    }
    Ok(())
}

/// Keep model-facing seat actors distinct from the privileged host principal.
pub(super) fn validate_seat_actors(seats: &[tinyhivemind_lang::Seat]) -> Result<()> {
    if seats.iter().any(|seat| seat.id == "host") {
        return Err("seat id collides with privileged host principal".into());
    }
    Ok(())
}

/// Verify the durable episode ended successfully and its completion body matches.
pub(super) fn verify_episode(
    state: &tinyhivemind_hives::StoredState,
    opened: u64,
    messages: &[Message],
    seat: &str,
    expected: &str,
) -> Result<()> {
    let episode = state
        .episodes
        .iter()
        .find(|e| e.opened_at == opened && e.finished && e.failure.is_none())
        .ok_or("missing successful finished episode")?;
    let checkpoint = episode
        .conductor
        .as_ref()
        .ok_or("missing completion checkpoint")?;
    if !completion_matches(
        checkpoint.state.episode(),
        Sequence(opened),
        messages,
        &episode.episode_id,
        seat,
        expected,
    ) {
        return Err(format!("native completion did not contain expected total for {seat}").into());
    }
    Ok(())
}
