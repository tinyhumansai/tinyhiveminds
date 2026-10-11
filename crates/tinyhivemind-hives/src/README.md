# Coordinator source

| File or directory | Responsibility |
| --- | --- |
| `lib.rs` | Public exports and minimal usage example |
| `error.rs` | Typed validation, persistence, and conductor failures |
| `coordinator/` | Dynamic registration, messaging, and scheduling |
| `storage/` | Serializable snapshots and transactional storage implementations |

OpenHuman types belong in the adapter crate. Core algebra remains pure.

The coordinator freezes per-hive `HiveSettings` and scheduled job provenance at
acceptance; its settings module owns the typed configuration boundary.
