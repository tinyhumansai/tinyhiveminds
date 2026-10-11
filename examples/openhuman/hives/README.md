# Checked-in OpenHuman hives

| Directory | Runtime and seats |
| --- | --- |
| `basic_hive/` | Read-only Alice and Bob; ephemeral workspace and continuing host sessions |
| `deepswe_hive/` | Lead, implementer, tester and reviewer; two MCP servers, 16 iterations, temperature zero, width four |
| `pe1006_hive/` | Five mathematical/research seats; native CortexDB port, durable Markdown memory, sealed/research context and original workspace templates |
| `multi_hive/` | Three reusable profiles, four persistent seats, two hives sharing one seat with narrower review authority |

Each `hive.json` is parsed through `HiveConfig::load_dir`. Profile Markdown
frontmatter supplies typed template metadata; its body supplies the native
system prompt. Context Markdown is selected explicitly by filename stem.
Conventional `README.md` files are ignored by both directory scanners.

Runtime-only executable paths, outboxes, Docker arguments and model routes are
set by the binary before validation. No manifest carries a literal credential.
The multi-hive live provider uses `{"env":"OPENROUTER_API_KEY"}`; offline runs
replace that reference with a trusted loopback provider port and a non-secret
fixture token. No operator credential is needed.

The Euler binary resolves its existing configured native CortexDB engine and
injects it through `BuildOptions.memory_engine`. If native memory is off because
its credential is unavailable, it continues with durable Markdown memory, as
before. The deterministic lowering test substitutes the native in-process
reference engine and never inspects operator credentials. Its acting directory
is selected per turn; workspace templates remain non-overwriting.

The DeepSWE and Euler hosts retain their pure completion drivers. Declared
routing values feed those drivers; the deployment policy view supplies the
DeepSWE episode width. Coordinator membership and role settings use the same
manifest. Direct native turns are admitted by the pure driver rather than the
coordinator's separate global concurrency gate.

See the [example guide](../README.md) for commands and the native denial/safe
recovery behavior demonstrated by `multi_hive`.
