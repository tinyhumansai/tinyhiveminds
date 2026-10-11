//! Episode walls, opening inputs and resumable host-owned conductor snapshots.

use super::{
    child::{Child, Concluded},
    wave::Wave,
};
use crate::{
    driver::{BoundAgent, CompletionDriver, DriverState, engine::BroadcastRouting},
    runtime::Sequence,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The walls a conducted episode runs inside.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConductPolicy {
    /// Turns a conversation may take before it concludes without an answer.
    pub child_turn_wall: u64,
    /// Turns the whole episode may take before it is abandoned.
    pub turn_wall: u64,
}

impl Default for ConductPolicy {
    fn default() -> Self {
        Self {
            child_turn_wall: 6,
            turn_wall: 60,
        }
    }
}

/// The door: what the desk is, who sits at it, and who starts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Door {
    /// The chat every tool call names: the desk id.
    pub chat: String,
    /// The desk's name, for the episode's conversation record.
    pub desk_name: String,
    /// Every seat, in desk order.
    pub members: Vec<String>,
    /// The seats the door route starts; the rest are completed at once.
    pub starters: Vec<String>,
    /// The task's row: where the passed-over seats are completed.
    pub opened_at: Sequence,
}

/// Everything a conductor needs to be rebuilt: the episode as the driver
/// folds it, the conversations open and concluded, and what each seat has
/// been shown, nudged for, or held on.
///
/// Carries the wave in progress too, so a snapshot is exact rather than
/// per-wave: a host checkpoints after every committed row, and a crash
/// replays at most the one row whose sequence had not been reported yet.
/// The only point a snapshot cannot be taken is while the host holds a
/// commit it has not reported -- the conductor does not know whether that
/// row landed -- and [`Conductor::snapshot`] answers `None` there.
///
/// The driver, the routing and the policy are **not** here. They are the
/// host's to supply again on resume, exactly as they were on open: a router
/// is a live object, and a policy the operator changed between restarts
/// should be the new one.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ConductorState {
    /// The desk's id, as every tool call names it.
    pub chat: String,
    /// The desk's display name.
    pub desk_name: String,
    /// The episode the driver folds.
    pub state: DriverState,
    /// The conversations still open, by their ask row.
    pub(super) children: Vec<(Sequence, Child)>,
    /// The conversations that concluded, oldest first.
    pub(super) concluded: Vec<Concluded>,
    /// How many concluded conversations each seat has been shown.
    pub(super) shown: BTreeMap<String, usize>,
    /// The assignment each seat was last nudged for on the desk.
    pub(super) desk_nudged: BTreeMap<String, Sequence>,
    /// Seats held on the host, by the thread they parked in.
    pub(super) parked: BTreeMap<String, Option<Sequence>>,
    /// Turns run so far.
    pub(super) turns: u64,
    /// Waves proposed so far.
    pub(super) waves: u64,
    /// Seats completed with their work for a spent broadcast budget.
    pub(super) discharged: u64,
    /// The wave in progress: what has been said and not yet committed, and
    /// the steps the host has not taken. Empty between waves.
    pub(super) wave: Wave,
}

impl ConductorState {
    /// Whether this snapshot was taken between waves, with nothing said and
    /// nothing left for the host to do.
    ///
    /// A host does not need this -- [`resume_episode`] drains whatever the
    /// wave holds either way -- but it is the difference between a restart
    /// that lost a whole wave and one that lost a row.
    ///
    /// [`resume_episode`]: https://docs.rs/tinyhivemind-openhuman
    #[must_use]
    pub fn mid_wave_is_empty(&self) -> bool {
        self.wave.is_idle()
    }
}
/// The desk episode, its conversations, and the rules between them.
pub struct Conductor<'a, A: BoundAgent> {
    pub(super) driver: &'a CompletionDriver<'a, A>,
    pub(super) routing: BroadcastRouting<'a>,
    pub(super) chat: String,
    pub(super) desk_name: String,
    pub(super) policy: ConductPolicy,
    pub(super) state: DriverState,
    pub(super) children: BTreeMap<Sequence, Child>,
    pub(super) concluded: Vec<Concluded>,
    /// How many concluded conversations each seat has been shown.
    pub(super) shown: BTreeMap<String, usize>,
    /// The assignment each seat was last nudged for on the desk.
    pub(super) desk_nudged: BTreeMap<String, Sequence>,
    /// Seats held on the host, by the thread they parked in (`None` for the
    /// desk): not nudged, not stalled, not proposed, until released.
    pub(super) parked: BTreeMap<String, Option<Sequence>>,
    pub(super) turns: u64,
    pub(super) waves: u64,
    pub(super) discharged: u64,
    pub(super) wave: Wave,
}

impl<A: BoundAgent> std::fmt::Debug for Conductor<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conductor")
            .field("chat", &self.chat)
            .field("turns", &self.turns)
            .field("waves", &self.waves)
            .field("conversations", &self.children.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}
