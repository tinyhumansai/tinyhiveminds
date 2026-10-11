//! Reject native representations that cannot retain manifest semantics.
use super::{
    BuildOptions, DeployResult,
    runtime::{budget_micros, unsupported},
};
use crate::config::{HiveConfig, WorkflowTarget};
pub(super) fn preflight(config: &HiveConfig, options: &BuildOptions) -> DeployResult<()> {
    ports(config, options)?;
    seats(config, options)?;
    children(config)?;
    mcp(config)?;
    for hive in &config.hives {
        if let Some(root) = &hive.memory_root {
            super::native_memory_root(root)?;
        }
        if hive.coordinator.retention != options.retention {
            return Err(unsupported("hive.coordinator.retention is global"));
        }
    }
    Ok(())
}
fn ports(config: &HiveConfig, options: &BuildOptions) -> DeployResult<()> {
    let runtime = &config.runtime;
    if options.provider.is_some()
        && (runtime.provider.endpoint.is_some() || runtime.provider.credential.is_some())
    {
        return Err(unsupported(
            "runtime.provider conflicts with injected provider",
        ));
    }
    if runtime.provider.credential.is_some()
        && runtime.provider.endpoint.is_none()
        && runtime.provider.kind.is_empty()
    {
        return Err(unsupported(
            "runtime.provider credential requires endpoint or kind",
        ));
    }
    if options.call_budget.is_some_and(|call| {
        call.input_tokens == 0 || call.output_tokens == 0 || call.cost_micros == 0
    }) {
        return Err(unsupported("call_budget bounds must be positive"));
    }
    if runtime.session_store == "host" && options.session_store.is_none() {
        return Err(unsupported("runtime.session_store"));
    }
    if runtime.session_store == "sqlite" && runtime.workspace.is_none() {
        return Err(unsupported(
            "runtime.session_store sqlite requires workspace",
        ));
    }
    if runtime.memory_engine == "host" && options.memory_engine.is_none() {
        return Err(unsupported("runtime.memory_engine"));
    }
    if runtime.provider.kind == "anthropic" && options.provider.is_none() {
        return Err(unsupported(
            "runtime.provider anthropic requires host provider",
        ));
    }
    if runtime.skills.len() > 1 {
        return Err(unsupported("runtime.skills supports one bundle root"));
    }
    Ok(())
}
fn seats(config: &HiveConfig, options: &BuildOptions) -> DeployResult<()> {
    for seat in &config.seats {
        let profile = config.profile_for(seat)?;
        let root = seat
            .overrides
            .memory
            .as_ref()
            .or(profile.memory.as_ref())
            .and_then(|memory| memory.root.clone())
            .unwrap_or_else(|| format!("seat:{}", seat.id));
        super::native_memory_root(&root)?;
        let roots: Vec<_> = config
            .runtime
            .skills
            .iter()
            .chain(&profile.skills)
            .chain(&seat.overrides.skills)
            .collect();
        if roots.len() > 1 {
            return Err(unsupported("seat.skills supports one bundle root"));
        }
        let limits = super::profile::limits(config, seat)?;
        if limits
            .budget_usd
            .is_some_and(|usd| budget_micros(usd).is_none())
        {
            return Err(unsupported(
                "limits.budget_usd exceeds native cost precision",
            ));
        }
        if limits.budget_usd.is_some() && options.call_budget.is_none() {
            return Err(unsupported("limits.budget_usd requires call_budget"));
        }
        if config
            .workflows
            .iter()
            .any(|job| matches!(&job.target, WorkflowTarget::Seat(id) if id == &seat.id))
            && (limits.timeout_ms.is_some() || limits.budget_usd.is_some())
        {
            return Err(unsupported(
                "native seat cron timeout/budget requires native baseline support",
            ));
        }
    }
    Ok(())
}
fn children(config: &HiveConfig) -> DeployResult<()> {
    for children in config
        .profiles
        .iter()
        .map(|profile| &profile.subagents)
        .chain(
            config
                .seats
                .iter()
                .filter_map(|seat| seat.overrides.subagents.as_ref()),
        )
    {
        for child in children {
            let child = config
                .profiles
                .iter()
                .find(|profile| &profile.id == child)
                .ok_or_else(|| crate::config::ConfigError::UnknownProfile(child.clone()))?;
            if !child.subagents.is_empty()
                || !child.mcp.is_empty()
                || !child.skills.is_empty()
                || child.memory.is_some()
                || child.model.name.is_some()
                || child.model.max_tokens.is_some()
                || child.limits.timeout_ms.is_some()
                || child.limits.budget_usd.is_some()
                || child.include_user_skills.is_some()
            {
                return Err(unsupported(
                    "subagent profile requires native instance settings",
                ));
            }
            if child
                .permission_profile
                .as_ref()
                .and_then(|id| {
                    config
                        .permission_profiles
                        .iter()
                        .find(|policy| &policy.id == id)
                })
                .is_some_and(|policy| {
                    policy.send_operations.is_some() || policy.send_destinations.is_some()
                })
            {
                return Err(unsupported(
                    "subagent send policy requires an authenticated child host belt",
                ));
            }
        }
    }
    Ok(())
}
fn mcp(config: &HiveConfig) -> DeployResult<()> {
    for server in config
        .runtime
        .mcp
        .iter()
        .chain(config.profiles.iter().flat_map(|profile| &profile.mcp))
        .chain(config.seats.iter().flat_map(|seat| &seat.overrides.mcp))
    {
        if server.transport == "sse" {
            return Err(unsupported(
                "mcp.sse requires a native transport constructor",
            ));
        }
        if server.tools.as_ref().is_some_and(Vec::is_empty) {
            return Err(unsupported("mcp empty allowlist means all in native API"));
        }
        if server.transport != "stdio" && (!server.env.is_empty() || !server.args.is_empty()) {
            return Err(unsupported("mcp network env/args"));
        }
        if server.transport == "stdio" && !server.headers.is_empty() {
            return Err(unsupported("mcp stdio headers"));
        }
    }
    Ok(())
}
