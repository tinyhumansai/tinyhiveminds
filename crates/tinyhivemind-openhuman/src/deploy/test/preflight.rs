//! Every refused native selector fails before runtime construction.
use super::support::*;
use crate::{
    config::*,
    deploy::{BuildOptions, runtime},
};
#[test]
fn refuses_unlowerable_ports_and_native_settings() -> anyhow::Result<()> {
    let mut cases: Vec<HiveConfig> = Vec::new();
    let mut config = manifest()?;
    config.runtime.memory_engine = "host".into();
    cases.push(config);
    let mut config = manifest()?;
    config.runtime.session_store = "sqlite".into();
    cases.push(config);
    let mut config = manifest()?;
    config.runtime.provider.kind = "anthropic".into();
    cases.push(config);
    let mut config = manifest()?;
    config.runtime.skills = vec!["a".into(), "b".into()];
    cases.push(config);
    let mut config = manifest()?;
    config.runtime.skills = vec!["a".into()];
    config.profiles[0].skills = vec!["b".into()];
    cases.push(config);
    let mut config = manifest()?;
    config.runtime.limits.budget_usd = Some(1.0);
    cases.push(config);
    let mut config = manifest()?;
    config.hives[0].coordinator.retention.settled_episodes = Some(0);
    cases.push(config);
    let mut config = manifest()?;
    config.runtime.provider.credential = Some(SecretRef::Env("KEY".into()));
    cases.push(config);
    let server = McpServer {
        id: "remote".into(),
        transport: "sse".into(),
        endpoint: "https://host".into(),
        args: Vec::new(),
        env: std::collections::BTreeMap::default(),
        headers: std::collections::BTreeMap::default(),
        tools: None,
        deny_tools: Vec::new(),
    };
    let mut config = manifest()?;
    config.runtime.mcp.push(server.clone());
    cases.push(config);
    let mut config = manifest()?;
    let mut server = server.clone();
    server.transport = "http".into();
    server.tools = Some(Vec::new());
    config.runtime.mcp.push(server.clone());
    cases.push(config);
    let mut config = manifest()?;
    let mut server = server.clone();
    server.transport = "http".into();
    server.args.push("ignored".into());
    config.runtime.mcp.push(server.clone());
    cases.push(config);
    let mut config = manifest()?;
    server.transport = "stdio".into();
    server
        .headers
        .insert("Authorization".into(), SecretRef::Env("KEY".into()));
    config.runtime.mcp.push(server.clone());
    cases.push(config);
    let mut config = manifest()?;
    config.profiles[0].limits.timeout_ms = Some(100);
    config.workflows.push(Workflow {
        id: "cron".into(),
        schedule: "0 8 * * *".into(),
        target: WorkflowTarget::Seat("alice".into()),
        prompt: "task".into(),
        enabled: true,
        retries: 0,
        single_flight: true,
    });
    cases.push(config);
    for config in cases {
        assert!(runtime::preflight(&config, &BuildOptions::default()).is_err());
    }
    let options = BuildOptions {
        call_budget: Some(openhuman_embed::budget::CallBudget {
            input_tokens: 0,
            output_tokens: 1,
            cost_micros: 1,
        }),
        ..Default::default()
    };
    assert!(runtime::preflight(&manifest()?, &options).is_err());
    assert!(runtime::preflight(&manifest()?, &BuildOptions::default()).is_ok());
    Ok(())
}
#[test]
fn child_instance_only_settings_are_explicitly_refused() -> anyhow::Result<()> {
    let mut config = manifest()?;
    config.profiles.push(Profile {
        id: "child".into(),
        memory: Some(Memory {
            root: Some("private".into()),
        }),
        ..Default::default()
    });
    config.profiles[0].subagents.push("child".into());
    assert!(runtime::preflight(&config, &BuildOptions::default()).is_err());
    config.profiles[1].memory = None;
    assert!(runtime::preflight(&config, &BuildOptions::default()).is_ok());
    config.permission_profiles[0].send_destinations = Some(vec!["a".into()]);
    config.profiles[1].permission_profile = Some("narrow".into());
    assert!(runtime::preflight(&config, &BuildOptions::default()).is_err());
    Ok(())
}
#[test]
fn logical_memory_roots_encode_without_collisions_and_reserve_native_agent_depth()
-> anyhow::Result<()> {
    use crate::deploy::native_memory_root;
    for root in [
        "seat:alice",
        "shared",
        "team:shared",
        "é😀",
        &"a".repeat(384),
    ] {
        let native = native_memory_root(root)?;
        assert!(openhuman_core::memory::scope::validate_root(&native).is_ok());
        assert_ne!(native_memory_root(&native).ok(), Some(native));
    }
    assert_ne!(
        native_memory_root("seat:alice")?,
        native_memory_root("seat:bob")?
    );
    assert!(native_memory_root("").is_err());
    assert!(native_memory_root(&"é".repeat(193)).is_err());
    let mut config = manifest()?;
    config.hives[0].memory_root = Some("a".repeat(385));
    assert!(runtime::preflight(&config, &BuildOptions::default()).is_err());
    config.hives[0].memory_root = None;
    config.seats[0].overrides.memory = Some(Memory {
        root: Some("a".repeat(385)),
    });
    assert!(runtime::preflight(&config, &BuildOptions::default()).is_err());
    Ok(())
}
