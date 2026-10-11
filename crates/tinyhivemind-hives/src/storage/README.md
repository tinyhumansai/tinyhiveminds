# Storage

| File | Responsibility |
| --- | --- |
| `mod.rs` | Async object-safe `Storage` port, `Commit`, memory implementation, commit validation |
| `types.rs` | Durable agents, transcript rows, queues, running turns, conductor state, retention |
| `sqlite.rs` | Default-feature schema-two SQLite storage and version-one migration |
| `test.rs` | Shared CAS and incremental-transcript contract, state-row shape, retention |
| `sqlite_test.rs` | SQLite reopen, competing writers, schema validation, v1 migration |

`commit(Commit { expected_revision, state, appended })` advances exactly one
revision, or it changes nothing. `state` is the bounded state row: serde skips
the transcript (`messages`) and the accepted retry payloads (`accepted`).
`appended` holds the `TranscriptRow`s added since `expected_revision`, in
ascending sequence order after the last stored row; anything else fails with
`TranscriptOutOfOrder`. `load` reassembles both through `StoredState::append`.
The port returns boxed `Send` futures and names no executor, so an async
database client (`MongoDB`) implements it directly. Its 16 MB document cap
applies only to the state row, which `RetentionPolicy` bounds.

SQLite writes the state row and the new `hivemind_messages` rows in one
immediate transaction. Revisions and sequences are stored as text to preserve
the full unsigned range. Opening a version-one file moves its embedded
transcript into rows. Storage holds no agent handles, closures, credentials, or
runtime singleton.

Storage is replaceable by the host. A newly loaded coordinator records running
reservations as interrupted, and requires supplied handles to be reattached
before unstarted jobs run. Durable session IDs remain bound across process
runtimes.

`StoredState::hive_settings` retains native policy and role metadata separately
from current memberships. Every episode freezes its settings and scheduled job
identity. Transcript rows persist scheduled authority on `Message`, while
running reservations retain it in `TurnRequest`; legacy rows default to no
explicit settings or scheduled origin.
