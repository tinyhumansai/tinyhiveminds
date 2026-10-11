# Configurable OpenHuman hives

Status: Accepted. Owner: the OpenHuman adapter. Implements
[#113](https://github.com/tinyhumansai/tinyhivemind/issues/113), stages
[#114](https://github.com/tinyhumansai/tinyhivemind/issues/114)–
[#118](https://github.com/tinyhumansai/tinyhivemind/issues/118).
Implementation order: [the plan](../plans/2026-10-10-openhuman-hive-config.md).

## Problem and boundary

A host currently repeats provider, prompt, permission, memory, MCP and seat
construction in each example. A reusable manifest defines the runtime once,
configures agents many times, and allocates those agents as seats into many
hives. Parsing a manifest must not construct agents or resolve credentials.

Implementation belongs in `crates/tinyhivemind-openhuman`: it already names
OpenHuman types and owns agent configuration. The hives crate supplies the
necessary coordinator policy and role boundary. Core changes only add serde to
`ConductPolicy` and `DivisionPolicy`; `CoordinatorOptions` and its embedded
`RetentionPolicy` get serde in hives.
Existing `EpisodePolicy`, `RoutingPolicy` and `ApprovalPolicy` are reused.
Coordinator changes beyond serde are a necessary boundary expansion: per-hive
policy, membership-role and scheduled-origin metadata must reach the existing
host-neutral coordinator. They belong in hives rather than OpenHuman-owned
algebra. No new crate, runtime, storage journal or ADR is required.

[#112](https://github.com/tinyhumansai/tinyhivemind/issues/112)'s future
`tinyhivemind-lang` layer lowers into this manifest. It must use this wire
contract rather than maintain a second deployment schema. `ProfileRuntime`
(openhuman#7250) is out of scope: it cannot coexist with `Runtime` in one
process and represents a future multi-tenant mode.

## Sections and precedence

JSON is canonical, with typed serde representations and defaults. Reject
unknown fields rather than silently ignoring operational settings. Each
named collection has unique, nonempty ids; seat ids also obey OpenHuman's
agent-id syntax. Declaration order is preserved where it affects prompts.

| Section | Meaning | Lowered value |
| --- | --- | --- |
| `runtime` | Exactly one provider/default model, workspace, session store, memory engine, sandbox, skills policy, MCP baseline, autonomy, tool rules and limits | `RuntimeBuilder` |
| `profiles` | Reusable system prompt/context, model/sampling, allowed/denied tools and scope, MCP, skills, subagents, permission reference, memory and turn limits | `Runtime::define_template` plus `AgentDefinitionSpec` and agent-spec settings |
| `permission_profiles` | Named access tier, `SandboxModeSpec`, tool rules, send policy, core approval policy and grants | `Access`, sandbox, `can_use_tool`, `SendAuthorizer` |
| `seats` | Seat id, profile id and typed overrides | `AgentSpec::new(id).extends(profile)` through `OpenHumanHost::register_spec` |
| `hives` | Hive id/name, members and per-hive policies/memory root | Coordinator `ManagementRequest::CreateHive` / `JoinHive` and hive policy/role settings |
| `workflows` | Named schedule, prompt and seat or hive target | `Runtime::cron`: seat `JobTarget::Agent`, hive System-job handler |

Runtime defaults precede profile settings; seat overrides precede membership
turn overlays. Overlays may change role, prompt context and restrictions,
never a provider credential, MCP connection or memory binding. Model defaults
and limits use typed optional fields so omission inherits a value and an
explicit zero or invalid value is rejected. Temperature zero is valid.

Provider credentials and MCP credential-bearing environment/header values
are `SecretRef`: exactly `{ "env": "NAME" }` or `{ "store": "KEY" }`.
Resolved secrets never enter a serializable manifest or error/Debug output.
References are resolved during deployment by a host-supplied resolver;
missing secrets return a typed error containing only the reference name.
A generic JSON escape hatch cannot smuggle inline credentials into overrides.

Runtime storage and engine selections are manifest data; the deployment
constructs supported built-in selections, and accepts explicit host-owned
ports for custom engines/session stores or runtime-only sandbox wiring. An
unsupported selector is an error, not an ignored configuration field.

## Prompt and context files

`HiveConfig::from_json(&str)` and `from_value(Value)` parse without filesystem
access. `HiveConfig::load_dir(path)` reads `hive.json`, `profiles/*.md` and
`context/*.md`; the free `load_dir` convenience may delegate to it.
Files are resolved relative to that directory, never the caller's cwd.

Markdown may start with YAML frontmatter between `---` delimiter lines.
Profile frontmatter carries typed profile fields; its body supplies the
system prompt. The filename stem supplies the id when frontmatter omits it.
Context files retain their Markdown body after removing frontmatter; named
context references must resolve. Malformed or unterminated frontmatter is a
typed parse error. Empty Markdown bodies and ordinary Markdown containing
later `---` lines remain valid. JSON and Markdown defining the same profile
id is a duplicate error rather than unspecified precedence.

Context is composed in declaration order: profile context, seat context, then
membership context. It is instructional text, not configuration authority;
its contents cannot alter sandbox, approval or tools. Scheduled prompts and
messages likewise cannot declare trusted permissions.

## One seat across many hives

A seat is one `Agent` on the one runtime, with one persistent coordinator
session. Register it once even when several hives reference it. Direct
messages continue that session using seat defaults. Hive turns capture the
active hive and membership before running and apply that captured overlay.
Never infer the active hive from a mutable global or the seat's first member.
Native seat-target cron jobs are the explicit exception: upstream runs them
in isolated cron sessions outside the coordinator. They reuse the configured
Agent but do not continue or replace its persistent coordinator session.

Membership has a nonempty freeform `role` label, optional context and optional
permission-profile restriction. The role appears both in the turn prompt
preamble and in `RouteCandidate.role` / `BriefedTeammate.role`; it must reach
routing and delegation, not merely a descriptive JSON field. Role labels are
values, not a hidden registry. An invalid/empty role is a typed role error;
an explicit role reference, if introduced later, must reject unknown labels.

Each hive configures `EpisodePolicy`, `RoutingPolicy`, `CoordinatorOptions`,
`DivisionPolicy` and `ConductPolicy`. The completion coordinator selects that
hive's routing/coordinator/conduct policy when opening and scheduling work.
`HiveSpec.coordinator.conduct_policy` and the separate hive `conduct` field
are simultaneous bounds: lower each effective `turn_wall` and
`child_turn_wall` to the minimum of their two configured values. Neither
field overrides or silently discards the other; omitted fields retain the
existing defaults. Host-neutral `HiveSettings` contains coordinator options,
routing and a seat-to-role map. Capture these settings atomically when opening
an episode and retain that frozen snapshot in `EpisodeRecord`, so a later
settings update cannot change a running episode's authorization or policy.
`EpisodePolicy` and `DivisionPolicy` belong to the separate pure deliberation
mechanism; expose the configured values through the deployment's per-hive
policy view for hosts invoking that mechanism. They must not be presented as
completion-coordinator settings when that coordinator does not consume them.
An attempt to apply unsupported nondefault settings through a completion-only
path returns `UnsupportedSetting`. Its global concurrency cap still bounds
all hives together; a hive's round width cannot exceed that cap. Native cron
scheduler admission has a separate ceiling with the same runtime concurrency
value. Direct native turns bypass coordinator admission; these ceilings do not
claim a combined bound over every native turn.
Persist or otherwise retain policy and role data alongside hive state so
joining, leaving or recovering a hive cannot discard its configured behavior.
Do not parse hive policies and then run every hive with global defaults.

### Memory decision

Use option (a): a default per-seat root `seat:<seat-id>`, shared by that seat
across all its hives. This follows the existing one-session identity and does
not require an impossible per-turn `MemoryBinding` switch. It also avoids
making membership order determine ownership of memory.

Logical roots cannot be passed verbatim to the pinned native namespace parser,
which has no `seat` segment kind. Encode every logical UTF-8 root injectively
below `project:hivemind`, using 64-byte chunks rendered as lowercase hex
`team:<chunk>` segments. Encode native-looking logical roots too, avoiding
pass-through collisions. Reject blank roots or roots over 384 UTF-8 bytes
before boot: at most seven root segments leave one native agent segment.
Expose `native_memory_root` for hosts sharing the same physical namespace.

An explicit profile/seat memory binding overrides that default and remains
fixed for the agent's lifetime. A hive's memory root describes its shared
recall namespace; joining it does not silently rebind the seat. Configuring
two seats is the supported way to obtain isolated sessions or different
roots for the same reusable profile. The deployment configures the binding on the seat spec and does not install
a host-wide hive memory wrapper that would overwrite it. Existing hosts
using `with_hive_memory` retain their established behavior.

### MCP and tools

MCP connections are fixed at agent creation. Runtime baseline and profile/
seat MCP declarations form the seat's one normalized server set. If a
membership declares a different set, validation returns `ConflictingMcp` and
explains that two seats are required. Reordering the same servers is harmless;
changes to endpoint, transport, credential reference or tool allowlist are
meaningful changes. Denying use of a connected server's tools per hive does
not require a new connection set.

Latest OpenHuman `Turn::tools` replaces the turn's host belt, including attached
sources; an empty belt revokes host tools. Builtin tools still follow the agent
definition. Agent and turn permission callbacks are additive: any denial wins.
Thus build the correct episode-bound host belt and install an effective
permission callback that also restricts builtin and MCP calls. Tool visibility
alone is not authorization, and a resumed prompt's stale catalogue must not
restore a denied tool.

## Permission and approval contract

The effective authority is the intersection of runtime baseline, profile,
seat overrides and membership restrictions. A hive must never widen a seat.
Validation rejects widening in access tier, sandbox, tool allow/deny, send
scope and approval behavior. Where arbitrary rule equivalence cannot be
proved, require a conservative structural restriction and reject ambiguous
widening; a claimed restriction is not accepted merely because it has a
smaller allowlist while removing a denial or weakening the sandbox.

A narrower allowlist is a subset; denylists retain every inherited denial.
Read-only cannot become supervised or autonomous. A restricted sandbox
cannot become disabled. Send destinations/operations can only be removed.
Approval rules, approver and standing-grant authority remain inherited or
are explicitly narrowed, never replaced with a more permissive default.
Runtime checks intersect all layers even after validation.

The bridge classifies each tool call into a core `ApprovalRequest` and
`Action`, using authenticated seat identity, active conversation, call id,
host sequence, consent epoch and grant snapshot. Tool classification is
explicit: known read operations are `ReadOnly`; writes and external effects
are `Mutating`; unknown tools/effects are `Unclassified` and deny. Do not
classify every tool as read-only to make an example run.

Call `tinyhivemind_core::approval::approve` for the total verdict. `Allow`
proceeds; native `Deny` refuses the effect and aborts the native turn. Ordinary
denials retain a failed/interrupted coordinator record. A host may explicitly
release a seat with a safe resumption note and enqueue new completion work;
the refused call and failed assignment are never replayed. `Ask` goes to OpenHuman's
`ApprovalHandler` through an adapter-owned correlated pending request.
A hook returning Ask creates neither a native approval-gate record nor a
native approval event; listening only to native `ApprovalHandler` subscriptions
would miss it. The bridge explicitly invokes the injectable handler with a
pending request/call correlation id and never waits for a nonexistent native
event. Returning OpenHuman `Ask` from `can_use_tool` alone denies the call and
is not an approval workflow.
For turns run by the coordinator, a pending host decision makes
`TurnHooks::after_turn` return `TurnDisposition::Parked`. Native seat cron
jobs run outside these hooks: agent-scoped approval settings enforce their
baseline, and they do not claim coordinator parking or host-release behavior. Deny the original tool call without executing its effect. After the turn
parks, the asynchronous handler decision is correlated to the adapter pending
request; explicit host release supplies the decision as a resumption note.
No uncertain turn or refused tool call is automatically replayed. A newly
requested effect must pass the current approval gate again.

The derived `SendAuthorizer` covers direct sends, hive sends, asks and
broadcasts using the captured active-hive overlay as well as seat policy.
Management remains explicitly authorized; `ManifestFactory` implements the
existing `AgentFactory` and validates dynamic overrides against configured
profiles. Unknown templates and secret-bearing overrides fail before creation.

Timeout is profile/seat specific, falling back to `with_turn_timeout`.
Use `Turn::timeout` or awaited `cancellation_handle` rather than dropping a
`tokio::time::timeout` future while tool effects may continue. Iteration and
model-budget limits are also lowered; they are not documentation-only fields.
Native seat cron jobs receive agent-scoped limit/stop policy rather than
assuming the coordinator runner applies timeouts or budgets to them.

## Workflows

Workflow names are unique and targets resolve before deployment. A seat target
lowers directly to `JobTarget::Agent`. Upstream runs it in an isolated cron
session, outside `OpenHumanHost`'s coordinator runner. Agent-scoped model,
tool/approval, sandbox, iteration, timeout and budget limits must therefore
enforce its baseline independently of coordinator hooks. Hive membership
turn overlays and coordinator parking are not promised for native seat jobs.
Native seat jobs cannot borrow an active continuing session's send authority.
A timeout or model-budget setting unsupported by the native agent baseline
returns `UnsupportedSetting` before runtime creation.

A hive target uses upstream's supported System-job handler to enqueue an
authorized host message to that hive and drain coordinator work; upstream
has no `JobTarget::Hive`. This is a deliberate correction to stage #117's
Agent-only sketch. Persist the scheduled job identity and automation origin
with the accepted message and episode, then capture it in each `TurnRequest`.
The host-neutral representation is optional `scheduled_job_id` on `Message`
and `TurnRequest`; absent preserves existing interactive behavior. A
`send_scheduled_as_host` entry point accepts trusted job provenance and checks
that retry deduplication cannot change the already accepted message's origin.
Capture provenance and the episode settings atomically, before dispatch.
Resumed, queued and delegated work inherits that origin from durable state;
it must not depend on whatever task-local origin happened to surround enqueue
or drain. Scope every resulting turn explicitly as
`TrustedAutomation { Cron }` before dispatch, including child/delegated turns.
Untrusted messages cannot mint or clear that origin.

Scheduled turns cannot use write or network tools (openhuman#7264). Native
seat jobs and hive-origin turns retain that restriction for builtin, MCP and
host tools; an otherwise permissive approval grant cannot override it. Never
relabel a scheduled turn as interactive to gain permissions. Jobs retain
schedule, enabled state, retry and single-flight settings. Registering jobs
and starting scheduler services are separate explicit runtime operations.
Construct with cron inactive, configure all seats/hives/jobs, then start
services only when requested. An offline test may register/run a job without
sleeping for a wall-clock deadline.

## Errors and validation order

Parsing/validation returns the first typed `ConfigError`; it never starts a
runtime. `ValidatedHiveConfig` is constructible only through validation.
Error messages are lowercase, without trailing punctuation or resolved secret
values. Every listed branch needs a deterministic test.

| Error family | Trigger |
| --- | --- |
| `Json` / frontmatter parse | Malformed canonical JSON or YAML metadata |
| `Io` | Missing/unreadable config, prompt or context file |
| `DuplicateId` | Duplicate profile, permission, seat, hive or workflow id; repeated hive member |
| `UnknownProfile` | Seat/subagent/dynamic template references an absent profile |
| `UnknownPermissionProfile` | Profile, seat or membership references an absent permission profile |
| `UnknownSeat` | Hive member or workflow references an absent seat |
| `UnknownHive` | Workflow references an absent hive |
| `UnknownContext` | Unresolved named Markdown context |
| `InvalidRole` / future `UnknownRole` | Empty role label, or unresolved explicit role reference |
| `PermissionWidening` | Any overlay adds authority above its parent |
| `ConflictingMcp` | Same seat requires different connections across hives |
| `InvalidWidth` | Zero width or a blind, revealed, routing, division or coordinator width above the runtime cap |
| `InlineSecret` | Credential-bearing field contains a literal instead of `SecretRef` |
| `EmptyHive` | No members at deployment |
| `InvalidId` / `InvalidLimit` | Invalid agent identity, zero timeout/iteration cap or impossible budget |
| `InvalidWorkflow` | Invalid schedule or unsupported target dispatch |
| `UnsupportedSetting` | Declared selector cannot be lowered by the selected deployment ports |

Resolve ids and references before comparing effective policies. Validate
permissions and immutable MCP requirements before constructing any agent.
Deployment errors separately cover secret resolution, provider/runtime build,
agent/template creation, coordinator operations and cron registration. Errors
identify the failing section/id without rendering credential-bearing values.

## Acceptance

- One runtime, one agent/session per seat, with different captured role/context
  overlays for that seat's turns in two hives.
- All declared runtime, profile, seat and hive policies reach their matching
  runtime or coordinator APIs; no field is silently ignored.
- Permission narrowing permits a tool in hive A and denies it in hive B;
  attempted widening fails validation. Unknown effects fail closed.
- Coordinator approval tests cover allow, deny, ask/parking and explicit
  release; timeout tests prove cancellation is awaited. Native seat cron
  tests enforce agent-scoped baseline limits in isolated sessions.
- Scheduled hive provenance survives queueing/recovery and delegation; write
  and network effects from builtin/MCP/host tools are denied before execution
  in both the first scheduled turn and its delegated turns.
- Serde representation and defaults are pinned for every payload, with a full
  fixture and every typed error covered. Files meet the coverage and size caps.
- `basic_hive`, `deepswe_hive` and `pe1006_hive` use checked-in config
  directories while keeping their CLI flags and runtime-only host settings.
  Equivalence tests compare their former construction with the loaded config.
- `multi_hive` runs offline with wiremock; one runtime, three profiles, four
  seats, two hives, differing roles/restrictions and both episodes complete.
- Root contract checks, purity and rustdoc pass; standalone examples build
  and offline basic/multi-hive runs pass. Live runs are optional.

## Pin maintenance

This PR refreshes the pin on demand. #113's proposed scheduled GitHub bump
writer is deferred until its schedule, ownership and acceptance are specified;
no unattended repository mutation is installed by the configuration layer.
Existing pin-consistency assertions remain required.

Open questions: none. The memory choice and additive permission semantics are
settled above; future config-language work consumes this contract.
