# Specifications

Specifications define what the system must do before implementation details
take over. Create one for behavior that changes a public API, crosses module
boundaries, introduces a durable data format, or has meaningful operational
constraints.

Use a short kebab-case filename such as `retry-policy.md`. Each specification
should contain:

1. **Status and owner** — Draft, Accepted, Implemented, or Superseded.
2. **Problem** — the user or system need, without prescribing a solution.
3. **Goals and non-goals** — the exact boundary of the work.
4. **Proposed behavior** — public API, inputs, outputs, errors, and examples.
5. **Invariants and constraints** — properties every implementation must keep.
6. **Acceptance criteria** — externally observable pass/fail conditions.
7. **Open questions** — unresolved decisions that block acceptance.

After the specification is accepted, create a linked implementation plan in
[`../plans/`](../plans/README.md). Keep code snippets small enough to clarify
the contract; production code still belongs under `src/`.

See [`example-retry-policy.md`](example-retry-policy.md) for a complete sample.

## Current specifications

- [`openhuman-hive-config.md`](openhuman-hive-config.md) — accepted: one OpenHuman runtime, reusable profiles, persistent seats across hives, permission narrowing and configured workflows.

- [`dynamic-hives.md`](dynamic-hives.md) — implemented: supplied agents on one
  runtime, permanent tools, dynamic membership, durable coordination, and
  continuing sessions. See the [migration guide](../opencompany-migration.md)
  and [runnable host example](../../examples/openhuman/README.md).

[ADR 0027](../adr/0027-retire-unused-adapters-and-optional-mechanisms.md)
retires several optional mechanisms and waiting wrappers. Older specifications
remain here as design records; their API sketches are not current usage guides.


- [`chat-identity.md`](chat-identity.md) — the four stored spellings of the
  General conversation.
- [`desks.md`](desks.md) — host-owned desk overlays and the borrowed membership
  algebra.
- [`grammar.md`](grammar.md) — the index to the authoritative grammar
  reference for both textual grammars, with the code/prose discrepancies it
  resolved.
  - [`grammar-mentions.md`](grammar-mentions.md) — the complete `@` grammar:
    lexical rules, the alias table, resolution, normalization, and which
    mention wins for each consumer.
  - [`grammar-traces.md`](grammar-traces.md) — the complete `!marker` grammar:
    the eight parsed kinds, including two whose policy effects were retired, the `#topic`, `>target` and `^cite` qualifiers, fence
    masking, and the markers that fail closed.
- [`mentions.md`](mentions.md) — roster records, mention grammar, normalization,
  and pure routing decisions.
- [`sessions.md`](sessions.md) — host-owned paging, attributed projection, and
  ephemeral team initialization.
- [`continuous-sharing.md`](continuous-sharing.md) — caller-owned watermarks
  and stateless attributed transcript deltas.
- [`responders.md`](responders.md) — deterministic one-responder selection and
  the model selector boundary. The waiting selector wrapper was retired.
- [`mention-dispatch.md`](mention-dispatch.md) — bounded one-target dispatch,
  the former atomic host enqueue contract, and refusal sentences. The runtime
  wrapper was retired.
- [`cross-desk-referral.md`](cross-desk-referral.md) — one bounded child turn
  that may run on another channel, and the one answer that comes back. The
  runtime queue wrapper was retired.
- [`hive-mind.md`](hive-mind.md) — bounded group deliberation: traces, salience,
  quorum with cross-inhibition, and the attention market. Its optional
  character allocator was retired.
- [`recall.md`](recall.md) — one selection ranking, the roster and desk
  pickers, bounded transcript search with optional regular expressions,
  pinning as a fold, and the stated per-message budget. Standalone
  find/select/search entry points were retired.
- [`hive-memory.md`](hive-memory.md) — a seat's persistent session, the desk
  delta by watermark, and the host memory ports recalled at session start, on
  rejoin, and after compaction. See
  [ADR 0030](../adr/0030-a-host-memory-port-feeds-seat-sessions.md).
- [`jev-integration.md`](jev-integration.md) — typed routing distributions,
  admission-gated probabilistic quorum, approval narrowing, and paired Jev
  versus strict-JSON evaluation.
- [`jev-first-routing.md`](jev-first-routing.md) — Jev-first primary selection,
  one reasoning escalation, and bounded specialist invitations.
- [`conversation-surfaces.md`](conversation-surfaces.md) — explicit desk,
  direct, general, and workflow semantics at the host boundary.
- [`completion-driven-episodes.md`](completion-driven-episodes.md) — explicit
  per-agent completion and TypeSafe-routed agent broadcasts.
- [`opencompany-routing-compatibility.md`](opencompany-routing-compatibility.md)
  — the earlier routing boundary; dynamic-hives supersedes its OpenHuman
  construction and session-seeding sketches.
- [`approval.md`](approval.md) — a pure gate for a side-effecting action:
  total approval, standing grants, and epoch-scoped consent. The runtime
  waiting wrapper was retired.
  - [`approval-testing.md`](approval-testing.md) — the full failure-path test
    matrix, split out to keep the specification within its line budget.

## Superseded experimental specifications

- [`refutation-and-grounds.md`](refutation-and-grounds.md) — a negative
  evidence-to-topic link, grounds weighed by evidential depth, and grounded
  objections.
- [`expert-delegation.md`](expert-delegation.md) — a transactive-memory
  directory folded from grounded deposits and the citations they drew,
  `BidReason::Knows`, and `!defer`.

The directory fold remains available. Its episode attention wiring and the
other optional mechanisms were measured and retired by
[ADR 0027](../adr/0027-retire-unused-adapters-and-optional-mechanisms.md).

## Draft and proposed specifications

- [`thread-scoped-conversations.md`](thread-scoped-conversations.md) — proposed:
  which half of OpenCompany's threads epic this layer owns, and which stays with
  the host.
- [`shared-medium-schema.md`](shared-medium-schema.md) — draft: what a projected
  message carries, per-conversation read state, digests, and supersession.
- [`private-asides.md`](private-asides.md) — draft: an audience on a stored row
  and a viewer on a query, so two agents on one desk can compare notes without
  the desk reading them; what a non-member sees instead, and what the exchange
  owes the room when it ends.
- [`seat-continuity.md`](seat-continuity.md) — draft: the private, superseding
  notebook a seat carries between turns, the one feedthrough row a turn that
  wrote files leaves behind, and the brief appended once. The `desk` example
  used during exploration has been retired.
- [`folding-by-size.md`](folding-by-size.md) — draft: the standing account
  folds when the room is large rather than only when it is long, stated as a
  character budget a host writes as a token budget, so a seat joining a big
  room is always handed an account of it.
- [`the-utterance-surface.md`](the-utterance-surface.md) — draft: the algebra of
  what a seat says; this was first explored in the retired `desk` example and moved into
  `tinyhivemind::speech` — the tool descriptions as data, one fold from an
  utterance to a row, and a policy refusal that reaches its author while the
  turn is still running. The mention grammar keeps the routing.
- [`thoughts-and-channels.md`](thoughts-and-channels.md) — draft: a seat's
  output text is its own thinking and it reaches the room through tools with a
  schema (`desk_post`, `desk_dm`, `desk_read`); a channel carries one bounded,
  superseding account of everything older than its live tail, folded only from
  rows every member may read. The library half is `tinyhivemind::digest`.

## Decisions these specifications rest on

An accepted specification cites the record that settled its contested question
rather than restating it. Two run across several specifications:

- [`off-floor-exchange.md`](off-floor-exchange.md) — draft: private exchange
  in rounds between turns, taking no floor and bounded by a host-set budget.
- [ADR 0008](../adr/0008-an-approval-decision-is-total.md) — an approval
  decision denies rather than fails, so a gate cannot be bypassed by failing.
- [ADR 0009](../adr/0009-a-refusal-renders-what-the-caller-already-holds.md) —
  a refusal renders only what the caller already holds. The library owns the
  words, and every reason that turns on a named other collapses to one shared
  sentence, so a set of refusals cannot be probed for a roster. It settles what
  [`mention-dispatch.md`](mention-dispatch.md), [`responders.md`](responders.md)
  and [`approval.md`](approval.md) each left open.
- [`working-memory.md`](working-memory.md) — the port seats carry observations across activations through, with the engine left to the host ([ADR 0029](../adr/0029-working-memory-is-a-host-adapter.md)).

- [`hive-language.md`](hive-language.md) — pure declarative hive packages, guarded edits, memory bindings, and evaluated lineage.
