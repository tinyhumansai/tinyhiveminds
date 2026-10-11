//! Dynamic agents use the same typed catalog and lowering path as fixed seats.
use super::{DeployError, DeployResult, Permissions, SecretResolver, profile, runtime};
use crate::{
    AgentFactory, AgentFuture,
    config::{Seat, SeatOverrides},
};
use std::sync::{Arc, PoisonError};
/// Instantiates typed manifest profiles on the deployment's existing runtime.
#[derive(Clone)]
pub struct ManifestFactory {
    runtime: Arc<openhuman_embed::Runtime>,
    pub(super) permissions: Arc<Permissions>,
    secrets: Arc<dyn SecretResolver>,
    options: super::BuildOptions,
    creation: Arc<tokio::sync::Mutex<()>>,
}
impl std::fmt::Debug for ManifestFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManifestFactory")
            .field("runtime_id", &self.runtime.runtime_id())
            .finish_non_exhaustive()
    }
}
impl ManifestFactory {
    pub(super) fn new(
        runtime: Arc<openhuman_embed::Runtime>,
        permissions: Arc<Permissions>,
        secrets: Arc<dyn SecretResolver>,
        options: super::BuildOptions,
    ) -> Self {
        Self {
            runtime,
            permissions,
            secrets,
            options,
            creation: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
    /// Build one new seat; callers then register it with their host.
    /// # Errors
    /// Unknown profiles, widening, credential literals or native creation failure.
    pub async fn create_seat(
        &self,
        id: impl Into<String>,
        template: impl Into<String>,
        overrides: SeatOverrides,
    ) -> DeployResult<openhuman_embed::Agent> {
        let _creation = self.creation.lock().await;
        let mut config = self.permissions.config();
        let seat = Seat {
            id: id.into(),
            profile: template.into(),
            overrides,
        };
        config.seats.push(seat.clone());
        let config = config.validate()?.into_config();
        let limits = profile::limits(&config, &seat)?;
        if limits.budget_usd.is_some() && self.options.call_budget.is_none() {
            return Err(runtime::unsupported(
                "limits.budget_usd requires call_budget",
            ));
        }
        runtime::preflight(&config, &self.options)?;
        let spec = profile::seat(
            &config,
            &seat,
            self.secrets.as_ref(),
            self.permissions.clone(),
        )?;
        let agent = self
            .runtime
            .agent(spec)
            .map_err(|_| DeployError::Agent(seat.id.clone()))?;
        *self
            .permissions
            .config
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = config;
        Ok(agent)
    }
}
impl AgentFactory for ManifestFactory {
    fn create(&self, template: String, value: serde_json::Value) -> AgentFuture {
        let factory = self.clone();
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Request {
                id: String,
                #[serde(default)]
                overrides: SeatOverrides,
            }
            let request: Request = serde_json::from_value(value)
                .map_err(|_| crate::Error::Unauthorized("invalid typed agent settings".into()))?;
            factory
                .create_seat(request.id, template, request.overrides)
                .await
                .map_err(|_| crate::Error::Unauthorized("manifest agent creation refused".into()))
        })
    }
}
pub(super) struct DenyManagement;
impl crate::ManagementAuthorizer for DenyManagement {
    fn authorize(&self, _: &str, _: &crate::ManagementRequest) -> crate::Result<()> {
        Err(crate::Error::Unauthorized(
            "management requires a host authorizer".into(),
        ))
    }
}
