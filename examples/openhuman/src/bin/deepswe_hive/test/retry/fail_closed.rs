//! Timeout and ambiguous action failures never retry or commit.
use super::*;

#[test]
fn timeout_with_no_action_fails_closed_without_retry() {
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
        // The native deadline includes MCP startup; leave enough time to reach
        // the delayed provider, while preserving the single-attempt assertion.
        let (provider, state) = provider(1, LeadFailure::Delay(Duration::from_secs(30))).await;
        let output_directory = TempDir::new().expect("output directory");
        let output = output_directory.path().join("output.json");
        let cli = Cli {
            task: directory.path().join("unused.json"),
            api_base: format!("{}/v1", provider.uri()),
            model: DEFAULT_MODEL.into(),
            output: output.clone(),
        };

        let error = tokio::spawn(async move {
            run_with_mcp_executable_and_timeout(
                cli,
                task,
                sandbox,
                "loopback-only".into(),
                &mcp,
                Duration::from_secs(10),
            )
            .await
        })
        .await
        .expect("adapter task")
        .expect_err("an ambiguous timeout must fail closed");
        assert!(error.to_string().contains("turn timed out"));
        assert!(!output.exists());

        let state = state.lock().expect("script state");
        assert_eq!(state.turn_starts.get("lead"), Some(&1));
    });
}

#[test]
fn multiple_actions_after_provider_failure_hard_fail_without_retry() {
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
        .expect_err("multiple actions must fail even after provider failure");
        assert_eq!(
            error.to_string(),
            "@lead emitted 2 hive actions on attempt 1; expected one"
        );
        assert!(!output.exists());

        let state = state.lock().expect("script state");
        assert_eq!(state.turn_starts.get("lead"), Some(&1));
    });
}
