//! Native runtime construction and early lowering compatibility checks.
use super::{BuildOptions, DeployError, DeployResult, SecretResolver};
use crate::config::{Access, HiveConfig, Model, Sandbox};
use openhuman_embed::{Runtime, RuntimeBuilder, RuntimeModule, SkillsPolicy, Workspace};
use std::sync::Arc;
pub(super) fn unsupported(setting: &str) -> DeployError {
    crate::config::ConfigError::UnsupportedSetting(setting.into()).into()
}
pub(super) use super::compatibility::preflight;
pub(super) fn access(value: Access) -> openhuman_embed::Access {
    match value {
        Access::ReadOnly => openhuman_embed::Access::readonly(),
        Access::Supervised => openhuman_embed::Access::supervised(),
        Access::Autonomous => openhuman_embed::Access::full(),
    }
}
pub(super) fn sandbox(value: Sandbox) -> openhuman_embed::SandboxModeSpec {
    match value {
        Sandbox::None => openhuman_embed::SandboxModeSpec::None,
        Sandbox::ReadOnly => openhuman_embed::SandboxModeSpec::ReadOnly,
        Sandbox::Sandboxed => openhuman_embed::SandboxModeSpec::Sandboxed,
    }
}
pub(super) fn model(value: &Model, iterations: Option<usize>) -> openhuman_embed::ModelDefaults {
    openhuman_embed::ModelDefaults {
        temperature: value.temperature,
        max_tokens: value.max_tokens,
        max_iterations: iterations,
        ..Default::default()
    }
}
pub(super) async fn build(
    config: &HiveConfig,
    secrets: &dyn SecretResolver,
    options: &BuildOptions,
) -> DeployResult<Runtime> {
    let section = &config.runtime;
    let mut native = options.config.clone().unwrap_or_default();
    native.agent.tool_dispatcher = "auto".into();
    native.scheduler.max_concurrent = section.concurrency;
    native.memory.recall.enabled = section.memory_engine != "disabled";
    native.memory.conversations.enabled = section.memory_engine != "disabled";
    let mut builder = RuntimeBuilder::new()
        .config(native)
        .modules([
            RuntimeModule::Agent,
            RuntimeModule::Inference,
            RuntimeModule::Memory,
            RuntimeModule::Mcp,
            RuntimeModule::Skills,
            RuntimeModule::Automation,
        ])
        .services(openhuman_embed::ServiceSet::none())
        .access(access(section.autonomy))
        .sandbox(sandbox(section.sandbox))
        .model_defaults(model(&section.model, section.limits.iterations))
        .skills(SkillsPolicy {
            root: section.skills.first().map(Into::into),
            include_user_skills: section.include_user_skills,
        });
    builder = match &section.workspace {
        Some(path) => builder.workspace_dir(path),
        None => builder.workspace(Workspace::Ephemeral),
    };
    if let Some(url) = &options.backend_url {
        builder = builder.backend_url(url);
    }
    builder = builder.max_agents(section.max_agents.unwrap_or(usize::MAX));
    if let Some(rules) = &section.tool_rules {
        builder = builder.tool_rules(rules.clone());
    }
    builder = builder.provider(provider(section, secrets, options)?);
    match section.session_store.as_str() {
        "memory" => {
            builder =
                builder.session_store(Arc::new(openhuman_embed::InMemorySessionStores::new()));
        }
        "host" => {
            if let Some(port) = &options.session_store {
                builder = builder.session_store(port.clone());
            }
        }
        "sqlite" => {
            let path = section
                .workspace
                .as_ref()
                .ok_or_else(|| unsupported("runtime.workspace"))?;
            std::fs::create_dir_all(path).map_err(|_| DeployError::Runtime)?;
            builder = builder.storage(format!("sqlite://{path}/sessions.sqlite"));
        }
        _ => return Err(unsupported("runtime.session_store")),
    }
    match section.memory_engine.as_str() {
        "disabled" => {}
        "memory" => {
            builder = builder.memory_engine(Arc::new(
                tinymemory_api::conformance::reference::ReferenceEngine::new(),
            ));
        }
        "host" => {
            if let Some(port) = &options.memory_engine {
                builder = builder.memory_engine(port.clone());
            }
        }
        _ => return Err(unsupported("runtime.memory_engine")),
    }
    let mut servers = Vec::new();
    for server in &section.mcp {
        servers.push(super::profile::mcp(server, secrets)?);
    }
    Box::pin(builder.mcp_baseline(servers).build())
        .await
        .map_err(|_| DeployError::Runtime)
}

pub(super) fn budget_micros(usd: f64) -> Option<u64> {
    format!("{:.0}", (usd * 1_000_000.0).floor()).parse().ok()
}

fn provider(
    section: &crate::config::RuntimeSection,
    secrets: &dyn SecretResolver,
    options: &BuildOptions,
) -> DeployResult<openhuman_embed::Provider> {
    let mut provider = if let Some(provider) = &options.provider {
        provider.clone()
    } else if let Some(endpoint) =
        section
            .provider
            .endpoint
            .as_deref()
            .or(match section.provider.kind.as_str() {
                "openrouter" => Some("https://openrouter.ai/api/v1"),
                "openai" => Some("https://api.openai.com/v1"),
                _ => None,
            })
    {
        let key = section
            .provider
            .credential
            .as_ref()
            .map(|reference| secrets.resolve(reference))
            .transpose()?
            .unwrap_or_default();
        openhuman_embed::Provider::openai_compatible(endpoint, key)
    } else {
        openhuman_embed::Provider::inherit()
    };
    if let Some(name) = &section.model.name {
        provider = provider.model(name);
    }
    Ok(provider)
}
#[cfg(test)]
#[path = "test/runtime.rs"]
mod test;
