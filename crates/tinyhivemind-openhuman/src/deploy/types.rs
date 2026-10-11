//! Deployment ports, resource policy and credential resolution.
use crate::config::SecretRef;
use openhuman_embed::{ApprovalHandler, HostTools, RuntimeConfig, SessionStoreProvider};
use std::{collections::BTreeMap, sync::Arc};
use tinyhivemind_core::approval::{ConsentEpoch, Millis, RememberedRefusal, StandingGrant};
/// Errors identify structural operations, never native credential-bearing payloads.
#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    /// Configuration cannot be faithfully lowered by the selected ports.
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    /// A credential reference could not be resolved.
    #[error("cannot resolve secret reference")]
    Secret,
    /// The runtime refused construction.
    #[error("runtime construction failed")]
    Runtime,
    /// A template or agent could not be registered.
    #[error("agent construction failed for {0}")]
    Agent(String),
    /// Durable coordinator operations failed.
    #[error("coordinator operation failed")]
    Coordinator,
    /// A workflow could not be registered or triggered.
    #[error("workflow operation failed for {0}")]
    Workflow(String),
}
/// Deployment result, with credential-safe errors.
pub type DeployResult<T> = std::result::Result<T, DeployError>;
/// Resolve references only during deployment, without returning diagnostic payloads.
pub trait SecretResolver: Send + Sync {
    /// Return the value or a sanitized resolution failure.
    /// # Errors
    /// Return `DeployError::Secret` when absent or unavailable.
    fn resolve(&self, reference: &SecretRef) -> DeployResult<String>;
}
/// Resolve environment references; store references require a host resolver.
#[derive(Debug)]
pub struct EnvironmentSecrets;
impl SecretResolver for EnvironmentSecrets {
    fn resolve(&self, reference: &SecretRef) -> DeployResult<String> {
        match reference {
            SecretRef::Env(name) => std::env::var(name).map_err(|_| DeployError::Secret),
            SecretRef::Store(_) => Err(DeployError::Secret),
        }
    }
}
/// Trusted effect classifier for application and MCP tools.
/// Unknown names must return `Unclassified`; arguments never grant authority.
pub trait EffectClassifier: Send + Sync {
    /// Trusted metadata for argument-sensitive rule evaluation. Unknown fields
    /// remain unknown and rules requiring them refuse the call.
    fn subject(
        &self,
        _context: &openhuman_embed::seams::ToolHookContext,
    ) -> Option<tinytools::ToolSubject> {
        None
    }
    /// Additional host-authenticated rule attributes. Captured actor/session/hive
    /// attributes supersede these values.
    fn rule_context(
        &self,
        _context: &openhuman_embed::seams::ToolHookContext,
    ) -> tinytools::RuleContext {
        tinytools::RuleContext::new()
    }
    /// Describe the externally visible effect of this authenticated tool call.
    fn classify(
        &self,
        context: &openhuman_embed::seams::ToolHookContext,
    ) -> tinyhivemind_core::approval::Effect;
}
/// Host snapshot for the pure approval fold. Hosts advance consent epochs on new direction.
#[derive(Clone, Debug)]
pub struct ApprovalSnapshot {
    /// People eligible to answer core approval questions.
    pub people: Vec<tinyhivemind_core::roster::Person>,
    /// Current consent epoch.
    pub epoch: ConsentEpoch,
    /// Monotonic host reading.
    pub now: Millis,
    /// Previously issued authority.
    pub grants: Vec<StandingGrant>,
    /// Same-epoch refusals.
    pub refusals: Vec<RememberedRefusal>,
}
impl Default for ApprovalSnapshot {
    fn default() -> Self {
        Self {
            people: Vec::new(),
            epoch: ConsentEpoch(0),
            now: Millis(0),
            grants: Vec::new(),
            refusals: Vec::new(),
        }
    }
}
/// Supplies live consent state and sequence numbers from the host's shared ordering.
/// Capturing state does not execute or authorize a tool itself.
pub trait ApprovalContext: Send + Sync {
    /// Current person/grant/refusal/epoch snapshot.
    fn snapshot(&self) -> ApprovalSnapshot;
    /// Mint the next sequence in the same ordering as standing grants.
    fn next_sequence(&self) -> u64;
}
/// Optional host ports. No credentials or provider payloads are formatted by Debug.
#[derive(Clone, Default)]
pub struct BuildOptions {
    /// Runtime config baseline, for host-only Docker or local runtime wiring.
    pub config: Option<RuntimeConfig>,
    /// Incidental backend endpoint, separate from model inference.
    pub backend_url: Option<String>,
    /// Host-owned transcript/session stores, required by selector `host`.
    pub session_store: Option<Arc<dyn SessionStoreProvider>>,
    /// Host-owned memory engine, required by selector `host`.
    pub memory_engine: Option<Arc<dyn tinymemory_api::MemoryEngine>>,
    /// Host-owned durable coordinator store; otherwise in-memory.
    pub coordinator_storage: Option<Arc<dyn tinyhivemind_hives::Storage>>,
    /// Retention is global to the durable coordinator store.
    pub retention: tinyhivemind_hives::RetentionPolicy,
    /// Trusted native provider, required for non-OpenAI provider kinds.
    pub provider: Option<openhuman_embed::Provider>,
    /// Additional host tool source, retained in every episode replacement belt.
    pub tools: Option<HostTools>,
    /// Application/MCP classification beyond builtin known effects.
    pub classifier: Option<Arc<dyn EffectClassifier>>,
    /// Explicit handler for adapter-owned Ask requests.
    pub approval_handler: Option<Arc<dyn ApprovalHandler>>,
    /// Host-owned consent/roster/grant/refusal snapshot.
    pub approval: ApprovalSnapshot,
    /// Optional live authority snapshot and shared sequence source.
    pub approval_context: Option<Arc<dyn ApprovalContext>>,
    /// Conservative per-request bound required when dollar budgets are declared.
    pub call_budget: Option<openhuman_embed::budget::CallBudget>,
    /// Optional explicit management authorization; default denies agent management.
    pub management: Option<Arc<dyn crate::ManagementAuthorizer>>,
    /// Extra hooks compose with deployment hooks and cannot override denials.
    pub hooks: Option<Arc<dyn crate::TurnHooks>>,
}
impl std::fmt::Debug for BuildOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildOptions").finish_non_exhaustive()
    }
}
/// Pure deliberation settings, deliberately exposed separately from completion scheduling.
#[derive(Clone, Debug)]
pub struct HivePolicyView {
    /// Accepted pure episode policy.
    pub episode: tinyhivemind_core::hive::EpisodePolicy,
    /// Accepted pure task division policy.
    pub division: tinyhivemind_core::hive::DivisionPolicy,
    /// Optional shared recall namespace; seat memory bindings remain fixed.
    pub memory_root: Option<String>,
}
/// A correlated adapter-owned question; raw arguments are never retained here.
#[derive(Clone, Debug)]
pub struct PendingRequest {
    /// Native sanitized request passed explicitly to the host handler.
    pub request: openhuman_embed::PendingApproval,
    /// Captured authoritative turn scope, absent for isolated native cron turns.
    pub scope: Option<crate::TurnScope>,
    /// Handler answer, waiting for an explicit host release.
    pub decision: Option<openhuman_embed::ApprovalDecision>,
}
/// Seat handles sharing the deployment's single runtime.
pub type Seats = BTreeMap<String, openhuman_embed::Agent>;
