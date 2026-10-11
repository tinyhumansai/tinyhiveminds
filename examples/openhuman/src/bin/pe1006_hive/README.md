# PE1006 hive support

| File | Purpose |
| --- | --- |
| `episode.rs` | Problem-specific roles, semantic candidates, queue helpers, and validated tool-envelope compatibility. |
| `tools.rs` | Local MCP rendering of `broadcast` and `complete_episode`, plus the host-owned event outbox. |
| `typesafe.rs` | Live System One HTTP transport and routing-request construction. |
| `workspace.rs` | Durable shared-workspace templates, initialization, and exact prompt/reply turn snapshots. |
| `test.rs` | Pins the exact 25-turn budget at concurrent-round boundaries. |

Before any pending or missing-tool retry round is launched, the runner requires
the complete concurrent round to fit within the remaining 25-turn budget. It
never launches a partial round merely to consume the remainder.

## Hive memory

| File | Purpose |
| --- | --- |
| `memory/` | Reference `WorkingMemory` adapter: `HIVE_MEMORY.md` with compaction (its own README). |
| `research.rs` | Mirrors the public research sources into the shared workspace before turns start. |

Seats get `hive_memory_recall`, `hive_memory_note` and `hive_memory_forget` from the
`tinyhive` MCP server (served by `tinyhivemind_tools::MemoryTools` over the markdown
adapter), and each turn opens with the recalled entries. The file lives in the
durable workspace, so it outlives a run.

`workspace_test.rs` pins non-overwriting workspace initialization and exact
prompt/reply snapshots. The initial templates are checked-in Markdown context
under `../../../hives/pe1006_hive/context/`.
