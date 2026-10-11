//! Additive tool gates and correlated approval questions.
use super::{ApprovalSnapshot, BuildOptions, EffectClassifier, PendingRequest};
use crate::{
    TurnScope,
    config::{
        Access, HiveConfig, Membership, PermissionProfile, Profile, Sandbox, Seat, ToolScope,
    },
};
use openhuman_embed::seams::{ToolHookContext, ToolHookDecision};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};
use tinyhivemind_core::{
    approval::{
        Action, ActionTarget, ApprovalDecision, ApprovalPolicy, ApprovalRequest, ApprovalRule,
        ApproverRule, DefaultVerdict, Effect, RuleVerdict, approve,
    },
    desk::{Desk, DeskSet, ResponderMode},
    dispatch::DispatchConversation,
    roster::{Roster, RosterMember},
};
pub(super) struct Permissions {
    pub config: Mutex<HiveConfig>,
    pub active: Mutex<BTreeMap<String, TurnScope>>,
    pub pending: Mutex<BTreeMap<String, PendingRequest>>,
    pub snapshot: Mutex<ApprovalSnapshot>,
    live_context: Option<Arc<dyn super::ApprovalContext>>,
    classifier: Option<Arc<dyn EffectClassifier>>,
    handler: Option<Arc<dyn openhuman_embed::ApprovalHandler>>,
    sequence: AtomicU64,
    pub changed: tokio::sync::Notify,
}
impl Permissions {
    pub(super) fn new(config: HiveConfig, options: &BuildOptions) -> Self {
        Self {
            config: Mutex::new(config),
            active: Mutex::new(BTreeMap::new()),
            pending: Mutex::new(BTreeMap::new()),
            snapshot: Mutex::new(options.approval.clone()),
            live_context: options.approval_context.clone(),
            classifier: options.classifier.clone(),
            handler: options.approval_handler.clone(),
            sequence: AtomicU64::new(0),
            changed: tokio::sync::Notify::new(),
        }
    }
    pub(super) fn config(&self) -> HiveConfig {
        self.config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    pub(super) fn member<'a>(config: &'a HiveConfig, scope: &TurnScope) -> Option<&'a Membership> {
        scope
            .episode
            .as_ref()
            .and_then(|episode| config.hives.iter().find(|hive| hive.id == episode.hive_id))
            .and_then(|hive| {
                hive.members
                    .iter()
                    .find(|member| member.seat == scope.agent_id)
            })
    }
    pub(super) fn decide(
        self: &Arc<Self>,
        seat_id: &str,
        scope: Option<&TurnScope>,
        context: &ToolHookContext,
    ) -> std::future::Ready<ToolHookDecision> {
        std::future::ready(self.decide_now(seat_id, scope, context))
    }
    fn decide_now(
        self: &Arc<Self>,
        seat_id: &str,
        scope: Option<&TurnScope>,
        context: &ToolHookContext,
    ) -> ToolHookDecision {
        let captured = scope.cloned().or_else(|| {
            self.active
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(seat_id)
                .filter(|scope| {
                    scope.session_id.is_some() && scope.session_id == context.session_id
                })
                .cloned()
        });
        let scope = captured.as_ref();
        let config = self.config();
        let Some(seat) = config.seats.iter().find(|seat| seat.id == seat_id) else {
            return deny("unknown seat");
        };
        let Ok(profile) = config.profile_for(seat) else {
            return deny("unknown profile");
        };
        let Ok(layers) =
            config.permission_layers(seat, scope.and_then(|scope| Self::member(&config, scope)))
        else {
            return deny("invalid permission layers");
        };
        let children = seat
            .overrides
            .subagents
            .as_ref()
            .unwrap_or(&profile.subagents);
        if matches!(
            context.tool_name.as_str(),
            "hivemind_ask" | "hivemind_broadcast" | "hivemind_post" | "hivemind_complete"
        ) && scope.is_none()
        {
            return deny("episode action requires authenticated continuing session");
        }
        let effect = if children
            .iter()
            .any(|id| context.tool_name == format!("delegate_{id}"))
        {
            Effect::ReadOnly
        } else {
            classify(context, self.classifier.as_deref())
        };
        if let Err(reason) = basic_gate(&config, seat, profile, &layers, context, scope, effect) {
            return deny(reason);
        }
        let child = match child_policy(&config, seat, profile, context, effect) {
            Ok(child) => child,
            Err(reason) => return deny(reason),
        };
        let rules = config.runtime.tool_rules.iter().chain(
            layers
                .iter()
                .chain(child.iter())
                .filter_map(|policy| policy.tool_rules.as_ref()),
        );
        let force_ask =
            match super::rules::gate(rules, context, seat_id, scope, self.classifier.as_deref()) {
                Ok(ask) => ask,
                Err(reason) => return deny(reason),
            };
        let snapshot = self.live_context.as_ref().map_or_else(
            || {
                self.snapshot
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            },
            |context| context.snapshot(),
        );
        let sequence = self.next_sequence(&snapshot);
        let mut request = request(seat_id, context, captured.as_ref(), &snapshot, sequence);
        request.action.effect = effect;
        let access = layers
            .iter()
            .chain(child.iter())
            .map(|policy| policy.access)
            .min()
            .unwrap_or(config.runtime.autonomy)
            .min(config.runtime.autonomy);
        let policies: Vec<_> = layers
            .iter()
            .chain(child.iter())
            .filter_map(|policy| policy.approval.as_ref())
            .collect();
        match approval_gate(&config, &request, &snapshot, &policies, access) {
            Err(reason) => deny(reason),
            Ok(ask) if ask || force_ask => {
                self.ask(seat_id, context, captured, sequence);
                deny("awaiting correlated host approval; original call refused")
            }
            Ok(_) => ToolHookDecision::Proceed,
        }
    }
    fn next_sequence(&self, snapshot: &ApprovalSnapshot) -> u64 {
        self.live_context.as_ref().map_or_else(
            || {
                self.sequence.fetch_max(
                    snapshot
                        .grants
                        .iter()
                        .map(|grant| grant.granted_at_sequence)
                        .max()
                        .unwrap_or(0),
                    Ordering::Relaxed,
                );
                self.sequence
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1)
            },
            |context| context.next_sequence(),
        )
    }
    fn ask(
        self: &Arc<Self>,
        seat: &str,
        context: &ToolHookContext,
        scope: Option<TurnScope>,
        sequence: u64,
    ) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.values().any(|request| {
            request.request.agent_id.as_deref() == Some(seat)
                && request.request.tool_call_id.as_deref() == Some(&context.call_id)
                && request.scope == scope
        }) {
            return;
        }
        let id = format!("hive-approval-{sequence}");
        let mut native = openhuman_embed::PendingApproval::new(
            &id,
            &context.tool_name,
            "classified action requires approval",
            serde_json::json!({"argument_count":context.arguments.as_object().map_or(0, serde_json::Map::len)}),
            None,
        );
        native.tool_call_id = Some(context.call_id.clone());
        native.agent_id = Some(seat.into());
        pending.insert(
            id.clone(),
            PendingRequest {
                request: native.clone(),
                scope,
                decision: None,
            },
        );
        drop(pending);
        if let Some(handler) = &self.handler {
            let handler = handler.clone();
            let weak = Arc::downgrade(self);
            tokio::spawn(async move {
                let answer = handler.decide(&native).await;
                if let Some(engine) = weak.upgrade() {
                    if let Some(pending) = engine
                        .pending
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get_mut(&id)
                    {
                        pending.decision = Some(answer);
                    }
                    engine.changed.notify_waiters();
                }
            });
        }
    }
}
fn basic_gate(
    config: &HiveConfig,
    seat: &Seat,
    profile: &Profile,
    layers: &[&PermissionProfile],
    context: &ToolHookContext,
    scope: Option<&TurnScope>,
    effect: Effect,
) -> Result<(), &'static str> {
    let children = seat
        .overrides
        .subagents
        .as_ref()
        .unwrap_or(&profile.subagents);
    if context
        .agent_id
        .as_ref()
        .is_none_or(|id| id != &seat.id && !children.contains(id))
    {
        return Err("unauthenticated actor");
    }
    let tool_scope = seat
        .overrides
        .tool_scope
        .as_ref()
        .unwrap_or(&profile.tool_scope);
    if matches!(tool_scope, ToolScope::Named(names) if !names.contains(&context.tool_name)) {
        return Err("tool scope denied");
    }
    let external_mutation = effect == Effect::Mutating && !internal(&context.tool_name);
    let scheduled = scope.is_some_and(|scope| scope.scheduled_job_id.is_some())
        || matches!(
            openhuman_core::agent::turn_origin::current(),
            Some(
                openhuman_core::agent::turn_origin::AgentTurnOrigin::TrustedAutomation {
                    source: openhuman_core::agent::turn_origin::TrustedAutomationSource::Cron,
                    ..
                }
            )
        );
    if scheduled
        && scope.is_none()
        && internal(&context.tool_name)
        && !matches!(
            context.tool_name.as_str(),
            "hivemind_read" | "hivemind_list_hives" | "hivemind_list_agents"
        )
    {
        return Err("isolated cron has no conductor send authority");
    }
    if effect == Effect::Unclassified || scheduled && external_mutation {
        return Err("unknown or scheduled external effect");
    }
    if matches!(tool_scope, ToolScope::HostOnly) && external_mutation {
        return Err("host-only tools must be readonly");
    }
    if layers
        .iter()
        .any(|policy| profile_denies(policy, context, external_mutation))
        || (config.runtime.autonomy == Access::ReadOnly
            || config.runtime.sandbox == Sandbox::ReadOnly)
            && external_mutation
        || profile.deny_tools.contains(&context.tool_name)
        || seat.overrides.deny_tools.contains(&context.tool_name)
    {
        return Err("tool permission denied");
    }
    Ok(())
}
fn profile_denies(
    policy: &PermissionProfile,
    context: &ToolHookContext,
    external_mutation: bool,
) -> bool {
    policy.deny_tools.contains(&context.tool_name)
        || policy
            .allow_tools
            .as_ref()
            .is_some_and(|names| !names.contains(&context.tool_name))
        || (policy.access == Access::ReadOnly || policy.sandbox == Sandbox::ReadOnly)
            && external_mutation
}
fn child_policy<'a>(
    config: &'a HiveConfig,
    seat: &Seat,
    _: &Profile,
    context: &ToolHookContext,
    effect: Effect,
) -> Result<Option<&'a PermissionProfile>, &'static str> {
    let Some(id) = context.agent_id.as_ref().filter(|id| *id != &seat.id) else {
        return Ok(None);
    };
    let Some(child) = config.profiles.iter().find(|profile| &profile.id == id) else {
        return Err("unknown child");
    };
    if child.deny_tools.contains(&context.tool_name)
        || matches!(&child.tool_scope, ToolScope::Named(names) if !names.contains(&context.tool_name))
        || child.tool_scope == ToolScope::HostOnly
            && effect == Effect::Mutating
            && !internal(&context.tool_name)
    {
        return Err("child tool denied");
    }
    let policy = child.permission_profile.as_ref().and_then(|id| {
        config
            .permission_profiles
            .iter()
            .find(|policy| &policy.id == id)
    });
    if policy.is_some_and(|policy| {
        profile_denies(
            policy,
            context,
            effect == Effect::Mutating && !internal(&context.tool_name),
        )
    }) {
        return Err("child permission denied");
    }
    Ok(policy)
}
fn request(
    seat_id: &str,
    context: &ToolHookContext,
    scope: Option<&TurnScope>,
    snapshot: &ApprovalSnapshot,
    sequence: u64,
) -> ApprovalRequest {
    let conversation = scope.map_or(
        DispatchConversation {
            desk_id: format!("direct:{seat_id}"),
            thread_root: None,
        },
        |scope| DispatchConversation {
            desk_id: scope.episode.as_ref().map_or_else(
                || format!("direct:{seat_id}"),
                |episode| episode.hive_id.clone(),
            ),
            thread_root: scope.thread,
        },
    );
    ApprovalRequest {
        epoch: snapshot.epoch,
        sequence,
        call_id: context.call_id.clone(),
        actor_id: seat_id.into(),
        conversation,
        action: Action {
            verb: context.tool_name.clone(),
            target: context
                .arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map_or_else(
                    || ActionTarget::Named {
                        name: context.tool_name.clone(),
                    },
                    |path| ActionTarget::Resource { path: path.into() },
                ),
            effect: classify(context, None),
        },
    }
}
fn approval_gate(
    config: &HiveConfig,
    request: &ApprovalRequest,
    snapshot: &ApprovalSnapshot,
    policies: &[&ApprovalPolicy],
    access: Access,
) -> Result<bool, &'static str> {
    let members: Vec<_> = config
        .seats
        .iter()
        .map(|seat| RosterMember {
            id: seat.id.clone(),
            name: None,
        })
        .collect();
    let roster = Roster::new(&members, &snapshot.people, &[]);
    let desks: Vec<_> = config
        .hives
        .iter()
        .map(|hive| Desk {
            id: hive.id.clone(),
            name: hive.id.clone(),
            description: None,
            members: hive
                .members
                .iter()
                .map(|member| member.seat.clone())
                .collect(),
            responder_mode: ResponderMode::Lead,
        })
        .collect();
    let desks = DeskSet::new(&desks, &[], &[], &[], &[]);
    let fallback = default_policy(access);
    let policies = if policies.is_empty() {
        vec![&fallback]
    } else {
        policies.to_vec()
    };
    let mut ask = false;
    for policy in policies {
        match approve(
            request,
            policy,
            &snapshot.grants,
            &snapshot.refusals,
            &roster,
            &desks,
            snapshot.now,
        ) {
            ApprovalDecision::Deny { .. } => return Err("core approval denied"),
            ApprovalDecision::Ask { .. } => ask = true,
            ApprovalDecision::Allow { .. } => {}
        }
    }
    Ok(ask)
}
pub(super) fn deny(reason: &str) -> ToolHookDecision {
    ToolHookDecision::Deny(reason.into())
}
pub(super) fn internal(name: &str) -> bool {
    matches!(
        name,
        "hivemind_send_agent"
            | "hivemind_send_hive"
            | "hivemind_ask"
            | "hivemind_broadcast"
            | "hivemind_post"
            | "hivemind_complete"
            | "hivemind_read"
            | "hivemind_list_agents"
            | "hivemind_list_hives"
    )
}
pub(super) fn classify(context: &ToolHookContext, custom: Option<&dyn EffectClassifier>) -> Effect {
    if internal(&context.tool_name)
        || matches!(
            context.tool_name.as_str(),
            "read"
                | "read_file"
                | "file_read"
                | "glob"
                | "grep"
                | "ls"
                | "list_files"
                | "recall"
                | "memory_recall"
                | "tool_search"
                | "mcp_list_tools"
                | "mcp_list_servers"
        )
    {
        Effect::ReadOnly
    } else if matches!(
        context.tool_name.as_str(),
        "shell"
            | "bash"
            | "exec"
            | "write"
            | "write_file"
            | "file_write"
            | "edit"
            | "file_edit"
            | "apply_patch"
            | "remember"
            | "memory_remember"
            | "http_request"
            | "fetch"
            | "web_fetch"
            | "hivemind_create_agent"
            | "hivemind_create_hive"
            | "hivemind_join_hive"
            | "hivemind_leave_hive"
    ) {
        Effect::Mutating
    } else {
        custom.map_or(Effect::Unclassified, |classifier| {
            classifier.classify(context)
        })
    }
}
fn default_policy(access: Access) -> ApprovalPolicy {
    ApprovalPolicy {
        enabled: true,
        default: DefaultVerdict::Deny,
        rules: vec![
            ApprovalRule {
                effect: Some(Effect::ReadOnly),
                verb: None,
                target: None,
                verdict: RuleVerdict::Allow,
            },
            ApprovalRule {
                effect: Some(Effect::Mutating),
                verb: None,
                target: None,
                verdict: if access == Access::Autonomous {
                    RuleVerdict::Allow
                } else {
                    RuleVerdict::Deny
                },
            },
        ],
        approver: ApproverRule::Absent,
        allow_grants: false,
        max_grant_ttl: None,
    }
}
