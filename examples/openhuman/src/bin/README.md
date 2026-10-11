# OpenHuman proof binaries

| File | Purpose |
| --- | --- |
| `multi_hive.rs` | Offline/live one-runtime shared-seat proof: distinct roles, allowed/refused file writes, explicit safe recovery, continuing session. |
| `basic_hive.rs` | Offline or live two-agent OpenHuman quickstart: host sessions, hive membership, private tasks, leaving, and session continuity. |
| `deepswe_hive.rs` | Hermetic four-agent external software-engineering adapter with host-side OpenHuman and Docker-confined tools. |
| `deepswe_hive/` | Task validation, MCP tools, Docker confinement, and contract tests for the DeepSWE adapter. |
| `pe1006_hive.rs` | Web-assisted five-agent experiment driven by `tinyhivemind_core::driver`, with a durable shared workspace, explicit agent memory, and one OpenHuman runtime per run. |
| `pe1006_hive/episode.rs` | PE1006/PE1008 roles, routing candidates, and completion-tool compatibility parsing. |
| `pe1006_hive/round.rs` | Frozen prompt/snapshot preparation and bounded concurrent OpenHuman turn execution. |
| `pe1006_hive/tools.rs` | Local MCP completion tools and per-seat outbox persistence. |
| `pe1006_hive/typesafe.rs` | Live TypeSafe transport plus the shared routing policy and thread context. |
| `pe1006_hive/workspace.rs` | Workspace templates, non-overwriting initialization, and exact per-turn prompt/reply snapshots. |
