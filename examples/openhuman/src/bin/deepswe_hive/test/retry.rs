//! Loopback coverage for bounded native-action protocol retries.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::super::{
    Cli, DEFAULT_MODEL, MAX_SEAT_ATTEMPTS, SEATS, run_with_mcp_executable,
    run_with_mcp_executable_and_timeout,
};
use super::fixture;
use crate::sandbox::{DockerSandbox, SandboxConfig};

#[path = "retry/response.rs"]
mod response;

#[path = "retry/fail_closed.rs"]
mod fail_closed;

use response::{
    completion_response, empty_completion_response, hive_action_response, provider_error_response,
    tool_call_response,
};

const PRINTED_JSON: &str =
    r#"{"tool":"tinyhive.complete_episode","arguments":{"message":"printed only"}}"#;

#[derive(Default)]
struct ScriptState {
    turn_starts: BTreeMap<String, u32>,
    start_requests: Vec<(String, Value)>,
    calls: u32,
}

#[derive(Clone)]
struct ScriptedHive {
    state: Arc<Mutex<ScriptState>>,
    lead_misses: u32,
    lead_failure: LeadFailure,
    lead_continuation_failure: Option<ContinuationFailure>,
    broadcast_first_turn: bool,
    refuse_after_stale_acceptance: bool,
}

#[derive(Clone, Copy)]
enum LeadFailure {
    Protocol,
    EmptyProviderResponse,
    Http(u16),
    Delay(Duration),
}

#[derive(Clone, Copy)]
enum ContinuationFailure {
    Http(u16),
}

impl Respond for ScriptedHive {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).expect("provider request JSON");
        let messages = body["messages"].as_array().expect("messages array");
        let seat = SEATS
            .iter()
            .find(|seat| {
                messages.iter().any(|message| {
                    message["role"] == "system"
                        && message["content"]
                            .as_str()
                            .is_some_and(|text| text.contains(&format!("the {seat} seat")))
                })
            })
            .expect("seat system prompt")
            .to_string();
        let last_role = messages
            .last()
            .and_then(|message| message["role"].as_str())
            .expect("last message role");
        let mut state = self.state.lock().expect("script state");
        state.calls = state.calls.saturating_add(1);

        if last_role == "tool" {
            if seat == "lead"
                && let Some(ContinuationFailure::Http(status)) = self.lead_continuation_failure
            {
                return provider_error_response(status);
            }
            return completion_response("native action accepted");
        }

        assert_eq!(last_role, "user", "unexpected provider turn boundary");
        if self.refuse_after_stale_acceptance
            && messages
                .iter()
                .filter(|message| message["role"] == "user")
                .count()
                > 1
        {
            return completion_response("stale action already accepted");
        }
        let attempt = {
            let entry = state.turn_starts.entry(seat.clone()).or_default();
            *entry = entry.saturating_add(1);
            *entry
        };
        state.start_requests.push((seat.clone(), body));
        if seat == "lead" && attempt <= self.lead_misses {
            return match self.lead_failure {
                LeadFailure::Protocol => completion_response(PRINTED_JSON),
                LeadFailure::EmptyProviderResponse => empty_completion_response(),
                LeadFailure::Http(status) => provider_error_response(status),
                LeadFailure::Delay(duration) => completion_response("too late").set_delay(duration),
            };
        }
        if self.broadcast_first_turn && attempt == 1 {
            return hive_action_response(&seat, state.calls, "broadcast");
        }
        tool_call_response(&seat, state.calls)
    }
}

fn fake_sandbox(directory: &TempDir, task: &crate::task::Task) -> DockerSandbox {
    let docker = directory.path().join("fake-docker-retry");
    let removed = directory.path().join("fake-container-removed");
    std::fs::write(
        &docker,
        format!(
            "#!/bin/sh\ncase \"$1\" in\nversion) printf fixture ;;\ncreate) rm -f '{}'; printf abcdef1234567890 ;;\ninspect) if test -e '{}'; then printf 'Error: No such object: %s' \"$2\" >&2; exit 1; fi; printf '[{{\"HostConfig\":{{\"NetworkMode\":\"none\"}},\"Mounts\":[{{\"Destination\":\"/workspace\",\"RW\":true}},{{\"Destination\":\"/workspace/.git\",\"RW\":false}}]}}]' ;;\nstart) printf sandbox-ready ;;\nrm) touch '{}' ;;\nrun) rm -f '{}'; shift; while test \"$#\" -gt 0; do if test \"$1\" = --cidfile; then shift; printf abcdef1234567890 > \"$1\"; break; fi; shift; done; exit 0 ;;\n*) exit 1 ;;\nesac\n",
            removed.display(),
            removed.display(),
            removed.display(),
            removed.display(),
        ),
    )
    .expect("fake Docker");
    make_executable(&docker);
    DockerSandbox::preflight(SandboxConfig {
        repo_path: task.repo_path.clone(),
        image: "local-fixture".into(),
        docker,
    })
    .expect("sandbox preflight")
}

fn fake_mcp(directory: &TempDir, action_copies: u32) -> std::path::PathBuf {
    let executable = directory.path().join("fake-mcp");
    let script = r##"#!/bin/sh
agent=fixture
outbox=/dev/null
while test "$#" -gt 0; do
  case "$1" in
    --agent) agent=$2; shift 2 ;;
    --outbox) outbox=$2; shift 2 ;;
    *) shift ;;
  esac
done
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -E 's/.*"id"[ ]*:[ ]*([^,}]+).*/\1/')
  case "$line" in
    *initialize*) result='{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}' ;;
    *tools/list*) result='{"tools":[{"name":"broadcast","description":"broadcast","inputSchema":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}},{"name":"complete_episode","description":"complete","inputSchema":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}}]}' ;;
    *tools/call*)
      kind=complete_episode
      message=complete
      case "$line" in *'"name":"broadcast"'*) kind=broadcast; message=broadcast ;; esac
      count=0
      while test "$count" -lt ACTION_COPIES; do
        printf '{"kind":"%s","message":"%s %s"}\n' "$kind" "$agent" "$message" >> "$outbox"
        count=$((count + 1))
      done
      result=$(printf '{"content":[{"type":"text","text":"accepted from @%s"}]}' "$agent")
      ;;
    *) result='{}' ;;
  esac
  printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done
"##
    .replace("ACTION_COPIES", &action_copies.to_string());
    std::fs::write(&executable, script).expect("fake MCP executable");
    make_executable(&executable);
    executable
}

fn make_executable(path: &Path) {
    let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("executable fixture");
}

async fn provider(
    lead_misses: u32,
    lead_failure: LeadFailure,
) -> (MockServer, Arc<Mutex<ScriptState>>) {
    provider_with_continuation(lead_misses, lead_failure, None).await
}

async fn provider_with_continuation(
    lead_misses: u32,
    lead_failure: LeadFailure,
    lead_continuation_failure: Option<ContinuationFailure>,
) -> (MockServer, Arc<Mutex<ScriptState>>) {
    let provider = MockServer::start().await;
    let state = Arc::new(Mutex::new(ScriptState::default()));
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ScriptedHive {
            state: Arc::clone(&state),
            lead_misses,
            lead_failure,
            lead_continuation_failure,
            broadcast_first_turn: false,
            refuse_after_stale_acceptance: false,
        })
        .mount(&provider)
        .await;
    (provider, state)
}

async fn two_round_provider() -> (MockServer, Arc<Mutex<ScriptState>>) {
    let provider = MockServer::start().await;
    let state = Arc::new(Mutex::new(ScriptState::default()));
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ScriptedHive {
            state: Arc::clone(&state),
            lead_misses: 0,
            lead_failure: LeadFailure::Protocol,
            lead_continuation_failure: None,
            broadcast_first_turn: true,
            refuse_after_stale_acceptance: true,
        })
        .mount(&provider)
        .await;
    (provider, state)
}

#[test]
fn a_new_hive_turn_does_not_inherit_a_prior_action_acceptance() {
    let _guard = retry_test_guard();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (directory, task) = fixture();
        let sandbox = fake_sandbox(&directory, &task);
        let mcp = fake_mcp(&directory, 1);
        let (provider, state) = two_round_provider().await;
        let output_directory = TempDir::new().expect("output directory");
        let output = output_directory.path().join("output.json");
        let cli = Cli {
            task: directory.path().join("unused.json"),
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output: output.clone(),
        };

        let error = tokio::spawn(async move {
            run_with_mcp_executable(cli, task, sandbox, "loopback-only".into(), &mcp).await
        })
        .await
        .expect("adapter task")
        .expect_err("empty patch makes the completed episode fail");
        assert!(
            error.to_string().contains("episode result is failed"),
            "a stale acceptance poisoned the next hive turn: {error:#}"
        );
        let result: Value = serde_json::from_slice(&std::fs::read(&output).expect("result file"))
            .expect("result JSON");
        assert_eq!(result["turns"], 8, "two four-seat rounds commit");

        let state = state.lock().expect("script state");
        for seat in SEATS {
            assert_eq!(state.turn_starts.get(seat), Some(&2), "@{seat} turn count");
        }
        let second_lead = state
            .start_requests
            .iter()
            .filter(|(seat, _)| seat == "lead")
            .nth(1)
            .map(|(_, request)| request)
            .expect("second lead turn");
        assert!(
            second_lead["messages"]
                .as_array()
                .expect("messages")
                .iter()
                .filter(|message| message["role"] == "user")
                .count()
                == 1,
            "the next hive turn retained an old prompt: {second_lead:#}"
        );
    });
}

#[test]
fn accepted_action_survives_post_tool_provider_failure_without_retry() {
    let _guard = retry_test_guard();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (directory, task) = fixture();
        let sandbox = fake_sandbox(&directory, &task);
        let mcp = fake_mcp(&directory, 1);
        let (provider, state) = provider_with_continuation(
            0,
            LeadFailure::Protocol,
            Some(ContinuationFailure::Http(429)),
        )
        .await;
        let output_directory = TempDir::new().expect("output directory");
        let output = output_directory.path().join("output.json");
        let cli = Cli {
            task: directory.path().join("unused.json"),
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output: output.clone(),
        };

        let error = tokio::spawn(async move {
            run_with_mcp_executable(cli, task, sandbox, "loopback-only".into(), &mcp).await
        })
        .await
        .expect("adapter task")
        .expect_err("empty patch makes the completed episode fail");
        assert!(error.to_string().contains("episode result is failed"));

        let result: Value = serde_json::from_slice(&std::fs::read(&output).expect("result file"))
            .expect("result JSON");
        assert_eq!(result["turns"], 4, "the accepted round commits once");
        let transcript = std::fs::read_to_string(output_directory.path().join("transcript.md"))
            .expect("transcript");
        for seat in SEATS {
            assert_eq!(
                transcript.matches(&format!("## @{seat}\n")).count(),
                1,
                "@{seat} was committed more than once"
            );
        }
        assert!(transcript.contains("COMPLETE: lead complete"));
        assert!(transcript.contains("Provider continuation failed after the accepted action"));
        let accepted =
            std::fs::read_to_string(output_directory.path().join("outboxes").join("lead.jsonl"))
                .expect("accepted lead action remains in the outbox");
        assert_eq!(accepted.lines().count(), 1);

        let state = state.lock().expect("script state");
        for seat in SEATS {
            assert_eq!(state.turn_starts.get(seat), Some(&1), "@{seat} reran");
        }
    });
}

#[test]
fn one_missing_seat_retries_in_a_fresh_session_and_round_commits_once() {
    let _guard = retry_test_guard();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (directory, task) = fixture();
        let sandbox = fake_sandbox(&directory, &task);
        let mcp = fake_mcp(&directory, 1);
        let (provider, state) = provider(1, LeadFailure::Protocol).await;
        let output_directory = TempDir::new().expect("output directory");
        let output = output_directory.path().join("output.json");
        let cli = Cli {
            task: directory.path().join("unused.json"),
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output: output.clone(),
        };

        let error = tokio::spawn(async move {
            run_with_mcp_executable(cli, task, sandbox, "loopback-only".into(), &mcp).await
        })
        .await
        .expect("adapter task")
        .expect_err("empty patch makes the completed episode fail");
        assert!(
            error.to_string().contains("episode result is failed"),
            "unexpected adapter error: {error:#}"
        );
        let result: Value = serde_json::from_slice(&std::fs::read(&output).expect("result file"))
            .expect("result JSON");
        assert_eq!(result["turns"], 4, "the missed attempt is not committed");

        let transcript = std::fs::read_to_string(output_directory.path().join("transcript.md"))
            .expect("transcript");
        let positions = SEATS.map(|seat| {
            transcript
                .find(&format!("## @{seat}"))
                .expect("seat transcript row")
        });
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

        let state = state.lock().expect("script state");
        assert_eq!(state.turn_starts.get("lead"), Some(&2));
        for seat in ["implementer", "tester", "reviewer"] {
            assert_eq!(state.turn_starts.get(seat), Some(&1), "@{seat} reran");
        }
        let lead_retry = state
            .start_requests
            .iter()
            .filter(|(seat, _)| seat == "lead")
            .nth(1)
            .map(|(_, body)| body)
            .expect("lead retry request");
        let messages = lead_retry["messages"].as_array().expect("retry messages");
        assert!(
            messages
                .iter()
                .all(|message| message["content"] != PRINTED_JSON),
            "the retry retained the invalid provider response"
        );
        let retry_prompt = messages
            .last()
            .and_then(|message| message["content"].as_str())
            .expect("retry prompt");
        assert!(retry_prompt.contains("prior attempt was invalid"));
        assert!(retry_prompt.contains("exactly one native mcp_call_tool"));
        assert!(retry_prompt.contains("accepted from @lead"));
        assert!(retry_prompt.contains("## New desk messages\n(none)"));
        for (_, request) in &state.start_requests {
            let prompt = request["messages"]
                .as_array()
                .expect("messages")
                .last()
                .and_then(|message| message["content"].as_str())
                .expect("turn prompt");
            assert!(prompt.contains("## New desk messages\n(none)"));
        }
    });
}

#[test]
fn zero_action_retry_exhaustion_is_bounded_without_a_commit() {
    let _guard = retry_test_guard();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (directory, task) = fixture();
        let sandbox = fake_sandbox(&directory, &task);
        let mcp = fake_mcp(&directory, 1);
        let (provider, state) = provider(MAX_SEAT_ATTEMPTS, LeadFailure::Protocol).await;
        let output_directory = TempDir::new().expect("output directory");
        let output = output_directory.path().join("output.json");
        let cli = Cli {
            task: directory.path().join("unused.json"),
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output: output.clone(),
        };

        let error = tokio::spawn(async move {
            run_with_mcp_executable(cli, task, sandbox, "loopback-only".into(), &mcp).await
        })
        .await
        .expect("adapter task")
        .expect_err("lead never makes a native action");
        assert_eq!(
            error.to_string(),
            format!("@lead emitted zero hive actions after {MAX_SEAT_ATTEMPTS} attempts")
        );
        assert!(!output.exists());
        assert!(!output_directory.path().join("transcript.md").exists());
        let state = state.lock().expect("script state");
        assert_eq!(state.turn_starts.get("lead"), Some(&MAX_SEAT_ATTEMPTS));
        for seat in ["implementer", "tester", "reviewer"] {
            assert_eq!(state.turn_starts.get(seat), Some(&1), "@{seat} reran");
        }
    });
}

#[test]
fn multiple_native_actions_fail_immediately_without_retry_or_commit() {
    let _guard = retry_test_guard();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (directory, task) = fixture();
        let sandbox = fake_sandbox(&directory, &task);
        let mcp = fake_mcp(&directory, 2);
        let (provider, state) = provider(0, LeadFailure::Protocol).await;
        let output_directory = TempDir::new().expect("output directory");
        let output = output_directory.path().join("output.json");
        let cli = Cli {
            task: directory.path().join("unused.json"),
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output: output.clone(),
        };

        let error = tokio::spawn(async move {
            run_with_mcp_executable(cli, task, sandbox, "loopback-only".into(), &mcp).await
        })
        .await
        .expect("adapter task")
        .expect_err("multiple native actions must fail");
        assert_eq!(
            error.to_string(),
            "@lead emitted 2 hive actions on attempt 1; expected one"
        );
        assert!(!output.exists());
        assert!(!output_directory.path().join("transcript.md").exists());
        let state = state.lock().expect("script state");
        for seat in SEATS {
            assert_eq!(state.turn_starts.get(seat), Some(&1), "@{seat} retried");
        }
    });
}

#[test]
fn empty_provider_response_retries_only_that_seat_and_commits_once() {
    let _guard = retry_test_guard();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (directory, task) = fixture();
        let sandbox = fake_sandbox(&directory, &task);
        let mcp = fake_mcp(&directory, 1);
        let (provider, state) = provider(1, LeadFailure::EmptyProviderResponse).await;
        let output_directory = TempDir::new().expect("output directory");
        let output = output_directory.path().join("output.json");
        let cli = Cli {
            task: directory.path().join("unused.json"),
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output: output.clone(),
        };

        let error = tokio::spawn(async move {
            run_with_mcp_executable(cli, task, sandbox, "loopback-only".into(), &mcp).await
        })
        .await
        .expect("adapter task")
        .expect_err("empty patch makes the completed episode fail");
        assert!(error.to_string().contains("episode result is failed"));
        let result: Value = serde_json::from_slice(&std::fs::read(&output).expect("result file"))
            .expect("result JSON");
        assert_eq!(result["turns"], 4, "provider failure is not committed");

        let state = state.lock().expect("script state");
        // The empty response, then the hive's own fresh retry. There is no
        // second fallback above it any more: an episode used to build its own
        // `OpenHumanSessionHost` beside the one the host already had, and both
        // turn loops answered an empty completion. Seating the episode on the
        // host's own agent left one loop, so one retry -- which is the point of
        // that change, not a regression in it.
        assert_eq!(
            state.turn_starts.get("lead"),
            Some(&2),
            "the empty response, then one fresh hive retry"
        );
        for seat in ["implementer", "tester", "reviewer"] {
            assert_eq!(state.turn_starts.get(seat), Some(&1), "@{seat} reran");
        }
    });
}

#[test]
fn non_retryable_provider_auth_failure_is_immediate() {
    let _guard = retry_test_guard();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (directory, task) = fixture();
        let sandbox = fake_sandbox(&directory, &task);
        let mcp = fake_mcp(&directory, 1);
        let (provider, state) = provider(MAX_SEAT_ATTEMPTS, LeadFailure::Http(401)).await;
        let output_directory = TempDir::new().expect("output directory");
        let output = output_directory.path().join("output.json");
        let cli = Cli {
            task: directory.path().join("unused.json"),
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output: output.clone(),
        };

        let error = tokio::spawn(async move {
            run_with_mcp_executable(cli, task, sandbox, "loopback-only".into(), &mcp).await
        })
        .await
        .expect("adapter task")
        .expect_err("authentication failure must abort");
        let message = error.to_string();
        assert!(message.starts_with("@lead provider failure on attempt 1/3:"));
        assert!(!output.exists());

        let state = state.lock().expect("script state");
        assert_eq!(state.turn_starts.get("lead"), Some(&1));
    });
}

#[test]
fn retryable_provider_exhaustion_is_bounded_without_a_commit() {
    let _guard = retry_test_guard();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (directory, task) = fixture();
        let sandbox = fake_sandbox(&directory, &task);
        let mcp = fake_mcp(&directory, 1);
        let (provider, state) = provider(u32::MAX, LeadFailure::Http(429)).await;
        let output_directory = TempDir::new().expect("output directory");
        let output = output_directory.path().join("output.json");
        let cli = Cli {
            task: directory.path().join("unused.json"),
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output: output.clone(),
        };

        let error = tokio::spawn(async move {
            run_with_mcp_executable(cli, task, sandbox, "loopback-only".into(), &mcp).await
        })
        .await
        .expect("adapter task")
        .expect_err("retryable provider failures must exhaust the bounded budget");
        let message = error.to_string();
        assert!(
            message.starts_with("@lead provider failure on attempt 3/3:"),
            "unexpected error: {message}"
        );
        assert!(!output.exists());

        let state = state.lock().expect("script state");
        let lead_prompts = state
            .start_requests
            .iter()
            .filter(|(seat, _)| seat == "lead")
            .filter_map(|(_, request)| request["messages"].as_array()?.last()?["content"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            lead_prompts.len(),
            MAX_SEAT_ATTEMPTS as usize,
            "each bounded retry gets its own prompt"
        );
        for seat in ["implementer", "tester", "reviewer"] {
            assert_eq!(state.turn_starts.get(seat), Some(&1), "@{seat} ran once");
        }
    });
}

fn retry_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static OPENHUMAN_RUNTIME: Mutex<()> = Mutex::new(());
    OPENHUMAN_RUNTIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
