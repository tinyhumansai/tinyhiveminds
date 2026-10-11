# DeepSWE security regression tests

| File | Purpose |
| --- | --- |
| `docker.rs` | Optional real Docker confinement and patch-capture integration proof. |
| `deadline.rs` | Distinguishes cancellation RPCs at the selected native deadline from early provider errors. |
| `security.rs` | Exercises hostile Git/output paths, gitlink refusal, bounded staged action input and patch capture, and verified container cleanup. |
| `retry.rs` | Scripts loopback provider failures and native MCP outboxes to prove exactly-once reconciliation, bounded safe retries, timeout refusal, and atomic round commit. |
| `retry/` | Holds focused response builders used by the retry scenarios. |
