//! Coordinator payloads and the host runner boundary.
use crate::{Result, RetentionPolicy};
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Arc};
use tinyhivemind_core::driver::ConductPolicy;

/// Reserved identity used only by explicit host operations.
pub const HOST_ID: &str = "hivemind:host";
/// Future returned by an attached host runner.
pub type TurnFuture = Pin<Box<dyn Future<Output = Result<TurnOutcome>> + Send>>;
/// Runs one turn of an already configured host agent.
pub trait AgentRunner: Send + Sync {
    /// Run the supplied context, continuing its session when present.
    fn run(&self, request: TurnRequest) -> TurnFuture;
}
/// Live registration; the runner is never serialized.
#[derive(Clone)]
pub struct AgentRegistration {
    /// Globally unique agent identity within the runtime.
    pub agent_id: String,
    /// Identity of the current shared host runtime.
    pub runtime_id: String,
    /// Already configured runner handle.
    pub runner: Arc<dyn AgentRunner>,
}
impl std::fmt::Debug for AgentRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentRegistration")
            .field("agent_id", &self.agent_id)
            .field("runtime_id", &self.runtime_id)
            .finish_non_exhaustive()
    }
}
/// Scheduling bounds, matching the existing driver defaults.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoordinatorOptions {
    /// Maximum concurrent turns and the per-conductor round width; nonzero.
    pub round_width: usize,
    /// Existing child and episode turn walls; both nonzero.
    pub conduct_policy: ConductPolicy,
    /// Broadcasts per assignment; `None` preserves the driver's default.
    pub broadcast_budget: Option<u32>,
    /// Bounds on settled records kept in the state row; keeps all by default.
    pub retention: RetentionPolicy,
}
impl Default for CoordinatorOptions {
    fn default() -> Self {
        Self {
            round_width: 1,
            conduct_policy: ConductPolicy::default(),
            broadcast_budget: None,
            retention: RetentionPolicy::default(),
        }
    }
}
/// One continuing agent turn with its captured authorization context.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TurnRequest {
    /// Unique durable reservation identity, renewed even when the same input is resumed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub turn_id: String,
    /// Durable scheduled authority, absent for interactive work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_job_id: Option<String>,
    /// Teammate roles captured from the episode's hive configuration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teammates: Vec<tinyhivemind_core::runtime::BriefedTeammate>,
    /// Bound agent identity.
    pub agent_id: String,
    /// Previously returned continuing session identity.
    pub session_id: Option<String>,
    /// Attributed new visible messages.
    pub messages: Vec<Message>,
    /// Membership snapshot captured before invocation.
    pub memberships: Vec<HiveInfo>,
    /// Active conductor assignment, absent for direct messages.
    pub episode: Option<EpisodeContext>,
    /// Host note from [`crate::Coordinator::release_with`], delivered once
    /// on the first turn claimed after the release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumption: Option<String>,
}
/// Successfully returned runner state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TurnOutcome {
    /// Committed host session identity, retained even when finalization failed.
    pub session_id: String,
    /// Optional reply, recorded as a post rather than implicit completion.
    pub reply: Option<String>,
    /// Whether the runner completed, awaits approval, or failed.
    pub disposition: TurnDisposition,
}
/// Host turn status; assignment completion requires an explicit action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TurnDisposition {
    /// Turn returned normally.
    Completed,
    /// Awaiting an explicit host release, normally for approval.
    Parked,
    /// Host finalization failed; a usable matching session is retained.
    /// Input is interrupted, and replies and staged episode actions are discarded.
    /// Uncertain external effects are not replayed.
    Failed(String),
}
/// Active episode and channel bound to one turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EpisodeContext {
    /// Durable episode identity.
    pub episode_id: String,
    /// Sole hive/desk identity.
    pub hive_id: String,
    /// Conversation root, absent on the open hive.
    pub thread: Option<u64>,
    /// Rendered existing conductor brief.
    pub brief: String,
}
/// Dynamic hive definition; membership is ordered and unique.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HiveInfo {
    /// Sole hive/desk identity.
    pub hive_id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional purpose.
    pub description: Option<String>,
    /// Registered agent identities.
    pub members: Vec<String>,
}
/// A hive channel or globally registered agent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Destination {
    /// Hive identity.
    Hive(String),
    /// Runtime agent identity.
    Agent(String),
}
/// Durable attributed message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    /// Scheduled job that authorized this work, retained through delegation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_job_id: Option<String>,
    /// Caller retry identity, or a coordinator-generated event identity.
    pub message_id: String,
    /// Globally monotonic durable sequence.
    pub sequence: u64,
    /// Bound sender identity.
    pub sender: String,
    /// Destination channel.
    pub destination: Destination,
    /// Text payload.
    pub body: String,
    /// Optional hive conversation root.
    pub thread: Option<u64>,
    /// Owning episode, absent for standalone direct messages.
    pub episode_id: Option<String>,
    /// Private readers; empty means all hive members.
    pub only_for: Vec<String>,
}
/// Message acceptance request; tools bind the sender on the host side.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SendMessage {
    /// Stable retry identity.
    pub message_id: String,
    /// Registered author; ignored by `send_as_host`.
    pub sender: String,
    /// Target hive or agent.
    pub destination: Destination,
    /// Message text.
    pub body: String,
    /// Optional existing visible hive conversation root.
    pub thread: Option<u64>,
    /// Optional private recipients within the target hive.
    pub only_for: Vec<String>,
    /// Hive members who start the episode; empty starts every recipient.
    /// Unlike `only_for` this narrows who acts first, not who can read:
    /// the message stays visible to all its readers. Hive messages only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starters: Vec<String>,
}
/// Receipt returned without waiting for the destination agent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Receipt {
    /// Stable retry identity.
    pub message_id: String,
    /// Accepted durable sequence.
    pub sequence: u64,
}
/// Episode tools collected during the active bound turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum EpisodeAction {
    /// Post without closing the assignment.
    Post {
        /// Message body.
        body: String,
    },
    /// Open one private child conversation.
    Ask {
        /// Captured hive peers.
        agents: Vec<String>,
        /// Question body.
        body: String,
    },
    /// Route work using the existing conductor handoff rules.
    Broadcast {
        /// Work body.
        body: String,
    },
    /// Report the active assignment complete.
    Complete {
        /// Completion body.
        body: String,
    },
}
/// Work observed during one eligible-work drain.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunReport {
    /// Normally returned turns.
    pub completed: usize,
    /// Failed turns or conductor episodes.
    pub failed: usize,
    /// Turns awaiting explicit release.
    pub parked: usize,
}
/// Host-facing status of one conducted hive episode.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EpisodeStatus {
    /// Durable episode identity.
    pub episode_id: String,
    /// Hive the episode runs in.
    pub hive_id: String,
    /// Sequence of the message that opened it.
    pub opened_at: u64,
    /// Addressed outer conversation, absent on the open hive.
    pub thread: Option<u64>,
    /// Members the opening message started.
    pub starters: Vec<String>,
    /// Where the episode stands.
    pub phase: EpisodePhase,
}
/// Lifecycle of an episode as a host observes it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum EpisodePhase {
    /// Running or queued behind its hive's earlier episode.
    Open,
    /// Every remaining seat is parked; waiting for `release`.
    AwaitingRelease,
    /// Finished normally.
    Settled,
    /// Stopped by a wall, a conductor error, or an interrupted turn.
    Failed(String),
}
/// Uncertain turn effects which must not be replayed automatically.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InterruptedTurn {
    /// Bound agent identity.
    pub agent_id: String,
    /// Owning episode when present.
    pub episode_id: Option<String>,
    /// Messages whose delivery may have begun.
    pub message_ids: Vec<String>,
    /// Cancellation, crash, or runner failure explanation.
    pub reason: String,
}
