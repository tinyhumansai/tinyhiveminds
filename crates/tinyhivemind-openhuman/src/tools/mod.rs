//! Stable native tools with bound attribution and weak service references.
mod types;
use crate::host::{Activation, Inner};
use crate::{Error, ManagementRequest, OpenHumanHost, Result, SendRequest};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Weak;
use tinyhivemind_hives::{Destination, EpisodeAction, HiveInfo, SendMessage};
use tinytools::{PermissionLevel, Tool, ToolResult};
use types::Kind;
#[cfg(test)]
pub(crate) fn belt(
    actor: &str,
    host: &Weak<Inner>,
    activation: &std::sync::Arc<Activation>,
    managed: bool,
) -> Vec<Box<dyn Tool>> {
    belt_in_session(actor, host, activation, managed, None)
}
pub(crate) fn belt_in_session(
    actor: &str,
    host: &Weak<Inner>,
    activation: &std::sync::Arc<Activation>,
    managed: bool,
    session: Option<&str>,
) -> Vec<Box<dyn Tool>> {
    Kind::all(managed)
        .into_iter()
        .map(|kind| {
            Box::new(HiveTool {
                actor: actor.into(),
                session: session.map(str::to_owned),
                host: host.clone(),
                kind,
                activation: activation.clone(),
            }) as Box<dyn Tool>
        })
        .collect()
}
struct HiveTool {
    actor: String,
    session: Option<String>,
    host: Weak<Inner>,
    kind: Kind,
    activation: std::sync::Arc<Activation>,
}
#[async_trait]
impl Tool for HiveTool {
    fn name(&self) -> &str {
        self.kind.name()
    }
    fn description(&self) -> &str {
        self.kind.description()
    }
    fn parameters_schema(&self) -> Value {
        self.kind.schema()
    }
    fn permission_level(&self) -> PermissionLevel {
        match self.kind {
            Kind::CreateHive | Kind::CreateAgent | Kind::JoinHive | Kind::LeaveHive => {
                PermissionLevel::Write
            }
            // Protocol state is internal to the shared conversation. Actor,
            // episode and send policy checks still authorize each operation.
            _ => PermissionLevel::ReadOnly,
        }
    }
    fn policy(&self) -> tinytools::ToolPolicy {
        if self.permission_level() == PermissionLevel::ReadOnly {
            tinytools::ToolPolicy::classified().with_side_effects(tinytools::ToolSideEffects {
                read_only: true,
                ..Default::default()
            })
        } else {
            tinytools::ToolPolicy::default()
        }
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let result = async {
            self.kind.validate(&args).map_err(Error::Harness)?;
            let host = OpenHumanHost {
                inner: self.host.upgrade().ok_or(Error::Unavailable)?,
            };
            if !self.activation.is_ready() {
                return Err(Error::RegistrationPending);
            }
            self.perform(&host, args).await
        }
        .await;
        Ok(match result {
            Ok(value) => ToolResult::success(serde_json::to_string(&value)?),
            Err(error) => ToolResult::error(error.to_string()),
        })
    }
}
impl HiveTool {
    async fn perform(&self, host: &OpenHumanHost, args: Value) -> Result<Value> {
        let coordinator = host.coordinator();
        let text = |name: &str| args[name].as_str().unwrap_or_default().to_owned();
        let actor = &self.actor;
        if let (Some(policy), Some(request)) = (&host.inner.send_policy, outbound(self.kind, &args))
        {
            policy.authorize_in_session(actor, &request, self.session.as_deref())?;
        }
        match self.kind {
            Kind::ListHives => Ok(serde_json::to_value(
                coordinator
                    .list_hives()?
                    .into_iter()
                    .filter(|hive| hive.members.contains(actor))
                    .collect::<Vec<_>>(),
            )?),
            Kind::ListAgents => Ok(serde_json::to_value(coordinator.list_agents()?)?),
            Kind::Read => {
                let rows = if let Some(peer) = args["agent_id"].as_str() {
                    coordinator.read_direct(actor, peer, args["after"].as_u64())?
                } else {
                    coordinator.read_hive(
                        actor,
                        &text("hive_id"),
                        args["after"].as_u64(),
                        args["thread"].as_u64(),
                    )?
                };
                Ok(serde_json::to_value(rows)?)
            }
            Kind::SendHive | Kind::SendAgent => {
                let destination = match self.kind {
                    Kind::SendHive => Destination::Hive(text("hive_id")),
                    _ => Destination::Agent(text("agent_id")),
                };
                Ok(serde_json::to_value(
                    coordinator
                        .send(SendMessage {
                            message_id: text("message_id"),
                            sender: actor.clone(),
                            destination,
                            body: text("body"),
                            thread: args["thread"].as_u64(),
                            only_for: strings(&args, "only_for"),
                            starters: Vec::new(),
                        })
                        .await?,
                )?)
            }
            Kind::Post | Kind::Ask | Kind::Broadcast | Kind::Complete => {
                let body = text("body");
                let action = match self.kind {
                    Kind::Post => EpisodeAction::Post { body },
                    Kind::Ask => EpisodeAction::Ask {
                        body,
                        agents: strings(&args, "agents"),
                    },
                    Kind::Broadcast => EpisodeAction::Broadcast { body },
                    _ => EpisodeAction::Complete { body },
                };
                submit(coordinator, actor, &text("episode_id"), action).await
            }
            Kind::CreateHive | Kind::CreateAgent | Kind::JoinHive | Kind::LeaveHive => {
                host.manage(actor, management(self.kind, &args)?).await
            }
        }
    }
}
/// The send a gated tool's validated arguments describe; `None` for tools the
/// send policy does not gate.
fn outbound(kind: Kind, args: &Value) -> Option<SendRequest> {
    let text = |name: &str| args[name].as_str().unwrap_or_default().to_owned();
    Some(match kind {
        Kind::SendAgent => SendRequest::Agent {
            agent_id: text("agent_id"),
            body: text("body"),
        },
        Kind::SendHive => SendRequest::Hive {
            hive_id: text("hive_id"),
            body: text("body"),
            thread: args["thread"].as_u64(),
            only_for: strings(args, "only_for"),
        },
        Kind::Ask => SendRequest::Ask {
            episode_id: text("episode_id"),
            agents: strings(args, "agents"),
            body: text("body"),
        },
        Kind::Broadcast => SendRequest::Broadcast {
            episode_id: text("episode_id"),
            body: text("body"),
        },
        _ => return None,
    })
}
/// The management request a management tool's validated arguments describe.
fn management(kind: Kind, args: &Value) -> Result<ManagementRequest> {
    let text = |name: &str| args[name].as_str().unwrap_or_default().to_owned();
    Ok(match kind {
        Kind::CreateHive => ManagementRequest::CreateHive(HiveInfo {
            hive_id: text("hive_id"),
            name: text("name"),
            description: args["description"].as_str().map(str::to_owned),
            members: strings(args, "members"),
        }),
        Kind::CreateAgent => ManagementRequest::CreateAgent {
            template: text("template"),
            config: args["config"].clone(),
            memory: args
                .get("memory")
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?,
        },
        Kind::JoinHive => ManagementRequest::JoinHive {
            hive_id: text("hive_id"),
            agent_id: text("agent_id"),
        },
        _ => ManagementRequest::LeaveHive {
            hive_id: text("hive_id"),
            agent_id: text("agent_id"),
        },
    })
}
#[cfg(test)]
mod direct_test;
#[cfg(test)]
mod policy_test;
#[cfg(test)]
mod test;

async fn submit(
    coordinator: &tinyhivemind_hives::Coordinator,
    actor: &str,
    episode: &str,
    action: EpisodeAction,
) -> Result<Value> {
    coordinator.submit_action(actor, episode, action).await?;
    Ok(serde_json::json!({"accepted":true}))
}

fn strings(args: &Value, name: &str) -> Vec<String> {
    args[name]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
