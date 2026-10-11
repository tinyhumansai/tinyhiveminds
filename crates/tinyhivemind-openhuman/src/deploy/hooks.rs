//! Captured membership context, turn gates and outbound send restrictions.
use super::{BuildOptions, Permissions, profile};
use crate::{HostedTurn, SendAuthorizer, SendRequest, TurnHooks, TurnOptions, TurnScope};
use std::{
    sync::{Arc, PoisonError},
    time::Duration,
};
use tinyhivemind_hives::TurnDisposition;
pub(super) struct Hooks {
    pub permissions: Arc<Permissions>,
    pub extra: Option<Arc<dyn TurnHooks>>,
    pub call: Option<openhuman_embed::budget::CallBudget>,
}
impl Hooks {
    pub(super) fn new(permissions: Arc<Permissions>, options: &BuildOptions) -> Self {
        Self {
            permissions,
            extra: options.hooks.clone(),
            call: options.call_budget,
        }
    }
}
impl TurnHooks for Hooks {
    fn context(&self, scope: &TurnScope) -> String {
        self.permissions
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(scope.agent_id.clone(), scope.clone());
        let config = self.permissions.config();
        let mut text = String::new();
        if let Some(member) = Permissions::member(&config, scope) {
            for id in &member.context {
                if let Some(context) = config.contexts.get(id) {
                    text.push_str(context);
                    text.push('\n');
                }
            }
        }
        if let Some(extra) = &self.extra {
            text.push_str(&extra.context(scope));
        }
        text
    }
    fn prepare(&self, scope: &TurnScope) -> TurnOptions {
        self.extra
            .as_ref()
            .map_or_else(TurnOptions::default, |extra| extra.prepare(scope))
    }
    fn turn_timeout(&self, scope: &TurnScope) -> Option<Duration> {
        let config = self.permissions.config();
        let configured = config
            .seats
            .iter()
            .find(|seat| seat.id == scope.agent_id)
            .and_then(|seat| profile::limits(&config, seat).ok())
            .and_then(|limits| limits.timeout_ms)
            .map(Duration::from_millis);
        let extra = self
            .extra
            .as_ref()
            .and_then(|hooks| hooks.turn_timeout(scope));
        match (configured, extra) {
            (Some(configured), Some(extra)) => Some(configured.min(extra)),
            (configured, extra) => configured.or(extra),
        }
    }
    fn configure(
        &self,
        scope: &TurnScope,
        mut turn: openhuman_embed::Turn,
    ) -> openhuman_embed::Turn {
        let config = self.permissions.config();
        if let Some(seat) = config.seats.iter().find(|seat| seat.id == scope.agent_id)
            && let Ok(limits) = profile::limits(&config, seat)
            && let (Some(usd), Some(call)) = (limits.budget_usd, self.call)
        {
            turn = turn.budget(openhuman_embed::budget::ModelBudget {
                ledger: openhuman_embed::budget::Budget::new(
                    openhuman_embed::budget::SpendLimits {
                        tokens: None,
                        cost_micros: super::runtime::budget_micros(usd),
                    },
                ),
                call,
            });
        }
        let permissions = self.permissions.clone();
        let captured = scope.clone();
        turn = turn.can_use_tool(move |context| {
            let permissions = permissions.clone();
            let captured = captured.clone();
            Box::pin(async move {
                permissions
                    .decide(&captured.agent_id, Some(&captured), context)
                    .await
            })
        });
        match &self.extra {
            Some(extra) => extra.configure(scope, turn),
            None => turn,
        }
    }
    fn progress(&self, scope: &TurnScope) -> Option<crate::TurnProgressSink> {
        self.extra.as_ref().and_then(|extra| extra.progress(scope))
    }
    fn wrap_turn<'a>(&'a self, scope: &'a TurnScope, turn: HostedTurn<'a>) -> HostedTurn<'a> {
        match &self.extra {
            Some(extra) => extra.wrap_turn(scope, turn),
            None => turn,
        }
    }
    fn after_turn(
        &self,
        scope: &TurnScope,
        usage: Option<&openhuman_core::agent::tinyagents::host::LastTurnUsage>,
    ) -> crate::Result<TurnDisposition> {
        self.permissions
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&scope.agent_id);
        let extra = self
            .extra
            .as_ref()
            .map(|extra| extra.after_turn(scope, usage))
            .transpose()?
            .unwrap_or(TurnDisposition::Completed);
        if self
            .permissions
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .any(|pending| pending.scope.as_ref() == Some(scope))
        {
            Ok(TurnDisposition::Parked)
        } else {
            Ok(extra)
        }
    }
}
impl SendAuthorizer for Hooks {
    fn authorize(&self, actor: &str, request: &SendRequest) -> crate::Result<()> {
        self.authorize_in_session(actor, request, None)
    }
    fn authorize_in_session(
        &self,
        actor: &str,
        request: &SendRequest,
        session: Option<&str>,
    ) -> crate::Result<()> {
        let config = self.permissions.config();
        let seat = config
            .seats
            .iter()
            .find(|seat| seat.id == actor)
            .ok_or_else(|| crate::Error::SendDenied("unknown seat".into()))?;
        let active = self
            .permissions
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(actor)
            .cloned();
        if active
            .as_ref()
            .is_some_and(|scope| scope.session_id.as_deref() != session)
        {
            return Err(crate::Error::SendDenied(
                "send belongs to another active session".into(),
            ));
        }
        let member = active
            .as_ref()
            .and_then(|scope| Permissions::member(&config, scope));
        let (operation, destinations) = match request {
            SendRequest::Agent { agent_id, .. } => ("agent", vec![agent_id.as_str()]),
            SendRequest::Hive { hive_id, .. } => ("hive", vec![hive_id.as_str()]),
            SendRequest::Ask {
                episode_id, agents, ..
            } => {
                if active
                    .as_ref()
                    .and_then(|scope| scope.episode.as_ref())
                    .is_none_or(|episode| &episode.episode_id != episode_id)
                {
                    return Err(crate::Error::SendDenied("inactive episode".into()));
                }
                ("ask", agents.iter().map(String::as_str).collect())
            }
            SendRequest::Broadcast { episode_id, .. } => {
                let episode = active
                    .as_ref()
                    .and_then(|scope| scope.episode.as_ref())
                    .filter(|episode| &episode.episode_id == episode_id)
                    .ok_or_else(|| crate::Error::SendDenied("inactive episode".into()))?;
                ("broadcast", vec![episode.hive_id.as_str()])
            }
        };
        for policy in config
            .permission_layers(seat, member)
            .map_err(|_| crate::Error::SendDenied("invalid policy".into()))?
        {
            if policy
                .send_operations
                .as_ref()
                .is_some_and(|names| !names.iter().any(|name| name == operation))
                || policy.send_destinations.as_ref().is_some_and(|names| {
                    destinations
                        .iter()
                        .any(|id| !names.iter().any(|name| name == id))
                })
            {
                return Err(crate::Error::SendDenied("send permission denied".into()));
            }
        }
        Ok(())
    }
}
