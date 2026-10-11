//! Assemble validated manifests on one `OpenHuman` runtime, without background work during construction.
mod compatibility;
mod factory;
mod hooks;
mod memory_root;
mod permission;
mod profile;
mod rules;
mod runtime;
mod types;
mod workflow;
use crate::{OpenHumanHost, config::ValidatedHiveConfig};
pub use factory::ManifestFactory;
pub use memory_root::native_memory_root;
use permission::Permissions;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
};
use tinyhivemind_hives::{Coordinator, CoordinatorOptions, HiveInfo, HiveSettings, MemoryStorage};
pub use types::*;
/// One runtime, registered seat handles and host-owned coordinator.
/// Native seat cron creates isolated sessions; coordinator turns continue each seat's bound session.
pub struct HiveDeployment {
    runtime: Arc<openhuman_embed::Runtime>,
    host: OpenHumanHost,
    seats: Seats,
    config: crate::config::HiveConfig,
    policies: BTreeMap<String, HivePolicyView>,
    permissions: Arc<Permissions>,
    factory: ManifestFactory,
    scheduler: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl std::fmt::Debug for HiveDeployment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HiveDeployment")
            .field("runtime_id", &self.runtime.runtime_id())
            .field("seats", &self.seats.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}
impl HiveDeployment {
    /// Build using in-process stores and no optional host ports.
    /// # Errors
    /// Unsupported settings, missing credentials or refused runtime/coordinator operations.
    pub async fn build(
        config: ValidatedHiveConfig,
        secrets: Arc<dyn SecretResolver>,
    ) -> DeployResult<Self> {
        Box::pin(Self::build_with(config, secrets, BuildOptions::default())).await
    }
    /// Lower the manifest with injected native host ports, validating compatibility before boot.
    /// # Errors
    /// Unsupported settings, missing credentials or refused runtime/coordinator operations.
    pub async fn build_with(
        validated: ValidatedHiveConfig,
        secrets: Arc<dyn SecretResolver>,
        options: BuildOptions,
    ) -> DeployResult<Self> {
        let config = validated.into_config();
        runtime::preflight(&config, &options)?;
        let runtime =
            Arc::new(Box::pin(runtime::build(&config, secrets.as_ref(), &options)).await?);
        for profile in &config.profiles {
            runtime
                .define_template(&profile.id, profile::definition(&config, profile))
                .map_err(|_| DeployError::Agent(profile.id.clone()))?;
        }
        let coordinator = Coordinator::new(
            runtime.runtime_id().into(),
            options
                .coordinator_storage
                .clone()
                .unwrap_or_else(|| Arc::new(MemoryStorage::new())),
            CoordinatorOptions {
                round_width: config.runtime.concurrency,
                retention: options.retention,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| DeployError::Coordinator)?;
        let permissions = Arc::new(Permissions::new(config.clone(), &options));
        let hooks = Arc::new(hooks::Hooks::new(permissions.clone(), &options));
        let factory = ManifestFactory::new(
            runtime.clone(),
            permissions.clone(),
            secrets.clone(),
            options.clone(),
        );
        let mut host = OpenHumanHost::new(runtime.runtime_id().into(), coordinator)
            .map_err(|_| DeployError::Coordinator)?
            .with_hooks(hooks.clone())
            .map_err(|_| DeployError::Coordinator)?
            .with_send_policy(hooks)
            .map_err(|_| DeployError::Coordinator)?
            .with_management(
                Arc::new(factory.clone()),
                options
                    .management
                    .clone()
                    .unwrap_or_else(|| Arc::new(factory::DenyManagement)),
            )
            .map_err(|_| DeployError::Coordinator)?;
        // Deployment owns every source in its replacement belt. Legacy hosts
        // retain supplied agents' own tools unless they explicitly opt in.
        let tools = options.tools.clone().unwrap_or_else(|| {
            Arc::new(|_| openhuman_embed::HostTurnTools::advertised(Vec::new()))
        });
        host = host
            .with_tools(tools)
            .map_err(|_| DeployError::Coordinator)?;
        let mut seats = BTreeMap::new();
        for seat in &config.seats {
            let spec = profile::seat(&config, seat, secrets.as_ref(), permissions.clone())?;
            let agent = host
                .register_spec(&runtime, spec)
                .await
                .map_err(|_| DeployError::Agent(seat.id.clone()))?;
            seats.insert(seat.id.clone(), agent);
        }
        let policies = register_hives(&host, &config).await?;
        let deployment = Self {
            runtime,
            host,
            seats,
            config,
            policies,
            permissions,
            factory,
            scheduler: Mutex::new(None),
        };
        workflow::register(&deployment)?;
        if deployment.config.runtime.start_scheduler {
            deployment.start_scheduler()?;
        }
        Ok(deployment)
    }
    /// Shared runtime; every seat belongs to this identity.
    #[must_use]
    pub fn runtime(&self) -> &openhuman_embed::Runtime {
        &self.runtime
    }
    /// Permanent tools and continuing-session coordinator.
    #[must_use]
    pub fn host(&self) -> &OpenHumanHost {
        &self.host
    }
    /// Fixed seats declared by the manifest.
    #[must_use]
    pub fn seats(&self) -> &Seats {
        &self.seats
    }
    /// Factory sharing the runtime and typed policy catalog.
    #[must_use]
    pub fn factory(&self) -> &ManifestFactory {
        &self.factory
    }
    /// Pure deliberation policy for a declared hive; completion uses its separate coordinator settings.
    #[must_use]
    pub fn policy(&self, hive: &str) -> Option<&HivePolicyView> {
        self.policies.get(hive)
    }
    /// Shared memory view, without rebinding any seat's memory root.
    /// # Errors
    /// Unknown hive, absent recall root or disabled native memory.
    pub fn memory(&self, hive: &str) -> DeployResult<openhuman_embed::memory::Memory> {
        let root = self
            .policy(hive)
            .and_then(|policy| policy.memory_root.as_deref())
            .ok_or_else(|| runtime::unsupported("hive.memory_root"))?;
        self.runtime
            .memory(&native_memory_root(root)?)
            .map_err(|_| DeployError::Runtime)
    }
    /// Replace the fallback consent snapshot for subsequent tool calls.
    /// A supplied `ApprovalContext` remains authoritative when present.
    pub fn update_approval(&self, snapshot: ApprovalSnapshot) {
        *self
            .permissions
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = snapshot;
    }
    /// Sanitized adapter-owned pending questions, including correlated handler answers.
    #[must_use]
    pub fn pending(&self) -> Vec<PendingRequest> {
        self.permissions
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }
    /// Wait for a correlated handler answer; original tool calls remain refused.
    pub async fn wait_decision(
        &self,
        request_id: &str,
    ) -> Option<openhuman_embed::ApprovalDecision> {
        loop {
            let changed = self.permissions.changed.notified();
            let pending = self
                .permissions
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(request_id)
                .cloned()?;
            if let Some(answer) = pending.decision {
                return Some(answer);
            }
            changed.await;
        }
    }
    /// Release a parked coordinator seat with the answer as a note, never replaying the original call.
    /// # Errors
    /// Unknown/unanswered request, isolated cron question or coordinator refusal.
    pub async fn release(&self, request_id: &str) -> DeployResult<()> {
        let pending = self
            .permissions
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(request_id)
            .cloned()
            .ok_or(DeployError::Coordinator)?;
        let scope = pending.scope.ok_or(DeployError::Coordinator)?;
        let answer = pending.decision.ok_or(DeployError::Coordinator)?;
        let turn = tinyhivemind_hives::ParkedTurn {
            turn_id: scope.turn_id,
            session_id: scope.session_id.ok_or(DeployError::Coordinator)?,
            episode_id: scope.episode.map(|episode| episode.episode_id),
            message_ids: scope.message_ids,
            scheduled_job_id: scope.scheduled_job_id,
        };
        self.host.coordinator().release_parked(&scope.agent_id, &turn, Some(format!("Host approval decision: {answer:?}. Original call was refused; no effect has been replayed."))).await.map_err(|_| DeployError::Coordinator)?;
        self.permissions
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(request_id);
        Ok(())
    }
    /// Start the native scheduler only after deployment assembly. Idempotent.
    /// # Errors
    /// Native runtime configuration unavailable.
    pub fn start_scheduler(&self) -> DeployResult<()> {
        let mut scheduler = self
            .scheduler
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if scheduler.is_none() {
            let config = self
                .runtime
                .core_runtime()
                .context()
                .embedder_config()
                .cloned()
                .ok_or(DeployError::Runtime)?;
            *scheduler = Some(tokio::spawn(async move {
                let _ = Box::pin(openhuman_core::cron::scheduler::run(config)).await;
            }));
        }
        Ok(())
    }
}
async fn register_hives(
    host: &OpenHumanHost,
    config: &crate::config::HiveConfig,
) -> DeployResult<BTreeMap<String, HivePolicyView>> {
    let mut policies = BTreeMap::new();
    for hive in &config.hives {
        host.coordinator()
            .create_hive(HiveInfo {
                hive_id: hive.id.clone(),
                name: if hive.name.is_empty() {
                    hive.id.clone()
                } else {
                    hive.name.clone()
                },
                description: None,
                members: hive
                    .members
                    .iter()
                    .map(|member| member.seat.clone())
                    .collect(),
            })
            .await
            .map_err(|_| DeployError::Coordinator)?;
        let mut options = hive.coordinator.clone();
        options.conduct_policy.turn_wall =
            options.conduct_policy.turn_wall.min(hive.conduct.turn_wall);
        options.conduct_policy.child_turn_wall = options
            .conduct_policy
            .child_turn_wall
            .min(hive.conduct.child_turn_wall);
        host.coordinator()
            .configure_hive(
                &hive.id,
                HiveSettings {
                    options,
                    routing: hive.routing.clone(),
                    roles: hive
                        .members
                        .iter()
                        .map(|member| (member.seat.clone(), member.role.clone()))
                        .collect(),
                },
            )
            .await
            .map_err(|_| DeployError::Coordinator)?;
        policies.insert(
            hive.id.clone(),
            HivePolicyView {
                episode: hive.episode,
                division: hive.division,
                memory_root: hive.memory_root.clone(),
            },
        );
    }
    Ok(policies)
}
impl Drop for HiveDeployment {
    fn drop(&mut self) {
        if let Some(task) = self
            .scheduler
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}
#[cfg(test)]
mod test;
