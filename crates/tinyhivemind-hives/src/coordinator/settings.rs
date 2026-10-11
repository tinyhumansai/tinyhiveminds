//! Per-hive host-neutral configuration captured at message acceptance.
use super::{Coordinator, CoordinatorOptions, identifier, known_agent};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tinyhivemind_core::{embed::RoutingPolicy, runtime::responder::Probability};

/// Completion and routing configuration for one hive.
///
/// Accepted episodes retain a snapshot even when the live hive is reconfigured.
/// Roles survive membership changes so a rejoining agent keeps its role.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct HiveSettings {
    /// Per-hive completion bounds; round width cannot exceed the global cap.
    pub options: CoordinatorOptions,
    /// Native routing thresholds and choice limits. Routing width is intersected
    /// with completion width and cannot exceed the global concurrent-turn cap.
    /// The coordinator currently has no semantic router: thresholds are passed
    /// to native routing, while broadcasts use its single-owner fallback.
    pub routing: RoutingPolicy,
    /// Registered identities and their role in this hive.
    pub roles: BTreeMap<String, String>,
}
impl Default for HiveSettings {
    fn default() -> Self {
        Self {
            options: CoordinatorOptions::default(),
            routing: RoutingPolicy {
                minimum_confidence: Probability::ZERO,
                high_impact_minimum_confidence: Probability::ONE,
                clarification_threshold: Probability::ONE,
                high_impact_threshold: Probability::ONE,
                round_width: 1,
                choice_option_limit: 8,
            },
            roles: BTreeMap::new(),
        }
    }
}
impl Coordinator {
    /// Configure future episodes of an existing hive atomically.
    ///
    /// Global retention and concurrent-turn admission stay coordinator-owned;
    /// per-hive `options.retention` must equal the global retention setting.
    /// # Errors
    /// Returns unknown hive/role agent, invalid bounds or role, or storage errors.
    pub async fn configure_hive(&self, hive_id: &str, settings: HiveSettings) -> Result<()> {
        if settings.options.round_width == 0
            || settings.options.round_width > self.inner.options.round_width
            || settings.routing.round_width == 0
            || settings.routing.round_width > self.inner.options.round_width
            || settings.routing.choice_option_limit == 0
            || settings.options.conduct_policy.turn_wall == 0
            || settings.options.conduct_policy.child_turn_wall == 0
            || settings.options.retention != self.inner.options.retention
        {
            return Err(Error::InvalidOptions);
        }
        self.update(|state| {
            if !state.hives.contains_key(hive_id) {
                return Err(Error::UnknownHive(hive_id.into()));
            }
            for (agent, role) in &settings.roles {
                known_agent(state, agent)?;
                identifier(role, "hive role")?;
            }
            state.hive_settings.insert(hive_id.into(), settings.clone());
            Ok(())
        })
        .await
    }
    /// Read the current explicit configuration; unconfigured hives use global defaults.
    /// # Errors
    /// Returns unknown hive or a poisoned shared lock.
    pub fn hive_settings(&self, hive_id: &str) -> Result<Option<HiveSettings>> {
        let live = self.lock()?;
        if !live.durable.hives.contains_key(hive_id) {
            return Err(Error::UnknownHive(hive_id.into()));
        }
        Ok(live.durable.hive_settings.get(hive_id).cloned())
    }
}
