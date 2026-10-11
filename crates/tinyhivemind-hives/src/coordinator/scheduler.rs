//! FIFO reservations, concurrent runner invocation, and cancellation recovery.
use super::{
    AgentRunner, Coordinator, Destination, EpisodeContext, RunReport, TurnDisposition, TurnOutcome,
    TurnRequest, conduct, interrupt,
};
use crate::{DeliveryStatus, Error, Result, RunningTurn, StoredState};
use futures::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::Ordering},
};

pub(super) struct Claim {
    runner: Arc<dyn AgentRunner>,
    request: TurnRequest,
}
#[derive(Clone)]
enum Work {
    Direct(usize),
    Episode(usize, usize),
}
/// Interrupted reservations are persisted even if the drain future is dropped.
struct Reservations {
    coordinator: Coordinator,
    agents: BTreeSet<String>,
}
impl Drop for Reservations {
    fn drop(&mut self) {
        // Drop cannot await storage. The interruption is applied to live state
        // now and persisted by the next commit; durable running records remain
        // discoverable by new() if the process stops first.
        self.coordinator
            .interrupt_unpersisted(std::mem::take(&mut self.agents));
    }
}
impl Coordinator {
    /// Drain eligible work, leaving parked or unattached agents queued.
    /// Dropping the future interrupts started turns without replaying their effects.
    /// # Errors
    /// Returns storage or malformed durable-conductor errors; runner failures are
    /// recorded in interruptions and the report while other agents continue.
    pub async fn run_until_idle(&self) -> Result<RunReport> {
        let _scheduler = self.inner.scheduler.lock().await;
        self.flush_unpersisted().await?;
        let mut guard = Reservations {
            coordinator: self.clone(),
            agents: BTreeSet::new(),
        };
        let mut futures = FuturesUnordered::new();
        let mut report = RunReport::default();
        loop {
            let (changed, conductor_failures) = self.advance_report().await?;
            report.failed += conductor_failures;
            if !self.inner.shutdown.load(Ordering::Acquire) {
                let capacity = self.inner.options.round_width.saturating_sub(futures.len());
                for claim in self.claim(capacity).await? {
                    let agent_id = claim.request.agent_id.clone();
                    guard.agents.insert(agent_id.clone());
                    futures.push(async move { (agent_id, claim.runner.run(claim.request).await) });
                }
            }
            if futures.is_empty() {
                if changed && !self.inner.shutdown.load(Ordering::Acquire) {
                    continue;
                }
                break;
            }
            let notification = self.inner.notify.notified();
            tokio::pin!(notification);
            tokio::select! {
                outcome = futures.next() => {
                    if let Some((agent_id, outcome)) = outcome {
                        match self.finish(&agent_id, outcome).await? {
                            TurnDisposition::Completed => report.completed += 1,
                            TurnDisposition::Parked => report.parked += 1,
                            TurnDisposition::Failed(_) => report.failed += 1,
                        }
                        guard.agents.remove(&agent_id);
                    }
                }
                () = &mut notification => {}
            }
        }
        Ok(report)
    }
    /// Wait for new work until shutdown, draining each eligible queue.
    /// # Errors
    /// Returns persistence or malformed conductor-state errors.
    pub async fn run(&self) -> Result<()> {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            // Register before draining so an arrival between idle and await is retained.
            notified.as_mut().enable();
            self.run_until_idle().await?;
            if self.inner.shutdown.load(Ordering::Acquire) {
                return Ok(());
            }
            notified.await;
        }
    }
    #[cfg(test)]
    pub(super) async fn advance(&self) -> Result<bool> {
        Ok(self.advance_report().await?.0)
    }
    async fn advance_report(&self) -> Result<(bool, usize)> {
        let _gate = self.inner.writer.lock().await;
        {
            let snapshot = self.snapshot()?;
            let original = &snapshot.base;
            let mut next = original.clone();
            conduct::prepare(&mut next, &self.inner.options).await?;
            if next.messages.len() == original.messages.len()
                && serde_json::to_vec(original)? == serde_json::to_vec(&next)?
            {
                return Ok((false, 0));
            }
            let failures = next
                .episodes
                .iter()
                .filter(|episode| {
                    episode.failure.is_some()
                        && original
                            .episodes
                            .iter()
                            .find(|old| old.episode_id == episode.episode_id)
                            .is_none_or(|old| old.failure.is_none())
                })
                .count();
            self.persist(&snapshot, next).await?;
            Ok((true, failures))
        }
    }
    pub(super) async fn claim(&self, capacity: usize) -> Result<Vec<Claim>> {
        if capacity == 0 {
            return Ok(Vec::new());
        }
        let gate = self.inner.writer.lock().await;
        // Registration also holds the gate, so this handle set stays current.
        let runners = self.lock()?.runners.clone();
        self.transact(&gate, |next| {
            let claims = self.reserve(next, &runners, capacity)?;
            Ok((claims.0 || !claims.1.is_empty(), claims.1))
        })
        .await
    }
    /// Select and durably reserve up to `capacity` claims on `next`.
    /// Returns whether pending seats were pruned, and the claims.
    fn reserve(
        &self,
        next: &mut StoredState,
        runners: &BTreeMap<String, Arc<dyn AgentRunner>>,
        capacity: usize,
    ) -> Result<(bool, Vec<Claim>)> {
        let pruned = conduct::prune_pending(next, &self.inner.options)?;
        let candidates = candidates(next);
        let mut selected = BTreeSet::new();
        let mut claims = Vec::new();
        // Pending positions are removed by seat after selection; saved indices
        // remain stable while this pass captures all proposed work.
        let mut removals = Vec::new();
        for (_, agent_id, work) in candidates {
            if claims.len() >= capacity {
                break;
            }
            if selected.contains(&agent_id)
                || next.running.contains_key(&agent_id)
                || next.agents.get(&agent_id).is_some_and(|agent| agent.parked)
            {
                continue;
            }
            let Some(runner) = runners.get(&agent_id).cloned() else {
                continue;
            };
            let memberships = next
                .hives
                .values()
                .filter(|hive| hive.members.contains(&agent_id))
                .cloned()
                .collect();
            let (messages, episode, turn, delivery_sequence, scheduled_job_id, teammates) =
                match work {
                    Work::Direct(index) => {
                        let delivery = &next.deliveries[index];
                        let message = next
                            .messages
                            .iter()
                            .find(|msg| msg.sequence == delivery.sequence)
                            .cloned()
                            .ok_or_else(|| {
                                Error::InvalidState("direct delivery missing message".into())
                            })?;
                        let sequence = delivery.sequence;
                        next.deliveries[index].status = DeliveryStatus::Running;
                        let origin = message.scheduled_job_id.clone();
                        (
                            vec![message],
                            None,
                            None,
                            Some(sequence),
                            origin,
                            Vec::new(),
                        )
                    }
                    Work::Episode(index, turn_index) => {
                        let turn = next.episodes[index].pending[turn_index].clone();
                        let (messages, brief) =
                            conduct::open(next, index, &turn, &self.inner.options)?;
                        let record = &next.episodes[index];
                        let context = EpisodeContext {
                            episode_id: record.episode_id.clone(),
                            hive_id: record.hive.hive_id.clone(),
                            thread: turn.thread().map(|root| root.0).or(record.thread),
                            brief,
                        };
                        removals.push((index, agent_id.clone()));
                        (
                            messages,
                            Some(context),
                            Some(turn),
                            None,
                            record.scheduled_job_id.clone(),
                            conduct::teammates_for(record, &agent_id),
                        )
                    }
                };
            let (session_id, resumption) = take_continuation(next, &agent_id);
            let request = TurnRequest {
                turn_id: format!("{}:{}:{agent_id}", next.writer_epoch, next.revision),
                scheduled_job_id,
                teammates,
                agent_id: agent_id.clone(),
                session_id,
                messages,
                memberships,
                episode,
                resumption,
            };
            next.running.insert(
                agent_id.clone(),
                RunningTurn {
                    request: request.clone(),
                    turn,
                    actions: Vec::new(),
                    delivery_sequence,
                },
            );
            selected.insert(agent_id);
            claims.push(Claim { runner, request });
        }
        for (index, agent_id) in removals {
            next.episodes[index]
                .pending
                .retain(|turn| turn.seat != agent_id);
        }
        Ok((pruned, claims))
    }
    async fn finish(
        &self,
        agent_id: &str,
        outcome: Result<TurnOutcome>,
    ) -> Result<TurnDisposition> {
        let outcome = outcome.map_err(|error| error.to_string());
        self.update(|state| {
            let outcome = match &outcome {
                Ok(outcome) => outcome.clone(),
                Err(error) => {
                    interrupt(state, agent_id, error);
                    return Ok(TurnDisposition::Failed(error.clone()));
                }
            };
            if outcome.session_id.trim().is_empty() {
                interrupt(state, agent_id, "runner returned empty session identity");
                return Ok(TurnDisposition::Failed(
                    "runner returned empty session identity".into(),
                ));
            }
            if state
                .running
                .get(agent_id)
                .and_then(|run| run.request.session_id.as_ref())
                .is_some_and(|session| *session != outcome.session_id)
            {
                interrupt(
                    state,
                    agent_id,
                    "runner changed continuing session identity",
                );
                return Ok(TurnDisposition::Failed(
                    "runner changed continuing session identity".into(),
                ));
            }
            if !state.running.contains_key(agent_id) {
                return Err(Error::InvalidState(
                    "runner returned without reservation".into(),
                ));
            }
            let agent = state
                .agents
                .get_mut(agent_id)
                .ok_or_else(|| Error::UnknownAgent(agent_id.into()))?;
            // A completed host turn has already committed its conversation,
            // even when finalization rejects acknowledgements and episode actions.
            agent.session_id = Some(outcome.session_id.clone());
            if let TurnDisposition::Failed(reason) = &outcome.disposition {
                interrupt(state, agent_id, reason);
                return Ok(outcome.disposition);
            }
            let running = state
                .running
                .remove(agent_id)
                .ok_or_else(|| Error::InvalidState("runner returned without reservation".into()))?;
            let agent = state
                .agents
                .get_mut(agent_id)
                .ok_or_else(|| Error::UnknownAgent(agent_id.into()))?;
            record_parked(agent, &outcome, &running.request);
            if let Some(sequence) = running.delivery_sequence {
                for delivery in &mut state.deliveries {
                    if delivery.sequence == sequence && delivery.agent_id == agent_id {
                        delivery.status = if outcome.disposition == TurnDisposition::Parked {
                            DeliveryStatus::Pending
                        } else {
                            DeliveryStatus::Delivered
                        };
                    }
                }
                if let Some(body) = &outcome.reply {
                    let sequence = super::messaging::next_sequence(state)?;
                    state.messages.push(super::Message {
                        scheduled_job_id: running.request.scheduled_job_id.clone(),
                        message_id: format!("hivemind:event:{sequence}"),
                        sequence,
                        sender: agent_id.into(),
                        destination: Destination::Agent(running.request.messages[0].sender.clone()),
                        body: body.clone(),
                        thread: None,
                        episode_id: None,
                        only_for: Vec::new(),
                    });
                }
            } else if let (Some(episode), Some(turn)) = (&running.request.episode, &running.turn) {
                let index = state
                    .episodes
                    .iter()
                    .position(|record| record.episode_id == episode.episode_id)
                    .ok_or_else(|| Error::StaleEpisode(episode.episode_id.clone()))?;
                conduct::record(
                    state,
                    index,
                    turn,
                    running.actions,
                    &outcome,
                    &self.inner.options,
                )?;
            }
            Ok(outcome.disposition)
        })
        .await
    }
}

fn candidates(state: &crate::StoredState) -> Vec<(u64, String, Work)> {
    let mut candidates = Vec::new();
    for (index, delivery) in state.deliveries.iter().enumerate() {
        if delivery.status == DeliveryStatus::Pending {
            candidates.push((
                delivery.sequence,
                delivery.agent_id.clone(),
                Work::Direct(index),
            ));
        }
    }
    for (index, episode) in state.episodes.iter().enumerate() {
        if episode.finished {
            continue;
        }
        for (turn_index, turn) in episode.pending.iter().enumerate() {
            candidates.push((
                episode.opened_at,
                turn.seat.clone(),
                Work::Episode(index, turn_index),
            ));
        }
    }
    candidates.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    candidates
}

/// Take the one-shot release note while preserving the continuing session.
fn take_continuation(state: &mut StoredState, agent_id: &str) -> (Option<String>, Option<String>) {
    state
        .agents
        .get_mut(agent_id)
        .map_or((None, None), |agent| {
            (agent.session_id.clone(), agent.resumption.take())
        })
}

/// Retain exact parked provenance for transactional approval release.
fn record_parked(
    agent: &mut crate::AgentRecord,
    outcome: &TurnOutcome,
    request: &super::TurnRequest,
) {
    agent.parked = outcome.disposition == TurnDisposition::Parked;
    agent.parked_turn = agent.parked.then(|| crate::ParkedTurn {
        turn_id: request.turn_id.clone(),
        session_id: outcome.session_id.clone(),
        episode_id: request
            .episode
            .as_ref()
            .map(|episode| episode.episode_id.clone()),
        message_ids: request
            .messages
            .iter()
            .map(|message| message.message_id.clone())
            .collect(),
        scheduled_job_id: request.scheduled_job_id.clone(),
    });
}
