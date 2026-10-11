//! Replaceable transactional persistence: a bounded state row plus an
//! append-only transcript.
#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(test)]
mod test;
mod types;
use crate::{Error, Result};
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStorage;
use std::{future::Future, pin::Pin, sync::Mutex};
pub use types::{
    AgentRecord, Delivery, DeliveryStatus, EpisodeRecord, ParkedTurn, RetentionPolicy, RunningTurn,
    StoredState, TranscriptRow,
};

/// Future returned by a [`Storage`] operation. Boxed and `Send` so the port
/// stays object-safe and names no executor: a host's async database client
/// drives it on whatever runtime the host already runs.
pub type StorageFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// One incremental commit: the next bounded state row and the transcript rows
/// appended since `expected_revision`.
#[derive(Clone, Copy, Debug)]
pub struct Commit<'a> {
    /// Revision the writer read; the store must still be at it.
    pub expected_revision: u64,
    /// Next state; `state.revision` is exactly `expected_revision + 1`. Its
    /// serialized form omits the transcript, which is never rewritten.
    pub state: &'a StoredState,
    /// Newly appended rows in ascending sequence order, each after the last
    /// stored row. Empty when the commit appended no message.
    pub appended: &'a [TranscriptRow],
}

/// Atomic incremental persistence with revision compare-and-swap.
///
/// A commit replaces the state row and appends its transcript rows in one
/// transaction, or changes nothing. Keeping the transcript out of the state
/// row is what lets a document store with a size cap (`MongoDB`'s 16 MB) hold
/// a long-running company: the row is bounded by [`RetentionPolicy`], the
/// transcript grows one row per message.
pub trait Storage: Send + Sync {
    /// Load the committed state row with its transcript reassembled through
    /// [`StoredState::append`].
    /// # Errors
    /// Returns storage or snapshot decoding errors.
    fn load(&self) -> StorageFuture<'_, StoredState>;
    /// Apply `commit` only when the stored revision equals its expected one.
    /// # Errors
    /// Returns [`Error::RevisionConflict`] for a stale writer,
    /// [`Error::InvalidRevision`] or [`Error::TranscriptOutOfOrder`] for a
    /// malformed commit, or persistence errors. A failed commit changes nothing.
    fn commit<'a>(&'a self, commit: Commit<'a>) -> StorageFuture<'a, ()>;
}
/// In-process implementation of the same transactional storage contract.
#[derive(Debug, Default)]
pub struct MemoryStorage {
    stored: Mutex<Stored>,
}
#[derive(Debug, Default)]
struct Stored {
    state: StoredState,
    rows: Vec<TranscriptRow>,
}
impl MemoryStorage {
    /// Create an empty store at revision zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}
impl Storage for MemoryStorage {
    fn load(&self) -> StorageFuture<'_, StoredState> {
        Box::pin(async move {
            let stored = self.stored.lock().map_err(|_| Error::Poisoned)?;
            let mut state = stored.state.clone();
            for row in &stored.rows {
                state.append(row.clone());
            }
            Ok(state)
        })
    }
    fn commit<'a>(&'a self, commit: Commit<'a>) -> StorageFuture<'a, ()> {
        Box::pin(async move {
            let mut stored = self.stored.lock().map_err(|_| Error::Poisoned)?;
            let last = stored.rows.last().map(|row| row.message.sequence);
            validate(stored.state.revision, last, &commit)?;
            stored.state = commit.state.without_transcript();
            stored.rows.extend_from_slice(commit.appended);
            Ok(())
        })
    }
}
/// Check a commit against the stored revision and last transcript sequence.
fn validate(actual: u64, last_sequence: Option<u64>, commit: &Commit<'_>) -> Result<()> {
    let expected = commit.expected_revision;
    if actual != expected {
        return Err(Error::RevisionConflict { expected, actual });
    }
    if expected.checked_add(1) != Some(commit.state.revision) {
        return Err(Error::InvalidRevision);
    }
    let mut last = last_sequence;
    for row in commit.appended {
        let sequence = row.message.sequence;
        if last.is_some_and(|last| sequence <= last) {
            return Err(Error::TranscriptOutOfOrder(sequence));
        }
        last = Some(sequence);
    }
    Ok(())
}
