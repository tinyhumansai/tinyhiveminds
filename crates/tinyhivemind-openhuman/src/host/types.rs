//! Host extension points and supplied handle types.
use crate::{Error, Result};
use openhuman_embed::Agent;
use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc, time::Duration};
use tinyhivemind_hives::{Destination, EpisodeContext, HiveInfo, TurnDisposition, TurnRequest};
use tinyhivemind_lang::LoweredMemoryBinding;
/// Default maximum duration of a supplied agent turn.
pub const TURN_TIMEOUT: Duration = Duration::from_secs(300);
/// Host factory result future.
pub type AgentFuture = Pin<Box<dyn Future<Output = Result<Agent>> + Send>>;
/// Host creates configured agents from nonsecret template references.
///
/// The factory owns the runtime and provider credentials. It must return a
/// configured handle from the host's runtime; the adapter rejects other runtimes.
pub trait AgentFactory: Send + Sync {
    /// Create on the shared runtime; host validates template and config.
    fn create(&self, template: String, config: serde_json::Value) -> AgentFuture;
    /// Create with an explicit memory contract before building the agent.
    ///
    /// Override to apply the binding to `AgentSpec` and implement every supplied
    /// memory setting, including read-only identities, recall, reaches and
    /// lifecycle. The default refuses bindings and preserves old factories for
    /// unbound requests. The adapter verifies the resulting agent id and root.
    fn create_with_memory(
        &self,
        template: String,
        config: serde_json::Value,
        memory: Option<LoweredMemoryBinding>,
    ) -> AgentFuture {
        if memory.is_some() {
            Box::pin(async { Err(Error::MemoryBindingUnsupported) })
        } else {
            self.create(template, config)
        }
    }
}
/// Explicit opt-in management authorization.
pub trait ManagementAuthorizer: Send + Sync {
    /// Authorize before any factory or coordinator mutation.
    /// # Errors
    /// Return a denial when the actor cannot perform this request.
    fn authorize(&self, actor: &str, request: &ManagementRequest) -> Result<()>;
}
/// Host policy over what an agent may send, mirroring [`ManagementAuthorizer`].
///
/// Consulted by `hivemind_send_agent`, `hivemind_send_hive`, `hivemind_ask` and
/// `hivemind_broadcast` before they execute. A refusal is returned to the model
/// as the tool's error text; the turn continues.
pub trait SendAuthorizer: Send + Sync {
    /// Admit or refuse one outbound request from `actor`.
    /// # Errors
    /// Return a refusal, conventionally [`crate::Error::SendDenied`], whose
    /// message the model reads.
    fn authorize(&self, actor: &str, request: &SendRequest) -> Result<()>;
    /// Authorize a send from the native session bound by the tool source.
    /// # Errors
    /// Returns the same refusal as `authorize` unless the host narrows sessions.
    fn authorize_in_session(
        &self,
        actor: &str,
        request: &SendRequest,
        _session: Option<&str>,
    ) -> Result<()> {
        self.authorize(actor, request)
    }
}
/// Outbound operation offered to the host [`SendAuthorizer`].
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum SendRequest {
    /// Direct message to a registered agent.
    Agent {
        /// Recipient agent identity.
        agent_id: String,
        /// Message text.
        body: String,
    },
    /// Message to a hive the actor belongs to.
    Hive {
        /// Hive identity.
        hive_id: String,
        /// Message text.
        body: String,
        /// Existing conversation root, when replying in a thread.
        thread: Option<u64>,
        /// Private readers; empty means every member.
        only_for: Vec<String>,
    },
    /// Question to peers in the actor's active episode.
    Ask {
        /// Episode the question is asked in.
        episode_id: String,
        /// Asked peers.
        agents: Vec<String>,
        /// Question text.
        body: String,
    },
    /// Work routed across the actor's active episode.
    Broadcast {
        /// Episode the work is broadcast in.
        episode_id: String,
        /// Work text.
        body: String,
    },
}
/// Management operation offered to the host authorizer.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum ManagementRequest {
    /// Create a hive using known agents.
    CreateHive(HiveInfo),
    /// Create a configured agent through a host-defined template.
    CreateAgent {
        /// Template reference.
        template: String,
        /// Nonsecret host-validated settings.
        config: serde_json::Value,
        /// Portable memory contract, applied by the factory before registration.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory: Option<Box<LoweredMemoryBinding>>,
    },
    /// Join a registered agent to a hive.
    JoinHive {
        /// Hive identity.
        hive_id: String,
        /// Agent identity.
        agent_id: String,
    },
    /// Leave a hive without deleting its history.
    LeaveHive {
        /// Hive identity.
        hive_id: String,
        /// Agent identity.
        agent_id: String,
    },
}
/// Future wrapped by a host turn scope.
pub type HostedTurn<'a> =
    Pin<Box<dyn Future<Output = Result<openhuman_embed::TurnOutcome>> + Send + 'a>>;
/// Drained progress channel owned by the host.
pub type TurnProgressSink =
    tokio::sync::mpsc::Sender<openhuman_embed::agent_progress::AgentProgress>;
/// What one turn is about, derived from its [`TurnRequest`].
///
/// Every hook receives it, so a host can route live progress, attribute an
/// approval, or file a card per hive, episode and thread rather than per agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnScope {
    /// Durable reservation identity from the authoritative coordinator request.
    pub turn_id: String,
    /// Native session authenticated by the runner before any hooks execute.
    pub session_id: Option<String>,
    /// Durable trusted scheduled provenance; cannot be supplied by message text.
    pub scheduled_job_id: Option<String>,
    /// The agent taking the turn.
    pub agent_id: String,
    /// Active conductor assignment, absent for a direct message.
    pub episode: Option<EpisodeContext>,
    /// Identities of the messages delivered in this turn, in order.
    pub message_ids: Vec<String>,
    /// Distinct senders of those messages, in first-seen order.
    pub senders: Vec<String>,
    /// The hive for an episode turn; otherwise the delivered message's
    /// destination, which is the agent itself.
    pub destination: Destination,
    /// Conversation root the turn answers, when it answers one.
    pub thread: Option<u64>,
}
impl TurnScope {
    /// Derive the scope of `request`.
    #[must_use]
    pub fn from_request(request: &TurnRequest) -> Self {
        let first = request.messages.first();
        let mut senders: Vec<String> = Vec::new();
        for message in &request.messages {
            if !senders.contains(&message.sender) {
                senders.push(message.sender.clone());
            }
        }
        let destination = match (&request.episode, first) {
            (Some(episode), _) => Destination::Hive(episode.hive_id.clone()),
            (None, Some(message)) => message.destination.clone(),
            (None, None) => Destination::Agent(request.agent_id.clone()),
        };
        Self {
            turn_id: request.turn_id.clone(),
            scheduled_job_id: request.scheduled_job_id.clone(),
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
            episode: request.episode.clone(),
            message_ids: request
                .messages
                .iter()
                .map(|message| message.message_id.clone())
                .collect(),
            senders,
            destination,
            thread: request
                .episode
                .as_ref()
                .map_or_else(|| first.and_then(|message| message.thread), |e| e.thread),
        }
    }
}
/// Per-turn settings a host chooses in [`TurnHooks::prepare`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TurnOptions {
    /// Working directory for the agent's filesystem and shell tools this
    /// turn; `None` keeps the agent's own.
    pub cwd: Option<PathBuf>,
}
/// Optional per-turn options, progress, usage, approval and scope hooks.
///
/// Called in order: `turn_timeout`, `context`, `prepare`, `configure`,
/// `progress`, `wrap_turn`, then `after_turn`.
pub trait TurnHooks: Send + Sync {
    /// Override the host's fallback native deadline for this captured turn.
    /// `None` retains the fallback; a zero duration refuses dispatch.
    /// This deadline is applied after `configure` so timeout classification and
    /// native cancellation use the same bound.
    fn turn_timeout(&self, _scope: &TurnScope) -> Option<std::time::Duration> {
        None
    }
    /// Add context bound to this authoritative coordinator request.
    fn context(&self, _scope: &TurnScope) -> String {
        String::new()
    }
    /// Configure a native turn, adding permission hooks or model budgets.
    /// Use `turn_timeout` for deadline overrides; its bound is applied afterward.
    fn configure(&self, _scope: &TurnScope, turn: openhuman_embed::Turn) -> openhuman_embed::Turn {
        turn
    }
    /// Choose this turn's options before it is built.
    fn prepare(&self, _scope: &TurnScope) -> TurnOptions {
        TurnOptions::default()
    }
    /// Host owes this sink a receiver throughout the turn.
    fn progress(&self, _scope: &TurnScope) -> Option<TurnProgressSink> {
        None
    }
    /// Install host context around the complete turn.
    fn wrap_turn<'a>(&'a self, _scope: &'a TurnScope, turn: HostedTurn<'a>) -> HostedTurn<'a> {
        turn
    }
    /// Called after successful or failed turns with fresh usage, never stale usage.
    /// # Errors
    /// Host metering or approval processing can refuse completion. When the
    /// agent already committed its turn, its session remains bound while the
    /// coordinator interrupts delivery and discards staged actions and replies.
    fn after_turn(
        &self,
        _scope: &TurnScope,
        _usage: Option<&openhuman_core::agent::tinyagents::host::LastTurnUsage>,
    ) -> Result<TurnDisposition> {
        Ok(TurnDisposition::Completed)
    }
}
#[derive(Debug)]
pub(super) struct DefaultHooks;
impl TurnHooks for DefaultHooks {}
/// An existing agent implementing core's bound-handle trait.
///
/// For hosts driving the pure completion driver directly. The trait's historical
/// `runtime_id` method returns this agent's ID; it is distinct from
/// [`Agent::runtime_id`], which identifies the shared `OpenHuman` runtime.
#[derive(Clone, Debug)]
pub struct RegisteredAgent(pub Agent);
impl tinyhivemind_core::driver::BoundAgent for RegisteredAgent {
    fn runtime_id(&self) -> &str {
        self.0.id()
    }
}
pub(super) struct Management {
    pub factory: Arc<dyn AgentFactory>,
    pub authorizer: Arc<dyn ManagementAuthorizer>,
}

#[cfg(test)]
#[path = "types_test.rs"]
mod test;
