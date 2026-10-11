//! Native agent definitions and config retain profile inheritance and connections.
use super::support::*;
use crate::{config::*, deploy::*};
use std::{
    collections::BTreeMap,
    sync::{Arc, PoisonError},
};
fn server(id: &str) -> McpServer {
    McpServer {
        id: id.into(),
        transport: "http".into(),
        endpoint: "http://127.0.0.1:9/mcp".into(),
        args: Vec::new(),
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        tools: None,
        deny_tools: Vec::new(),
    }
}
#[test]
fn inherited_runtime_mcp_is_registered_once_after_tool_list_normalization() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (options, _backend, _model) = build_options().await;
        let mut config = manifest()?;
        let mut inherited = server("shared");
        inherited.tools = Some(vec!["z".into(), "a".into(), "a".into()]);
        inherited.deny_tools = vec!["blocked".into(), "blocked".into()];
        config.runtime.mcp.push(inherited);
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        for agent in deployment.seats().values() {
            assert_eq!(
                agent
                    .config()
                    .mcp_client
                    .servers
                    .iter()
                    .filter(|server| server.name == "shared")
                    .count(),
                1,
                "inherited server registered twice"
            );
        }
        Ok(())
    })
}
#[test]
fn host_only_profile_and_readonly_permissions_remove_native_mutating_surfaces() -> anyhow::Result<()>
{
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (options, _backend, _model) = build_options().await;
        let mut config = manifest()?;
        config.profiles[0].tool_scope = ToolScope::HostOnly;
        config.profiles[0].permission_profile = Some("narrow".into());
        config.permission_profiles[0].access = Access::ReadOnly;
        config.permission_profiles[0].sandbox = Sandbox::ReadOnly;
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        let hosted = openhuman_core::agent::host_agents::resolve("alice")
            .ok_or_else(|| anyhow::anyhow!("missing native hosted agent"))?;
        assert!(
            matches!(hosted.definition.tools, openhuman_core::agent::harness::definition::ToolScope::Named(ref names) if names.is_empty())
        );
        assert!(matches!(
            hosted.definition.sandbox_mode,
            openhuman_core::agent::harness::definition::SandboxMode::ReadOnly
        ));
        assert_eq!(hosted.definition.subagents.len(), 0);
        assert!(matches!(
            deployment.seats()["alice"].config().autonomy.level,
            openhuman_core::security::AutonomyLevel::ReadOnly
        ));
        Ok(())
    })
}
#[test]
fn native_definition_retains_bare_prompt_sampling_tools_skills_and_mcp_credentials()
-> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (options, _backend, model) = build_options().await;
        super::turns::mount(&model, super::turns::Script::default()).await;
        let directory = tempfile::tempdir()?;
        let bundle = directory.path().join("fixture-skill");
        std::fs::create_dir_all(&bundle)?;
        std::fs::write(
            bundle.join("SKILL.md"),
            "---\nname: fixture-skill\ndescription: fixture\n---\nRead carefully\n",
        )?;
        let config = configured_profile(directory.path())?;
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        let agent = &deployment.seats()["alice"];
        assert_eq!(agent.provider().model_id(), Some("seat-model"));
        Box::pin(
            agent
                .turn("Inspect configured sampling")
                .session("profile-proof")
                .send(),
        )
        .await?;
        let requests = model
            .received_requests()
            .await
            .ok_or_else(|| anyhow::anyhow!("provider request recording unavailable"))?;
        let request = requests
            .iter()
            .find(|request| {
                request.method.as_str() == "POST"
                    && request.url.path().ends_with("/chat/completions")
            })
            .ok_or_else(|| anyhow::anyhow!("no native provider dispatch"))?;
        let body: serde_json::Value = serde_json::from_slice(&request.body)?;
        assert_eq!(body["model"], "seat-model");
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["max_tokens"], 111);
        assert_eq!(
            std::fs::read_to_string(agent.skills_dir().join("fixture-skill/SKILL.md"))?,
            std::fs::read_to_string(bundle.join("SKILL.md"))?
        );
        let hosted = openhuman_core::agent::host_agents::resolve("alice")
            .ok_or_else(|| anyhow::anyhow!("missing native hosted agent"))?;
        assert!(!hosted.context.user_skill_roots());
        let definition = hosted.definition;
        assert!(
            matches!(definition.system_prompt, openhuman_core::agent::harness::definition::PromptSource::Verbatim(ref prompt) if prompt == "Seat prompt\nProfile context\nSeat context")
        );
        assert_eq!(definition.temperature, 0.2);
        assert_eq!(definition.max_iterations, 2);
        assert!(
            matches!(definition.tools, openhuman_core::agent::harness::definition::ToolScope::Named(ref names) if names.contains(&"file_read".into()))
        );
        assert!(matches!(
            definition.sandbox_mode,
            openhuman_core::agent::harness::definition::SandboxMode::Sandboxed
        ));
        assert!(definition.tool_rules.is_some());
        let native = &agent.config().mcp_client.servers;
        let local = native
            .iter()
            .find(|server| server.name == "local")
            .ok_or_else(|| anyhow::anyhow!("missing local MCP"))?;
        assert_eq!(local.command, "sh");
        assert_eq!(local.args, ["-c", "exit 0"]);
        assert_eq!(
            local.env.get("FIXTURE_AUTH").map(String::as_str),
            Some("fixture")
        );
        assert_eq!(local.allowed_tools, ["read"]);
        assert_eq!(local.disallowed_tools, ["write"]);
        let remote = native
            .iter()
            .find(|server| server.name == "remote")
            .ok_or_else(|| anyhow::anyhow!("missing remote MCP"))?;
        assert!(
            matches!(&remote.auth, openhuman_embed::McpAuthConfig::Headers { headers } if headers.len()==1 && headers[0].name=="Authorization" && headers[0].value=="fixture")
        );
        Ok(())
    })
}

fn configured_profile(skills_root: &std::path::Path) -> anyhow::Result<HiveConfig> {
    let mut config = manifest()?;
    config
        .contexts
        .insert("profile-context".into(), "Profile context".into());
    config
        .contexts
        .insert("seat-context".into(), "Seat context".into());
    let profile = &mut config.profiles[0];
    profile.definition_base = Some("orchestrator".into());
    profile.tool_scope = ToolScope::Named(vec!["file_read".into()]);
    profile.bare_prompt = true;
    profile.system_prompt = "Profile prompt".into();
    profile.context.push("profile-context".into());
    profile.model = Model {
        name: Some("profile-model".into()),
        temperature: Some(0.6),
        max_tokens: Some(333),
    };
    profile.limits.iterations = Some(7);
    profile
        .skills
        .push(skills_root.to_string_lossy().into_owned());
    profile.include_user_skills = Some(false);
    profile.permission_profile = Some("narrow".into());
    let mut local = server("local");
    local.transport = "stdio".into();
    local.endpoint = "sh".into();
    local.args = vec!["-c".into(), "exit 0".into()];
    local
        .env
        .insert("FIXTURE_AUTH".into(), SecretRef::Store("local-key".into()));
    local.tools = Some(vec!["read".into()]);
    local.deny_tools.push("write".into());
    profile.mcp.push(local);
    let mut remote = server("remote");
    remote.headers.insert(
        "Authorization".into(),
        SecretRef::Store("header-key".into()),
    );
    profile.mcp.push(remote);
    config.permission_profiles[0].sandbox = Sandbox::Sandboxed;
    config.permission_profiles[0].tool_rules = Some(serde_json::from_value(
        serde_json::json!({"rules":[{"effect":"deny","match":{"name":"blocked"}}]}),
    )?);
    let seat = &mut config.seats[0];
    seat.overrides.system_prompt = Some("Seat prompt".into());
    seat.overrides.context.push("seat-context".into());
    seat.overrides.model = Model {
        name: Some("seat-model".into()),
        temperature: Some(0.2),
        max_tokens: Some(111),
    };
    seat.overrides.limits.iterations = Some(2);
    seat.overrides.tool_scope = Some(ToolScope::Named(vec!["file_read".into()]));
    Ok(config)
}
