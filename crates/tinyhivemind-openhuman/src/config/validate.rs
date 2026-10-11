//! Referential validation and conservative authority intersection.
use super::{
    ConfigError, HiveConfig, Limits, McpServer, Membership, Model, PermissionProfile, Profile,
    Result, Sandbox, Seat, ToolScope, ValidatedHiveConfig, WorkflowTarget,
};
use std::collections::BTreeSet;

impl HiveConfig {
    /// Validate references and permissions without constructing any runtime.
    ///
    /// # Errors
    /// Returns the first typed identity, reference, limit or authority failure.
    pub fn validate(self) -> Result<ValidatedHiveConfig> {
        super::parse::reject_inline(
            &serde_json::to_value(&self).map_err(|_| ConfigError::Json)?,
            "config",
        )?;
        unique("profile", self.profiles.iter().map(|v| v.id.as_str()))?;
        unique(
            "permission",
            self.permission_profiles.iter().map(|v| v.id.as_str()),
        )?;
        unique("seat", self.seats.iter().map(|v| v.id.as_str()))?;
        unique("hive", self.hives.iter().map(|v| v.id.as_str()))?;
        unique("workflow", self.workflows.iter().map(|v| v.id.as_str()))?;
        self.check_references()?;
        self.check_settings()?;
        self.check_authority()?;
        Ok(ValidatedHiveConfig(self))
    }
    /// Find the reusable profile for a seat.
    ///
    /// # Errors
    /// Returns `UnknownProfile` if the seat references an absent template.
    pub fn profile_for(&self, seat: &Seat) -> Result<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.id == seat.profile)
            .ok_or_else(|| ConfigError::UnknownProfile(seat.profile.clone()))
    }
    /// Runtime/profile/seat/membership authority layers, in precedence order.
    ///
    /// # Errors
    /// Returns unknown-profile or unknown-permission-reference errors.
    pub fn permission_layers(
        &self,
        seat: &Seat,
        member: Option<&Membership>,
    ) -> Result<Vec<&PermissionProfile>> {
        let profile = self.profile_for(seat)?;
        let mut layers = vec![];
        for reference in [
            self.runtime.permission_profile.as_ref(),
            profile.permission_profile.as_ref(),
            seat.overrides.permission_profile.as_ref(),
            member.and_then(|m| m.permission_profile.as_ref()),
        ]
        .into_iter()
        .flatten()
        {
            layers.push(
                self.permission_profiles
                    .iter()
                    .find(|p| &p.id == reference)
                    .ok_or_else(|| ConfigError::UnknownPermissionProfile(reference.clone()))?,
            );
        }
        Ok(layers)
    }
    /// One normalized immutable runtime/profile/seat connection set.
    ///
    /// # Errors
    /// Returns `UnknownProfile` or `ConflictingMcp` for conflicting named servers.
    pub fn normalized_mcp(&self, seat: &Seat) -> Result<Vec<McpServer>> {
        let profile = self.profile_for(seat)?;
        let mut servers = std::collections::BTreeMap::new();
        for server in self
            .runtime
            .mcp
            .iter()
            .chain(&profile.mcp)
            .chain(&seat.overrides.mcp)
        {
            let server = normalize_server(server.clone());
            if servers.get(&server.id).is_some_and(|old| old != &server) {
                return Err(ConfigError::ConflictingMcp(seat.id.clone()));
            }
            servers.insert(server.id.clone(), server);
        }
        Ok(servers.into_values().collect())
    }
    fn check_references(&self) -> Result<()> {
        for reference in self
            .runtime
            .permission_profile
            .iter()
            .chain(
                self.profiles
                    .iter()
                    .filter_map(|p| p.permission_profile.as_ref()),
            )
            .chain(
                self.seats
                    .iter()
                    .filter_map(|s| s.overrides.permission_profile.as_ref()),
            )
            .chain(
                self.hives
                    .iter()
                    .flat_map(|h| &h.members)
                    .filter_map(|m| m.permission_profile.as_ref()),
            )
        {
            if !self.permission_profiles.iter().any(|p| &p.id == reference) {
                return Err(ConfigError::UnknownPermissionProfile(reference.clone()));
            }
        }
        for profile in &self.profiles {
            self.context_refs(&profile.context)?;
            for child in &profile.subagents {
                self.profile_ref(child)?;
            }
        }
        for seat in &self.seats {
            self.profile_for(seat)?;
            self.context_refs(&seat.overrides.context)?;
            for child in seat.overrides.subagents.iter().flatten() {
                self.profile_ref(child)?;
            }
        }
        for hive in &self.hives {
            unique("member", hive.members.iter().map(|m| m.seat.as_str()))?;
            for member in &hive.members {
                self.seat_ref(&member.seat)?;
                self.context_refs(&member.context)?;
            }
        }
        for job in &self.workflows {
            match &job.target {
                WorkflowTarget::Seat(id) => {
                    self.seat_ref(id)?;
                }
                WorkflowTarget::Hive(id) if !self.hives.iter().any(|h| &h.id == id) => {
                    return Err(ConfigError::UnknownHive(id.clone()));
                }
                WorkflowTarget::Hive(_) => {}
            }
        }
        Ok(())
    }
    fn profile_ref(&self, id: &str) -> Result<()> {
        if self.profiles.iter().any(|p| p.id == id) {
            Ok(())
        } else {
            Err(ConfigError::UnknownProfile(id.into()))
        }
    }
    fn seat_ref(&self, id: &str) -> Result<&Seat> {
        self.seats
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| ConfigError::UnknownSeat(id.into()))
    }
    fn context_refs(&self, refs: &[String]) -> Result<()> {
        for id in refs {
            if !self.contexts.contains_key(id) {
                return Err(ConfigError::UnknownContext(id.clone()));
            }
        }
        Ok(())
    }
    fn check_settings(&self) -> Result<()> {
        let runtime = &self.runtime;
        if runtime.concurrency == 0 {
            return Err(ConfigError::InvalidWidth("runtime".into()));
        }
        if runtime
            .max_agents
            .is_some_and(|max| max == 0 || self.seats.len() > max)
        {
            return Err(ConfigError::InvalidLimit("runtime.max_agents".into()));
        }
        for (field, value, allowed) in [
            (
                "provider",
                runtime.provider.kind.as_str(),
                &["", "openai", "openrouter", "anthropic"][..],
            ),
            (
                "session_store",
                runtime.session_store.as_str(),
                &["memory", "sqlite", "host"][..],
            ),
            (
                "memory_engine",
                runtime.memory_engine.as_str(),
                &["disabled", "memory", "host"][..],
            ),
        ] {
            if !allowed.contains(&value) {
                return Err(ConfigError::UnsupportedSetting(field.into()));
            }
        }
        limits(&runtime.limits, "runtime")?;
        model(&runtime.model, "runtime")?;
        for profile in &self.profiles {
            limits(&profile.limits, &profile.id)?;
            model(&profile.model, &profile.id)?;
        }
        for seat in &self.seats {
            let bytes = seat.id.as_bytes();
            if bytes.len() > 64
                || !bytes
                    .first()
                    .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                || bytes.iter().any(|b| {
                    !b.is_ascii_lowercase() && !b.is_ascii_digit() && !matches!(b, b'_' | b'-')
                })
            {
                return Err(ConfigError::InvalidId(seat.id.clone()));
            }
            limits(&seat.overrides.limits, &seat.id)?;
            model(&seat.overrides.model, &seat.id)?;
        }
        for hive in &self.hives {
            if hive.members.is_empty() {
                return Err(ConfigError::EmptyHive(hive.id.clone()));
            }
            for member in &hive.members {
                if member.role.trim().is_empty() {
                    return Err(ConfigError::InvalidRole(member.seat.clone()));
                }
            }
            let cap = runtime.concurrency;
            if hive.coordinator.round_width == 0
                || hive.coordinator.round_width > cap
                || hive.routing.round_width == 0
                || hive.routing.round_width > cap
                || hive.episode.round_width == 0
                || hive.episode.revealed_width == 0
                || hive.episode.round_width as usize > cap
                || hive.episode.revealed_width as usize > cap
                || hive.division.round_width == 0
                || hive.division.round_width as usize > cap
            {
                return Err(ConfigError::InvalidWidth(hive.id.clone()));
            }
            if hive.conduct.turn_wall == 0
                || hive.conduct.child_turn_wall == 0
                || hive.coordinator.conduct_policy.turn_wall == 0
                || hive.coordinator.conduct_policy.child_turn_wall == 0
                || hive.episode.turn_budget == 0
            {
                return Err(ConfigError::InvalidLimit(hive.id.clone()));
            }
        }
        self.check_workflows_and_mcp()?;
        Ok(())
    }
    fn check_workflows_and_mcp(&self) -> Result<()> {
        for job in &self.workflows {
            if job.prompt.trim().is_empty()
                || job.schedule.split_whitespace().count() != 5
                || openhuman_core::cron::validate_schedule(
                    &openhuman_core::cron::Schedule::Cron {
                        expr: job.schedule.clone(),
                        tz: None,
                        active_hours: None,
                    },
                    chrono::DateTime::from_timestamp(0, 0).unwrap_or_default(),
                )
                .is_err()
            {
                return Err(ConfigError::InvalidWorkflow(job.id.clone()));
            }
        }
        for seat in &self.seats {
            for server in self.normalized_mcp(seat)? {
                if server.id.trim().is_empty()
                    || server.endpoint.trim().is_empty()
                    || !matches!(server.transport.as_str(), "stdio" | "sse" | "http")
                {
                    return Err(ConfigError::UnsupportedSetting(
                        "mcp transport or endpoint".into(),
                    ));
                }
            }
        }
        Ok(())
    }
    fn check_authority(&self) -> Result<()> {
        let baseline = PermissionProfile {
            access: self.runtime.autonomy,
            sandbox: self.runtime.sandbox,
            tool_rules: self.runtime.tool_rules.clone(),
            ..PermissionProfile::default()
        };
        for seat in &self.seats {
            let profile = self.profile_for(seat)?;
            let scope = seat
                .overrides
                .tool_scope
                .as_ref()
                .unwrap_or(&profile.tool_scope);
            if scope == &ToolScope::HostOnly && !self.normalized_mcp(seat)?.is_empty() {
                return Err(ConfigError::UnsupportedSetting("host_only mcp".into()));
            }
            limits_narrow(&self.runtime.limits, &profile.limits, &seat.id)?;
            let effective_limits = Limits {
                timeout_ms: profile.limits.timeout_ms.or(self.runtime.limits.timeout_ms),
                iterations: profile.limits.iterations.or(self.runtime.limits.iterations),
                budget_usd: profile.limits.budget_usd.or(self.runtime.limits.budget_usd),
            };
            limits_narrow(&effective_limits, &seat.overrides.limits, &seat.id)?;
            let layers = self.permission_layers(seat, None)?;
            for (index, layer) in layers.iter().enumerate() {
                narrows(&baseline, layer, &seat.id)?;
                for ancestor in &layers[..index] {
                    narrows(ancestor, layer, &seat.id)?;
                }
            }
            if let Some(scope) = &seat.overrides.tool_scope
                && !scope_narrows(&profile.tool_scope, scope)
            {
                return Err(ConfigError::PermissionWidening(seat.id.clone()));
            }
            if let Some(children) = &seat.overrides.subagents
                && children.iter().any(|id| !profile.subagents.contains(id))
            {
                return Err(ConfigError::PermissionWidening(seat.id.clone()));
            }
            for hive in &self.hives {
                for member in hive.members.iter().filter(|m| m.seat == seat.id) {
                    if let Some(id) = &member.permission_profile {
                        let child = self
                            .permission_profiles
                            .iter()
                            .find(|p| &p.id == id)
                            .ok_or_else(|| ConfigError::UnknownPermissionProfile(id.clone()))?;
                        narrows(&baseline, child, &seat.id)?;
                        for ancestor in &layers {
                            narrows(ancestor, child, &seat.id)?;
                        }
                    }
                    if let Some(mcp) = &member.mcp {
                        let mut requested: Vec<_> =
                            mcp.iter().cloned().map(normalize_server).collect();
                        requested.sort();
                        requested.dedup();
                        let mut expected = self.normalized_mcp(seat)?;
                        expected.sort();
                        if requested != expected {
                            return Err(ConfigError::ConflictingMcp(seat.id.clone()));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
fn unique<'a>(section: &str, ids: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.trim().is_empty() {
            return Err(ConfigError::InvalidId(section.into()));
        }
        if !seen.insert(id) {
            return Err(ConfigError::DuplicateId {
                section: section.into(),
                id: id.into(),
            });
        }
    }
    Ok(())
}
fn limits(limits: &Limits, id: &str) -> Result<()> {
    if limits.timeout_ms == Some(0)
        || limits.iterations == Some(0)
        || limits
            .budget_usd
            .is_some_and(|b| !b.is_finite() || b <= 0.0)
    {
        return Err(ConfigError::InvalidLimit(id.into()));
    }
    Ok(())
}
fn model(model: &Model, id: &str) -> Result<()> {
    if model.name.as_ref().is_some_and(|n| n.trim().is_empty())
        || model.max_tokens == Some(0)
        || model
            .temperature
            .is_some_and(|t| !t.is_finite() || !(0.0..=2.0).contains(&t))
    {
        return Err(ConfigError::InvalidLimit(id.into()));
    }
    Ok(())
}
fn subset(parent: Option<&Vec<String>>, child: Option<&Vec<String>>) -> bool {
    match (parent, child) {
        (Some(p), Some(c)) => c.iter().all(|id| p.contains(id)),
        (None, _) | (Some(_), None) => true,
    }
}
fn narrows(parent: &PermissionProfile, child: &PermissionProfile, id: &str) -> Result<()> {
    let sandbox_ok = parent.sandbox == Sandbox::None
        || child.sandbox == parent.sandbox
        || child.sandbox == Sandbox::ReadOnly;
    let approval_ok = match (&parent.approval, &child.approval) {
        (None, _) | (Some(_), None) => true,
        (Some(p), Some(c)) => {
            p == c
                || (!c.enabled && p.approver == c.approver && p.rules == c.rules && !c.allow_grants)
        }
    };
    let rules_ok = match (&parent.tool_rules, &child.tool_rules) {
        (Some(p), Some(c)) => p == c,
        _ => true,
    };
    if child.access > parent.access
        || !sandbox_ok
        || !subset(parent.allow_tools.as_ref(), child.allow_tools.as_ref())
        || parent
            .deny_tools
            .iter()
            .any(|t| !child.deny_tools.contains(t))
        || !subset(
            parent.send_destinations.as_ref(),
            child.send_destinations.as_ref(),
        )
        || !subset(
            parent.send_operations.as_ref(),
            child.send_operations.as_ref(),
        )
        || !approval_ok
        || !rules_ok
    {
        return Err(ConfigError::PermissionWidening(id.into()));
    }
    Ok(())
}
fn scope_narrows(parent: &ToolScope, child: &ToolScope) -> bool {
    match (parent, child) {
        (ToolScope::HostOnly, ToolScope::HostOnly) | (ToolScope::Wildcard, _) => true,
        (ToolScope::Named(p), ToolScope::Named(c)) => c.iter().all(|n| p.contains(n)),
        _ => false,
    }
}
fn normalize_server(mut server: McpServer) -> McpServer {
    if let Some(tools) = &mut server.tools {
        tools.sort();
        tools.dedup();
    }
    server.deny_tools.sort();
    server.deny_tools.dedup();
    server
}

fn limits_narrow(parent: &Limits, child: &Limits, id: &str) -> Result<()> {
    if matches!((parent.timeout_ms, child.timeout_ms), (Some(p), Some(c)) if c > p)
        || matches!((parent.iterations, child.iterations), (Some(p), Some(c)) if c > p)
        || matches!((parent.budget_usd, child.budget_usd), (Some(p), Some(c)) if c > p)
    {
        return Err(ConfigError::PermissionWidening(id.into()));
    }
    Ok(())
}
