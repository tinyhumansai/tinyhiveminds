//! Hermetic DeepSWE-style software-engineering hive backed by OpenHuman.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::future::join_all;
use openhuman_embed::{Agent, CoreError, Provider, RuntimeConfig};
use serde::{Deserialize, Serialize};
use tinyhivemind_core::driver::{
    AgentBinding, BoundHive, BroadcastRouting, CommittedUtterance, CompletionDriver, HiveGraph,
};
use tinyhivemind_core::hive::{CompletionEpisodeState, CompletionStep, completion_status};
use tinyhivemind_core::runtime::desk::{Desk, ResponderMode};
use tinyhivemind_openhuman::RegisteredAgent;
use tinyhivemind_openhuman::deploy::BuildOptions;
use tinyhivemind_openhuman_example::{deploy, manifest};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[path = "deepswe_hive/artifacts.rs"]
mod artifacts;
#[path = "deepswe_hive/mcp.rs"]
mod mcp;
#[path = "deepswe_hive/sandbox.rs"]
mod sandbox;
#[path = "deepswe_hive/task.rs"]
mod task;

#[cfg(test)]
use artifacts::validate_output_paths;
use artifacts::{ArtifactPaths, prepare_artifacts};
use sandbox::{DockerSandbox, SandboxConfig};
use task::Task;

const DEFAULT_MODEL: &str = "openai/gpt-oss-120b:nitro";
const MAX_TURNS: u32 = 24;
const MAX_SEAT_ATTEMPTS: u32 = 3;
const TURN_TIMEOUT: Duration = Duration::from_secs(600);
const SEATS: [&str; 4] = ["lead", "implementer", "tester", "reviewer"];

#[derive(Debug)]
struct Cli {
    task: PathBuf,
    api_base: String,
    model: String,
    output: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResultDocument {
    instance_id: String,
    status: String,
    model: String,
    turns: u32,
    patch: String,
    test_exit_code: Option<i32>,
    transcript_path: String,
}

#[derive(Clone, Debug)]
struct DeskMessage {
    author: String,
    body: String,
}

#[derive(Default)]
struct Visibility(BTreeMap<String, BTreeSet<usize>>);

fn main() -> anyhow::Result<()> {
    if let Some(server) = mcp::requested()? {
        return mcp::serve(&server);
    }
    let cli = parse_cli(std::env::args().skip(1))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()?;
    runtime.block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let task = Task::load(&cli.task)?;
    let artifacts = prepare_artifacts(&task, &cli.output)?;
    let sandbox = DockerSandbox::preflight(SandboxConfig::from_env(task.repo_path.clone()))?;
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| anyhow::anyhow!("OPENROUTER_API_KEY is required"))?;
    run_with_prepared(cli, task, sandbox, api_key, artifacts).await
}

async fn run_with_prepared(
    cli: Cli,
    task: Task,
    sandbox: DockerSandbox,
    api_key: String,
    artifacts: ArtifactPaths,
) -> anyhow::Result<()> {
    let executable = std::env::current_exe()?;
    run_prepared_with_mcp_executable(
        cli,
        task,
        sandbox,
        api_key,
        &executable,
        TURN_TIMEOUT,
        artifacts,
    )
    .await
}

#[cfg(test)]
async fn run_with_mcp_executable(
    cli: Cli,
    task: Task,
    sandbox: DockerSandbox,
    api_key: String,
    mcp_executable: &Path,
) -> anyhow::Result<()> {
    run_with_mcp_executable_and_timeout(cli, task, sandbox, api_key, mcp_executable, TURN_TIMEOUT)
        .await
}

#[cfg(test)]
async fn run_with_mcp_executable_and_timeout(
    cli: Cli,
    task: Task,
    sandbox: DockerSandbox,
    api_key: String,
    mcp_executable: &Path,
    turn_timeout: Duration,
) -> anyhow::Result<()> {
    let artifacts = prepare_artifacts(&task, &cli.output)?;
    run_prepared_with_mcp_executable(
        cli,
        task,
        sandbox,
        api_key,
        mcp_executable,
        turn_timeout,
        artifacts,
    )
    .await
}

async fn run_prepared_with_mcp_executable(
    cli: Cli,
    task: Task,
    sandbox: DockerSandbox,
    api_key: String,
    mcp_executable: &Path,
    turn_timeout: Duration,
    artifacts: ArtifactPaths,
) -> anyhow::Result<()> {
    let transcript_path = artifacts.transcript;
    let outbox = artifacts.outbox;

    let backend = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "deepswe-hive", "email": "local@openhuman.local"}
        })))
        .mount(&backend)
        .await;
    let mut config = manifest("deepswe_hive")?;
    config.runtime.workspace = Some(artifacts.runtime.display().to_string());
    config.runtime.mcp[0].endpoint = mcp_executable.display().to_string();
    config.runtime.mcp[0].args = sandbox.mcp_args();
    for profile in &mut config.profiles {
        let mcp = &mut profile.mcp[0];
        mcp.endpoint = mcp_executable.display().to_string();
        mcp.args = vec![
            "--mcp-hive".into(),
            "--agent".into(),
            profile.id.clone(),
            "--outbox".into(),
            outbox
                .join(format!("{}.jsonl", profile.id))
                .display()
                .to_string(),
        ];
    }
    let routing_policy = config.hives[0].routing.clone();
    let deployment = deploy(
        config,
        BuildOptions {
            config: Some(offline_config()),
            backend_url: Some(backend.uri()),
            provider: Some(Provider::openai_compatible(&cli.api_base, api_key).model(&cli.model)),
            ..Default::default()
        },
    )
    .await?;
    let bindings = SEATS
        .iter()
        .map(|id| AgentBinding::new(*id, RegisteredAgent(deployment.seats()[*id].clone())))
        .collect();
    let hive = BoundHive::new(
        HiveGraph::new(
            Desk {
                id: format!("deepswe-{}", task.instance_id),
                name: format!("DeepSWE {}", task.instance_id),
                description: Some("hermetic software-engineering episode".into()),
                members: SEATS.iter().map(|id| (*id).into()).collect(),
                responder_mode: ResponderMode::Auto,
            },
            SEATS.iter().map(|id| task.candidate(id)).collect(),
        ),
        bindings,
    )?;
    let round_width = usize::try_from(
        deployment
            .policy("deepswe")
            .ok_or_else(|| anyhow::anyhow!("missing declared hive policy"))?
            .episode
            .round_width,
    )?;
    let driver = CompletionDriver::new(&hive, round_width)?;
    let episode = CompletionEpisodeState::opened(
        tinyhivemind_core::runtime::Conversation {
            desk_id: hive.desk().id.clone(),
            desk_name: hive.desk().name.clone(),
            thread_root: None,
        },
        tinyhivemind_core::runtime::Sequence(0),
        SEATS,
    )?;
    let mut state = driver.start(episode)?;
    let mut transcript = Vec::new();
    let mut visibility = Visibility::default();
    let mut sequence = 0_u64;
    let mut turns = 0_u32;

    while !matches!(
        completion_status(state.episode()),
        CompletionStep::Complete { .. }
    ) {
        if turns >= MAX_TURNS {
            anyhow::bail!("episode exceeded {MAX_TURNS} turns");
        }
        let pending = driver.pending_round(&state)?;
        if pending.is_empty() {
            anyhow::bail!("episode is incomplete but has no pending agents");
        }
        ensure_round_fits(turns, pending.agents().len())?;
        let frozen = transcript.clone();
        let jobs = pending.agents().iter().map(|seat| {
            run_seat(
                seat.agent.0.clone(),
                seat.hive_agent_id.to_owned(),
                task.clone(),
                frozen.clone(),
                visible_delta(&visibility, seat.hive_agent_id, &frozen),
                outbox.join(format!("{}.jsonl", seat.hive_agent_id)),
                turn_timeout,
            )
        });
        let outcomes = join_all(jobs).await;
        let mut committed = Vec::new();
        let delivered = transcript.len();
        for outcome in outcomes {
            let (id, reply, utterance) = outcome?;
            visibility
                .0
                .entry(id.clone())
                .or_default()
                .extend(0..delivered);
            sequence = sequence.saturating_add(1);
            transcript.push(DeskMessage {
                author: id.clone(),
                body: reply,
            });
            visibility
                .0
                .entry(id.clone())
                .or_default()
                .insert(transcript.len() - 1);
            committed.push(CommittedUtterance {
                author_id: id,
                sequence: tinyhivemind_core::runtime::Sequence(sequence),
                utterance,
            });
        }
        turns = turns.saturating_add(u32::try_from(pending.agents().len()).unwrap_or(u32::MAX));
        let routing = committed.iter().any(|event| {
            matches!(
                event.utterance,
                tinyhivemind_core::runtime::speech::Utterance::Broadcast { .. }
            )
        });
        state = driver
            .apply_committed_round(
                &state,
                &pending,
                committed,
                routing.then_some(BroadcastRouting {
                    primary: None,
                    reasoning: None,
                    policy: &routing_policy,
                    roster_version: 1,
                    thread_context: &[],
                }),
            )
            .await?
            .state;
    }

    let transcript_text = transcript
        .iter()
        .map(|row| format!("## @{}\n\n{}", row.author, row.body))
        .collect::<Vec<_>>()
        .join("\n\n");
    std::fs::write(&transcript_path, transcript_text)?;
    let patch = sandbox.patch(&task.base_commit)?;
    let test = sandbox.shell(&task.test_command)?;
    let status = if !patch.is_empty() && test.code == Some(0) {
        "passed"
    } else {
        "failed"
    };
    let result = ResultDocument {
        instance_id: task.instance_id,
        status: status.into(),
        model: cli.model,
        turns,
        patch,
        test_exit_code: test.code,
        transcript_path: transcript_path.display().to_string(),
    };
    if let Some(parent) = cli.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&cli.output, serde_json::to_vec_pretty(&result)?)?;
    if status != "passed" {
        anyhow::bail!("episode result is failed; see {}", cli.output.display());
    }
    Ok(())
}

fn ensure_round_fits(turns: u32, round_size: usize) -> anyhow::Result<()> {
    let round_size = u32::try_from(round_size)
        .map_err(|_| anyhow::anyhow!("round size exceeds the turn counter"))?;
    if round_size > MAX_TURNS.saturating_sub(turns) {
        anyhow::bail!("next round of {round_size} would exceed {MAX_TURNS} turns");
    }
    Ok(())
}

async fn run_seat(
    agent: Agent,
    id: String,
    task: Task,
    _transcript: Vec<DeskMessage>,
    delta: String,
    outbox: PathBuf,
    turn_timeout: Duration,
) -> anyhow::Result<(
    String,
    String,
    tinyhivemind_core::runtime::speech::Utterance,
)> {
    let mut retry_reason = None;
    mcp::clear(&outbox)?;
    for attempt in 1..=MAX_SEAT_ATTEMPTS {
        let retry_instruction = retry_reason.as_deref().map(|reason| {
            format!(
                "Your prior attempt was invalid because {reason}. This is retry {attempt} of {MAX_SEAT_ATTEMPTS}. Invoke exactly one native mcp_call_tool call on server tinyhive using broadcast or complete_episode, wait for its result to say `accepted from @{id}`, and do no other work or prose."
            )
        });
        let prompt = seat_prompt(&task, &id, &delta, retry_instruction.as_deref());
        let started = tokio::time::Instant::now();
        let outcome = agent
            .turn(prompt)
            .cwd(&task.repo_path)
            .timeout(turn_timeout)
            .send()
            .await;
        let send = deadline_result(outcome, started.elapsed(), turn_timeout);
        let utterances = mcp::drain(&outbox)?;
        if utterances.len() > 1 {
            anyhow::bail!(
                "@{id} emitted {} hive actions on attempt {attempt}; expected one",
                utterances.len()
            );
        }
        if let Some(utterance) = utterances.first() {
            if let Err(error) = &send {
                anyhow::bail!(
                    "@{id} turn timed out after {turn_timeout:?} on attempt {attempt}/{MAX_SEAT_ATTEMPTS} with one accepted hive action; refusing an ambiguous retry: {error}"
                );
            }
            let provider_error = match &send {
                Ok(Err(error)) => Some(error),
                Ok(Ok(_)) => None,
                Err(_) => unreachable!("timeout was handled above"),
            };
            let body = accepted_action_reply(&id, utterance, provider_error)?;
            return Ok((id, body, utterance.clone()));
        }

        match send {
            Ok(Ok(_)) => {
                if attempt == MAX_SEAT_ATTEMPTS {
                    anyhow::bail!(
                        "@{id} emitted zero hive actions after {MAX_SEAT_ATTEMPTS} attempts"
                    );
                }
                retry_reason = Some("it produced no TinyHiveMind action".to_owned());
            }
            Ok(Err(error)) if retryable_provider_error(&error) => {
                if attempt == MAX_SEAT_ATTEMPTS {
                    return Err(provider_attempt_error(&id, attempt, &error));
                }
                retry_reason = Some("the inference provider failed transiently".to_owned());
            }
            Ok(Err(error)) => return Err(provider_attempt_error(&id, attempt, &error)),
            Err(error) => {
                anyhow::bail!(
                    "@{id} provider failure on attempt {attempt}/{MAX_SEAT_ATTEMPTS}: turn timed out after {turn_timeout:?}: {error}"
                );
            }
        }
        mcp::clear(&outbox)?;
    }
    unreachable!("the bounded attempt loop either returns or reports exhaustion")
}

fn deadline_result<T>(
    result: Result<T, CoreError>,
    elapsed: Duration,
    timeout: Duration,
) -> Result<Result<T, CoreError>, CoreError> {
    match result {
        Err(error) if matches!(error, CoreError::DeadlineExceeded { .. }) || elapsed >= timeout => {
            Err(error)
        }
        result => Ok(result),
    }
}

fn accepted_action_reply(
    id: &str,
    utterance: &tinyhivemind_core::runtime::speech::Utterance,
    provider_error: Option<&CoreError>,
) -> anyhow::Result<String> {
    let mut body = match utterance {
        tinyhivemind_core::runtime::speech::Utterance::Broadcast { message } => {
            format!("BROADCAST: {message}")
        }
        tinyhivemind_core::runtime::speech::Utterance::CompleteEpisode { message } => {
            format!("COMPLETE: {message}")
        }
        tinyhivemind_core::runtime::speech::Utterance::Post { .. }
        | tinyhivemind_core::runtime::speech::Utterance::Dm { .. }
        | tinyhivemind_core::runtime::speech::Utterance::Ask { .. } => {
            anyhow::bail!("@{id} emitted an unsupported hive action")
        }
    };
    if let Some(error) = provider_error {
        body.push_str("\n\nProvider continuation failed after the accepted action: ");
        body.push_str(&error.to_string());
    }
    Ok(body)
}

fn provider_attempt_error(id: &str, attempt: u32, source: &CoreError) -> anyhow::Error {
    anyhow::anyhow!("@{id} provider failure on attempt {attempt}/{MAX_SEAT_ATTEMPTS}: {source}")
}

fn retryable_provider_error(error: &CoreError) -> bool {
    match error {
        CoreError::Domain { data, message, .. } => data
            .as_ref()
            .and_then(|value| value.get("retryable"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_else(|| retryable_provider_message(message)),
        CoreError::Rpc { message, .. } => retryable_provider_message(message),
        CoreError::Cancelled { .. }
        | CoreError::DeadlineExceeded { .. }
        | CoreError::StructuredOutput { .. }
        | CoreError::BudgetExceeded { .. }
        | CoreError::TurnCancelled { .. }
        | CoreError::AgentRemoved { .. }
        | CoreError::Unavailable { .. }
        | CoreError::Encode { .. }
        | CoreError::Decode { .. }
        | CoreError::InsecureRoute { .. }
        | CoreError::InvalidRoute { .. } => false,
    }
}

fn retryable_provider_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    if lower.contains("retryable=false") {
        return false;
    }
    lower.contains("retryable=true")
        || lower.contains("model returned an empty response")
        || lower.contains("429 too many requests")
        || lower.contains("502 bad gateway")
        || lower.contains("503 service unavailable")
        || lower.contains("504 gateway timeout")
        || lower.contains("529")
        || lower.contains("rate limit")
        || lower.contains("temporarily overloaded")
        || lower.contains("temporarily unavailable")
        || lower.contains("error sending request")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("dns error")
}

fn seat_prompt(task: &Task, id: &str, delta: &str, retry_instruction: Option<&str>) -> String {
    let mut prompt = format!(
        "## Task\n{}\n\n## Repository\n/workspace\n\n## New desk messages\n{}\n\n## Role\n{}\n\nUse only the deepswe MCP file/shell tools and tinyhive broadcast/complete_episode tools. End with exactly one tinyhive action. Do not access the network, credentials, browser, integrations, or paths outside /workspace.",
        task.problem_statement,
        if delta.is_empty() { "(none)" } else { delta },
        role(id),
    );
    if let Some(instruction) = retry_instruction {
        prompt.push_str("\n\n## Protocol retry\n");
        prompt.push_str(instruction);
    }
    prompt
}

fn visible_delta(visibility: &Visibility, id: &str, transcript: &[DeskMessage]) -> String {
    let seen = visibility.0.get(id);
    transcript
        .iter()
        .enumerate()
        .filter(|(index, _)| !seen.is_some_and(|set| set.contains(index)))
        .map(|(_, row)| format!("@{}: {}", row.author, row.body))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn role(id: &str) -> &'static str {
    match id {
        "lead" => {
            "Coordinate the diagnosis, delegate concrete work, and complete after reviewer sign-off."
        }
        "implementer" => {
            "Inspect the checkout, make the smallest correct edit, and broadcast changed files."
        }
        "tester" => {
            "Run the supplied tests in the sandbox, diagnose failures, and broadcast exact evidence."
        }
        "reviewer" => {
            "Review the patch and test evidence, request fixes or complete with a concise verdict."
        }
        _ => "Advance the task.",
    }
}

fn offline_config() -> RuntimeConfig {
    let mut config = RuntimeConfig::default();
    // The scripted route answers with native tool calls, so the seats have to
    // be reading them that way.
    //
    // OpenHuman's own default for this moved -- `"auto"` (native where the
    // provider supports it) to `"python"` (calls parsed out of prose against
    // Python signatures) -- and a harness that inherited it stopped seeing the
    // script's calls as calls. Nothing errored: each seat "emitted zero hive
    // actions", retried to its attempt cap, and the run failed as though the
    // model had gone quiet.
    //
    // A harness that scripts one dialect names it rather than inheriting
    // whichever is current.
    config.agent.tool_dispatcher = "auto".into();
    config.local_ai.runtime_enabled = false;
    config.runtime_python.enabled = false;
    // No memory engine answers a scripted run: skip the per-turn recall pack
    // and the per-turn conversation log.
    config.memory.recall.enabled = false;
    config.memory.conversations.enabled = false;
    config.agent.compact_context = true;
    config.default_temperature = 0.0;
    config
}

fn parse_cli(args: impl Iterator<Item = String>) -> anyhow::Result<Cli> {
    let mut values = BTreeMap::new();
    let mut args = args;
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--task" | "--api-base" | "--model" | "--output"
        ) {
            anyhow::bail!("unknown argument {flag}");
        }
        values.insert(
            flag,
            args.next()
                .ok_or_else(|| anyhow::anyhow!("missing argument value"))?,
        );
    }
    let output = PathBuf::from(required(&mut values, "--output")?);
    if !output.is_absolute() {
        anyhow::bail!("--output must be absolute");
    }
    Ok(Cli {
        task: required(&mut values, "--task")?.into(),
        api_base: required(&mut values, "--api-base")?,
        model: values
            .remove("--model")
            .unwrap_or_else(|| DEFAULT_MODEL.into()),
        output,
    })
}

fn required(values: &mut BTreeMap<String, String>, name: &str) -> anyhow::Result<String> {
    values
        .remove(name)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing {name}"))
}

#[cfg(test)]
#[path = "deepswe_hive/test.rs"]
mod test;
