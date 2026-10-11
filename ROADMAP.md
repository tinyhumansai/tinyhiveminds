# Roadmap

`tinyhivemind` began by moving the shared-conversation layer out of
[`opencompany`](https://github.com/tinyhumansai/opencompany) and fixing two
transcript defects. This table records the order in which that work landed.

For changes used by OpenCompany, this repository lands first. OpenCompany then
updates its pinned `vendor/tinyhivemind` submodule. The pin is the version that
consumer builds.

| Phase | What lands | State |
| --- | --- | --- |
| OpenHuman configuration | One runtime, reusable Markdown profiles, persistent seats, narrowed hive permissions and native workflows; [accepted contract](docs/specs/openhuman-hive-config.md), [example manifests](examples/openhuman/hives/README.md) | Implemented for review in #113 |
| P0 | Reshape the TinyBus module template into a plain library workspace | **done** |
| P1 | Chat identity: `MAIN_THREAD_ID`, `GENERAL_DESK`, `is_general_chat`, `same_conversation` | **done** |
| P2 | Desk types, then the membership algebra behind `DeskSet<'a>` | **done** |
| P3 | The `@` grammar, `Mention`/`MentionTarget`, and resolution over `Roster`/`Person` | **done** |
| P4 | `crates/tinyhivemind`: the `SessionLog` port, the paging walk, and the **attributed** transcript projection | **done** |
| P5 | Continuous sharing — re-seed on a watermark rather than only on a rebind | **done** |
| P6 | The responder ladder and a model-backed `Selector` port | **core plan retained**; unused waiting port retired |
| P7 | A bounded mention-dispatch edge | **core decision retained**; unused queue wrapper retired |
| P8 | `crates/tinyhivemind-hive`: bounded group deliberation — the trace grammar, salience, quorum with cross-inhibition, the attention market, and the episode state machine | **done** |
| P9 | `!refute`, evidential grounding, grounded objections, and the benchmark arm that scored them | Historical experiment; refutation trace retained, losing quorum knobs retired |
| P10 | A transactive-memory directory folded from traces, `BidReason::Knows`, and `!defer` | Directory retained for division; losing delegation and deferral paths retired |
| P11 | `SessionMessage.parent` and the structured trace sidecar | planned |
| P12 | Per-conversation read state | planned |
| P13 | Digests and supersession | planned |
| P15 | Cross-desk referral and the federated benchmark that scored it | Core referral decision retained; unused queue wrapper retired |
| P14 | Selection ranking, bounded transcript search, pinning, and a stated per-message budget | Pinning and budget retained; unused selection and search APIs retired |
| P16 | A pure approval gate and a waiting `ApprovalGate` port | Pure approval fold retained; unused waiting port retired |
| P17 | Private asides: an audience on a stored row, a viewer on a query, the collapsed redaction stub and its settlement pointer, and the rule that an aside carries information rather than support | **done**, **off by default** — the benchmark arm says asides do not improve a decision, see below |
| P18 | The utterance surface: a seat speaks by calling a tool rather than emitting a fence — the tool descriptions, the validation and the utterance-to-row fold live in `tinyhivemind::speech`, and a refused aside reaches its author inside the turn | **done** — see [`docs/specs/the-utterance-surface.md`](docs/specs/the-utterance-surface.md) |
| P19 | Folding by size: the standing account triggers on the characters of foldable content as well as its row count, stated by a host as a token budget, and the fold is told which messages the room pinned so it cannot drop one | **done** — see [`docs/specs/folding-by-size.md`](docs/specs/folding-by-size.md) |
| P20 | A provider layer for the former `desk` example | Example retired; [ADR 0013](docs/adr/0013-a-vendored-crate-is-an-example-dependency.md) records the earlier dependency boundary |
| P21 | Concurrent rounds: a step authorizes a bounded *round* of turns rather than one, `next_state` moves onto the round, and a peer row written in the same round is invisible to it — depth becomes rounds rather than turns | **done** — see [`docs/specs/concurrent-rounds.md`](docs/specs/concurrent-rounds.md) and [ADR 0014](docs/adr/0014-a-round-authorizes-concurrent-turns.md) |
| P22 | A task with a horizon: `--stages` runs a chain of decisions on one accumulating window, against a soloist handed the whole brief that compacts by eviction or by a superseding account | **done** — see [`docs/specs/long-horizon-tasks.md`](docs/specs/long-horizon-tasks.md) and [the experiment](docs/experiments/2026-09-09-the-long-horizon.md) |
| P23 | A task with variety: `--facets` runs several independent sub-decisions belonging to one task, each with an owner, against the soloist that won the horizon — and the room wins from two facets on | **done** — see [`docs/specs/task-variety.md`](docs/specs/task-variety.md) and [the experiment](docs/experiments/2026-09-09-variety-and-roles.md) |
| P24 | The seat-per-facet shape as the default: `division` folds a task's facets across the seats that own them, `Division::scoped` gives each owner its own facet and none of the others, and `DivisionPolicy::DEFAULT` is the one default in this crate that is **on** | **done** — see [ADR 0015](docs/adr/0015-the-division-of-labour-is-the-default-shape.md); the benchmark's `hive+fold` now calls the library and reproduces every cell bit-for-bit |
| P28 | Typed semantic decisions: fixed-point selector distributions, admission-gated probabilistic quorum, native Jev example integration, and a paired strict-JSON baseline | **done** — see [`docs/specs/jev-integration.md`](docs/specs/jev-integration.md) |
| P25 | Scale: the harness runs a thousand agents across a hundred desks, the sample loops spread across cores, and a cross-channel question is asked **off the floor** so a large federation still decides something. Host-side only — no library crate touched | **done** — see [the write-up](docs/experiments/2026-09-10-hive-at-scale.md) |
| P26 | What the scale run found, fixed in the library: `EpisodePolicy::for_room` scales the three bounds `DEFAULT` states absolutely, `HiveStep::Exhausted` reports the standings and visibility it ended at, distance is measurable in the rows a fold reads, the room-size hot loops stop being quadratic, and the harness gains a federation-wide digest for desks that share a blind spot | **done** — see [the write-up](docs/experiments/2026-09-10-what-the-scale-run-found.md) and [ADR 0016](docs/adr/0016-distance-is-measured-in-the-rows-a-fold-reads.md) |
| P27 | Evidence rather than opinion: a federation's disqualifying **facts** are planted on a desk other than the one that needs them, and a member states what it can rule out alongside what it scores. A broadcast of evidence closes the gap to the free-information ceiling at a thousand agents where a broadcast of opinion plateaus at 62.5% | **done** — see [the write-up](docs/experiments/2026-09-11-evidence-not-opinion.md) |

P15 is also out of order, and for a related reason: it is not a wire-format
change either, and it answers a pressure none of P11 through P13 address. Every
mechanism before it stops at the edge of one conversation, so a room of agents
can pool what its members know and a *company* of them cannot. A desk is a
correlation boundary — members of one desk are wrong about the same things —
and no amount of deliberating inside a channel cancels an error every member
shares. See [`docs/specs/cross-desk-referral.md`](docs/specs/cross-desk-referral.md),
[ADR 0006](docs/adr/0006-a-referral-crosses-one-channel-at-a-time.md) and
[the federated experiment](docs/experiments/2026-09-02-federated-hidden-profile.md).

P17 is also out of order, and unlike P14 and P15 it *is* a wire-format change —
so it owes the compatibility story P11 and P12 owe, and
[`docs/specs/private-asides.md`](docs/specs/private-asides.md) carries it. It
comes first because it answers a pressure none of P11 through P13 touch. Every
mechanism to date narrows what a turn *sees* by time or by conversation, never
by *reader*: two agents on one desk receive byte-identical projections, so a
room where everyone reads everything is the only room this library can express.
That is a group chat. The measured cost is already in the harness — ADR 0005
records a 24-point gap between a blind opening and full visibility — and P17
generalises that one crude knob from per-turn and time-based to per-message and
addressee-based. See [`docs/specs/private-asides.md`](docs/specs/private-asides.md),
[ADR 0010](docs/adr/0010-an-aside-carries-information-never-support.md) and
[the reading](docs/research/context-in-agent-teams.md).

The on-floor check **lost**, and the loss is published:
[`docs/experiments/2026-09-07-do-asides-help.md`](docs/experiments/2026-09-07-do-asides-help.md)
records a 2.5-point loss at the tuned turn budget and a 15.3-point loss on a
hidden profile. The later matched-turn control found that spending a floor turn
caused the loss; it retracted the earlier explanation about averaging correlated
error. Privacy itself did not change the measured outcome. An off-floor aimed
check gained 3.2 points on that hidden profile, while costing additional model
calls. See [the follow-up](docs/experiments/2026-09-07-why-asides-lose.md).

P14 addressed the bounded-window problem without storing a second index.
Pinning still keeps a small working set in view, and `BrevityPolicy` states
the budget each message spends. The unused selection and search APIs were
retired. The [original spec](docs/specs/recall.md) records the broader design.

P16 is the next number free, and it sits after the phases that have landed
rather than inside the P11 through P13 block for the same reason P14 and P15
did: it is not a wire-format change, it does not wait on `SessionMessage.parent`
or on read state, and it answers a pressure none of them address. It also needs
nothing from the hive crate. See [`docs/specs/approval.md`](docs/specs/approval.md)
and [ADR 0008](docs/adr/0008-an-approval-decision-is-total.md).

The phase table is a historical record. Current crate roles and their direct
dependencies are in the [workspace dependency map](docs/crate-dependencies.md).

P11 through P13 come out of a survey of the biology, the group-decision
literature, and the open-source landscape of shared agent memory, recorded in
[`docs/research/`](docs/research/README.md) and specified in
[`docs/specs/shared-medium-schema.md`](docs/specs/shared-medium-schema.md). They
are ordered by leverage, and P11 and P12 are wire-format changes that need their
serde-compatibility story written down before any code.

## What P8 adds, and what it deliberately does not

P8 answers a question the first seven phases do not: how a *room* of agents
reaches a decision, rather than how one message finds its one responder. It adds
a trace grammar over the shared transcript, a decaying salience field, quorum
counted as distinct grounded supporters, cross-inhibition that silences an
advocate rather than debiting an option, and an attention market that ranks
eligible speakers. A later step may authorize a bounded round of them.

It adds **no port**. An episode is a pure fold, and the host does its waiting
through the session log and turn scheduler it already implements.
`crates/tinyhivemind-hive` is in the `pure_crates` list in
`.github/scripts/assert-pure.sh`.

It is also not a claim that group deliberation produces better answers. Almost
every positive multi-agent result in the literature is confounded by compute,
and self-consistency at a matched token budget is the honest control. P8 is a
protocol for bounded deliberation with an auditable termination reason, and
nothing more. See
[ADR 0014](docs/adr/0014-a-round-authorizes-concurrent-turns.md), which
supersedes the original sequential episode decision.

## Retired P9 and P10 experiments

P9 tested a refutation cap and evidential quorum. Both were off by default
because they reduced accuracy in the original uniform benchmark. A later
hidden-profile run with an evidence-first opening also found losses: `hive+`
scored 66.3%, against 53.3% for the refutation cap and 26.0% for evidential
quorum. The optional quorum rules have been removed. `!refute` remains a trace
that records disagreement without changing whether a topic carries. The
[original experiment](docs/experiments/2026-09-01-refutation-and-grounds.md)
and [later matrix](docs/experiments/2026-09-05-expert-delegation.md) retain the
measurements.

P10 tested a transcript-folded directory, `BidReason::Knows`, and `!defer`.
Neither delegation nor deferral improved the measured decisions. The live
matrix recorded no `Knows` floor award and no deferral use across 27 rounds.
The directory fold remains because task division uses its expertise estimate;
the episode's delegation bonus and deferral path have been removed. See the
[experiment](docs/experiments/2026-09-05-expert-delegation.md) and
[division decision](docs/adr/0015-the-division-of-labour-is-the-default-shape.md).

## What P16 adds, and what it deliberately does not

P16 answers the largest well-evidenced gap in
[the Grok Bot survey](docs/research/grok-bots/README.md): this library can say
who is here, who a mention addresses, who takes the next turn and how a room
reaches a decision, and it cannot say whether the thing that decision leads to
is allowed to happen. Six of the twelve surveyed projects gate side-effecting
actions, and in every one of them the decision is already a pure function
separated from the IO that enacts it — the core/port line this workspace draws,
arrived at independently five times.

It adds an `approval` module to `crates/tinyhivemind-core`: `approve` as a
**total** fold returning `Allow`, `Deny { reason }` or `Ask { who }`; a scope
key over `(actor, call, verb, target)` with call, action and resource-pinned
grant scopes; standing grants as a pure liveness and coverage predicate over
records the caller supplies, with `now` passed in rather than read; and
epoch-scoped consent, under which a grant issued later cannot cover a request
minted earlier. `Ask` names exactly one person, resolved through the existing
roster and desk algebra, and never an agent.

The original phase also added an `ApprovalGate` waiting port in
`crates/tinyhivemind`. That unused wrapper has since been retired; a host
handles the wait after the pure decision. The ADR's lasting choice is that the
fold is total: a gate that can fail can be bypassed by failing, so `approve`
returns no `Result` and adds no `Error` variant. See
[ADR 0008](docs/adr/0008-an-approval-decision-is-total.md).

It does **not** execute anything, define a tool surface, embed a policy
language, store an audit trail, handle a credential, or sandbox a command. The
resource predicate is lexical path containment, not a kernel boundary, and the
spec says so where a reader would otherwise assume otherwise. An approval
decision authorizes no turn: no variant carries one, an `Ask` is not a mention
and never becomes a `MentionTurnRequest`, and approval expresses no edge to the
dispatch or referral folds in either direction.

## The two defects P4 and P5 fix

**The transcript is first-person-collapsed.** The host's projection discards
the author of every reply, so on a shared desk agent B reads agent A's replies
as B's own prior turns. A system notice, a workflow report and a real teammate
are indistinguishable. P4 replaces the `(role, content)` pair with an
attributed `SessionMessage`.

**The transcript is not continuously shared.** It is re-read only when an agent
rebinds to a different chat, so an agent misses a peer's interleaved reply on
consecutive turns in one thread. P5 replaces that gate with a watermark.

## Non-goals

- **A second journal.** The host owns the append-only log; this crate borrows it
  through a port. Messages are addressed by sequence number across surfaces the
  host owns (reactions, board cards, run rows), so a second log cannot be made
  consistent with the first.
- **A web framework, or HTTP handlers.** Routes stay with the host.
- **Unbounded fan-out.** A step may authorize a *round* of concurrent turns, but
  never more than `round_width` of them and never without an approval in sight —
  the bound is the invariant, and the serialization never was. Independence is
  still a visibility filter: members writing simultaneously cannot read each
  other, so a concurrent round is a blind round. See
  [ADR 0014](docs/adr/0014-a-round-authorizes-concurrent-turns.md), which
  supersedes ADR 0002 on the terms ADR 0002 itself set, and P21 below.
