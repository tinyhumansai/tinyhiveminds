# Implement configurable OpenHuman hives

Status: Accepted implementation sequence for
[#113](https://github.com/tinyhumansai/tinyhivemind/issues/113), delivered in
one ready-for-review PR. Contract:
[openhuman-hive-config](../specs/openhuman-hive-config.md).

The host defines one runtime, registers reusable templates, creates one
persistent agent per seat, then joins seats to any number of hives. Memory
bindings stay fixed. Hive permissions intersect seat authority. No new crate,
second transcript journal, `ProfileRuntime` or runtime-global turn overlay.

## 1. Refresh and prove the OpenHuman boundary (#114)

1. In `vendor/openhuman`, inspect the latest canonical main APIs and advance
   the gitlink to a reviewed commit that includes `RuntimeBuilder` defaults,
   templates, agent memory, permission hooks, `Turn::tools` and awaited timeout.
   Keep the vendor as a submodule; do not patch its source to hide mismatches.
2. Update only API-incompatible adapter/example call sites. Inspect
   `crates/tinyhivemind-openhuman/src/host/mod.rs`, `host/runner.rs` and
   `examples/openhuman/src/` before changing assumptions about host belts.
3. Test current supplied-agent registration, continuing sessions and native
   tool binding against the new pin. Pin the fact that `Turn::tools` replaces
   host tools while `can_use_tool` callbacks compose through denial.
4. Run focused adapter tests; stop on upstream incompatibility that prevents
   the existing contract. Do not introduce broad lints or ignored tests.

## 2. Record the accepted behavior (#115)

Create `docs/specs/openhuman-hive-config.md` before code. Link this plan and
both README indexes. Settle per-seat memory (`seat:<id>` unless explicitly
bound), immutable MCP connections, named credential references, policy
narrowing and actual workflow lowering. The spec remains authoritative if an
issue's API sketch differs from the current upstream API.

## 3. Add policy serde without changing defaults (#116)

Files:

- `crates/tinyhivemind-core/src/driver/conduct/mod.rs` and its test module.
- `crates/tinyhivemind-core/src/hive/division/types.rs` and its test module.
- `crates/tinyhivemind-hives/src/coordinator/types.rs` and matching tests.
- The `RetentionPolicy` definition/test module, needed by `CoordinatorOptions`.

Write wire-form and empty-object/default tests first. Add `Serialize` and
`Deserialize` with `#[serde(default)]` where omission inherits current defaults.
Keep existing serialized enum spelling consistent with adjacent core types.
Verify `cargo test -p tinyhivemind-core`, `cargo test -p tinyhivemind-hives` and
`.github/scripts/assert-pure.sh`. No new runtime/transport dependency in core.

## 4. Parse and validate the manifest (#116)

Create the adapter's `src/config/`:

| File | Ownership |
| --- | --- |
| `mod.rs` | Module docs, small deliberate exports and `#[cfg(test)] mod test;` |
| `types.rs` | `HiveConfig`, `RuntimeSection`, `Profile`, `PermissionProfile`, `Seat`, `HiveSpec`, `Membership`, `Workflow`, `SecretRef`, validated wrapper |
| `parse.rs` | JSON/value parsing, directory loading, Markdown/frontmatter parsing |
| `validate.rs` | Referential checks and effective-policy/MCP/limit validation |
| `README.md` | Wire contract, precedence and file map |
| `test.rs` or `test/` | Behavioral tests and shared fixtures |

Add `ConfigError` to `src/error.rs` and deliberate exports in `src/lib.rs`.
Use the existing core/hives policy types in `HiveSpec`, not copied structs.
Use typed override fields rather than an arbitrary secret-bearing JSON blob.

Public boundary:

```rust
HiveConfig::from_json(&str) -> Result<HiveConfig, ConfigError>
HiveConfig::from_value(serde_json::Value) -> Result<HiveConfig, ConfigError>
HiveConfig::load_dir(path) -> Result<HiveConfig, ConfigError>
HiveConfig::validate(self) -> Result<ValidatedHiveConfig, ConfigError>
```

The validated wrapper's inner value is private; provide immutable access or
consuming access to validated data. Parsing must not resolve a credential or
construct OpenHuman objects. Injected ports belong to deployment build options,
not the serde data type.

Test-first sequence:

1. Full JSON fixture round trip; pin every type's wire form and defaults.
2. Unknown references and duplicates in every namespace and membership.
3. Zero/invalid limits, empty role labels and empty hives.
4. Every widening dimension: access, sandbox, tool allow/deny, send,
   approval/grants. Test a valid restriction independently.
5. Same MCP set in different orders succeeds; different endpoints,
   transports, credential refs or tool selections fail for one shared seat.
6. Reject inline secrets in provider, MCP and dynamic override fields.
7. Load Markdown profiles/context relative to config root, strip valid
   frontmatter, preserve body/order, reject malformed metadata and duplicate
   definitions. Missing files return a typed IO error.
8. Workflow target/schedule checks and unsupported runtime selector errors.

Then implement the smallest parser and validator that passes those tests.
Split files along behavioral seams before reaching 700 Rust/500 Markdown
lines. Update crate and src README indexes.

## 5. Give the coordinator explicit hive settings (#117)

Inspect `crates/tinyhivemind-hives/src/coordinator/conduct.rs`, coordinator state
and routing before choosing the smallest host-neutral API. Add a narrow
per-hive policy/role configuration boundary that retains existing defaults for
unconfigured hosts. Use host-neutral `HiveSettings` with
`options: CoordinatorOptions`, `routing: RoutingPolicy` and
`roles: BTreeMap<String, String>`. Persist settings and freeze them atomically
in `EpisodeRecord` when opening work. Later settings changes affect new
episodes only. No OpenHuman types may enter hives or core.

Write regressions first proving two hives actually use different configured
conduct/routing/coordinator bounds, and proving a member's role reaches
`RouteCandidate`/`BriefedTeammate`. Configure policy before opening any episode.
Intersect `HiveSpec.coordinator.conduct_policy` and the separate hive
`conduct` field using the minimum `turn_wall` and minimum `child_turn_wall`;
test both orderings so neither configured bound is ignored. Omitted settings
keep existing core defaults. Preserve the global concurrency bound.
Joining/leaving/recovering must not
throw away retained role/policy metadata. Expose EpisodePolicy/DivisionPolicy
through an explicit per-hive deployment policy view for pure deliberation
hosts; reject unsupported application to the completion-only path. Do not
merely render policies in a
prompt while the coordinator uses old global defaults.

## 6. Lower the validated config (#117)

Create `src/deploy/mod.rs`, `types.rs`, runtime/profile/seat lowering modules,
`permission.rs`, `workflow.rs`, tests and `README.md`. The deployment owns the
single `Runtime`, `OpenHumanHost` and seat-to-`Agent` map. Its public boundary
is `HiveDeployment::build(validated, secrets)` plus a build-options variant
for host-owned memory/session/approval/sandbox ports. Build may be async to
match upstream runtime construction; document the exact implemented signature.

Test with a local wiremock endpoint and isolated runtime lifetime:

1. Runtime defaults lower to the matching `RuntimeBuilder` methods; a declared
   unsupported value errors rather than being ignored. Explicit built-in
   storage/engine choices and injected ports both have observable tests.
2. Register each profile through `define_template`; retain additional
   profile-only settings for lowering onto `AgentSpec`.
3. Create seats through `AgentSpec::new(id).extends(profile)` and
   `host.register_spec`, applying model, access, MCP, skills, subagents,
   tool scope and limits. Assert every seat shares one runtime identity.
4. Preserve explicit memory binding; otherwise assign `seat:<id>`. Assert a
   seat in two hives has one registration and one continuing session.
5. Authorize deployment's create/join operations, create empty shells only
   internally if needed, add members and configure role/policy before turns.
6. Resolve `SecretRef` only here through a host resolver. Test missing refs
   and prove resolved values appear in neither Debug nor errors.

`ManifestFactory` stores the runtime and validated profile catalog and
implements existing `AgentFactory::create`. Dynamic `CreateAgent` requests
must parse typed nonsecret overrides, validate their permissions and use the
same seat-lowering path; test known and unknown templates, inline-secret
rejection and narrowing. Ensure dynamically created handles receive the same
memory/turn policy treatment when registered by the host.

## 7. Enforce permissions and captured turn overlays (#117)

Modify `src/host/runner.rs` and the smallest host settings boundary needed to
capture each hive's role/context/tool restrictions. Keep custom hooks
composable; never install a process-global current-hive variable.

Regression-first tasks:

1. Run the same agent in hive A and B with different roles and context,
   capture provider requests, and verify each turn uses the correct overlay.
2. Set `Turn::tools` to the episode-bound host belt, and apply effective
   `can_use_tool` to builtin/MCP/host tools. Prove A allows and B denies the
   same classified operation. Agent-level denials must survive turn allows.
3. Map authenticated call context to core `ApprovalRequest` and `Action`;
   use explicit classification and deny `Unclassified`.
4. Invoke `approve` with supplied actor/person roster, monotonic sequence,
   consent epoch, grants and refusals. Test Allow, Deny and Ask independently.
5. On hook Ask, record an adapter-owned pending request with call correlation
   and explicitly invoke the injectable `ApprovalHandler`; native approval
   subscriptions alone cannot observe a hook-created request. Deny the
   original tool call, return Parked in `after_turn`, then accept the
   asynchronous correlated handler decision and explicitly release with a
   resumption note. Test handler invocation, no mutation before approval,
   correlation, release, and no automatic replay of the refused call or turn.
6. Enforce all send operations through the seat plus active-hive
   `SendAuthorizer`; a direct message uses the seat baseline.
7. Replace outer drop-only timeout with `Turn::timeout` or an awaited
   cancellation handle. Test an interrupted slow tool releases its scope
   before the runner returns; retain session/uncertain-effect rules.
8. Lower per-seat iterations and model budget, testing explicit settings and
   fallback to the existing host timeout.

## 8. Register workflows (#117)

Seat workflows use `JobSpec` / `JobTarget::Agent`. Upstream creates isolated
cron sessions and runs outside the coordinator runner: install baseline
permission/sandbox/iteration/timeout/budget policy at agent scope. Do not
promise continuing coordinator sessions, membership overlays or coordinator
parking for these jobs. Test isolation and baseline enforcement directly.

Current upstream has no `JobTarget::Hive`; use its supported System-job handler
to enqueue an authorized hive message and drain coordinator work. Add a
host-neutral scheduled-origin/job-id value at the coordinator message/episode
boundary: optional `scheduled_job_id` on `Message` and `TurnRequest`, plus
`send_scheduled_as_host` for trusted provenance. Persist it with that work,
atomically with captured episode settings. Test that retry deduplication
rejects changing an accepted message's origin. Carry
that value into child/delegated assignments and queued/recovered work. In the
OpenHuman runner, explicitly scope each such turn as
`TrustedAutomation { Cron }`; do not rely on task-local origin at enqueue or
on the System-job handler's surrounding scope.

Write regressions first for mutating builtin, MCP and host tools in the first
scheduled hive turn and a delegated turn. Assert refusal happens before the
effect executes even when normal tool/approval rules permit it. Add a queued
or recovered-work test proving automation provenance survives after the
handler returns. Untrusted message content must not remove that restriction.
Register and trigger jobs directly in tests without waiting for real time.

Build with cron services inactive while assembling templates, agents and
hives. Register all jobs after validation and start services only when the
configured lifecycle explicitly requests it. Verify schedule, enabled state,
retries and single-flight are actually retained. Errors must identify job ids
without including credentials or prompts.

## 9. Migrate examples and prove equivalence (#118)

Create checked-in config directories under `examples/openhuman/hives/` named
`basic_hive`, `deepswe_hive`, `pe1006_hive` and `multi_hive`, each with
`hive.json`, `profiles/*.md` and `context/*.md` as needed.

- `basic_hive`: alice/bob, read-only access, ephemeral workspace.
- `deepswe_hive`: lead/implementer/tester/reviewer; existing MCP endpoints and
  allowlists, Named scope, 16 iterations, temperature zero, width four.
- `pe1006_hive`: theory/solver/checker/lead/researcher; existing disallow rules,
  memory, AGENTS/MEMORY/TASK templates moved to context files.
- `multi_hive`: three profiles, four seats, two hives; a shared seat has
  different roles and narrowed authority in the second hive.

Write lowering-equivalence tests for each migrated binary before replacing
its manual seat setup. Keep CLI flags and genuine runtime-only Docker wiring
through build options. Offline basic and multi-hive runs use wiremock; both
multi-hive episodes must settle, and the same tool must succeed in A and be
refused in B. `--live` reads `OPENROUTER_API_KEY` through a reference and is
optional verification, never a deterministic-test dependency.

Update `examples/openhuman/README.md`, `ROADMAP.md`, crate/module READMEs and
wiki `Host-Integration`. The wiki is its own repo: push its documentation
branch before committing the pointer bump, and keep code changes in the one
canonical upstream PR. Link spec/plan from #113's PR, which closes the parent
and stages when their acceptance is satisfied.

## 10. Verify and publish one reviewable change

Run the four commands from the repository root and read their full results:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
.github/scripts/assert-pure.sh
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

Also build the standalone OpenHuman workspace and run its offline basic and
multi-hive binaries plus lowering-equivalence tests. Check per-file coverage
at the repository's 90% threshold where supported; record any deliberate
untested external/lifecycle edge with manual verification rather than claim
unmeasured coverage. Check source/Markdown line caps and local links.

Do not squash automatic checkpoints. Push this feature branch and open one
ready PR against `tinyhumansai/tinyhivemind` with issue links, public behavior,
actual verification and any material limitation. Do not deploy a scheduled
GitHub writer as part of this finite config implementation.
