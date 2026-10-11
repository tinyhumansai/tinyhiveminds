# Implementation plans

> Plans record the implementation sequence at the time they were written.
> Some paths and optional mechanisms in older plans were retired by
> [ADR 0027](../adr/0027-retire-unused-adapters-and-optional-mechanisms.md);
> use current crate documentation for active APIs.


Plans turn an accepted specification into a reviewable sequence of small,
verifiable changes. They explain how to build the behavior; the linked
specification remains the source of truth for what the behavior must be.

Use the same kebab-case stem as the specification. A useful plan includes:

- a link to the accepted specification;
- the goal, non-goals, and assumptions relevant to implementation;
- ordered tasks with exact file paths;
- a failing test before each behavior change;
- the minimal implementation needed to pass that test;
- documentation and public-export updates;
- focused and full verification commands;
- a completion checklist updated as tasks land.

Prefer tasks that can be implemented and reviewed independently. Include short
code snippets when they remove ambiguity, but do not paste entire future files
into the plan.

See [`example-retry-policy.md`](example-retry-policy.md) for a test-first sample.

## Plans

- [`2026-10-10-openhuman-hive-config.md`](2026-10-10-openhuman-hive-config.md) — accepted implementation sequence for configuration, deployment and multi-hive examples.

- [`chat-identity.md`](chat-identity.md) — implemented P1 conversation identity.
- [`desks.md`](desks.md) — P2 desk DTOs, validation, and membership overlay.
- [`mentions.md`](mentions.md) — P3 roster and mention resolution.
- [`sessions.md`](sessions.md) — P4 attributed session projection and team
  initialization.
- [`continuous-sharing.md`](continuous-sharing.md) — P5 watermark-based
  continuous transcript sharing.
- [`responders.md`](responders.md) — P6 responder ladder and selector port.
- [`mention-dispatch.md`](mention-dispatch.md) — P7 bounded dispatch decision
  and atomic enqueue port.
- [`hive-mind.md`](hive-mind.md) — P8 the `tinyhivemind-hive` crate and its
  deliberation episode.
- [`off-floor-exchange.md`](off-floor-exchange.md) — private exchange that takes
  no floor, bounded by a host-set budget in model calls.
- [`promote-the-utterance-surface.md`](promote-the-utterance-surface.md) — move
  the room's tool surface and the utterance fold from the `desk` example into
  `tinyhivemind::speech`, and give a refused aside a way back to its author.
- [`fold-by-size.md`](fold-by-size.md) — trigger the standing account on the
  size of the scrollback, not only its row count, and give a joining seat the
  account by contract.
- [`a-real-provider-layer.md`](a-real-provider-layer.md) — back the `desk`
  example with `tinyinference` instead of `curl`, and render the room's tool
  surface through `tinytools`, both as example-only dev-dependencies.
- [`jev-integration.md`](jev-integration.md) — land P16, typed selection,
  probabilistic quorum, the native Jev adapter, and paired evaluation.
- [`jev-first-routing.md`](jev-first-routing.md) — host-neutral conversation
  surfaces, Jev-first desk routing, bounded hive invitations, and fallback.

- [`2026-10-10-hive-language.md`](2026-10-10-hive-language.md) — implement the accepted hive language and adapter conversions.
