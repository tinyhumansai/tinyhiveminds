//! Serializable snapshot format; no live handles or callbacks.
use crate::{EpisodeAction, HiveInfo, InterruptedTurn, Message, SendMessage, TurnRequest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tinyhivemind_core::driver::{ConductorState, Turn};

/// Versioned transactional snapshot shared by all storage implementations.
///
/// The serialized form is the bounded *state row*: the transcript and the
/// retry payloads accepted with it are skipped by serde and travel as
/// append-only [`TranscriptRow`]s instead, so the row a store rewrites on every
/// commit does not grow with the conversation. [`crate::Storage::load`]
/// reassembles both with [`StoredState::append`].
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StoredState {
    /// Explicit per-hive policy and retained membership roles.
    #[serde(default)]
    pub hive_settings: BTreeMap<String, crate::HiveSettings>,
    /// CAS revision; each commit advances exactly one.
    pub revision: u64,
    /// Writer epoch: claimed by exactly one live Coordinator at a time. A
    /// Coordinator increments this on startup (`Coordinator::new`) to fence out
    /// any previous owner. Commits from lower epochs are rejected with
    /// `Error::Fenced`. Absent from snapshots written before fencing existed,
    /// which decode as epoch zero.
    #[serde(default)]
    pub writer_epoch: u64,
    /// Next global message sequence.
    pub next_sequence: u64,
    /// Dynamic definitions and current memberships.
    pub hives: BTreeMap<String, HiveInfo>,
    /// Stable agent identities and continuing session records.
    pub agents: BTreeMap<String, AgentRecord>,
    /// Ordered attributed transcript; persisted as transcript rows.
    #[serde(skip)]
    pub messages: Vec<Message>,
    /// Original caller payloads for exact retry comparison; persisted with
    /// the transcript row of the message they produced.
    #[serde(skip)]
    pub accepted: BTreeMap<String, SendMessage>,
    /// Direct delivery queues and acknowledgements.
    pub deliveries: Vec<Delivery>,
    /// Ordered hive episodes with exact core checkpoints.
    pub episodes: Vec<EpisodeRecord>,
    /// Durably claimed turns whose effects may already have begun.
    pub running: BTreeMap<String, RunningTurn>,
    /// Recovery and cancellation records, never retried automatically.
    pub interruptions: Vec<InterruptedTurn>,
}
impl StoredState {
    /// Append one loaded transcript row, restoring its accepted payload.
    /// Storage implementations call this from `load`, in sequence order.
    pub fn append(&mut self, row: TranscriptRow) {
        if let Some(accepted) = row.accepted {
            self.accepted.insert(accepted.message_id.clone(), accepted);
        }
        self.messages.push(row.message);
    }
    /// Transcript rows appended after the first `len` messages.
    pub(crate) fn rows_since(&self, len: usize) -> Vec<TranscriptRow> {
        self.messages
            .get(len..)
            .unwrap_or_default()
            .iter()
            .map(|message| TranscriptRow {
                accepted: self.accepted.get(&message.message_id).cloned(),
                message: message.clone(),
            })
            .collect()
    }
    /// A copy of the bounded state row, without cloning the transcript.
    pub(crate) fn without_transcript(&self) -> Self {
        Self {
            revision: self.revision,
            hive_settings: self.hive_settings.clone(),
            writer_epoch: self.writer_epoch,
            next_sequence: self.next_sequence,
            hives: self.hives.clone(),
            agents: self.agents.clone(),
            messages: Vec::new(),
            accepted: BTreeMap::new(),
            deliveries: self.deliveries.clone(),
            episodes: self.episodes.clone(),
            running: self.running.clone(),
            interruptions: self.interruptions.clone(),
        }
    }
}
/// One append-only transcript row: a message and, when a caller submitted
/// it, the exact request accepted for retry comparison.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TranscriptRow {
    /// The durable attributed message.
    pub message: Message,
    /// Caller payload; absent for coordinator-generated events and replies.
    pub accepted: Option<SendMessage>,
}
/// Bounds on the settled records the state row keeps.
///
/// The default keeps everything, which is the behaviour before retention
/// existed. The transcript is never pruned: it is append-only by contract.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionPolicy {
    /// Most recent finished episodes to keep; `None` keeps all. An episode a
    /// running turn still reports to is always kept.
    pub settled_episodes: Option<usize>,
    /// Most recent acknowledged ([`DeliveryStatus::Delivered`]) deliveries to
    /// keep; `None` keeps all. Pending and interrupted deliveries are kept.
    pub delivered: Option<usize>,
    /// Most recent terminal interruption records to keep; `None` keeps all.
    /// This bounds both [`DeliveryStatus::Interrupted`] deliveries and
    /// [`InterruptedTurn`] records so long-running hosts do not grow the
    /// state row unboundedly.
    pub interrupted: Option<usize>,
    /// Most undelivered ([`DeliveryStatus::Pending`]) direct messages one
    /// agent may hold; `None` is unbounded. Pending deliveries are live work,
    /// so they are never pruned: a send that would exceed the bound is
    /// refused with [`crate::Error::InboxFull`] instead, which keeps an
    /// offline or unattached agent from growing the state row without limit
    /// and tells the sender why.
    pub pending_per_agent: Option<usize>,
}
impl RetentionPolicy {
    /// Refuse a delivery that would push `agent_id`'s pending inbox past
    /// [`Self::pending_per_agent`].
    pub(crate) fn admit_pending(&self, state: &StoredState, agent_id: &str) -> crate::Result<()> {
        let Some(limit) = self.pending_per_agent else {
            return Ok(());
        };
        let pending = state
            .deliveries
            .iter()
            .filter(|d| d.agent_id == agent_id && d.status == DeliveryStatus::Pending)
            .count();
        if pending >= limit {
            return Err(crate::Error::InboxFull {
                agent_id: agent_id.to_owned(),
                limit,
            });
        }
        Ok(())
    }
    /// Drop the oldest settled records beyond each bound.
    pub(crate) fn apply(&self, state: &mut StoredState) {
        if let Some(keep) = self.settled_episodes {
            let referenced: Vec<_> = state
                .running
                .values()
                .filter_map(|run| run.request.episode.as_ref())
                .map(|episode| episode.episode_id.clone())
                .collect();
            let prunable = |episode: &EpisodeRecord| {
                episode.finished && !referenced.contains(&episode.episode_id)
            };
            let mut excess = state
                .episodes
                .iter()
                .filter(|e| prunable(e))
                .count()
                .saturating_sub(keep);
            state.episodes.retain(|episode| {
                let drop = excess > 0 && prunable(episode);
                excess -= usize::from(drop);
                !drop
            });
        }
        if let Some(keep) = self.delivered {
            let delivered = |d: &Delivery| d.status == DeliveryStatus::Delivered;
            let mut excess = state
                .deliveries
                .iter()
                .filter(|d| delivered(d))
                .count()
                .saturating_sub(keep);
            state.deliveries.retain(|delivery| {
                let drop = excess > 0 && delivered(delivery);
                excess -= usize::from(drop);
                !drop
            });
        }
        if let Some(keep) = self.interrupted {
            // Prune oldest interrupted deliveries while keeping the most recent.
            let interrupted = |d: &Delivery| d.status == DeliveryStatus::Interrupted;
            let mut excess = state
                .deliveries
                .iter()
                .filter(|d| interrupted(d))
                .count()
                .saturating_sub(keep);
            state.deliveries.retain(|delivery| {
                let drop = excess > 0 && interrupted(delivery);
                excess -= usize::from(drop);
                !drop
            });
            // Prune oldest interrupted turn records as well.
            excess = state.interruptions.len().saturating_sub(keep);
            state.interruptions.retain(|_| {
                let drop = excess > 0;
                excess = excess.saturating_sub(1);
                !drop
            });
        }
    }
}
/// Durable identity; a new process must reattach its live runner.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentRecord {
    /// Continuing session returned by the host.
    pub session_id: Option<String>,
    /// Agent waits for explicit host release.
    pub parked: bool,
    /// Exact completed turn awaiting release; older snapshots have no correlated identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked_turn: Option<ParkedTurn>,
    /// Release note awaiting the agent's next claimed turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumption: Option<String>,
}
/// Authenticated identity of a completed parked turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParkedTurn {
    /// Reservation identity distinguishing resumed attempts of the same input.
    pub turn_id: String,
    /// Continuing native session which returned the parked outcome.
    pub session_id: String,
    /// Captured episode, absent for a direct delivery.
    pub episode_id: Option<String>,
    /// Accepted input message identities, in delivery order.
    pub message_ids: Vec<String>,
    /// Trusted automation provenance of the parked turn.
    pub scheduled_job_id: Option<String>,
}
/// One direct-agent inbox entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Delivery {
    /// Accepted global sequence.
    pub sequence: u64,
    /// Registered recipient.
    pub agent_id: String,
    /// Delivery acknowledgement.
    pub status: DeliveryStatus,
}
/// Direct message delivery lifecycle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DeliveryStatus {
    /// Runner has never been invoked.
    Pending,
    /// Claimed durably before invocation.
    Running,
    /// Successfully returned.
    Delivered,
    /// Effects uncertain and not replayable.
    Interrupted,
}
/// One conducted hive episode, frozen membership and wave included.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EpisodeRecord {
    /// Configuration frozen at acceptance; absent on legacy/unconfigured work.
    #[serde(default)]
    pub settings: Option<crate::HiveSettings>,
    /// Scheduled authority inherited by every descendant assignment.
    #[serde(default)]
    pub scheduled_job_id: Option<String>,
    /// Durable episode identity.
    pub episode_id: String,
    /// Membership snapshot for the core driver.
    pub hive: HiveInfo,
    /// Accepted triggering message.
    pub opened_at: u64,
    /// Addressed outer conversation; core child channels remain nested beneath it.
    #[serde(default)]
    pub thread: Option<u64>,
    /// Seats allowed to receive the initial task.
    pub starters: Vec<String>,
    /// Existing conductor checkpoint, absent before opening.
    pub conductor: Option<ConductorState>,
    /// Proposed wave turns not yet invoked.
    pub pending: Vec<Turn>,
    /// True while a conductor wave has not closed.
    pub wave_open: bool,
    /// Empty wave waiting for host release or a missing runner.
    pub waiting: bool,
    /// Terminal episodes allow the next hive message to start.
    pub finished: bool,
    /// Failure/wall explanation when the episode stopped.
    pub failure: Option<String>,
}
/// Reservation and captured authorization for one agent invocation.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunningTurn {
    /// Captured request passed to the host.
    pub request: TurnRequest,
    /// Core proposed turn, absent for direct messages.
    pub turn: Option<Turn>,
    /// Active-turn actions, in call order.
    pub actions: Vec<EpisodeAction>,
    /// Accepted direct message sequence when present.
    pub delivery_sequence: Option<u64>,
}
