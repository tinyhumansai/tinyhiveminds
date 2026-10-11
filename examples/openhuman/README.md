# Supplied OpenHuman agents and dynamic hives

`HiveDeployment` builds one OpenHuman `Runtime` from a validated manifest,
registers reusable profiles and persistent seats, and joins them to configured
hives. Runtime-only model routes, Docker paths and storage ports remain host
inputs. `OpenHumanHost` also accepts independently constructed agents. Each
coordinator seat continues one conversation across all its joined hives.

For a short introduction, run the offline two-agent example:

```sh
cargo run --manifest-path examples/openhuman/Cargo.toml --bin basic_hive
```

The [checked-in manifests](hives/README.md) provide the profiles, permission
baselines, memberships, policies and Markdown context for `basic_hive`,
`deepswe_hive`, `pe1006_hive` and `multi_hive`. The three existing binaries load
these files before adding CLI-selected endpoints and run paths.

```sh
cargo run --manifest-path examples/openhuman/Cargo.toml --bin multi_hive
cargo test --manifest-path examples/openhuman/Cargo.toml --test manifests
```

`multi_hive` uses a local model by default: three profiles create four seats on
one runtime; two hives share the same seat with implementer and observer roles.
The same native `file_write` succeeds in hive A and is refused in hive B. The
native denial aborts B's turn and remains an audited failure. The host explicitly
releases the seat with a safe note and submits a fresh assignment, which completes
without replaying the write. Both hives finish successfully, and the shared seat
keeps its original native session. `--live` resolves `OPENROUTER_API_KEY` through
the manifest reference; `OPENROUTER_MODEL` optionally selects the model.

The pure completion experiments retain their host-owned `CompletionDriver` and
outbox tools. Deployment supplies their native seats, MCP connections and
configured policy views; per-turn `cwd` keeps the Docker or durable Euler acting
root. PE's configured CortexDB engine is supplied through the native engine port.
Native harness initialization starts explicitly after assembly. The current
OpenHuman `memory_queue` service flag has no queue implementation; the shared
Markdown memory tools remain active independently.

For Docker-isolated offline and live OpenRouter runs, use the
[basic hive run script](basic-hive/README.md).

It creates Alice and Bob on one host runtime, starts ordinary conversations,
registers those same agents in a hive, sends each a private task, removes Bob
from the hive, and continues his original host conversation. A local scripted
model uses the actual attached `hivemind_complete` tool; request captures verify
that Bob's host and hive turns stayed in one session. This offline command
needs no credentials or live provider.

Run the deterministic example (Rust and `python3` are required):

```sh
cargo run --manifest-path examples/openhuman/Cargo.toml --bin tinyhivemind-openhuman-example
cargo test --manifest-path examples/openhuman/Cargo.toml
```

The loopback provider captures actual OpenAI-compatible requests for four
configurations: one agent in one hive, three in one hive, three in two hives,
and one in three hives. An ordinary host conversation starts before registration.
The adapter binds that same session, adds its tools, and delivers hive messages
through the coordinator. The model calls native completion tools to finish each
assignment. The captured requests verify:

- each agent sees its original prompt and its own skill, memory, and connected
  MCP catalogue, while other agents' private configuration stays absent;
- the nine permanent Hivemind tools appear exactly once in both system prompt
  and native schemas on every later request, without tool discovery;
- earlier host input and assistant replies survive registration and later hive
  turns in the same session;
- ordinary host turns can discover hives and send work through the same tools.

A final scenario enables management with an explicit authorizer and a host
factory. Native model calls create a hive, create a configured agent on the same
runtime, and join it to the hive. A denied template never reaches the factory;
unknown templates and invalid agent IDs leave no registered agent.
Agent IDs are validated before deriving filesystem paths. Both failures are returned to the
model as tool receipts. The dynamically supplied agent then completes a hive
assignment using its own configuration.

## Host configuration and catalogues

This host supplies a separate `workspace_dir` for each agent through
`AgentSpec::config`. Native skill bundles live under that private workspace, so
OpenHuman's skill readers see only that agent's resources. (OpenHuman no longer
renders a workspace `MEMORY.md` into the prompt; agent memory is a recalled pack
from the configured memory engine, bound per agent with `AgentSpec::memory`.) Skills are installed at `<workspace_dir>/skills/<name>/SKILL.md`.
`AgentSpec::skills_dir` currently installs under the agent's home directory;
this example uses the explicit workspace discovery root instead.

OpenHuman treats a custom inline system prompt as host-authored text and does
not automatically add its orchestrator's installed-skill and MCP catalogue.
Before constructing each agent, this host builds its prompt from the installed
skill file and the configured MCP metadata. The proof then calls `use_skill`
to describe the actual installed skill, and invokes `mcp_list_tools` and
`mcp_call_tool` against the agent's private server. Captured native tool results
must contain the agent's own markers and exclude peers' markers. These
capabilities run both before registration and afterward on every continuing
agent session; post-registration assertions match newly issued phase-tagged
call IDs to their native tool results, so carried earlier receipts cannot pass. Prompt text
alone is insufficient to pass these assertions. Workflow tools stay packed;
Hivemind registration does not advertise `describe_workflow` directly.

## Host integration

```rust,ignore
let coordinator = Coordinator::new(
    existing_agent.runtime_id().into(),
    Arc::new(MemoryStorage::new()),
    CoordinatorOptions::default(),
)
.await?;
let host = OpenHumanHost::new(existing_agent.runtime_id().into(), coordinator)?;
host.register_agent_in_session(existing_agent.clone(), "existing-conversation")
    .await?;
host.coordinator().create_hive(hive).await?;
host.coordinator().join_hive("engineering", existing_agent.id()).await?;
host.coordinator().run_until_idle().await?;
```

Keep the `OpenHumanHost` alive while the agent's attached tools are used.
Configure optional management before registering any agents. Its factory owns
runtime and credential configuration; model arguments contain only template
references and ordinary settings. Applications can call coordinator creation,
registration, and membership APIs directly without exposing management tools.

## Files

| File | Purpose |
| --- | --- |
| `src/main.rs` | Runs the offline acceptance example. |
| `src/bin/basic_hive.rs` | Small offline or live host integration with two OpenHuman agents joining and leaving a hive. |
| `src/proof/fixture.rs` | Host construction, private MCP fixtures, loopback provider captures. |
| `src/proof/topology.rs` | Four hive shapes, session continuity, permanent tool assertions. |
| `src/proof/dynamic.rs` | Authorized host factory and native dynamic management calls. |
| `src/bin/deepswe_hive.rs` | Docker-confined four-agent software-engineering experiment. |
| `src/bin/pe1006_hive.rs` | Five-agent research experiment with live TypeSafe routing. |

The research binaries retain their explicit host loops and use `RegisteredAgent`
to bind host-created handles to the pure driver. The obsolete runner comparison
binary was removed; these examples do not construct agents inside the adapter.

## Hermetic DeepSWE adapter

Build the sandbox image once (the build may download operating-system packages;
the adapter itself never pulls images or installs anything):

```sh
docker build -t tinyhivemind-deepswe:local examples/openhuman/deepswe-sandbox
```

Prepare a clean, disposable local Git checkout and a task JSON containing
`instance_id`, absolute `repo_path`, `base_commit`, `problem_statement`, and
`test_command`. Then run:

```sh
OPENROUTER_API_KEY=... cargo run --release \
  --manifest-path examples/openhuman/Cargo.toml \
  --bin deepswe_hive -- \
  --task /absolute/path/task.json \
  --api-base https://openrouter.ai/api/v1 \
  --output /absolute/path/result.json
```

`--model` defaults exactly to `openai/gpt-oss-120b:nitro`. The host process
owns OpenHuman, the provider request, sessions, transcript, and outboxes. The
four initially open seats (`lead`, `implementer`, `tester`, `reviewer`) execute
bounded same-snapshot rounds through `CompletionDriver`; broadcasts use its
deterministic per-author fallback and require no TypeSafe key.

The episode admits at most 24 committed seat turns. Each authorized seat gets
at most three provider attempts and each attempt has a 600-second deadline.
After every outcome the host reconciles the native MCP outbox before deciding
whether retry is safe. A proven zero-action protocol miss or a structured
retryable provider failure may retry in the same OpenHuman session against the
same frozen round view. A timeout is ambiguous and fails closed; one accepted
action is committed even if the provider continuation then fails; multiple
actions, authentication/configuration failures, and tool or sandbox failures
fail immediately. Printed JSON or prose never counts as an action, and a round
is committed only after every seat has produced exactly one native action.

The adapter refuses a non-absolute checkout, a path other than the canonical
Git root, a non-commit base, a HEAD different from that resolved base, or any
tracked, untracked, or ignored starting entry (including an ignored `.env`). It
also rejects every Git index entry with mode `160000`: submodules/gitlinks are
unsupported whether populated, configured, ignored, or absent on disk. It
resolves both Git's absolute directory and common directory before Docker or
the provider starts. Metadata inside the checkout is accepted only for the
standard `<repo>/.git` directory; alternate in-tree metadata such as
`git init --separate-git-dir .realgit` is rejected. External metadata for a
real linked worktree remains supported. `--output` must be absolute, and the
result, transcript, outboxes, and runtime workspace are all resolved outside
the canonical checkout before sandbox or provider setup. Every destination is
checked with `symlink_metadata`; all four targets must be absent, and existing
files, empty or nonempty directories, and broken symlinks are rejected. The
output parent may already contain the task JSON and unrelated caller files.
The runtime and outbox directories are then claimed with atomic `create_dir`
calls before the provider key is read or Docker starts, so fixed session IDs
cannot resume stale state and no preexisting outbox child can be reused.

Every agent file read/write/edit and shell/test call uses a fresh container
with `--network none`, 1 GiB memory, 2 CPUs, 256 PIDs, dropped capabilities,
no-new-privileges, the host process's numeric UID/GID, an `env -i` process
environment, and the checkout at `/workspace`. A read-only empty mount covers
`/workspace/.git`, including when the checkout's `.git` is a worktree pointer,
so agent commands cannot reach or mutate the source history. The file tools
also reject targets whose resolved path leaves `/workspace`, including through
a symlink. Model-supplied file content is limited to exactly
1 MiB (1,048,576 bytes), staged before Docker starts in a host-owned temporary
file outside the checkout, and mounted read-only at `/tmp/deepswe-input`; the
fixed container wrapper consumes that path, so Docker receives no agent-chosen
FIFO or unbounded stdin stream. The temporary file is removed after the action.
Provider credentials remain host-side.

The final patch is produced by a separate no-network inspector container with
the same resource and privilege caps. Only discovered external Git metadata is
mounted, read-only, for linked worktrees. It runs `git diff --binary
--no-ext-diff --no-textconv` and appends deterministic binary no-index
additions for every untracked, nonignored file into a fixed temporary file. A
host-side 600-second deadline kills a hung Docker CLI, and a 32 MiB patch cap is
enforced before stdout is emitted or buffered; either condition fails the run.
Docker version/create/inspect/start preflight calls have a separate 10-second
host deadline and 64 KiB stdout/stderr caps. Before create, the runner assigns
both a unique container name and cidfile, so even a create CLI that hangs after
the daemon creates the container can be cleaned up. Every cidfile-identified
action/inspector container and every uniquely named preflight container is
force-removed under its own two-second deadline. The runner then
performs a second bounded inspect to prove the container is absent; removal
failure or timeout fails the run as `CleanupFailed` or `CleanupTimeout`.
The runner never clones, fetches, pulls, resets, cleans, or commits. Agent code
can still modify or delete files in the caller-supplied disposable workspace:
confinement protects paths outside that mount and the original Git history,
not the disposable workspace contents. The result is `passed` only when the
episode completes, that bounded patch is nonempty, and the final no-network
container test exits zero; an already-passing checkout with no edit fails.

Run the real Docker fixture regression after building the image:

```sh
DEEPSWE_REAL_DOCKER_TEST=1 cargo test \
  --manifest-path examples/openhuman/Cargo.toml \
  --bin deepswe_hive real_docker
```

### Recorded local acceptance

One hermetic local fixture was run end to end and passed (`1/1`) with model id
`openai/gpt-oss-120b:nitro`. The episode committed 11 seat turns, changed the
fixture answer from `wrong` to `right`, produced a nonempty patch, and finished
with test exit code `0`. Agent action containers had internet access blocked,
and the post-run container check found no residual action, inspector, or
preflight containers.

This is acceptance evidence for the adapter and its local fixture only. It is
not an official DeepSWE score, benchmark result, or claim about corpus-wide
quality. A real score requires a supplied local DeepSWE corpus and its scorer;
this runner does not download either one.

Run the live hive experiment through OpenRouter:

```sh
cargo run --release --manifest-path examples/openhuman/Cargo.toml --bin pe1006_hive
```

The run requires `OPENROUTER_API_KEY`, authenticated `gh` access for one
research source, and a machine OpenHuman configuration whose memory engine is
`cortexdb` (`[memory] engine = "cortexdb"`). It uses model id `openai/gpt-oss-120b:nitro` unconditionally and
writes into one durable shared workspace. By default that workspace is
`examples/openhuman/workspace/pe1006`; set `OPENHUMAN_HIVE_WORKSPACE` to use a
different directory.

The runner creates these files without overwriting existing agent edits:

```text
AGENTS.md                 shared working agreement and role boundaries
MEMORY.md                 durable, evidence-linked agent learnings
TASK.md                   official task statement
research_sources/         mirrored public research inputs
runs/run-<pid>/            one attributed transcript and OpenHuman runtime
  turns/README.md          index of every agent turn and stable session id
  turns/NNN-agent/         exact prompt, reply, and JSON metadata snapshot
```

All five agents use the workspace root as their `action_dir`, can read and
write shared files, and are instructed to update `MEMORY.md` only with
reproduced findings. Per-run OpenHuman/TinyCortex state remains isolated under
that run's directory, while the explicit workspace memory survives. The turn
snapshots record application-level prompts and final replies; OpenHuman's raw
session data and tool events remain under the same run's `openhuman-runtime/`
tree.

The live runner exposes `broadcast` and `complete_episode` through a local MCP
server. A broadcast receives a fresh TypeSafe Choice over eligible teammates;
the Choice maximum and any option strictly above 20% are assigned. Agents stay
pending after broadcasts and finish only through explicit completion calls.
`CompletionDriver` supplies the pending concrete OpenHuman agents and advances
only after the example has committed each tool utterance to its transcript.

This remains an experiment: GPT-OSS produced several false checker sign-offs
whose claimed files did not exist or whose algorithms failed executable
checks. A later clean run staged a newly published public implementation,
required the checker to execute its built-in brute-force checkpoints, and
independently matched the sealed oracle. The answer and derivation remain
outside the repository; the run artifacts stay under the ignored workspace.

The multi-hive host injects a native, classified `file_write` tool scoped to an
isolated `proof.txt`. The deployment replaces the native host tool belt, so this
example explicitly supplies its filesystem tool through `BuildOptions.tools`.
