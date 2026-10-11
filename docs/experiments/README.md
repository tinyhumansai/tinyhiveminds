# Experiments

Dated records of running the library for real. An experiment answers a question
that was written down before the numbers existed, reports what happened run by
run so a small sample reads as one, and says plainly what should change because
of it — including when the answer is that the mechanism under test should not
ship.

These are not marketing. Several of the records below are negative results, and
one supersedes itself in its own opening paragraph. That is the point: an
experiment that could not have lost was not an experiment.

## Conventions

The filename is `YYYY-MM-DD-a-short-slug.md`. Each record opens with its
**Date**, a **Status** (Recorded, or Recorded and later superseded), the
**Code** that produced it — the exact example and flags — and links to the
**Spec** it tests and the **Decision** it fed. State the sample size, and mark
an estimate as an estimate. Keep every file at 500 lines or fewer; when a matrix
outgrows that, split the raw rows into a companion file and link it, as
[`2026-09-05-expert-delegation-live-matrix.md`](2026-09-05-expert-delegation-live-matrix.md)
does.

## The record

| date | question | answer |
| --- | --- | --- |
| [2026-10-10](2026-10-10-live-hive-language.md) | Can a live model propose a guarded hive edit and evaluate the lowered candidate? | Three repeated smoke runs accepted a 3/3 correction and rejected a 0/3 negative control; lineage and immutable-setting guards held. |
| [2026-09-22](2026-09-22-live-through-the-crate.md) | Do the crate, the driver and the ledger hold under a real model and router? | Yes. Four runs: the first hunted for a diff, the second pooled all four private facts, the third reached quiescence with queue, hold and a five-hop handoff chain, the fourth ran real conversations and lost its handoffs to a refused in-thread broadcast, the fifth joined chain and conversations and found that every refusal inside a thread breeds a phantom call, the sixth dropped `post` and produced both deliverables and all four facts, the seventh stalled on two seats that typed their handoff deliverables instead of recording them, the eighth -- told how to record them -- reached quiescence with both deliverables in nine turns, the ninth ran a second desk built to fire the budget, the broadcast that completes its author, and the in-thread `ask` refusal -- the first two fired, and the tenth fired all three and reached quiescence with every private fact on the desk |
| [2026-09-22](2026-09-22-live-through-the-raw-runner.md) | Do the driver, the ledger and the record hold with no MCP server -- raw sessions with the tools on their belt? | Yes, on the first attempt once the runner booted the core as a library host: eleven turns, three conversations, every handoff placed by routing, both deliverables on the desk, every refusal in the record's words; the one thing found was that a raw session with no ambient core context is treated as the desktop and waits on a sign-in |
| [2026-09-22](2026-09-22-the-driver-under-the-benchmark.md) | Does the real completion driver cost what the prototype loop cost? | Within two percent on every column; the one wrong prediction was a host delivering after the turn instead of before; rerun under ADR 0024 matches to the second decimal, after the simulated host learned to take a completion its own broadcast had already made, and the driver learned not to let a broadcast complete an assignment the seat had not seen |
| [2026-09-01](2026-09-01-live-hidden-profile.md) | Does a live room beat a poll on a problem that has an answer? | The synthetic brief was not enough; a real problem was needed to tell them apart |
| [2026-09-01](2026-09-01-refutation-and-grounds.md) | Do refutation and evidential grounds earn their place? | The arm was able to lose, and it lost |
| [2026-09-02](2026-09-02-federated-hidden-profile.md) | Can several channels pool what only one of them knows? | Yes, across one referral hop at a time |
| [2026-09-05](2026-09-05-expert-delegation.md) | Who knows, and what is that worth? | The directory and deferral did not improve accuracy; the directed ladder arm leaks the correct topic. See the [full live matrix](2026-09-05-expert-delegation-live-matrix.md) |
| [2026-09-07](2026-09-07-do-asides-help.md) | Do private asides help a room decide? | No — a pairwise check is pure cost, and privacy is never the variable. Partly superseded |
| [2026-09-07](2026-09-07-private-asides.md) | What does an aside look like on a live desk? | Three agents, one desk, recorded end to end |
| [2026-09-07](2026-09-07-why-asides-lose.md) | Why did asides lose, and what is peer information actually worth? | +9 to +31 points for the information; the floor turn was what cost too much |
| [2026-09-07](2026-09-07-pe1006-desk.md) | Can a desk of real agents close one genuinely hard problem? | The harness works; the problem is not closed |
| [2026-09-09](2026-09-09-topology-at-scale.md) | At what room size does the channel start to matter, and which channel? | Floor-bound mechanisms reach 0% on this hidden profile by 32 members; an off-floor aimed check retains a small gain. A twelve-desk federation exhausts turns asking for referrals |
| [2026-09-09](2026-09-09-run29-narration-as-speech.md) | Does the account fold on a live run? | Aborted at turn 2: a router rung advance killed both turns, and the host delivered their narration as speech. Both fixed; nothing about folding established |
| [2026-09-09](2026-09-09-desk-lessons.md) | What should be built next after PE 1006? | Working notes and measurements from runs 21–27 |
| [2026-09-08](2026-09-08-pe1006-tool-room.md) | Does a tool-call room with a standing account beat a fenced one? | PE 1006 solved; re-reading fell to 3% of calls, but no fold ever fired |
| [2026-09-17](2026-09-17-jev-decision-evaluation.md) | What does Jev buy and cost against GPT-5-mini strict JSON on typed hive decisions? | 43–55× lower tail latency and 99.2% lower estimated cost, with an 11.45-point accuracy tradeoff concentrated in evidence scoring |
| [2026-09-18](2026-09-18-deepswe-openhuman-smoke.md) | Can the offline OpenHuman hive solve a real three-task DeepSWE smoke? | No: 0/3; every run exhausted the bounded round budget with an empty patch |

## Reading order

[`2026-09-07-do-asides-help.md`](2026-09-07-do-asides-help.md) →
[`2026-09-07-why-asides-lose.md`](2026-09-07-why-asides-lose.md) is the clearest
worked example of the method: a mechanism is specified, measured, found to lose,
and then decomposed until the part that actually carried the value is separated
from the part that was paying for it. The decisions that followed are
[ADR 0011](../adr/0011-an-aside-rides-alongside-a-turn.md) and
[ADR 0012](../adr/0012-an-exchange-round-spends-model-calls-not-turns.md).

[`2026-09-07-pe1006-desk.md`](2026-09-07-pe1006-desk.md) →
[`2026-09-09-desk-lessons.md`](2026-09-09-desk-lessons.md) are a pair: what
happened, then what to build because of it.
[`2026-09-08-pe1006-tool-room.md`](2026-09-08-pe1006-tool-room.md) closes that
thread: the desk finally solves PE 1006, and the record separates the part of
the win that was measured from the mechanism that never ran.
[`2026-09-09-run29-narration-as-speech.md`](2026-09-09-run29-narration-as-speech.md)
is the first attempt to run that mechanism, and is a defect report rather than
a measurement: it establishes nothing about folding and two things about how a
turn can fail without anybody noticing.

The harnesses that produce these numbers are documented in
[`../../crates/tinyhivemind-hive/examples/bench/README.md`](../../crates/tinyhivemind-hive/examples/bench/README.md)
; the historical `crosstalk` harness has been retired. The headline benchmark report lives on the wiki's
[Benchmarks](https://github.com/tinyhumansai/tinyhivemind/wiki/Benchmarks) page.
