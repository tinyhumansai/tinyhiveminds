# Adapter modules

| Path | Responsibility |
| --- | --- |
| `deploy/` | One-runtime manifest assembly, captured permissions, approvals and workflows |
| `config/` | Typed manifests, relative Markdown loading and permission validation |
| `host/` | Supplied handle registration, continuing sessions, host hooks and management |
| `tools/` | Stable native specifications, argument validation and bound execution |
| `memory/` | Hive-shared OpenHuman memory: the hive root, per-seat memory agent ids, seat binding, and core `Recall`/`Remember` |
| `language/` | Portable definition conversions for coordinator settings, seat requests and memory bindings |
| `journal/` | Optional in-memory host log used by standalone research examples |
| `offline/` | Feature gated loopback backend and host runtime configuration fixtures |
| `error.rs` | Typed adapter errors |
| `lib.rs` | Public exports and registration example |

The host supplies runtime construction ports; `config/` provides the typed
deployment input for MCP, skills, memory and prompts; `memory/` only decides which root and memory agent
id each seat is bound to. Durable scheduling and episode state belong to `tinyhivemind-hives`.
