# Adapter live examples

| File | Purpose |
| --- | --- |
| `live_language/` | Exact completion and audience evidence, plus offline regressions |
| `live_language.rs` | Loads a live-evaluated language package, creates two model-backed seats through the management adapter with a shared memory binding, and verifies native completion, private delivery and leave access. |

Run from the repository root, after the language live example has produced its
accepted package:

```sh
cargo run -p tinyhivemind-openhuman --features offline --example live_language -- \
  --live target/live-self-edit-run-1/accepted-package.json target/live-language-host.json
```

Requires `OPENROUTER_API_KEY`; `OPENROUTER_MODEL` optionally overrides
`openai/gpt-oss-120b:nitro`. Incidental backend calls use a local fixture;
inference uses OpenRouter. This host supports inert, read-only run memory
bindings: no recall or store is attached. It verifies identity installation,
not persistent memory contents or cross-run learning.

The completion check matches the assignment ledger’s exact completion sequence,
episode and author. Privacy is checked in both directions by task receipt
sequence. Package seats named `host` are rejected before registration to keep
model actors distinct from the privileged management principal. Run the offline
regressions with:

```sh
cargo test -p tinyhivemind-openhuman --features offline --example live_language
```
