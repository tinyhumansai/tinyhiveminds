# Language examples

| File | Purpose |
| --- | --- |
| `live_self_edit/` | Provider-failure regression tests and their directory guide |
| `live_self_edit.rs` | Opt-in OpenRouter proposal, exact-match held-out evaluation, negative control and telemetry/contamination guard checks |
| `self_edit.rs` | Propose a prompt delta, guard the candidate, supply a host evaluation, and archive accepted and rejected designs |

Run from the repository root:

```sh
cargo run -p tinyhivemind-lang --example self_edit
```

The example uses a deterministic toy rubric: a prompt containing `evidence`
scores higher. This demonstrates host ownership of evaluation and acceptance;
it does not measure model quality or execute the benchmark harness. The archive
retains a rejected candidate while `accepted_head` selects the last accepted
candidate. Neither evaluation nor archive persistence opens a store or network.
A real host substitutes its held-out evaluation and persists candidate snapshots
with their records, including persistent memory watermarks when present.

## Live model check

```sh
cargo run -p tinyhivemind-lang --example live_self_edit -- \
  --live target/live-self-edit-run-1
```

Set `OPENROUTER_API_KEY` in the environment. `OPENROUTER_MODEL` optionally
changes the default `openai/gpt-oss-120b:nitro`. The example host invokes curl;
the library retains its pure boundary. Credentials pass on curl stdin and are
excluded from artifacts. Every model request, response, candidate and lineage
record is saved below the chosen output directory, which must be different
for each run.

The incumbent deliberately contains a status-filter bug. The editor sees that
training feedback, but not the three held-out invoice inputs. The host checks
numeric answers exactly, requires all three to pass, and rejects a deliberately
broken prompt after evaluating it with the same live model. It also requests a telemetry mutation and checks a contamination canary.
A provider refusal is retained explicitly; the host then tests the requested
mutation fixture itself and records its origin. This is a small
integration smoke test, not evidence of general self-improvement on SWE tasks.

Use the resulting `accepted-package.json` with the adapter's
[live language example](../../tinyhivemind-openhuman/examples/README.md) to
exercise native tool completion and registration from the lowered package.

Failed provider calls retain redacted response bodies in `response-N.error`
and include a bounded, redacted stderr excerpt in the returned error. Run the
offline regression with `cargo test -p tinyhivemind-lang --example live_self_edit`.
