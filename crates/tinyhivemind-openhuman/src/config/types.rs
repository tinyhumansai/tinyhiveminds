//! Canonical serializable manifest and its runtime-independent selections.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tinyhivemind_core::{
    approval::ApprovalPolicy,
    driver::ConductPolicy,
    embed::RoutingPolicy,
    hive::{DivisionPolicy, EpisodePolicy},
};
use tinyhivemind_hives::CoordinatorOptions;

/// Credential location; never a resolved credential.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SecretRef {
    /// Environment variable name.
    Env(String),
    /// Host secret-store key.
    Store(String),
}
/// Provider endpoint and credential reference.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Provider {
    /// Provider selector; `openai` supports compatible endpoints.
    pub kind: String,
    /// Optional compatible HTTP endpoint.
    pub endpoint: Option<String>,
    /// Credential reference resolved only during deployment.
    pub credential: Option<SecretRef>,
}
/// Model and sampling defaults.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Model {
    /// Provider model identifier.
    pub name: Option<String>,
    /// Sampling temperature, including zero.
    pub temperature: Option<f64>,
    /// Maximum generated tokens per model request.
    pub max_tokens: Option<u32>,
}
/// Turn resource limits; omission inherits the parent.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// Wall-clock timeout in milliseconds.
    pub timeout_ms: Option<u64>,
    /// Maximum model/tool iterations.
    pub iterations: Option<usize>,
    /// Maximum monetary model spend in dollars.
    pub budget_usd: Option<f64>,
}
/// Ordered access tiers, from least to greatest authority.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// No mutating tools.
    ReadOnly,
    /// Human-supervised mutation.
    #[default]
    Supervised,
    /// Autonomous mutation subject to other gates.
    Autonomous,
}
/// Shell/file confinement selection.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Sandbox {
    /// Access and path rules only.
    #[default]
    None,
    /// Disable writes and execution.
    ReadOnly,
    /// Host-provided jail or Docker backend.
    Sandboxed,
}
/// Tool visibility selector.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolScope {
    /// Every registered tool subject to permission gates.
    #[default]
    Wildcard,
    /// Only explicitly named tools.
    Named(Vec<String>),
    /// Only host-supplied tools, with read-only access.
    HostOnly,
}
/// Immutable MCP connection and server tool selection.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    /// Unique connection name.
    pub id: String,
    /// `stdio`, `sse` or `http`.
    pub transport: String,
    /// URL for network transports, executable for stdio.
    pub endpoint: String,
    /// Stdio command arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Credential-bearing environment values, all references.
    #[serde(default)]
    pub env: BTreeMap<String, SecretRef>,
    /// Credential-bearing HTTP headers, all references.
    #[serde(default)]
    pub headers: BTreeMap<String, SecretRef>,
    /// None exposes all server tools; Some limits them.
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Server tools denied independently of its allowlist.
    #[serde(default)]
    pub deny_tools: Vec<String>,
}
/// Fixed lifetime agent memory binding.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Memory {
    /// Logical namespace root; absent defaults to `seat:<seat-id>`.
    /// Deployment encodes it with [`crate::deploy::native_memory_root`].
    pub root: Option<String>,
}
/// Named authority baseline or restriction.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionProfile {
    /// Unique profile id.
    pub id: String,
    /// Access tier.
    pub access: Access,
    /// Sandbox confinement.
    pub sandbox: Sandbox,
    /// None inherits unrestricted names; Some is a finite allowlist.
    pub allow_tools: Option<Vec<String>>,
    /// Tool names always denied.
    pub deny_tools: Vec<String>,
    /// Allowed direct/hive destination ids; None is unrestricted.
    pub send_destinations: Option<Vec<String>>,
    /// Allowed send operations; None is unrestricted.
    pub send_operations: Option<Vec<String>>,
    /// Native total core approval policy; omission inherits.
    pub approval: Option<ApprovalPolicy>,
    /// Native argument-sensitive tool rules; omission inherits.
    pub tool_rules: Option<tinytools::ToolRules>,
}
/// Global runtime baseline, shared by every seat.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeSection {
    /// Provider connection.
    pub provider: Provider,
    /// Default model settings.
    pub model: Model,
    /// Workspace directory; absent creates an ephemeral workspace.
    pub workspace: Option<String>,
    /// Session store selector (`memory`, `sqlite`, or `host`).
    pub session_store: String,
    /// Memory engine selector (`disabled`, `memory`, or `host`).
    pub memory_engine: String,
    /// Optional named authority baseline.
    pub permission_profile: Option<String>,
    /// Baseline confinement.
    pub sandbox: Sandbox,
    /// Global tool rules.
    pub tool_rules: Option<tinytools::ToolRules>,
    /// Baseline MCP connections.
    pub mcp: Vec<McpServer>,
    /// Enabled skill directories.
    pub skills: Vec<String>,
    /// Autonomy tier.
    pub autonomy: Access,
    /// Runtime limit defaults.
    pub limits: Limits,
    /// Global coordinator-turn bound across hives. Native cron has a separate
    /// scheduler ceiling with this value; direct turns bypass coordinator admission.
    pub concurrency: usize,
    /// Include installed user skills in the runtime.
    pub include_user_skills: bool,
    /// Maximum seats; omission leaves the deployment unbounded.
    pub max_agents: Option<usize>,
    /// Start scheduler services after deployment construction.
    pub start_scheduler: bool,
}
impl Default for RuntimeSection {
    fn default() -> Self {
        Self {
            provider: Provider::default(),
            model: Model::default(),
            workspace: None,
            session_store: "memory".into(),
            memory_engine: "disabled".into(),
            permission_profile: None,
            sandbox: Sandbox::None,
            tool_rules: None,
            mcp: vec![],
            skills: vec![],
            autonomy: Access::Supervised,
            limits: Limits::default(),
            concurrency: 4,
            include_user_skills: false,
            max_agents: None,
            start_scheduler: false,
        }
    }
}
/// Reusable agent template.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    /// Unique template id.
    pub id: String,
    /// Optional builtin definition name.
    pub definition_base: Option<String>,
    /// Replace the entire composed system prompt when true.
    pub bare_prompt: bool,
    /// Override user-skill discovery.
    pub include_user_skills: Option<bool>,
    /// Fixed system prompt body.
    pub system_prompt: String,
    /// Named Markdown context, in prompt composition order.
    pub context: Vec<String>,
    /// Model/sampling overrides.
    pub model: Model,
    /// Optional authority restriction.
    pub permission_profile: Option<String>,
    /// Tool visibility.
    pub tool_scope: ToolScope,
    /// Additional denied tool names.
    pub deny_tools: Vec<String>,
    /// Additional immutable MCP servers.
    pub mcp: Vec<McpServer>,
    /// Additional enabled skills.
    pub skills: Vec<String>,
    /// Allowed subagent template ids.
    pub subagents: Vec<String>,
    /// Fixed memory binding override.
    pub memory: Option<Memory>,
    /// Turn limits.
    pub limits: Limits,
}
/// Typed dynamic or configured seat overrides; credential fields are absent.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SeatOverrides {
    /// Replacement fixed system prompt body.
    pub system_prompt: Option<String>,
    /// Additional context references.
    pub context: Vec<String>,
    /// Model/sampling overrides.
    pub model: Model,
    /// Optional authority restriction.
    pub permission_profile: Option<String>,
    /// Optional tool visibility restriction.
    pub tool_scope: Option<ToolScope>,
    /// Additional denied tool names.
    pub deny_tools: Vec<String>,
    /// Additional immutable MCP connections.
    pub mcp: Vec<McpServer>,
    /// Additional skills.
    pub skills: Vec<String>,
    /// Allowed subagent template ids; omission inherits.
    pub subagents: Option<Vec<String>>,
    /// Fixed memory binding override.
    pub memory: Option<Memory>,
    /// Turn limit overrides.
    pub limits: Limits,
}
/// One persistent agent/session identity.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Seat {
    /// OpenHuman-compatible agent id.
    pub id: String,
    /// Reusable profile reference.
    pub profile: String,
    /// Typed final seat settings.
    pub overrides: SeatOverrides,
}
/// Per-hive captured overlay; cannot rebind memory or credentials.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Membership {
    /// Existing seat id.
    pub seat: String,
    /// Freeform nonempty routing/delegation role.
    pub role: String,
    /// Additional context references.
    pub context: Vec<String>,
    /// Optional further authority restriction.
    pub permission_profile: Option<String>,
    /// Optional assertion of the immutable seat MCP set.
    pub mcp: Option<Vec<McpServer>>,
}
/// Hive membership and native policy values.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct HiveSpec {
    /// Unique hive identity.
    pub id: String,
    /// Display name; empty falls back to id.
    pub name: String,
    /// Members, in declaration order.
    pub members: Vec<Membership>,
    /// Shared recall namespace; does not rebind seats.
    pub memory_root: Option<String>,
    /// Pure deliberation episode policy.
    pub episode: EpisodePolicy,
    /// Semantic completion routing policy.
    pub routing: RoutingPolicy,
    /// Per-hive scheduling settings.
    pub coordinator: CoordinatorOptions,
    /// Pure deliberation task division policy.
    pub division: DivisionPolicy,
    /// Completion turn walls.
    pub conduct: ConductPolicy,
}
impl Default for HiveSpec {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            members: Vec::new(),
            memory_root: None,
            episode: EpisodePolicy {
                round_width: 1,
                ..EpisodePolicy::default()
            },
            routing: RoutingPolicy {
                minimum_confidence: tinyhivemind_core::responder::Probability::new(600_000)
                    .unwrap_or(tinyhivemind_core::responder::Probability::ONE),
                high_impact_minimum_confidence: tinyhivemind_core::responder::Probability::new(
                    800_000,
                )
                .unwrap_or(tinyhivemind_core::responder::Probability::ONE),
                clarification_threshold: tinyhivemind_core::responder::Probability::new(700_000)
                    .unwrap_or(tinyhivemind_core::responder::Probability::ONE),
                high_impact_threshold: tinyhivemind_core::responder::Probability::new(700_000)
                    .unwrap_or(tinyhivemind_core::responder::Probability::ONE),
                round_width: 4,
                choice_option_limit: 8,
            },
            coordinator: CoordinatorOptions::default(),
            division: DivisionPolicy::default(),
            conduct: ConductPolicy::default(),
        }
    }
}
/// Explicit workflow dispatch target.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTarget {
    /// Native isolated agent cron session.
    Seat(String),
    /// Authorized coordinator hive message.
    Hive(String),
}
/// Scheduled named prompt; services start separately.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    /// Unique job id.
    pub id: String,
    /// Cron expression, parsed before runtime construction.
    pub schedule: String,
    /// Authenticated deployment target.
    pub target: WorkflowTarget,
    /// Prompt submitted when the job runs.
    pub prompt: String,
    /// Whether this job is enabled.
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// Maximum retry count.
    #[serde(default)]
    pub retries: u32,
    /// At most one invocation at once.
    #[serde(default = "enabled")]
    pub single_flight: bool,
}
fn enabled() -> bool {
    true
}
/// Canonical configuration; parsing has no runtime or secret side effects.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct HiveConfig {
    /// One runtime's defaults.
    pub runtime: RuntimeSection,
    /// Reusable agent templates.
    pub profiles: Vec<Profile>,
    /// Named baseline and restriction policies.
    pub permission_profiles: Vec<PermissionProfile>,
    /// Persistent seats.
    pub seats: Vec<Seat>,
    /// Hives sharing the seats.
    pub hives: Vec<HiveSpec>,
    /// Named cron workflows.
    pub workflows: Vec<Workflow>,
    /// Named Markdown context bodies, resolved by directory loading.
    pub contexts: BTreeMap<String, String>,
}
/// Manifest that passed all referential and authority checks.
#[derive(Clone, Debug)]
pub struct ValidatedHiveConfig(pub(super) HiveConfig);
impl ValidatedHiveConfig {
    /// Immutable validated manifest.
    #[must_use]
    pub fn config(&self) -> &HiveConfig {
        &self.0
    }
    /// Consume validation proof to recover an editable manifest.
    #[must_use]
    pub fn into_config(self) -> HiveConfig {
        self.0
    }
}
