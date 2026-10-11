//! Registration and execution of host-owned agent handles.
mod activation;
mod runner;
mod types;
use crate::{Error, HiveMemory, Result};
pub(crate) use activation::Activation;
use openhuman_embed::{Agent, AgentSpec, HostTools, HostTurnTools, Runtime};
use runner::SuppliedRunner;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tinyhivemind_hives::{AgentRegistration, Coordinator};
pub use types::*;

pub(crate) struct Inner {
    pub coordinator: Coordinator,
    runtime_id: String,
    agents: Mutex<BTreeMap<String, Entry>>,
    management: Option<Management>,
    hooks: Arc<dyn TurnHooks>,
    memory: Option<HiveMemory>,
    extra_tools: Option<HostTools>,
    turn_timeout: Duration,
    pub send_policy: Option<Arc<dyn SendAuthorizer>>,
}
struct Entry {
    /// `None` while a replacement is being built, or after one failed.
    agent: Option<Agent>,
    source: HostTools,
    runner: Arc<SuppliedRunner>,
    activation: Arc<Activation>,
}
/// Shares one coordinator and permanent attachment per supplied agent.
///
/// The host configures agents on one runtime before registration. Each handle
/// keeps one continuing conversation across hives. Keep this host alive while
/// its tools are used: attachment services hold weak references to it.
#[derive(Clone)]
pub struct OpenHumanHost {
    pub(crate) inner: Arc<Inner>,
}
impl std::fmt::Debug for OpenHumanHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenHumanHost")
            .field("runtime_id", &self.inner.runtime_id)
            .finish_non_exhaustive()
    }
}
impl OpenHumanHost {
    /// Bind an adapter to the same runtime as its coordinator.
    /// # Errors
    /// Reject mismatched runtime identities.
    pub fn new(runtime_id: String, coordinator: Coordinator) -> Result<Self> {
        if runtime_id != coordinator.runtime_id() {
            return Err(Error::RuntimeMismatch);
        }
        Ok(Self {
            inner: Arc::new(Inner {
                coordinator,
                runtime_id,
                agents: Mutex::new(BTreeMap::new()),
                management: None,
                hooks: Arc::new(DefaultHooks),
                memory: None,
                extra_tools: None,
                turn_timeout: TURN_TIMEOUT,
                send_policy: None,
            }),
        })
    }
    /// Configure management before sharing or registering agents.
    /// # Errors
    /// Refuse changed settings after registration or sharing the host.
    pub fn with_management(
        mut self,
        factory: Arc<dyn AgentFactory>,
        authorizer: Arc<dyn ManagementAuthorizer>,
    ) -> Result<Self> {
        let inner = self.configurable_inner()?;
        inner.management = Some(Management {
            factory,
            authorizer,
        });
        Ok(self)
    }
    /// Configure per-turn host hooks before sharing or registration.
    /// # Errors
    /// Refuse changed settings once agents or another host clone exist.
    pub fn with_hooks(mut self, hooks: Arc<dyn TurnHooks>) -> Result<Self> {
        let inner = self.configurable_inner()?;
        inner.hooks = hooks;
        Ok(self)
    }
    /// Supply extra host tools in every continuing turn's replacement belt.
    /// Configure before registering or sharing this host.
    /// # Errors
    /// Refuse settings after registration or sharing.
    pub fn with_tools(mut self, tools: HostTools) -> Result<Self> {
        self.configurable_inner()?.extra_tools = Some(tools);
        Ok(self)
    }
    /// Gate the outbound tools — `hivemind_send_agent`, `hivemind_send_hive`,
    /// `hivemind_ask`, `hivemind_broadcast` — behind `policy`; configure
    /// before sharing or registration. Without one every send is admitted.
    /// # Errors
    /// Refuse changed settings once agents or another host clone exist.
    pub fn with_send_policy(mut self, policy: Arc<dyn SendAuthorizer>) -> Result<Self> {
        let inner = self.configurable_inner()?;
        inner.send_policy = Some(policy);
        Ok(self)
    }
    /// Bound every supplied agent turn by `timeout` instead of
    /// [`TURN_TIMEOUT`]; configure before sharing or registration.
    /// # Errors
    /// Refuse a zero timeout, and changed settings once agents or another
    /// host clone exist.
    pub fn with_turn_timeout(mut self, timeout: Duration) -> Result<Self> {
        if timeout.is_zero() {
            return Err(Error::InvalidTurnTimeout);
        }
        let inner = self.configurable_inner()?;
        inner.turn_timeout = timeout;
        Ok(self)
    }
    /// The wall applied to each supplied agent turn.
    #[must_use]
    pub fn turn_timeout(&self) -> Duration {
        self.inner.turn_timeout
    }
    /// Share one hive memory among every seat registered from now on.
    ///
    /// Each seat then logs its turns under its own memory agent id (its seat
    /// id) below the hive's root, and recalls the root's shared learnings and
    /// peer turns. [`Self::register_spec`] binds a seat's spec itself; an
    /// agent built elsewhere must carry the binding from
    /// [`HiveMemory::bind`], or registration refuses it.
    /// # Errors
    /// Refuse changed settings once agents or another host clone exist.
    pub fn with_hive_memory(mut self, memory: HiveMemory) -> Result<Self> {
        let inner = self.configurable_inner()?;
        inner.memory = Some(memory);
        Ok(self)
    }
    /// The hive memory seats are bound to, when one was configured.
    #[must_use]
    pub fn hive_memory(&self) -> Option<&HiveMemory> {
        self.inner.memory.as_ref()
    }
    fn configurable_inner(&mut self) -> Result<&mut Inner> {
        if !self
            .inner
            .agents
            .lock()
            .map_err(|_| Error::Poisoned)?
            .is_empty()
        {
            return Err(Error::ManagementAlreadyStarted);
        }
        Arc::get_mut(&mut self.inner).ok_or(Error::ManagementAlreadyStarted)
    }
    /// Register an existing handle; repeated clones use the identical factory.
    ///
    /// Retains the supplied handle and installs nine permanent tools, or thirteen
    /// when management was configured. Configuration remains owned by the agent.
    /// An existing host conversation should use [`Self::register_agent_in_session`].
    /// # Errors
    /// Reject another runtime, conflicting handles, tool collisions or storage
    /// failures, and, with hive memory configured, a seat not bound to it.
    pub async fn register_agent(&self, agent: Agent) -> Result<()> {
        self.register(agent, None).await
    }
    /// Build a seat from `spec` on `runtime` and register it.
    ///
    /// With hive memory configured the spec is first bound to it, so the
    /// seat recalls and writes the hive's shared memory; without, the spec is
    /// built as given. Returns the registered handle.
    /// # Errors
    /// An unusable seat id for memory, the runtime refusing the spec, or any
    /// [`Self::register_agent`] failure.
    pub async fn register_spec(&self, runtime: &Runtime, spec: AgentSpec) -> Result<Agent> {
        let spec = match &self.inner.memory {
            Some(memory) => memory.bind(spec)?,
            None => spec,
        };
        let agent = runtime.agent(spec)?;
        self.register_agent(agent.clone()).await?;
        Ok(agent)
    }
    async fn register(&self, agent: Agent, session_id: Option<&str>) -> Result<()> {
        if agent.runtime_id() != self.inner.runtime_id {
            return Err(Error::RuntimeMismatch);
        }
        if let Some(memory) = &self.inner.memory {
            memory.check(&agent)?;
        }
        let id = agent.id().to_owned();
        let (runner, activation) = self.attach(&id, &agent)?;
        self.register_runner(
            AgentRegistration {
                agent_id: id,
                runtime_id: self.inner.runtime_id.clone(),
                runner,
            },
            session_id,
        )
        .await?;
        activation.activate();
        Ok(())
    }
    /// Attach the permanent tools, reusing the identical source for a known
    /// handle. The entry is retained even if durable registration then fails.
    fn attach(&self, id: &str, agent: &Agent) -> Result<(Arc<SuppliedRunner>, Arc<Activation>)> {
        let mut entries = self.inner.agents.lock().map_err(|_| Error::Poisoned)?;
        if let Some(entry) = entries.get(id) {
            if !entry
                .agent
                .as_ref()
                .is_some_and(|known| known.same_agent(agent))
            {
                return Err(Error::AgentConflict(id.into()));
            }
            agent.attach_tools("hivemind", entry.source.clone())?;
            return Ok((entry.runner.clone(), entry.activation.clone()));
        }
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let actor = id.to_owned();
        let managed = self.inner.management.is_some();
        let activation = Arc::new(Activation::default());
        let attached_activation = activation.clone();
        let extra = self.inner.extra_tools.clone();
        let source: HostTools = Arc::new(move |context| {
            let tools = crate::tools::belt_in_session(
                &actor,
                &weak,
                &attached_activation,
                managed,
                context.session_id(),
            );
            let permanent = tools.iter().map(|tool| tool.name().to_owned()).collect();
            let mut belt = HostTurnTools {
                permanent,
                ..HostTurnTools::advertised(tools)
            };
            if let Some(extra) = &extra {
                let other = extra(context);
                belt.tools.extend(other.tools);
                belt.permanent.extend(other.permanent);
                belt.visible.extend(other.visible);
                belt.withheld.extend(other.withheld);
                belt.policy = other.policy;
            }
            belt
        });
        agent.attach_tools("hivemind", source.clone())?;
        let runner = Arc::new(SuppliedRunner {
            agent: Arc::new(tokio::sync::RwLock::new(Some(agent.clone()))),
            hooks: self.inner.hooks.clone(),
            activation: activation.clone(),
            timeout: self.inner.turn_timeout,
            source: self.inner.extra_tools.as_ref().map(|_| source.clone()),
        });
        // Keep the exact source for retry even if durable registration fails.
        entries.insert(
            id.into(),
            Entry {
                agent: Some(agent.clone()),
                source,
                runner: runner.clone(),
                activation: activation.clone(),
            },
        );
        Ok((runner, activation))
    }
    /// Rebuild the handle behind an already registered agent id — after the
    /// host changed its configuration, say — and return the new handle.
    ///
    /// `OpenHuman` keeps agent ids unique while any clone of a handle is
    /// alive, so the host cannot build the replacement first: this waits for
    /// any running turn of the agent to finish, holds off new ones, drops the
    /// adapter's handle and only then calls `build`. The host must not keep
    /// clones of the old handle itself, or `build` fails with a duplicate id.
    /// The durable registration, continuing session binding and queued work
    /// are unchanged; the next turn continues the same session on the new
    /// handle, which carries the hivemind tools.
    /// # Errors
    /// An id never registered with this host; `build`'s error; a built agent
    /// with another id, runtime, or hive memory binding; a refused tool
    /// attachment. After `build` ran, a failure leaves the agent without a
    /// handle: its claimed turns fail until a retry succeeds.
    pub async fn replace_agent(
        &self,
        agent_id: &str,
        build: impl FnOnce() -> Result<Agent>,
    ) -> Result<Agent> {
        let (runner, source) = {
            let entries = self.inner.agents.lock().map_err(|_| Error::Poisoned)?;
            let entry = entries
                .get(agent_id)
                .ok_or_else(|| tinyhivemind_hives::Error::UnknownAgent(agent_id.into()))?;
            (entry.runner.clone(), entry.source.clone())
        };
        let mut handle = runner.agent.write().await;
        *handle = None;
        self.set_entry_agent(agent_id, None)?;
        let agent = build()?;
        if agent.id() != agent_id {
            return Err(Error::AgentConflict(agent.id().into()));
        }
        if agent.runtime_id() != self.inner.runtime_id {
            return Err(Error::RuntimeMismatch);
        }
        if let Some(memory) = &self.inner.memory {
            memory.check(&agent)?;
        }
        agent.attach_tools("hivemind", source)?;
        *handle = Some(agent.clone());
        self.set_entry_agent(agent_id, Some(agent.clone()))?;
        Ok(agent)
    }
    fn set_entry_agent(&self, agent_id: &str, agent: Option<Agent>) -> Result<()> {
        if let Some(entry) = self
            .inner
            .agents
            .lock()
            .map_err(|_| Error::Poisoned)?
            .get_mut(agent_id)
        {
            entry.agent = agent;
        }
        Ok(())
    }
    /// Bind a supplied agent to an already running host conversation.
    ///
    /// Commits the validated session binding before publishing its runner, so
    /// concurrent claims continue this conversation from their first turn.
    /// Failed durable registration keeps the identical attachment for retry;
    /// its tools remain inactive until registration succeeds.
    /// # Errors
    /// Registration failures or conflicting continuing-session bindings.
    pub async fn register_agent_in_session(&self, agent: Agent, session_id: &str) -> Result<()> {
        self.register(agent, Some(session_id)).await
    }
    async fn register_runner(
        &self,
        registration: AgentRegistration,
        session_id: Option<&str>,
    ) -> Result<()> {
        match session_id {
            Some(session) => {
                self.inner
                    .coordinator
                    .register_agent_in_session(registration, session)
                    .await?;
            }
            None => self.inner.coordinator.register_agent(registration).await?,
        }
        Ok(())
    }

    /// Access host APIs for hive creation, membership, sends and scheduling.
    #[must_use]
    pub fn coordinator(&self) -> &Coordinator {
        &self.inner.coordinator
    }
    /// Execute an authorized management request.
    /// # Errors
    /// Disabled management, host denial, factory, runtime or coordinator errors.
    pub async fn manage(
        &self,
        actor: &str,
        request: ManagementRequest,
    ) -> Result<serde_json::Value> {
        let management = self
            .inner
            .management
            .as_ref()
            .ok_or(Error::ManagementDisabled)?;
        management.authorizer.authorize(actor, &request)?;
        match request {
            ManagementRequest::CreateHive(hive) => {
                self.coordinator().create_hive(hive.clone()).await?;
                Ok(serde_json::to_value(hive)?)
            }
            ManagementRequest::CreateAgent {
                template,
                config,
                memory,
            } => {
                if let Some(memory) = &memory {
                    crate::language::validate_binding(memory)?;
                }
                let agent = management
                    .factory
                    .create_with_memory(template, config, memory.as_deref().cloned())
                    .await?;
                if let Some(memory) = &memory {
                    let installed = &agent.config().memory;
                    if installed.agent_id.as_deref() != Some(memory.agent_id.as_str())
                        || installed.root.as_deref() != Some(memory.root.as_str())
                    {
                        return Err(Error::UnboundSeat {
                            seat: agent.id().to_owned(),
                            reason: "factory did not install the requested memory binding".into(),
                        });
                    }
                }
                let id = agent.id().to_owned();
                self.register_agent(agent).await?;
                Ok(serde_json::json!({"agent_id":id}))
            }
            ManagementRequest::JoinHive { hive_id, agent_id } => {
                self.coordinator().join_hive(&hive_id, &agent_id).await?;
                Ok(serde_json::json!({"joined":true}))
            }
            ManagementRequest::LeaveHive { hive_id, agent_id } => {
                self.coordinator().leave_hive(&hive_id, &agent_id).await?;
                Ok(serde_json::json!({"left":true}))
            }
        }
    }
}
#[cfg(test)]
mod test;
