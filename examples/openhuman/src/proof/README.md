# Supplied-agent acceptance proof

`types.rs` holds the host fixture and factory handles.

`fixture.rs` constructs one host-owned runtime, per-agent skill bundles,
private MCP servers, private workspaces, and host-authored prompts.
Skills use the supported explicit workspace discovery root; the example does
not use `AgentSpec::skills_dir`'s separate agent-home installation path.
Native `use_skill`, MCP catalogue and MCP invocation receipts prove that those
installed resources are accessible independently of prompt text. Each
continuing agent repeats those native calls after registration; new phase-tagged
call IDs tie success assertions to their newly returned receipts. It also records actual model
requests and returns deterministic native tool calls.

`topology.rs` binds conversations started before registration, then delivers
real hive episodes in four topology shapes. Request assertions cover continuing
history, configuration isolation, and stable prompt/schema tool definitions.

`dynamic.rs` installs host authorization and a factory, exercises management
through model tool calls, and sends work to the newly configured agent.

`test.rs` runs the whole example on a Tokio runtime whose worker stacks match
OpenHuman's embedded loop requirements. The benign stdio MCP fixture requires
`python3`; all other services run on loopback and require no credentials.

The management agent has native write authority as well as the host management
authorizer. Prompt preservation compares the exact host-authored system row;
native policy-repair and permanent-tool rows are checked separately from it.

The pinned native generic MCP bridge requires an acting access tier even for
the benign evidence server. These supplied-agent fixtures therefore use full
access with an explicit three-tool native scope (`use_skill`, `mcp_list_tools`,
`mcp_call_tool`) and a per-server allowlist containing only that agent’s evidence
verb. Host authorization independently refuses the denied management template.
The separate manifest-backed basic hive retains its read-only access proof.
