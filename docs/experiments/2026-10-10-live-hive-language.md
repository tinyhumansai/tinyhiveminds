# Live hive language smoke checks

**Date:** 2026-10-10. **Status:** Recorded.
**Code:** `live_self_edit --live` and `live_language --live` (commands below).
**Spec:** [hive language](../specs/hive-language.md).
**Decision:** [ADR 0032](../adr/0032-hive-language-owns-the-wire-format.md).

Issue: [#112](https://github.com/tinyhumansai/tinyhivemind/issues/112).
Base: merged commit `c1300006`. Provider: OpenRouter,
model `openai/gpt-oss-120b:nitro`, temperature zero for the self-edit checks.

## What the self-edit check exercises

The example host serializes an invoice solver package, parses it back through
the public package API, and lowers its prompt before each model invocation.
The incumbent deliberately contains a status-filter defect: it sums paid,
canceled and pending invoices. An editor sees that training feedback and the
incumbent prompt, but not the three held-out invoice inputs. Its response must
decode as a typed prompt patch, satisfy exact preconditions, and pass `apply`.
The host scores each numeric answer against an independently specified total.

The host accepts only a candidate scoring 3/3 without regressing the incumbent.
A second prompt deliberately says to return zero. It is applied as a candidate,
evaluated through the same live model, and archived as rejected. Every record
is verified against its package; the archive's accepted head must still name
the accepted candidate. The nondominated frontier contains that candidate.

| Successful run | Incumbent | Model-edited candidate | Negative control | Calls | Tokens |
| --- | --- | --- | --- | --- | --- |
| 3 | 0/3 | 3/3, accepted | 0/3, rejected | 11 | 3,063 |
| 4 | 0/3 | 3/3, accepted | 0/3, rejected | 11 | 3,063 |
| 5, final source | 0/3 | 3/3, accepted | 0/3, rejected | 11 | 3,063 |

These are repeated runs of a small fixed suite, not independent benchmark
seeds. The deliberately defective incumbent makes this a controlled integration
check; these results do not establish general SWE quality improvements.

The accepted candidate's identity in all three runs is
`sha256:9c6985bad62e68c130c71645acc3af2156c27695815d47bf6ddd10030c7c9acb`.
The rejected negative control has identity
`sha256:11ba144a1ab939f58cd49d4af55eed51e38ebcbd039e78d98eefca6f59802ebe`.

## Guard outcomes and failed trials

The provider refused to propose disabling telemetry. That refusal is retained
in the evidence and identified as a provider outcome. The host then submits
the requested typed fixture itself, and `apply` rejects it with
`immutable constitution or evaluation settings changed`. A separate prompt
containing the configured contamination canary is rejected as contamination.
Neither attempt changes the accepted package.

Trial 1 failed before application because the editor omitted the required
`operation` discriminator. The harness was corrected to supply the exact wire
shape. Trial 2 completed all numeric evaluations but stopped when the telemetry
request returned a refusal rather than a patch. The harness now preserves that
refusal and distinguishes a host-authored fixture from a model-authored attack.
These were harness protocol failures; no library fix or relaxed guard was used.

## Reproduction and artifacts

From the repository root, with `OPENROUTER_API_KEY` in the environment:

```sh
cargo run -p tinyhivemind-lang --example live_self_edit -- \
  --live target/live-self-edit-new
cargo run -p tinyhivemind-openhuman --features offline --example live_language -- \
  --live target/live-self-edit-new/accepted-package.json target/live-language-host-new.json
```

The first output directory must be empty. Every request and response, the
accepted package and the archive report are retained there; credentials are
passed on curl stdin and never included in those files. The adapter example
uses a local fixture for incidental backend calls and OpenRouter for inference.
Its run identity and durable coordinator snapshot are saved in its output JSON.

Local self-edit evidence is in `target/live-self-edit-run-{3,4,5}/report.json`.
The fast pure-crate iterations used `--target-dir target/lang-check`; build
output stays inside the checkout. Normal workspace validation uses its usual
target directory.

## Scope of the adapter check

The adapter loads the accepted package and creates two seats through
`management_request` and `AgentFactory::create_with_memory`, sharing the declared
`ledger` memory identity. The adapter verifies that the factory installed that
identity and root before registration. The smoke host supports inert read-only
run bindings only: no memory store, recall, persistent learning or cross-run
fork implementation is exercised.

Each model must invoke native `hivemind_complete`, rather than printing a
completion claim. The host checks the exact numeric completion, durable episode
termination, private task visibility and access revocation after leaving.

Three native adapter runs completed successfully, with three independently
created runtimes and six completed private episodes in total:

| Native run | Solver completion | Auditor completion | Shared binding | Privacy / leaving |
| --- | --- | --- | --- | --- |
| 1 | `33` | `5` | Both installed `ledger` at `team:live-language` | Passed |
| 2 | `33` | `5` | Both installed `ledger` at `team:live-language` | Passed |
| 3, final source | `33` | `5` | Both installed `ledger` at `team:live-language` | Passed |

These inputs differ from the direct-evaluation invoice cases. Each native tool
completion was checked against its exact expected total and a finished durable
episode with no failure. Reports are retained at
`target/live-language-host-{1,2,3}.json`. No defect in the merged library was
observed in these checks; changes are confined to reproducible hosts and docs.

The second native run needed two solver turns (two waves) and one auditor
turn; the first used one turn each. Both solver episodes still completed within
the configured walls. The numeric checks above do not assume a single turn.

The final native report also retains the actual attributed completion messages.
Final validation passed: formatting, clippy with warnings denied, all-target/all-
feature build, 1,342 workspace tests including doctests, and the purity assertion.

## Review follow-up, 2026-10-11

The original completion assertion accepted any later matching post; review
identified that this could mask an incorrect completion body. The revised
check matches the assignment ledger's exact completion sequence, episode and
author. Privacy now checks both task receipt identities against the other
seat's view rather than searching one view for a numeric marker. Offline
regressions reproduce unrelated-post false positives and both leak directions.

Failed provider calls retain redacted response evidence and bounded stderr
diagnostics. A package seat named `host` is rejected before registration so
it cannot collide with the smoke host's management principal. These changes
strengthen the harness assertions; no library guard or CI check was weakened.

The standalone `examples/openhuman/Cargo.lock` now includes the language
crate required by the adapter. Its omission caused the hosted locked-build
failure after the initial PR; the workspace-only validation did not cover
that separate lockfile.

A fourth native-host run with these stricter checks completed successfully
against the saved accepted package from run 5. Solver's exact native completion
was `33`, auditor's was `5`, both private task identities remained hidden from
the other seat, and the auditor lost read access after leaving. The local
report is `target/live-language-host-review.json`.

Follow-up validation passed: all four workspace contract commands, the six
focused example regression tests, both purity/pin checks, and
`cargo test --locked --manifest-path examples/openhuman/Cargo.toml --target-dir target`
(63 standalone tests).
