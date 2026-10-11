# Deployment regression tests

All model and MCP traffic uses loopback wiremock fixtures. Native runtime tests
share the host tests' process lock; approval folds need no native runtime.

| File | Behavior |
| --- | --- |
| `mod.rs` | Missing host ports fail before runtime construction |
| `support.rs` | Manifest, runtime, context and credential fixtures |
| `preflight.rs` | Unsupported native representations and reservation bounds |
| `construction.rs` | One runtime, roles, memory, storage and cycle-free drop |
| `permission.rs` | Captured restrictions, unknown effects and explicit Ask handler |
| `turns.rs` | Actual effects, continuing roles, parking and release |
| `factory.rs` | Typed dynamic creation and catalog restrictions |
| `workflow.rs` | Concurrent invocation identity, isolation and durable private-child origin |
| `mcp.rs` | Real MCP envelopes refuse scheduled effects before remote execution |
| `cancellation.rs` | Native deadlines reap registered subprocesses before returning |
| `delegation.rs` | Real native delegates preserve referenced-child effect restrictions |
| `sends.rs` | All four sends enforce membership, destinations and authenticated sessions |
| `runtime.rs` | Provider credentials, diagnostic redaction and native bound mappings |
| `profile.rs` | Native inherited definitions, memory-safe tool surfaces, skills and MCP configuration |

`timeout.rs` verifies inherited deadlines and host-hook narrowing.
