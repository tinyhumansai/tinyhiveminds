//! Agent registration, messaging, and conducted scheduling.
mod conduct;
mod messaging;
mod observe;
mod scheduler;
mod settings;
pub use settings::HiveSettings;
#[cfg(test)]
mod test;
mod transaction;
mod types;
use crate::{AgentRecord, Commit, Error, Result, Storage, StoredState};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tokio::sync::{Mutex as AsyncMutex, Notify, watch};
pub use types::{
    AgentRegistration, AgentRunner, CoordinatorOptions, Destination, EpisodeAction, EpisodeContext,
    EpisodePhase, EpisodeStatus, HOST_ID, HiveInfo, InterruptedTurn, Message, Receipt, RunReport,
    SendMessage, TurnDisposition, TurnFuture, TurnOutcome, TurnRequest,
};

/// Cloneable shared coordinator; all clones share runners, inboxes and locks.
#[derive(Clone)]
pub struct Coordinator {
    inner: Arc<Inner>,
}
struct Inner {
    runtime_id: String,
    options: CoordinatorOptions,
    storage: Arc<dyn Storage>,
    state: Mutex<LiveState>,
    scheduler: AsyncMutex<()>,
    /// Serializes writers so a commit can await storage outside `state`.
    writer: AsyncMutex<()>,
    /// Latest committed revision, for host observers.
    committed: watch::Sender<u64>,
    notify: Notify,
    shutdown: AtomicBool,
    /// The writer epoch this coordinator claimed in [`Coordinator::new`].
    writer_epoch: u64,
    /// Highest writer epoch the store has reported above ours; nonzero once a
    /// newer coordinator has fenced this one out.
    fenced_by: AtomicU64,
}
/// An interruption recorded while a dropped drain could not await, kept with
/// the reservation it interrupts until a commit persists it.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct DeferredInterruption {
    reason: String,
    /// Delivery sequence the interrupted turn was claiming, if present.
    delivery_sequence: Option<u64>,
    /// Episode the interrupted turn was working on, if present.
    episode_id: Option<String>,
}
struct LiveState {
    durable: StoredState,
    runners: BTreeMap<String, Arc<dyn AgentRunner>>,
    /// Interruptions applied to `durable` that no commit has persisted yet,
    /// keyed by agent with their reservation identity preserved so they are
    /// only reapplied if the same reservation is still running after a
    /// conflict reload.
    unpersisted: BTreeMap<String, DeferredInterruption>,
}
impl std::fmt::Debug for Coordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Coordinator")
            .field("runtime_id", &self.inner.runtime_id)
            .field("options", &self.inner.options)
            .finish_non_exhaustive()
    }
}
impl Coordinator {
    /// Load durable state and record any previously running turns as interrupted.
    /// Host handles must be reattached before pending work can run.
    /// # Errors
    /// Returns invalid identifiers/bounds, storage failures or invalid snapshots.
    pub async fn new(
        runtime_id: String,
        storage: Arc<dyn Storage>,
        options: CoordinatorOptions,
    ) -> Result<Self> {
        identifier(&runtime_id, "runtime id")?;
        if options.round_width == 0
            || options.conduct_policy.turn_wall == 0
            || options.conduct_policy.child_turn_wall == 0
        {
            return Err(Error::InvalidOptions);
        }
        let (durable, writer_epoch) = claim(storage.as_ref()).await?;
        let (committed, _) = watch::channel(durable.revision);
        Ok(Self {
            inner: Arc::new(Inner {
                runtime_id,
                options,
                storage,
                state: Mutex::new(LiveState {
                    durable,
                    runners: BTreeMap::new(),
                    unpersisted: BTreeMap::new(),
                }),
                scheduler: AsyncMutex::new(()),
                writer: AsyncMutex::new(()),
                committed,
                notify: Notify::new(),
                shutdown: AtomicBool::new(false),
                writer_epoch,
                fenced_by: AtomicU64::new(0),
            }),
        })
    }
    /// Identity of the current shared host runtime.
    #[must_use]
    pub fn runtime_id(&self) -> &str {
        &self.inner.runtime_id
    }
    /// Register or reattach a supplied runner from the same runtime.
    /// Repeated registration is idempotent only for the same `Arc` handle.
    /// # Errors
    /// Returns invalid ID, runtime mismatch, handle conflict or storage errors.
    pub async fn register_agent(&self, registration: AgentRegistration) -> Result<()> {
        self.register(registration, None).await
    }
    /// Atomically bind a host conversation before publishing its runner.
    /// Pending recovered work and concurrent schedulers can only claim the
    /// runner after its continuing session is committed. Identical handles
    /// and session bindings are idempotent.
    /// # Errors
    /// Returns invalid IDs, runtime/handle/session conflicts or storage errors.
    /// Failure publishes neither a runner nor a changed session binding.
    pub async fn register_agent_in_session(
        &self,
        registration: AgentRegistration,
        session_id: &str,
    ) -> Result<()> {
        identifier(session_id, "session id")?;
        self.register(registration, Some(session_id)).await
    }
    async fn register(
        &self,
        registration: AgentRegistration,
        session_id: Option<&str>,
    ) -> Result<()> {
        identifier(&registration.agent_id, "agent id")?;
        if registration.runtime_id != self.inner.runtime_id {
            return Err(Error::RuntimeMismatch);
        }
        // The gate keeps the handle check, the commit and the publication one step.
        let gate = self.inner.writer.lock().await;
        {
            let live = self.lock()?;
            if let Some(existing) = live.runners.get(&registration.agent_id) {
                if !Arc::ptr_eq(existing, &registration.runner) {
                    return Err(Error::AgentConflict(registration.agent_id));
                }
                if session_id.is_none()
                    || live
                        .durable
                        .agents
                        .get(&registration.agent_id)
                        .is_some_and(|agent| agent.session_id.as_deref() == session_id)
                {
                    return Ok(());
                }
            }
        }
        self.update_locked(&gate, |next| {
            next.agents
                .entry(registration.agent_id.clone())
                .or_insert_with(AgentRecord::default);
            if let Some(session_id) = session_id {
                bind_session_state(next, &registration.agent_id, session_id)?;
            }
            Ok(())
        })
        .await?;
        // Published only after its session binding is committed and visible.
        self.lock()?
            .runners
            .insert(registration.agent_id, registration.runner);
        self.inner.notify.notify_one();
        Ok(())
    }
    /// Bind a supplied agent's existing session before its first claimed turn.
    /// Identical bindings are idempotent; a continuing session cannot be switched.
    /// # Errors
    /// Returns unknown agent, invalid session identity, conflicting binding or storage errors.
    pub async fn bind_session(&self, agent_id: &str, session_id: &str) -> Result<()> {
        identifier(session_id, "session id")?;
        self.update(|state| bind_session_state(state, agent_id, session_id))
            .await
    }

    /// Create an optionally empty hive; identical definitions are idempotent.
    /// # Errors
    /// Returns malformed membership, unknown agents, conflicting IDs or storage errors.
    pub async fn create_hive(&self, hive: HiveInfo) -> Result<()> {
        identifier(&hive.hive_id, "hive id")?;
        identifier(&hive.name, "hive name")?;
        self.update(|state| {
            let mut unique = BTreeSet::new();
            for member in &hive.members {
                if !state.agents.contains_key(member) {
                    return Err(Error::UnknownAgent(member.clone()));
                }
                if !unique.insert(member) {
                    return Err(Error::DuplicateMember(member.clone()));
                }
            }
            if let Some(existing) = state.hives.get(&hive.hive_id) {
                return if existing == &hive {
                    Ok(())
                } else {
                    Err(Error::HiveConflict(hive.hive_id.clone()))
                };
            }
            state.hives.insert(hive.hive_id.clone(), hive.clone());
            Ok(())
        })
        .await
    }
    /// List current dynamic hive definitions in ID order.
    /// # Errors
    /// Returns a poisoned shared lock error.
    pub fn list_hives(&self) -> Result<Vec<HiveInfo>> {
        Ok(self.lock()?.durable.hives.values().cloned().collect())
    }
    /// List registered durable agent IDs, including handles awaiting reattachment.
    /// # Errors
    /// Returns a poisoned shared lock error.
    pub fn list_agents(&self) -> Result<Vec<String>> {
        Ok(self.lock()?.durable.agents.keys().cloned().collect())
    }
    /// Join an existing hive; repeated joins are idempotent.
    /// # Errors
    /// Returns unknown hive/agent or storage errors.
    pub async fn join_hive(&self, hive_id: &str, agent_id: &str) -> Result<()> {
        self.update(|state| {
            known_agent(state, agent_id)?;
            let hive = state
                .hives
                .get_mut(hive_id)
                .ok_or_else(|| Error::UnknownHive(hive_id.into()))?;
            if !hive.members.iter().any(|id| id == agent_id) {
                hive.members.push(agent_id.into());
            }
            Ok(())
        })
        .await
    }
    /// Leave a hive; active turns keep their captured authorization and history.
    /// # Errors
    /// Returns unknown hive/agent or storage errors.
    pub async fn leave_hive(&self, hive_id: &str, agent_id: &str) -> Result<()> {
        self.update(|state| {
            known_agent(state, agent_id)?;
            let hive = state
                .hives
                .get_mut(hive_id)
                .ok_or_else(|| Error::UnknownHive(hive_id.into()))?;
            hive.members.retain(|id| id != agent_id);
            Ok(())
        })
        .await
    }
    /// Record an action for the caller's currently running episode.
    /// # Errors
    /// Returns stale episode, unauthorized peers or storage errors.
    pub async fn submit_action(
        &self,
        agent_id: &str,
        episode_id: &str,
        action: EpisodeAction,
    ) -> Result<()> {
        self.update(|state| {
            let running = state
                .running
                .get_mut(agent_id)
                .ok_or_else(|| Error::StaleEpisode(episode_id.into()))?;
            let episode = running
                .request
                .episode
                .as_ref()
                .filter(|ep| ep.episode_id == episode_id)
                .ok_or_else(|| Error::StaleEpisode(episode_id.into()))?;
            if let EpisodeAction::Ask { agents, .. } = &action {
                if agents.is_empty() {
                    return Err(Error::InvalidIdentifier("ask recipients"));
                }
                let hive = running
                    .request
                    .memberships
                    .iter()
                    .find(|hive| hive.hive_id == episode.hive_id)
                    .ok_or_else(|| Error::StaleEpisode(episode_id.into()))?;
                let episode_members = state
                    .episodes
                    .iter()
                    .find(|record| record.episode_id == episode_id)
                    .map(|record| &record.hive.members)
                    .ok_or_else(|| Error::StaleEpisode(episode_id.into()))?;
                let mut unique = BTreeSet::new();
                for target in agents {
                    if target == agent_id
                        || !hive.members.contains(target)
                        || !episode_members.contains(target)
                        || !unique.insert(target)
                    {
                        return Err(Error::NotMember {
                            agent_id: target.clone(),
                            hive_id: hive.hive_id.clone(),
                        });
                    }
                }
            }
            running.actions.push(action.clone());
            Ok(())
        })
        .await
    }
    /// Release a parked agent after the host settles its approval.
    /// Equivalent to [`Self::release_with`] without a note.
    /// # Errors
    /// Returns unknown agent, invalid conductor snapshots or storage errors.
    pub async fn release(&self, agent_id: &str) -> Result<()> {
        self.release_with(agent_id, None).await
    }
    /// Release a parked agent, attaching a host note — an approval decision,
    /// say — to the next turn it is claimed for, as
    /// [`TurnRequest::resumption`]. A later note replaces an undelivered one;
    /// `None` leaves an undelivered note in place.
    /// # Errors
    /// Returns unknown agent, invalid conductor snapshots or storage errors.
    pub async fn release_with(&self, agent_id: &str, note: Option<String>) -> Result<()> {
        self.update(|state| {
            known_agent(state, agent_id)?;
            if let Some(agent) = state.agents.get_mut(agent_id) {
                agent.parked = false;
                agent.parked_turn = None;
                if let Some(note) = &note {
                    agent.resumption = Some(note.clone());
                }
            }
            conduct::release(state, agent_id, &self.inner.options)
        })
        .await
    }
    /// Release only the exact completed parked turn captured by a host approval.
    /// The identity check and release share one storage transaction.
    /// # Errors
    /// Unknown agent, running/unparked or mismatched turn, conductor or storage failure.
    pub async fn release_parked(
        &self,
        agent_id: &str,
        turn: &crate::ParkedTurn,
        note: Option<String>,
    ) -> Result<()> {
        self.update(|state| {
            known_agent(state, agent_id)?;
            let agent = state
                .agents
                .get_mut(agent_id)
                .ok_or_else(|| Error::UnknownAgent(agent_id.into()))?;
            if state.running.contains_key(agent_id)
                || !agent.parked
                || agent.parked_turn.as_ref() != Some(turn)
            {
                return Err(Error::InvalidState(
                    "approval does not match a completed parked turn".into(),
                ));
            }
            agent.parked = false;
            agent.parked_turn = None;
            if let Some(note) = &note {
                agent.resumption = Some(note.clone());
            }
            conduct::release(state, agent_id, &self.inner.options)
        })
        .await
    }
    /// Inspect uncertain turn records without replaying them.
    /// # Errors
    /// Returns a poisoned shared lock error.
    pub fn interruptions(&self) -> Result<Vec<InterruptedTurn>> {
        Ok(self.lock()?.durable.interruptions.clone())
    }
    /// Stop new claims, wake waiters, and let active turns return.
    pub fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::Release);
        self.inner.notify.notify_one();
    }
    fn lock(&self) -> Result<MutexGuard<'_, LiveState>> {
        self.inner.state.lock().map_err(|_| Error::Poisoned)
    }
}
fn identifier(value: &str, field: &'static str) -> Result<()> {
    if value.trim().is_empty() || value == HOST_ID {
        Err(Error::InvalidIdentifier(field))
    } else {
        Ok(())
    }
}
fn known_agent(state: &StoredState, id: &str) -> Result<()> {
    if state.agents.contains_key(id) {
        Ok(())
    } else {
        Err(Error::UnknownAgent(id.into()))
    }
}
fn interrupt(state: &mut StoredState, agent_id: &str, reason: &str) {
    let Some(turn) = state.running.remove(agent_id) else {
        return;
    };
    if let Some(sequence) = turn.delivery_sequence {
        for delivery in &mut state.deliveries {
            if delivery.agent_id == agent_id && delivery.sequence == sequence {
                delivery.status = crate::DeliveryStatus::Interrupted;
            }
        }
    }
    let episode_id = turn
        .request
        .episode
        .as_ref()
        .map(|ep| ep.episode_id.clone());
    if let Some(id) = &episode_id {
        for episode in &mut state.episodes {
            if episode.episode_id == *id {
                episode.finished = true;
                episode.failure = Some(reason.into());
                episode.pending.clear();
            }
        }
    }
    state.interruptions.push(InterruptedTurn {
        agent_id: agent_id.into(),
        episode_id,
        message_ids: turn
            .request
            .messages
            .iter()
            .map(|msg| msg.message_id.clone())
            .collect(),
        reason: reason.into(),
    });
}

fn bind_session_state(state: &mut StoredState, agent_id: &str, session_id: &str) -> Result<()> {
    known_agent(state, agent_id)?;
    let agent = state
        .agents
        .get(agent_id)
        .ok_or_else(|| Error::UnknownAgent(agent_id.into()))?;
    if let Some(existing) = &agent.session_id {
        return if existing == session_id {
            Ok(())
        } else {
            Err(Error::SessionConflict(agent_id.into()))
        };
    }
    if state.running.contains_key(agent_id) {
        return Err(Error::SessionConflict(agent_id.into()));
    }
    if let Some(agent) = state.agents.get_mut(agent_id) {
        agent.session_id = Some(session_id.into());
    }
    Ok(())
}
/// Attempts [`claim`] makes before giving up on a store that keeps changing.
const CLAIM_ATTEMPTS: usize = 4;
/// Take ownership of `storage`: load it, advance `writer_epoch`, recover the
/// previous owner's running turns as interruptions, and commit that as one
/// revision. A conflict means another process wrote between the load and the
/// commit, so the claim is retried on a fresh load; the newest claimant wins.
async fn claim(storage: &dyn Storage) -> Result<(StoredState, u64)> {
    let mut attempts = 0;
    loop {
        let mut durable = storage.load().await?;
        let previous = durable.revision;
        let writer_epoch = durable
            .writer_epoch
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        durable.writer_epoch = writer_epoch;
        let agents: Vec<_> = durable.running.keys().cloned().collect();
        for agent in agents {
            interrupt(&mut durable, &agent, "process restarted during turn");
        }
        durable.revision = previous.checked_add(1).ok_or(Error::Exhausted)?;
        let committed = storage
            .commit(Commit {
                expected_revision: previous,
                state: &durable,
                appended: &[],
            })
            .await;
        match committed {
            Ok(()) => return Ok((durable, writer_epoch)),
            Err(Error::RevisionConflict { .. }) if attempts + 1 < CLAIM_ATTEMPTS => {
                attempts += 1;
            }
            Err(error) => return Err(error),
        }
    }
}
