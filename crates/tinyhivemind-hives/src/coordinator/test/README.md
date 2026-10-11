# Coordinator tests

| File | Responsibility |
| --- | --- |
| `options.rs` | Scheduling option defaults, retention settings and strict serialized fields |
| `mod.rs` | Fixtures, registration, visibility, shared sessions, and child asks |
| `lifecycle.rs` | Parked release, recovery, cancellation, session adoption, shutdown |
| `scheduling.rs` | Concurrent agents, membership snapshots, walls and budgets |
| `privacy.rs` | Private child reads, SQLite reopen, addressed-thread attribution |
| `failures.rs` | Boundary errors and failed-runner isolation |
| `review_regressions.rs` | Leave-before-claim admission, private initial outputs, direct replies |
| `transactions.rs` | Incremental appends, conflict reload/retry, storage failure, cancelled-turn flush, retention |
| `observation.rs` | Host transcript reads, the revision watch, episode phases |
| `starters.rs` | Host-chosen starters, their validation, and the unchanged wire form |
| `release.rs` | Release notes delivered once, across restart, and the unchanged wire form |

Tests use scripted runner futures, barriers, and notifications. They require no
network, clock-based sleeps, or OpenHuman model calls.

`registration.rs` exercises atomic session adoption with a live scheduler and
pending recovered work, mismatches and failed storage commits.

`finalization.rs` covers failed finalizers, retained sessions, suppressed staged
actions, invalid session rejection, and SQLite reopen.

`settings.rs` pins frozen per-hive policy/roles and durable scheduled authority,
including recovered work, delegated children and origin-conflicting retries.
