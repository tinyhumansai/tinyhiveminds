//! Definitions, immutable connections and fixed seat lowering.
use super::{DeployError, DeployResult, SecretResolver, permission::Permissions};
use crate::config::{HiveConfig, Limits, Profile, Seat, ToolScope};
use openhuman_embed::{AgentDefinitionSpec, AgentSpec, MemoryBinding, ToolScopeSpec};
use std::sync::Arc;
pub(super) fn limits(config: &HiveConfig, seat: &Seat) -> DeployResult<Limits> {
    let profile = config.profile_for(seat)?;
    Ok(Limits {
        timeout_ms: seat
            .overrides
            .limits
            .timeout_ms
            .or(profile.limits.timeout_ms)
            .or(config.runtime.limits.timeout_ms),
        iterations: seat
            .overrides
            .limits
            .iterations
            .or(profile.limits.iterations)
            .or(config.runtime.limits.iterations),
        budget_usd: seat
            .overrides
            .limits
            .budget_usd
            .or(profile.limits.budget_usd)
            .or(config.runtime.limits.budget_usd),
    })
}
pub(super) fn scope(value: &ToolScope) -> ToolScopeSpec {
    match value {
        ToolScope::Wildcard => ToolScopeSpec::Wildcard,
        ToolScope::Named(names) => ToolScopeSpec::Named(names.clone()),
        ToolScope::HostOnly => ToolScopeSpec::HostOnly,
    }
}
pub(super) fn prompt(config: &HiveConfig, profile: &Profile, seat: Option<&Seat>) -> String {
    let mut prompt = seat
        .and_then(|seat| seat.overrides.system_prompt.clone())
        .unwrap_or_else(|| profile.system_prompt.clone());
    for id in profile
        .context
        .iter()
        .chain(seat.into_iter().flat_map(|seat| &seat.overrides.context))
    {
        if let Some(context) = config.contexts.get(id) {
            prompt.push('\n');
            prompt.push_str(context);
        }
    }
    prompt
}
pub(super) fn definition(config: &HiveConfig, profile: &Profile) -> AgentDefinitionSpec {
    let mut definition = profile
        .definition_base
        .as_ref()
        .map_or_else(AgentDefinitionSpec::new, |base| {
            AgentDefinitionSpec::from_base(base.clone())
        });
    definition = if profile.bare_prompt {
        definition.bare_prompt(prompt(config, profile, None))
    } else {
        definition.system_prompt(prompt(config, profile, None))
    };
    definition = definition
        .tools(scope(&profile.tool_scope))
        .disallow_tools(profile.deny_tools.clone());
    if let Some(n) = profile.limits.iterations {
        definition = definition.max_iterations(n);
    }
    if let Some(t) = profile.model.temperature {
        definition = definition.temperature(t);
    }
    if let Some(policy) = profile.permission_profile.as_ref().and_then(|id| {
        config
            .permission_profiles
            .iter()
            .find(|policy| &policy.id == id)
    }) {
        definition = definition.sandbox(super::runtime::sandbox(policy.sandbox));
        if let Some(rules) = &policy.tool_rules {
            definition = definition.tool_rules(rules.clone());
        }
    }
    definition
}
pub(super) fn mcp(
    server: &crate::config::McpServer,
    secrets: &dyn SecretResolver,
) -> DeployResult<openhuman_embed::McpServer> {
    let mut native = if server.transport == "stdio" {
        openhuman_embed::McpServer::stdio(&server.id, &server.endpoint, server.args.clone())
    } else {
        openhuman_embed::McpServer::http(&server.id, &server.endpoint)
    };
    let env = server
        .env
        .iter()
        .map(|(name, value)| Ok((name.clone(), secrets.resolve(value)?)))
        .collect::<DeployResult<Vec<_>>>()?;
    native = native.env(env).deny_tools(server.deny_tools.clone());
    if let Some(tools) = &server.tools {
        native = native.allow_tools(tools.clone());
    }
    if !server.headers.is_empty() {
        let headers = server
            .headers
            .iter()
            .map(|(name, value)| {
                Ok(openhuman_embed::HttpHeader {
                    name: name.clone(),
                    value: secrets.resolve(value)?,
                })
            })
            .collect::<DeployResult<Vec<_>>>()?;
        native = native.auth(openhuman_embed::McpAuthConfig::Headers { headers });
    }
    Ok(native)
}
pub(super) fn seat(
    config: &HiveConfig,
    seat: &Seat,
    secrets: &dyn SecretResolver,
    permissions: Arc<Permissions>,
) -> DeployResult<AgentSpec> {
    let profile = config.profile_for(seat)?;
    let limits = limits(config, seat)?;
    let permission_layers = config.permission_layers(seat, None)?;
    let access = permission_layers
        .iter()
        .map(|policy| policy.access)
        .min()
        .unwrap_or(config.runtime.autonomy)
        .min(config.runtime.autonomy);
    let mut definition = AgentDefinitionSpec::new()
        .tools(scope(
            seat.overrides
                .tool_scope
                .as_ref()
                .unwrap_or(&profile.tool_scope),
        ))
        .disallow_tools(seat.overrides.deny_tools.clone());
    if profile.bare_prompt {
        definition = definition.bare_prompt(prompt(config, profile, Some(seat)));
    } else {
        definition = definition.system_prompt(prompt(config, profile, Some(seat)));
    }
    let root = seat
        .overrides
        .memory
        .as_ref()
        .or(profile.memory.as_ref())
        .and_then(|memory| memory.root.clone())
        .unwrap_or_else(|| format!("seat:{}", seat.id));
    let mut model = profile.model.clone();
    model.temperature = seat.overrides.model.temperature.or(model.temperature);
    model.max_tokens = seat.overrides.model.max_tokens.or(model.max_tokens);
    model.name = seat.overrides.model.name.clone().or(model.name);
    if permission_layers
        .iter()
        .any(|policy| policy.sandbox == crate::config::Sandbox::ReadOnly)
    {
        definition = definition.sandbox(openhuman_embed::SandboxModeSpec::ReadOnly);
    } else if permission_layers
        .iter()
        .any(|policy| policy.sandbox == crate::config::Sandbox::Sandboxed)
    {
        definition = definition.sandbox(openhuman_embed::SandboxModeSpec::Sandboxed);
    }
    let mut spec = AgentSpec::new(&seat.id)
        .extends(&seat.profile)
        .definition(definition)
        .access(super::runtime::access(access))
        .memory(MemoryBinding::new(&seat.id).root(super::native_memory_root(&root)?))
        .model_defaults(super::runtime::model(&model, limits.iterations));
    if let Some(name) = &model.name {
        spec = spec.model(name);
    }
    if let Some(include) = profile.include_user_skills {
        spec = spec.include_user_skills(include);
    }
    if let Some(path) = profile.skills.first().or(seat.overrides.skills.first()) {
        spec = spec.skills_dir(path);
    }
    for server in config.normalized_mcp(seat)? {
        if !config
            .runtime
            .mcp
            .iter()
            .any(|baseline| baseline.id == server.id)
        {
            spec = spec.mcp(mcp(&server, secrets)?);
        }
    }
    let subagents = seat
        .overrides
        .subagents
        .as_ref()
        .unwrap_or(&profile.subagents);
    let mut definitions = Vec::new();
    for id in subagents {
        let child = config
            .profiles
            .iter()
            .find(|profile| &profile.id == id)
            .ok_or_else(|| {
                DeployError::Config(crate::config::ConfigError::UnknownProfile(id.clone()))
            })?;
        definitions.push((id.clone(), self::definition(config, child)));
    }
    spec = spec.subagents(definitions);
    let id = seat.id.clone();
    Ok(spec.can_use_tool(move |context| {
        let permissions = permissions.clone();
        let id = id.clone();
        Box::pin(async move { permissions.decide(&id, None, context).await })
    }))
}
