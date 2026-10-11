//! The division of labour: its policy, its assignments, and what a caller
//! reads off one.

use serde::{Deserialize, Serialize};

use crate::runtime::{SessionAuthor, SessionMessage};

use crate::hive::episode::DEFAULT_ROUND_WIDTH;
use crate::hive::trace::{TopicId, read};

/// Why a facet's owner is its owner.
///
/// Recorded rather than inferred, because the two are worth very different
/// amounts and a caller deciding whether to trust an assignment should not
/// have to guess which it got. The benchmark is explicit about it: dividing a
/// task buys nothing on its own, and dividing it **along a line where
/// competence differs** is what buys. A [`Self::Rotation`] assignment is the
/// first of those; a [`Self::Knows`] assignment is the second.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerReason {
    /// The transactive-memory directory names this seat on the facet: it
    /// deposited grounds on it and was cited for them.
    Knows,
    /// No seat is known on the facet, so the rotation took it. Deterministic,
    /// and no better informed than any other seat.
    Rotation,
}

/// One facet of a task, and the seat that owns it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assignment {
    /// The sub-decision this seat is answering.
    pub facet: TopicId,
    /// The agent that answers it.
    pub owner: String,
    /// Which concurrent round it runs in, counting from zero.
    ///
    /// Every assignment in one round is authorized against the same
    /// transcript and none can read another, so a host running async seats
    /// pays one round of wall clock for all of them — the same accounting
    /// [`crate::hive::HiveStep::Speak`] uses for the turns inside one episode.
    pub round: u32,
    /// How the owner was chosen.
    pub reason: OwnerReason,
}

/// A task's facets, divided across the seats that will decide them.
///
/// The output of [`divide`](super::divide), and a plain fold over what the
/// caller already held: nothing here is stored between calls, and the same
/// arguments produce the same division every time.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Division {
    pub(super) assignments: Vec<Assignment>,
}

impl Division {
    /// Every assignment, in the order the facets were given.
    #[must_use]
    pub fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }

    /// Rounds this division takes: what a host running async seats waits for.
    ///
    /// One for a task nobody has to wait on twice, and `⌈facets / width⌉` at
    /// the worst. This is the number the benchmark reports as `rounds/ep`, and
    /// it is the quantity a division buys: eight facets across five seats are
    /// two rounds rather than eight.
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.assignments
            .iter()
            .map(|held| held.round.saturating_add(1))
            .max()
            .unwrap_or(0)
    }

    /// Assignments in the widest round: the most seats that ever run at once.
    #[must_use]
    pub fn width(&self) -> u32 {
        (0..self.depth())
            .map(|round| u32::try_from(self.round(round).len()).unwrap_or(u32::MAX))
            .max()
            .unwrap_or(0)
    }

    /// The assignments of one round, in facet order.
    #[must_use]
    pub fn round(&self, round: u32) -> Vec<&Assignment> {
        self.assignments
            .iter()
            .filter(|held| held.round == round)
            .collect()
    }

    /// Who owns one facet, if anybody does.
    #[must_use]
    pub fn owner_of(&self, facet: &TopicId) -> Option<&str> {
        self.assignments
            .iter()
            .find(|held| &held.facet == facet)
            .map(|held| held.owner.as_str())
    }

    /// The facets one seat owns, in order.
    #[must_use]
    pub fn facets_of(&self, seat: &str) -> Vec<&TopicId> {
        self.assignments
            .iter()
            .filter(|held| held.owner == seat)
            .map(|held| &held.facet)
            .collect()
    }

    /// Whether this task is one facet answered by one seat.
    ///
    /// The measured rule stated as a predicate: **one task, one agent.** At a
    /// single facet a room is the wrong tool — the benchmark puts a room and a
    /// soloist within a decimal of each other there — and a caller that wants
    /// to skip the protocol entirely can ask.
    #[must_use]
    pub fn is_alone(&self) -> bool {
        self.assignments.len() == 1
    }

    /// Whether this division is empty, which a task with no facets is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    /// The rows of a transcript that belong to one facet.
    ///
    /// **The half of the mechanism that actually wins.** A seat's accuracy
    /// stays flat as a task widens because it never holds the facets it is not
    /// deciding; a soloist's decays because it holds all of them. That is a
    /// property of what each participant reads, so it belongs here beside the
    /// assignment rather than in a host that might forget it.
    ///
    /// A row is kept unless every topic it names belongs to another facet. A
    /// row that names this facet is kept, and so is one that names no topic at
    /// all — the task statement, a question, an answer with no marker — because
    /// dropping shared context would be scoping the *conversation* rather than
    /// the work.
    #[must_use]
    pub fn scoped<'a>(
        &self,
        facet: &TopicId,
        transcript: &'a [SessionMessage],
    ) -> Vec<&'a SessionMessage> {
        let mine: Vec<&TopicId> = self.assignments.iter().map(|held| &held.facet).collect();
        let traces = read(transcript);
        transcript
            .iter()
            .filter(|message| {
                let mut named = traces
                    .iter()
                    .filter(|trace| trace.sequence == message.sequence)
                    .filter_map(|trace| trace.topic.as_ref())
                    // A topic no assignment claims is not another facet's; it
                    // is something this division does not divide, and it stays
                    // visible to everybody.
                    .filter(|topic| mine.contains(topic))
                    .peekable();
                named.peek().is_none() || named.any(|topic| topic == facet)
            })
            .collect()
    }

    /// Whether one agent authored a message, for a caller scoping a seat's own
    /// rows back in.
    #[must_use]
    pub fn authored_by(message: &SessionMessage, seat: &str) -> bool {
        matches!(&message.author, SessionAuthor::Agent { id, .. } if id == seat)
    }
}

/// How a task's facets are divided across seats.
///
/// Unlike every other opt-in mechanism in this crate, [`Self::DEFAULT`] is
/// **on**. That is not a change of taste: the division is the one arm this
/// repository measured beating the best single agent it could build, and a
/// default that made a caller opt into the thing that wins would be the wrong
/// way round. See `docs/experiments/2026-09-09-variety-and-roles.md`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DivisionPolicy {
    /// Assignments one round may authorize concurrently.
    ///
    /// Bounded for the reason a deliberation's round is bounded: what the
    /// removed one-turn rule was really guarding was unbounded fan-out, and
    /// the bound is the invariant that replaced it. A division is also capped
    /// by the seats available, since one seat cannot answer two facets at
    /// once.
    pub round_width: u32,
    /// Whether the transactive-memory directory picks owners.
    ///
    /// `true` by default, and it degrades rather than fails: a facet no seat
    /// is known on falls to the rotation, which is what a directory with no
    /// signal yet produces for every facet. Set it `false` for a division that
    /// is deliberately blind to competence — the control the benchmark runs to
    /// show that dividing the load alone buys nothing.
    pub follow_directory: bool,
}

impl DivisionPolicy {
    /// The measured default: divide, up to [`DEFAULT_ROUND_WIDTH`] at a time,
    /// following the directory where it has an opinion.
    pub const DEFAULT: Self = Self {
        round_width: DEFAULT_ROUND_WIDTH,
        follow_directory: true,
    };
}

impl Default for DivisionPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}
