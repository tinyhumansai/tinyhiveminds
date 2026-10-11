//! Contract tests for DeepSWE input, output, Git, and confinement behavior.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use wiremock::MockServer;

use super::sandbox::{
    DockerSandbox, InspectorFailure, SandboxConfig, action_arguments, patch_with_limits,
};
use super::{
    Cli, DEFAULT_MODEL, ResultDocument, ensure_round_fits, parse_cli, run, validate_output_paths,
};
use crate::task::Task;

#[path = "test/deadline.rs"]
mod deadline;
#[path = "test/docker.rs"]
mod docker;
#[path = "test/retry.rs"]
mod retry;
#[path = "test/security.rs"]
mod security;

fn real_docker_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static REAL_DOCKER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    REAL_DOCKER.lock().expect("real Docker test lock")
}

fn fixture() -> (TempDir, Task) {
    let directory = TempDir::new().expect("temporary repository");
    std::fs::write(directory.path().join("answer.txt"), "wrong\n").expect("fixture source");
    std::fs::write(
        directory.path().join("test.sh"),
        "#!/bin/sh\ntest \"$(cat answer.txt)\" = right\n",
    )
    .expect("fixture test");
    git(directory.path(), &["init", "-q"]);
    git(
        directory.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    git(
        directory.path(),
        &["config", "user.name", "DeepSWE Fixture"],
    );
    git(directory.path(), &["add", "."]);
    git(directory.path(), &["commit", "-qm", "fixture"]);
    let base = git_output(directory.path(), &["rev-parse", "HEAD"]);
    let task = Task {
        instance_id: "local-1".into(),
        repo_path: directory.path().to_path_buf(),
        base_commit: base.trim().into(),
        problem_statement: "Make test.sh pass.".into(),
        test_command: "sh test.sh".into(),
    };
    (directory, task)
}

#[test]
fn cli_defaults_to_exact_gpt_oss_model() {
    let cli = parse_cli(
        [
            "--task",
            "/tmp/task.json",
            "--api-base",
            "http://127.0.0.1",
            "--output",
            "/tmp/out.json",
        ]
        .into_iter()
        .map(str::to_string),
    )
    .expect("valid CLI");
    assert_eq!(cli.model, DEFAULT_MODEL);
}

#[test]
fn rejects_blank_fields_and_relative_paths() {
    let (_directory, mut task) = fixture();
    task.problem_statement = "  ".into();
    assert!(
        task.validate()
            .expect_err("blank rejected")
            .to_string()
            .contains("problem_statement")
    );
    task.problem_statement = "fix it".into();
    task.repo_path = "relative".into();
    assert!(
        task.validate()
            .expect_err("relative rejected")
            .to_string()
            .contains("absolute")
    );
}

#[test]
fn rejects_an_absolute_directory_that_is_not_a_git_checkout() {
    let (_directory, mut task) = fixture();
    let outside = TempDir::new().expect("non-Git directory");
    task.repo_path = outside.path().to_path_buf();
    assert!(
        task.validate()
            .expect_err("non-Git directory rejected")
            .to_string()
            .contains("validate git checkout")
    );
}

#[test]
fn rejects_unknown_base_and_dirty_checkout() {
    let (directory, mut task) = fixture();
    task.base_commit = "deadbeef".into();
    assert!(
        task.validate()
            .expect_err("unknown base rejected")
            .to_string()
            .contains("base_commit")
    );
    task.base_commit = git_output(directory.path(), &["rev-parse", "HEAD"])
        .trim()
        .into();
    std::fs::write(directory.path().join("answer.txt"), "dirty\n").expect("dirty fixture");
    assert!(
        task.validate()
            .expect_err("dirty rejected")
            .to_string()
            .contains("start clean")
    );
}

#[test]
fn rejects_ignored_files_including_environment_secrets() {
    let (directory, task) = fixture();
    std::fs::write(directory.path().join(".gitignore"), ".env\n").expect("ignore file");
    git(directory.path(), &["add", ".gitignore"]);
    git(directory.path(), &["commit", "-qm", "ignore environment"]);
    let mut task = task;
    task.base_commit = git_output(directory.path(), &["rev-parse", "HEAD"])
        .trim()
        .into();
    std::fs::write(directory.path().join(".env"), "SECRET=must-not-leak\n")
        .expect("ignored secret");
    assert!(
        task.validate()
            .expect_err("ignored file rejected")
            .to_string()
            .contains("ignored")
    );
}

#[test]
fn rejects_alternate_git_metadata_inside_the_checkout() {
    let parent = TempDir::new().expect("repository parent");
    let repo = parent.path().join("repo");
    std::fs::create_dir(&repo).expect("repository directory");
    let git_dir = repo.join(".realgit");
    let status = Command::new("git")
        .args(["init", "-q", "--separate-git-dir"])
        .arg(&git_dir)
        .arg(&repo)
        .status()
        .expect("git init runs");
    assert!(status.success());
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    git(&repo, &["config", "user.name", "DeepSWE Fixture"]);
    std::fs::write(repo.join("answer.txt"), "wrong\n").expect("fixture source");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let task = Task {
        instance_id: "alternate-git-dir".into(),
        repo_path: repo,
        base_commit: git_output(parent.path().join("repo").as_path(), &["rev-parse", "HEAD"])
            .trim()
            .into(),
        problem_statement: "change the answer".into(),
        test_command: "true".into(),
    };
    assert!(
        task.validate()
            .expect_err("in-checkout Git metadata rejected")
            .to_string()
            .contains("Git metadata")
    );
}

#[test]
fn output_and_all_side_artifacts_must_be_absolute_and_outside_checkout() {
    let (directory, task) = fixture();
    assert!(
        parse_cli(
            [
                "--task",
                "/tmp/task.json",
                "--api-base",
                "http://127.0.0.1",
                "--output",
                "result.json",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .expect_err("relative output rejected")
        .to_string()
        .contains("absolute")
    );
    let inside = directory.path().join("results/result.json");
    std::fs::create_dir_all(inside.parent().expect("inside parent")).expect("inside output parent");
    assert!(
        validate_output_paths(&task, &inside)
            .expect_err("inside output rejected")
            .to_string()
            .contains("outside")
    );
    let outside = directory
        .path()
        .parent()
        .expect("temporary parent")
        .join("deepswe-output/result.json");
    std::fs::create_dir_all(outside.parent().expect("outside parent"))
        .expect("outside output parent");
    validate_output_paths(&task, &outside).expect("outside artifacts accepted");
}

#[test]
fn ignored_secret_refusal_happens_before_docker_or_provider() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (directory, task) = fixture();
        std::fs::write(directory.path().join(".gitignore"), ".env\n").expect("ignore file");
        git(directory.path(), &["add", ".gitignore"]);
        git(directory.path(), &["commit", "-qm", "ignore environment"]);
        std::fs::write(directory.path().join(".env"), "SECRET=must-not-leak\n")
            .expect("ignored secret");
        let value = serde_json::json!({
            "instance_id": task.instance_id,
            "repo_path": task.repo_path,
            "base_commit": git_output(directory.path(), &["rev-parse", "HEAD"]).trim(),
            "problem_statement": task.problem_statement,
            "test_command": task.test_command,
        });
        let task_path = directory
            .path()
            .parent()
            .expect("temporary parent")
            .join("ignored-task.json");
        std::fs::write(&task_path, serde_json::to_vec(&value).expect("task JSON"))
            .expect("task file");
        let provider = MockServer::start().await;
        let output = directory
            .path()
            .parent()
            .expect("temporary parent")
            .join("ignored-result.json");
        let cli = Cli {
            task: task_path,
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output,
        };
        let error = run(cli).await.expect_err("ignored checkout refused");
        assert!(error.to_string().contains("ignored"));
        assert!(
            provider
                .received_requests()
                .await
                .expect("requests")
                .is_empty()
        );
    });
}

#[test]
fn rejects_a_tag_even_when_it_resolves_to_head() {
    let (directory, mut task) = fixture();
    git(directory.path(), &["tag", "-am", "tag", "fixture-tag"]);
    task.base_commit = "fixture-tag".into();
    assert!(
        task.validate()
            .expect_err("tag rejected")
            .to_string()
            .contains("commit object directly")
    );
}

#[test]
fn rejects_repo_subdirectory_and_head_different_from_base() {
    let (directory, mut task) = fixture();
    std::fs::create_dir(directory.path().join("subdir")).expect("subdir");
    task.repo_path = directory.path().join("subdir");
    assert!(
        task.validate()
            .expect_err("subdir rejected")
            .to_string()
            .contains("root")
    );
    task.repo_path = directory.path().to_path_buf();
    std::fs::write(directory.path().join("second.txt"), "second\n").expect("second source");
    git(directory.path(), &["add", "."]);
    git(directory.path(), &["commit", "-qm", "second"]);
    assert!(
        task.validate()
            .expect_err("different HEAD rejected")
            .to_string()
            .contains("HEAD")
    );
}

#[test]
fn task_json_rejects_unknown_fields() {
    let (directory, task) = fixture();
    let input = directory.path().join("task.json");
    let value = serde_json::json!({
        "instance_id": task.instance_id,
        "repo_path": task.repo_path,
        "base_commit": task.base_commit,
        "problem_statement": task.problem_statement,
        "test_command": task.test_command,
        "unexpected": true
    });
    std::fs::write(&input, serde_json::to_vec(&value).expect("JSON")).expect("task input");
    assert!(
        Task::load(&input)
            .expect_err("unknown field rejected")
            .to_string()
            .contains("unknown field")
    );
}

#[test]
fn output_schema_has_only_the_public_result_fields() {
    let value = serde_json::to_value(ResultDocument {
        instance_id: "fixture".into(),
        status: "passed".into(),
        model: DEFAULT_MODEL.into(),
        turns: 4,
        patch: "diff".into(),
        test_exit_code: Some(0),
        transcript_path: "/result/transcript.md".into(),
    })
    .expect("serialize result");
    let keys = value
        .as_object()
        .expect("object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        [
            "instance_id",
            "model",
            "patch",
            "status",
            "test_exit_code",
            "transcript_path",
            "turns"
        ]
    );
    assert!(!value.to_string().contains("key"));
    let mut with_extra = value;
    with_extra["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ResultDocument>(with_extra).is_err());
}

#[test]
fn refuses_to_schedule_a_round_past_the_exact_turn_cap() {
    ensure_round_fits(20, 4).expect("exact cap accepted");
    assert!(
        ensure_round_fits(21, 4)
            .expect_err("oversized round rejected")
            .to_string()
            .contains("exceed")
    );
}

#[test]
fn docker_mounts_reject_delimiter_paths_before_argument_rendering() {
    for (source, destination) in [
        ("/tmp/source,comma", "/workspace"),
        ("/tmp/source", "/workspace,comma"),
    ] {
        let error = super::sandbox::mount(std::path::Path::new(source), destination, false)
            .expect_err("comma cannot enter Docker mount syntax");
        assert!(error.to_string().contains("commas"));
    }
}

#[test]
fn hive_mcp_writes_only_native_tool_calls_and_separates_protocol_errors() {
    let directory = TempDir::new().expect("outbox directory");
    let outbox = directory.path().join("lead.jsonl");
    super::mcp::clear(&outbox).expect("clear outbox");
    let server = super::mcp::Server::Hive {
        agent: "lead".into(),
        outbox: outbox.clone(),
    };
    let response = super::mcp::response(
        &server,
        &serde_json::json!({
            "method": "tools/call",
            "params": {"name": "broadcast", "arguments": {"message": "test passed"}}
        }),
        serde_json::json!(1),
    );
    assert_eq!(response["result"]["content"][0]["type"], "text");
    assert!(matches!(
        super::mcp::drain(&outbox).expect("drain outbox").as_slice(),
        [tinyhivemind_core::runtime::speech::Utterance::Broadcast { message }] if message == "test passed"
    ));
    for request in [
        serde_json::json!({"method": "unknown"}),
        serde_json::json!({"method": "tools/call", "params": {"name": "unknown", "arguments": {}}}),
        serde_json::json!({"method": "tools/call", "params": {"name": "broadcast", "arguments": {}}}),
    ] {
        let response = super::mcp::response(&server, &request, serde_json::json!(2));
        let expected = if request["method"] == "unknown" {
            -32601
        } else {
            -32602
        };
        assert_eq!(response["error"]["code"], expected);
    }
    std::fs::remove_file(&outbox).expect("remove prepared outbox");
    let response = super::mcp::response(
        &server,
        &serde_json::json!({
            "method": "tools/call",
            "params": {"name": "broadcast", "arguments": {"message": "retry"}}
        }),
        serde_json::json!(3),
    );
    assert_eq!(response["result"]["isError"], true);
    std::fs::write(&outbox, vec![b'x'; 64 * 1024 + 1]).expect("oversized outbox");
    assert!(
        super::mcp::drain(&outbox)
            .expect_err("oversized outbox is rejected")
            .to_string()
            .contains("exceeds")
    );
}

#[test]
fn inspector_timeout_kills_a_hung_docker_cli() {
    let (directory, task) = fixture();
    let docker = directory.path().join("fake-docker-timeout");
    let removed = directory.path().join("fake-container-removed");
    std::fs::write(
        &docker,
        format!(
            "#!/bin/sh\ncase \"$1\" in\nversion) printf fixture ;;\ncreate) rm -f '{}'; printf abcdef1234567890 ;;\ninspect) if test -e '{}'; then printf 'Error: No such object: %s' \"$2\" >&2; exit 1; fi; printf '[{{\"HostConfig\":{{\"NetworkMode\":\"none\"}},\"Mounts\":[{{\"Destination\":\"/workspace\",\"RW\":true}},{{\"Destination\":\"/workspace/.git\",\"RW\":false}}]}}]' ;;\nstart) printf sandbox-ready ;;\nrm) touch '{}' ;;\nrun) rm -f '{}'; shift; while test \"$#\" -gt 0; do if test \"$1\" = --cidfile; then shift; printf abcdef1234567890 > \"$1\"; break; fi; shift; done; exec sleep 30 ;;\n*) exit 1 ;;\nesac\n",
            removed.display(),
            removed.display(),
            removed.display(),
            removed.display(),
        ),
    )
    .expect("fake Docker");
    let mut permissions = std::fs::metadata(&docker).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&docker, permissions).expect("executable fake Docker");
    let sandbox = DockerSandbox::preflight(SandboxConfig {
        repo_path: task.repo_path.clone(),
        image: "local-fixture".into(),
        docker,
    })
    .expect("preflight");
    let started = Instant::now();
    let error = patch_with_limits(&sandbox, &task.base_commit, Duration::from_millis(50), 1024)
        .expect_err("hung inspector rejected");
    assert!(error.to_string().contains("timed out"));
    assert!(matches!(
        error.downcast_ref::<InspectorFailure>(),
        Some(InspectorFailure::TimedOut { .. })
    ));
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[test]
fn sandbox_runs_as_the_checkout_owner() {
    let (directory, task) = fixture();
    let docker = directory.path().join("fake-docker-owner");
    let removed = directory.path().join("fake-container-removed");
    std::fs::write(
        &docker,
        format!(
            "#!/bin/sh\ncase \"$1\" in\nversion) printf fixture ;;\ncreate) rm -f '{}'; printf abcdef1234567890 ;;\ninspect) if test -e '{}'; then printf 'Error: No such object: %s' \"$2\" >&2; exit 1; fi; printf '[{{\"HostConfig\":{{\"NetworkMode\":\"none\"}},\"Mounts\":[{{\"Destination\":\"/workspace\",\"RW\":true}},{{\"Destination\":\"/workspace/.git\",\"RW\":false}}]}}]' ;;\nstart) printf sandbox-ready ;;\nrm) touch '{}' ;;\n*) exit 1 ;;\nesac\n",
            removed.display(),
            removed.display(),
            removed.display(),
        ),
    )
    .expect("fake Docker");
    let mut permissions = std::fs::metadata(&docker).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&docker, permissions).expect("executable fake Docker");
    let sandbox = DockerSandbox::preflight(SandboxConfig {
        repo_path: task.repo_path.clone(),
        image: "local-fixture".into(),
        docker,
    })
    .expect("preflight");
    let metadata = std::fs::metadata(&task.repo_path).expect("checkout metadata");
    let expected = format!("{}:{}", metadata.uid(), metadata.gid());
    let args = action_arguments(&sandbox).expect("action arguments");

    assert!(
        args.windows(2)
            .any(|pair| pair == ["--user", expected.as_str()])
    );
}

fn copy_fixture(source: std::path::PathBuf, destination: &Path) {
    for entry in std::fs::read_dir(source).expect("fixture directory") {
        let entry = entry.expect("fixture entry");
        std::fs::copy(entry.path(), destination.join(entry.file_name()))
            .expect("copy fixture file");
    }
}

fn git(directory: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(directory)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

fn git_output(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).expect("utf8 git output")
}
