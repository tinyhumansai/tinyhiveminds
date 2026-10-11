# TinyHivemind OpenHuman adapter

The host constructs configured `Agent` instances on one OpenHuman `Runtime`.
`OpenHumanHost` receives those handles, attaches permanent hive tools, and hands
continuing turns to the durable `tinyhivemind-hives` coordinator.

```rust,ignore
let coordinator = Coordinator::new(
    runtime.runtime_id().into(), Arc::new(MemoryStorage::new()),
    CoordinatorOptions::default(),
).await?;
let host = OpenHumanHost::new(runtime.runtime_id().into(), coordinator)?;
host.register_agent(agent).await?;
```

Register an existing conversation with `register_agent_in_session(agent, session_id)`.
Repeated clones are idempotent. Failed durable registrations retain the same
attachment source for retries, while tool execution stays disabled until
registration succeeds. Agents from another runtime are rejected.
The adapter never reconstructs an agent or clears its conversation.
Session registration validates and commits the supplied binding before the
runner is visible to a live scheduler. A storage failure publishes neither.

The native tools cover discovery, reads, hive and direct messaging, and explicit
episode actions. All schemas bind the sender to the supplied agent. Tools remain
in the system catalogue and provider schemas, even when unrelated tools use deferred
discovery. Membership changes do not multiply definitions.
The ordinary family has nine tools; management adds four. `hivemind_read`
requires exactly one hive or peer destination. Peer reads expose the caller's
durable direct messages and replies without scheduling a return turn.

Configure `with_management(factory, authorizer)` before registering or cloning
the host to enable the four management tools. The host factory creates fully
configured agents from nonsecret template references. Authorization runs before
factory or coordinator mutations.

`TurnHooks` provides per-turn options (`prepare` → `TurnOptions { cwd }`),
progress, a scoped turn wrapper, and usage/approval finalization on both
successful and failed turns. Every hook receives the turn's `TurnScope` (agent,
episode, message ids, senders, destination, thread). The default wall is 300
seconds; `with_turn_timeout` changes it. Return `TurnDisposition::Parked` to
hold the agent until coordinator `release`, or `release_with(agent, note)`,
whose note the runner renders at the top of the next turn's prompt.
After a successful provider turn, finalization failure preserves its committed
session while interrupting delivery and suppressing staged actions and replies.

`with_send_policy(Arc<dyn SendAuthorizer>)` gates `hivemind_send_agent`,
`hivemind_send_hive`, `hivemind_ask` and `hivemind_broadcast`; a refusal is
the tool's error text, not a turn failure. `replace_agent(agent_id, build)`
rebuilds a registered agent's handle after any running turn, keeping its
session and reattaching its tools. OpenHuman ids stay unique while a clone
lives, so the adapter drops its handle before calling `build`.

## Hive memory

Seats can share one OpenHuman memory per hive. Configure it before registering:

```rust,ignore
let memory = HiveMemory::for_hive("run-42")?;            // root `team:run-42`
let host = OpenHumanHost::new(runtime.runtime_id().into(), coordinator)?
    .with_hive_memory(memory)?;
let scout = host.register_spec(&runtime, AgentSpec::new("scout")).await?;
```

`register_spec` binds the seat's spec with
`AgentSpec::memory(MemoryBinding::new("scout").root("team:run-42"))`. Each seat
logs its turns under its seat id and recalls everything shared below the root.
An agent built elsewhere must carry `HiveMemory::bind`'s binding, or
registration fails with `Error::UnboundSeat`. An invalid hive id, root, or seat
id fails with `InvalidMemoryRoot` or `InvalidMemoryAgentId`. The default root is
refused because it isolates nothing.
`HiveMemory::recall_budget_tokens` plus `HiveMemory::configure` on the runtime's
base config size the per-turn pack.

The host supplies the engine: `[memory] engine = "cortexdb"`,
`[memory.engines.cortexdb] endpoint = "<url>"`, and the API key in the keychain
under `memory-cortexdb`. Seat sessions persist in OpenHuman across turns, and
compaction is the only thing that drops history from the live context.

`HiveMemoryStore` implements core's `Recall` and `Remember` ports over the same
engine and namespace. Use `HiveMemoryStore::from_config(&config, memory)` for
the engine OpenHuman binds, or `HiveMemoryStore::new(engine, memory)` for one
the host supplies. It serves session-start, rejoin and compaction recalls and
structured entries (failed attempts are marked). OpenHuman already injects a
memory pack into every bound seat's turn, so the adapter's runner adds no
second block. See
[`src/memory`](src/memory/README.md) and the wiki's Host integration page.

The host must keep its `OpenHumanHost` alive while attached tools are in use.
Attachments keep weak service references; dropping it releases the coordinator
without retaining an agent/attachment cycle. Definitions remain available on a
surviving agent, and execution reports unavailable services.

Runnable host construction and topology proofs are in
[`examples/openhuman`](../../examples/openhuman/README.md).
See the [OpenCompany migration guide](../../docs/opencompany-migration.md) for
construction, single-runtime dependency unification, and recovery.

For a model-backed package registration check, see the
[live language example](examples/README.md).
