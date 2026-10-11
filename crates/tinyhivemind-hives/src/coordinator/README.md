# Dynamic coordinator

`release_parked` matches the unique reservation, native session, episode,
accepted message identities and scheduled provenance against a completed
parked turn in the transaction that releases it. Approval adapters use this
port to reject early answers and stale records, including a later park on the
same input. `release_with` remains the host's manual release operation.

| File | Responsibility |
| --- | --- |
| `mod.rs` | Shared handle, dynamic APIs, and authorization snapshots |
| `transaction.rs` | Writer gate, incremental commits outside the live lock, conflict reload |
| `settings.rs` | Per-hive native policy and retained roles, frozen at acceptance |
| `types.rs` | Host runner port and public payloads |
| `messaging.rs` | Atomic acceptance, retry IDs, starters, attribution, and private reads |
| `observe.rs` | Host transcript, committed-revision watch, episode status |
| `conduct.rs` | Actual `CompletionDriver`/`Conductor` checkpoint lifecycle |
| `scheduler.rs` | FIFO reservations, concurrent futures, shutdown, cancellation |
| `test/` | Deterministic contract fixtures and behavior tests |

All clones of `Coordinator` share one scheduler and state. Transactions clone
state under the live lock, release it, and persist the copy under an async
writer gate. The storage commit carries the state row plus only the transcript
rows appended since the base, and the copy is published only after the CAS
succeeds. Reads never wait on storage. A revision conflict, which means another
process wrote the store, reloads and recomputes up to four times. Runner
futures execute outside every lock. A dropped drain applies its interruptions
to live state immediately, and the next commit persists them. No external tool
is repeated.

`release_with(agent, note)` stores a note on the agent record. The next claim
moves it into `TurnRequest::resumption`, once. Host-chosen `starters` must be
distinct readers of the hive message; they open the episode while every reader
still sees the message.

Work is ordered by accepted sequence, then agent ID within a round. Shared
agents retain their queued positions and never run two turns concurrently.
Each hive has one active episode; the real conductor supplies child conversations,
nudges, broadcasts, completion admission, approval parking, and turn walls.
Normal runner return means its turn ended; only `EpisodeAction::Complete` closes
an assignment. A reply is recorded as a post. For standalone direct turns, `read_direct`
exposes both sent messages and returned replies only to the two participants,
with an exclusive `after` cursor. Returned replies do not enqueue return turns;
an agent sends an explicit follow-up to continue the exchange.

`register_agent` accepts handles from the current runtime. Existing durable IDs
are reattached after restart without overwriting session bindings. Optional
`register_agent_in_session` atomically commits the host's conversation before
publishing its runner, including when recovered work is already pending and a
scheduler is running. A failed commit changes neither the session nor the live
runner. `bind_session` remains available for separately registered agents before
their first claim.
Joining/leaving affects later turns; active turns retain captured membership.
Removing membership cancels unstarted deliveries while preserving transcript.
Claims revalidate pending seats under the reservation lock and retire removed
seats through conductor bookkeeping, including a leave after wave preparation.

An initiating private message bounds ordinary posts, completions, runner replies,
and public system notes to its sender and named readers. Explicit asks and
broadcasts retain their deliberate delegation audience.
Private child posts and completions inherit the ask root's participant list.
Reads also verify every ancestor thread's visibility. Messages addressed to an
existing thread keep that root in the execution context and all ordinary outputs;
private threads admit only their original participants, including after restart.

`run_until_idle` drains eligible work and returns with unattached/parked work
retained. `run` waits on notifications. Shutdown stops claims and waits for
active runner futures to return. Dropping a drain interrupts its durable running
reservations. Recovery records crashed running turns without replaying uncertain
external effects; pending turns which never started remain eligible.

A failed host finalizer can follow a successfully committed agent turn. A
`Failed` outcome with a nonempty matching session binds that conversation in
the same transaction as interruption. Its input is not acknowledged, and its
reply and staged episode actions are discarded. Later inputs continue that
conversation, including after SQLite reopen. Empty or changed session IDs fail
validation without replacing a prior binding.

`configure_hive(id, HiveSettings)` configures future episodes. Acceptance freezes
native completion options, routing thresholds, and roles; reconfiguration never
changes already accepted work. Roles reach native routing candidates, rendered
briefs, and `TurnRequest::teammates`. Retained role entries survive leave/rejoin.
Both per-hive widths must fit the global concurrent-turn cap; routing width is
intersected with completion width. Retention remains global: differing per-hive
retention settings are refused rather than ignored.

`send_scheduled_as_host(job_id, request)` commits scheduled authority with the
accepted transcript row and episode. Every descendant assignment and claimed
turn carries `scheduled_job_id`, including queued or recovered work. Retrying
with a changed job identity or interactive origin fails with `MessageConflict`.
Sends attributed to an actively scheduled agent inherit the same durable authority;
forwarding to another agent or hive cannot strip it. The host must scope automation
authority from each captured request explicitly.

The current completion coordinator attaches no semantic routing provider
(`BroadcastRouting.primary` and `.reasoning` are absent). Broadcasts therefore
use the driver's deterministic single-owner fallback. Native routing policy and
roles reach the routing request, and widths remain bounded, but confidence,
clarification and high-impact thresholds cannot change fallback selection.
Configuring these thresholds does not activate a model client.
