//! Checked-in manifests preserve the seats, policy and native tool boundaries.
use tinyhivemind_openhuman::config::{Access, HiveConfig, ToolScope};

fn load(name: &str) -> HiveConfig {
    tinyhivemind_openhuman_example::manifest(name)
        .unwrap()
        .validate()
        .unwrap()
        .into_config()
}

#[test]
fn basic_preserves_two_readonly_seats_and_ephemeral_workspace() {
    let config = load("basic_hive");
    assert_eq!(
        config
            .seats
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["alice", "bob"]
    );
    assert_eq!(config.runtime.autonomy, Access::ReadOnly);
    assert!(config.runtime.workspace.is_none());
    assert_eq!(config.hives[0].members.len(), 2);
    assert!(
        config
            .profiles
            .iter()
            .all(|p| p.system_prompt.contains("hivemind_complete"))
    );
}

#[test]
fn deepswe_preserves_roles_mcp_iterations_sampling_and_width() {
    let config = load("deepswe_hive");
    assert_eq!(
        config
            .seats
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["lead", "implementer", "tester", "reviewer"]
    );
    assert_eq!(config.hives[0].episode.round_width, 4);
    assert_eq!(config.hives[0].coordinator.round_width, 4);
    for profile in &config.profiles {
        assert_eq!(profile.limits.iterations, Some(16));
        assert_eq!(profile.model.temperature, Some(0.0));
        assert_eq!(
            profile.tool_scope,
            ToolScope::Named(vec!["mcp_list_tools".into(), "mcp_call_tool".into()])
        );
    }
    assert_eq!(
        config.runtime.mcp[0].tools.as_ref().unwrap(),
        &["file_read", "file_write", "file_edit", "shell", "test"]
    );
    assert_eq!(
        config.profiles[0].mcp[0].tools.as_ref().unwrap(),
        &["broadcast", "complete_episode"]
    );
}

#[test]
fn euler_preserves_five_roles_native_rules_limits_and_workspace_templates() {
    let config = load("pe1006_hive");
    assert_eq!(config.seats.len(), 5);
    for profile in &config.profiles {
        assert_eq!(profile.limits.iterations, Some(12));
        assert_eq!(profile.model.temperature, Some(0.0));
        assert_eq!(profile.deny_tools, ["run_code", "ask_docs"]);
        assert_eq!(
            profile.tool_scope,
            ToolScope::Named(vec![
                "file_read".into(),
                "file_write".into(),
                "mcp_list_tools".into(),
                "mcp_call_tool".into(),
                "shell".into()
            ])
        );
    }
    assert!(config.contexts["agents"].contains("Never overwrite another role"));
    assert!(config.contexts["memory"].contains("Psi(3) = 20302"));
    assert!(config.contexts["task"].contains("Psi(10^18)"));
    assert!(
        config.profiles[0].mcp[0]
            .tools
            .as_ref()
            .unwrap()
            .contains(&"hive_memory_note".into())
    );
}

#[test]
fn multi_shares_one_seat_with_distinct_roles_and_narrower_second_hive() {
    let config = load("multi_hive");
    assert_eq!(config.profiles.len(), 3);
    assert_eq!(config.seats.len(), 4);
    assert_eq!(config.hives.len(), 2);
    let a = config.hives[0]
        .members
        .iter()
        .find(|m| m.seat == "shared")
        .unwrap();
    let b = config.hives[1]
        .members
        .iter()
        .find(|m| m.seat == "shared")
        .unwrap();
    assert_ne!(a.role, b.role);
    assert!(a.permission_profile.is_none());
    assert_eq!(b.permission_profile.as_deref(), Some("readonly"));
}

#[test]
fn all_manifests_lower_into_one_native_runtime_with_the_declared_seats_and_limits()
-> anyhow::Result<()> {
    use tinyhivemind_openhuman::deploy::{BuildOptions, native_memory_root};
    let executor = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()?;
    executor.block_on(async {
        tokio::spawn(async {
            let backend = tinyhivemind_openhuman::offline::backend().await;
            for name in ["basic_hive", "deepswe_hive", "pe1006_hive", "multi_hive"] {
                let mut config = load(name);
                // A deterministic reference engine substitutes for operator CortexDB
                // credentials. Provider and MCP connections are never invoked here.
                if name == "pe1006_hive" {
                    config.runtime.memory_engine = "memory".into();
                }
                config.runtime.provider = Default::default();
                let expected = config.clone();
                let deployment = tinyhivemind_openhuman_example::deploy(
                    config,
                    BuildOptions {
                        config: Some(tinyhivemind_openhuman::offline::config()),
                        backend_url: Some(backend.uri()),
                        provider: Some(
                            openhuman_embed::Provider::openai_compatible(
                                "http://127.0.0.1:1/v1",
                                "fixture",
                            )
                            .model("fixture"),
                        ),
                        ..Default::default()
                    },
                )
                .await?;
                assert_eq!(deployment.seats().len(), expected.seats.len(), "{name}");
                assert_eq!(
                    deployment.host().coordinator().list_hives()?.len(),
                    expected.hives.len(),
                    "{name}"
                );
                for seat in &expected.seats {
                    let native = &deployment.seats()[&seat.id];
                    assert_eq!(native.runtime_id(), deployment.runtime().runtime_id());
                    assert_eq!(native.id(), seat.id);
                    assert_eq!(
                        native.config().memory.root.as_deref(),
                        Some(native_memory_root(&format!("seat:{}", seat.id))?.as_str())
                    );
                    let profile = expected.profile_for(seat)?;
                    if let Some(iterations) = profile.limits.iterations {
                        assert_eq!(
                            native.config().agent.max_tool_iterations_override,
                            Some(iterations),
                            "{name}:{}",
                            seat.id
                        );
                    }
                    if let Some(temperature) = profile.model.temperature {
                        assert_eq!(
                            native.config().default_temperature,
                            temperature,
                            "{name}:{}",
                            seat.id
                        );
                    }
                }
                for hive in &expected.hives {
                    let settings = deployment
                        .host()
                        .coordinator()
                        .hive_settings(&hive.id)?
                        .unwrap();
                    for member in &hive.members {
                        assert_eq!(settings.roles[&member.seat], member.role);
                    }
                    assert_eq!(deployment.policy(&hive.id).unwrap().episode, hive.episode);
                }
                drop(deployment);
            }
            Ok::<_, anyhow::Error>(())
        })
        .await?
    })
}
