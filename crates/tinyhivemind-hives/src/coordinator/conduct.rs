//! Rehydrate the real driver and conductor around each durable transition.
use super::messaging::{narrow_readers, next_sequence, visible_in};
use super::{
    CoordinatorOptions, Destination, EpisodeAction, HiveInfo, Message, TurnDisposition, TurnOutcome,
};
use crate::{EpisodeRecord, Error, Result, StoredState};
use std::collections::BTreeSet;
use tinyhivemind_core::{
    driver::{
        AgentBinding, BoundAgent, BoundHive, BroadcastRouting, CompletionDriver, Conductor, Door,
        HiveGraph, Step, Turn,
    },
    embed::{RouteCandidate, RoutingPolicy},
    runtime::{
        Sequence,
        desk::{Desk, ResponderMode},
        responder::Probability,
        speech::{ToolCall, Utterance},
    },
};
#[derive(Clone, Debug)]
struct Seat(String);
impl BoundAgent for Seat {
    fn runtime_id(&self) -> &str {
        &self.0
    }
}
struct Environment {
    hive: BoundHive<Seat>,
    routing: RoutingPolicy,
}
impl Environment {
    fn new(
        hive: &HiveInfo,
        options: &CoordinatorOptions,
        settings: Option<&crate::HiveSettings>,
    ) -> Result<Self> {
        let graph = HiveGraph::new(
            Desk {
                id: hive.hive_id.clone(),
                name: hive.name.clone(),
                description: hive.description.clone(),
                members: hive.members.clone(),
                responder_mode: ResponderMode::Auto,
            },
            hive.members
                .iter()
                .map(|id| RouteCandidate {
                    id: id.clone(),
                    label: id.clone(),
                    role: settings.and_then(|settings| settings.roles.get(id).cloned()),
                    description: None,
                    capabilities: Vec::new(),
                    learned_topics: Vec::new(),
                    available: true,
                })
                .collect(),
        );
        let bound = BoundHive::new(
            graph,
            hive.members
                .iter()
                .map(|id| AgentBinding::new(id, Seat(id.clone())))
                .collect(),
        )?;
        let mut routing = settings.map_or(
            RoutingPolicy {
                minimum_confidence: Probability::ZERO,
                high_impact_minimum_confidence: Probability::ONE,
                clarification_threshold: Probability::ONE,
                high_impact_threshold: Probability::ONE,
                round_width: options.round_width,
                choice_option_limit: 8,
            },
            |settings| settings.routing.clone(),
        );
        routing.round_width = routing.round_width.min(options.round_width);
        Ok(Self {
            hive: bound,
            routing,
        })
    }
    fn driver(&self, options: &CoordinatorOptions) -> Result<CompletionDriver<'_, Seat>> {
        Ok(CompletionDriver::new(&self.hive, options.round_width)?
            .with_broadcast_budget(options.broadcast_budget))
    }
    fn conductor<'a>(
        &'a self,
        driver: &'a CompletionDriver<'a, Seat>,
        episode: &EpisodeRecord,
        options: &CoordinatorOptions,
    ) -> Result<Conductor<'a, Seat>> {
        let routing = BroadcastRouting {
            primary: None,
            reasoning: None,
            policy: &self.routing,
            roster_version: 0,
            thread_context: &[],
        };
        Ok(match &episode.conductor {
            Some(snapshot) => {
                Conductor::resume(driver, routing, options.conduct_policy, snapshot.clone())?
            }
            None => Conductor::open(
                driver,
                routing,
                options.conduct_policy,
                Door {
                    chat: episode.hive.hive_id.clone(),
                    desk_name: episode.hive.name.clone(),
                    members: episode.hive.members.clone(),
                    starters: episode.starters.clone(),
                    opened_at: Sequence(episode.opened_at),
                },
            )?,
        })
    }
}
fn checkpoint(conductor: &Conductor<'_, Seat>, episode: &mut EpisodeRecord) -> Result<()> {
    episode.conductor = Some(
        conductor
            .snapshot()
            .ok_or_else(|| Error::InvalidState("unreported conductor commit".into()))?,
    );
    episode.finished = conductor.finished();
    Ok(())
}
fn append(
    state: &mut StoredState,
    episode: &EpisodeRecord,
    sender: String,
    body: String,
    thread: Option<Sequence>,
    mut only_for: Vec<String>,
    inherit_audience: bool,
) -> Result<Sequence> {
    let destination = Destination::Hive(episode.hive.hive_id.clone());
    let thread = thread.map(|s| s.0).or(episode.thread);
    if inherit_audience && only_for.is_empty() && thread == episode.thread {
        only_for = narrow_readers(state, &destination, Some(episode.opened_at), only_for)?;
    }
    let only_for = narrow_readers(state, &destination, thread, only_for)?;
    let sequence = next_sequence(state)?;
    state.messages.push(Message {
        scheduled_job_id: episode.scheduled_job_id.clone(),
        message_id: format!("hivemind:event:{sequence}"),
        sequence,
        sender,
        destination,
        body,
        thread,
        episode_id: Some(episode.episode_id.clone()),
        only_for,
    });
    Ok(Sequence(sequence))
}
fn note(state: &mut StoredState, episode: &EpisodeRecord, step: Step) -> Result<()> {
    if let Step::Note(note) = step {
        append(
            state,
            episode,
            format!("hivemind:hive:{}", episode.hive.hive_id),
            note.body,
            note.thread,
            note.only_for.into_iter().collect(),
            true,
        )?;
    }
    Ok(())
}
async fn drain(
    state: &mut StoredState,
    episode: &EpisodeRecord,
    conductor: &mut Conductor<'_, Seat>,
) -> Result<()> {
    while let Some(step) = conductor.step()? {
        if let Step::Commit(commit) = step {
            let sequence = append(
                state,
                episode,
                commit.author,
                commit.utterance.message().into(),
                commit.thread,
                commit.only_for,
                !matches!(
                    commit.utterance,
                    Utterance::Ask { .. } | Utterance::Broadcast { .. }
                ),
            )?;
            conductor.committed(sequence).await?;
        } else {
            note(state, episode, step)?;
        }
    }
    Ok(())
}
/// Advance idle episodes and propose each next actual conductor wave.
pub(super) async fn prepare(state: &mut StoredState, options: &CoordinatorOptions) -> Result<()> {
    let mut active_hives = BTreeSet::new();
    for index in 0..state.episodes.len() {
        let mut episode = state.episodes[index].clone();
        if episode.finished || !active_hives.insert(episode.hive.hive_id.clone()) {
            continue;
        }
        prune_removed(state, &mut episode, options)?;
        state.episodes[index] = episode.clone();
        let running = state.running.values().any(|run| {
            run.request
                .episode
                .as_ref()
                .is_some_and(|ep| ep.hive_id == episode.hive.hive_id)
        });
        if running || !episode.pending.is_empty() || episode.waiting {
            continue;
        }
        let effective = episode
            .settings
            .as_ref()
            .map(|settings| settings.options.clone());
        let options = effective.as_ref().unwrap_or(options);
        let environment = Environment::new(&episode.hive, options, episode.settings.as_ref())?;
        let driver = environment.driver(options)?;
        let mut conductor = environment.conductor(&driver, &episode, options)?;
        if episode.wave_open {
            if let Err(error) = drain(state, &episode, &mut conductor).await {
                episode.finished = true;
                episode.failure = Some(error.to_string());
                episode.pending.clear();
                episode.wave_open = false;
                episode.conductor = conductor.snapshot();
                state.episodes[index] = episode;
                continue;
            }
            episode.wave_open = false;
        }
        if conductor.finished() {
            checkpoint(&conductor, &mut episode)?;
            state.episodes[index] = episode;
            continue;
        }
        for step in conductor.begin_wave() {
            note(state, &episode, step)?;
        }
        match conductor.turns() {
            Ok(turns) => {
                episode.wave_open = true;
                for turn in turns {
                    let current = state
                        .hives
                        .get(&episode.hive.hive_id)
                        .is_some_and(|h| h.members.contains(&turn.seat));
                    if current {
                        episode.pending.push(turn);
                    } else {
                        conductor.open_turn(
                            &turn,
                            state.messages.last().map(|msg| Sequence(msg.sequence)),
                            Vec::new(),
                            |_| Vec::new(),
                        );
                        conductor.record(
                            &turn,
                            [ToolCall::Speak(Utterance::CompleteEpisode {
                                message: "membership removed before delivery".into(),
                            })],
                        );
                    }
                }
                // An empty parked wave must wait for an explicit release.
                episode.waiting = episode.pending.is_empty() && !conductor.parked().is_empty();
                checkpoint(&conductor, &mut episode)?;
            }
            Err(error) => {
                // The conductor still reports itself unfinished after a stall,
                // so settle after checkpointing; otherwise the episode is
                // re-prepared, and fails again, on every pass.
                checkpoint(&conductor, &mut episode)?;
                episode.finished = true;
                episode.failure = Some(error.to_string());
                episode.pending.clear();
                episode.wave_open = false;
            }
        }
        state.episodes[index] = episode;
    }
    Ok(())
}
/// Open the selected wave turn, capture visible messages and the real brief.
pub(super) fn open(
    state: &mut StoredState,
    index: usize,
    turn: &Turn,
    options: &CoordinatorOptions,
) -> Result<(Vec<Message>, String)> {
    let mut episode = state.episodes[index].clone();
    let effective = episode
        .settings
        .as_ref()
        .map(|settings| settings.options.clone());
    let options = effective.as_ref().unwrap_or(options);
    let environment = Environment::new(&episode.hive, options, episode.settings.as_ref())?;
    let driver = environment.driver(options)?;
    let mut conductor = environment.conductor(&driver, &episode, options)?;
    let messages: Vec<_> = state
        .messages
        .iter()
        .filter(|msg| {
            msg.destination == Destination::Hive(episode.hive.hive_id.clone())
                && msg.episode_id.as_ref() == Some(&episode.episode_id)
                && visible_in(state, msg, &turn.seat)
                && turn.since.is_none_or(|since| msg.sequence > since.0)
                && match turn.thread() {
                    Some(root) => msg.sequence == root.0 || msg.thread == Some(root.0),
                    None => msg.thread == episode.thread || msg.sequence == episode.opened_at,
                }
        })
        .cloned()
        .collect();
    let latest = messages
        .last()
        .map(|message| Sequence(message.sequence))
        .or(Some(Sequence(episode.opened_at)));
    let rows = messages
        .iter()
        .map(|msg| format!("@{}: {}", msg.sender, msg.body))
        .collect();
    let mut brief = conductor.open_turn(turn, latest, rows, |root| {
        state
            .messages
            .iter()
            .filter(|msg| {
                msg.destination == Destination::Hive(episode.hive.hive_id.clone())
                    && visible_in(state, msg, &turn.seat)
                    && (msg.sequence == root.0 || msg.thread == Some(root.0))
            })
            .map(|msg| format!("@{}: {}", msg.sender, msg.body))
            .collect()
    });
    if turn.thread().is_none()
        && let Some(root) = episode.thread
    {
        brief.channel = tinyhivemind_core::driver::Channel::Thread {
            root: Sequence(root),
            others: episode
                .hive
                .members
                .iter()
                .filter(|member| *member != &turn.seat)
                .cloned()
                .collect(),
            opened_it: false,
        };
    }
    let mut brief = brief.render();
    let teammates = teammates(&episode);
    for teammate in &teammates {
        if let Some(role) = &teammate.role {
            brief.push_str("\n@");
            brief.push_str(&teammate.id);
            brief.push_str(": ");
            brief.push_str(role);
        }
    }
    checkpoint(&conductor, &mut episode)?;
    state.episodes[index] = episode;
    Ok((messages, brief))
}
pub(super) fn record(
    state: &mut StoredState,
    index: usize,
    turn: &Turn,
    actions: Vec<EpisodeAction>,
    outcome: &TurnOutcome,
    options: &CoordinatorOptions,
) -> Result<()> {
    let mut episode = state.episodes[index].clone();
    if episode.finished {
        return Ok(());
    }
    let effective = episode
        .settings
        .as_ref()
        .map(|settings| settings.options.clone());
    let options = effective.as_ref().unwrap_or(options);
    let environment = Environment::new(&episode.hive, options, episode.settings.as_ref())?;
    let driver = environment.driver(options)?;
    let mut conductor = environment.conductor(&driver, &episode, options)?;
    let mut calls: Vec<_> = actions
        .into_iter()
        .map(|action| {
            ToolCall::Speak(match action {
                EpisodeAction::Post { body } => Utterance::Post { message: body },
                EpisodeAction::Ask { agents, body } => Utterance::Ask {
                    to: agents,
                    message: body,
                },
                EpisodeAction::Broadcast { body } => Utterance::Broadcast { message: body },
                EpisodeAction::Complete { body } => Utterance::CompleteEpisode { message: body },
            })
        })
        .collect();
    if let Some(reply) = &outcome.reply {
        calls.push(ToolCall::Speak(Utterance::Post {
            message: reply.clone(),
        }));
    }
    if outcome.disposition == TurnDisposition::Parked {
        conductor.record_parked(turn, calls);
    } else {
        conductor.record(turn, calls);
    }
    checkpoint(&conductor, &mut episode)?;
    state.episodes[index] = episode;
    Ok(())
}
pub(super) fn release(
    state: &mut StoredState,
    agent_id: &str,
    options: &CoordinatorOptions,
) -> Result<()> {
    for index in 0..state.episodes.len() {
        let mut episode = state.episodes[index].clone();
        if episode.finished
            || episode.conductor.is_none()
            || !episode.hive.members.iter().any(|id| id == agent_id)
        {
            continue;
        }
        let effective = episode
            .settings
            .as_ref()
            .map(|settings| settings.options.clone());
        let options = effective.as_ref().unwrap_or(options);
        let environment = Environment::new(&episode.hive, options, episode.settings.as_ref())?;
        let driver = environment.driver(options)?;
        let mut conductor = environment.conductor(&driver, &episode, options)?;
        conductor.resume_seat(agent_id);
        episode.waiting = false;
        checkpoint(&conductor, &mut episode)?;
        state.episodes[index] = episode;
    }
    Ok(())
}

fn prune_removed(
    state: &StoredState,
    episode: &mut EpisodeRecord,
    options: &CoordinatorOptions,
) -> Result<()> {
    let removed: Vec<_> = episode
        .pending
        .iter()
        .filter(|turn| {
            !state
                .hives
                .get(&episode.hive.hive_id)
                .is_some_and(|hive| hive.members.contains(&turn.seat))
        })
        .cloned()
        .collect();
    if !removed.is_empty() {
        let effective = episode
            .settings
            .as_ref()
            .map(|settings| settings.options.clone());
        let options = effective.as_ref().unwrap_or(options);
        let environment = Environment::new(&episode.hive, options, episode.settings.as_ref())?;
        let driver = environment.driver(options)?;
        let mut conductor = environment.conductor(&driver, episode, options)?;
        for turn in &removed {
            conductor.open_turn(
                turn,
                state.messages.last().map(|msg| Sequence(msg.sequence)),
                Vec::new(),
                |_| Vec::new(),
            );
            conductor.record(
                turn,
                [ToolCall::Speak(Utterance::CompleteEpisode {
                    message: "membership removed before delivery".into(),
                })],
            );
        }
        episode
            .pending
            .retain(|turn| !removed.iter().any(|removed| removed.seat == turn.seat));
        checkpoint(&conductor, episode)?;
    }
    Ok(())
}

/// Revalidate pending reservations under the same lock that starts runners.
pub(super) fn prune_pending(state: &mut StoredState, options: &CoordinatorOptions) -> Result<bool> {
    let mut changed = false;
    for index in 0..state.episodes.len() {
        let mut episode = state.episodes[index].clone();
        let previous = episode.pending.len();
        prune_removed(state, &mut episode, options)?;
        changed |= previous != episode.pending.len();
        state.episodes[index] = episode;
    }
    Ok(changed)
}

/// Explicit host-neutral teammate payload, using the frozen episode roles.
pub(super) fn teammates(
    episode: &EpisodeRecord,
) -> Vec<tinyhivemind_core::runtime::BriefedTeammate> {
    episode
        .hive
        .members
        .iter()
        .map(|id| tinyhivemind_core::runtime::BriefedTeammate {
            id: id.clone(),
            label: id.clone(),
            role: episode
                .settings
                .as_ref()
                .and_then(|settings| settings.roles.get(id).cloned()),
            description: None,
        })
        .collect()
}

#[cfg(test)]
mod test;

/// Visible teammates exclude the viewing agent.
pub(super) fn teammates_for(
    episode: &EpisodeRecord,
    agent_id: &str,
) -> Vec<tinyhivemind_core::runtime::BriefedTeammate> {
    teammates(episode)
        .into_iter()
        .filter(|teammate| teammate.id != agent_id)
        .collect()
}
