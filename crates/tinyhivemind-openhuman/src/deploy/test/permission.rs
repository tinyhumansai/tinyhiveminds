//! Pure approval integration, tool narrowing and trusted scheduled effects.
use super::support::*;
use crate::deploy::*;
use openhuman_embed::seams::ToolHookDecision;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[tokio::test]
async fn tool_allowed_in_a_is_denied_in_b_and_under_scheduled_origin() -> anyhow::Result<()> {
    let engine = permissions(manifest()?, &BuildOptions::default());
    assert!(matches!(
        engine
            .decide("alice", Some(&scope("a")), &context("write"))
            .await,
        ToolHookDecision::Proceed
    ));
    assert!(matches!(
        engine
            .decide("alice", Some(&scope("b")), &context("write"))
            .await,
        ToolHookDecision::Deny(_)
    ));
    let mut scheduled = scope("a");
    scheduled.scheduled_job_id = Some("job".into());
    for name in ["write", "shell", "http_request"] {
        assert!(matches!(
            engine
                .decide("alice", Some(&scheduled), &context(name))
                .await,
            ToolHookDecision::Deny(_)
        ));
    }
    assert!(matches!(
        engine
            .decide("alice", Some(&scheduled), &context("hivemind_complete"))
            .await,
        ToolHookDecision::Proceed
    ));
    assert!(matches!(
        engine
            .decide("alice", Some(&scope("a")), &context("unrecognized"))
            .await,
        ToolHookDecision::Deny(_)
    ));
    let mut forged = context("read");
    forged.agent_id = Some("outsider".into());
    assert!(matches!(
        engine.decide("alice", None, &forged).await,
        ToolHookDecision::Deny(_)
    ));
    Ok(())
}
struct Handler(AtomicUsize);
#[async_trait::async_trait]
impl openhuman_embed::ApprovalHandler for Handler {
    async fn decide(
        &self,
        request: &openhuman_embed::PendingApproval,
    ) -> openhuman_embed::ApprovalDecision {
        self.0.fetch_add(1, Ordering::Relaxed);
        assert_eq!(request.tool_call_id.as_deref(), Some("call"));
        assert!(!format!("{request:?}").contains("secret-user-payload"));
        openhuman_embed::ApprovalDecision::ApproveOnce
    }
}
#[tokio::test]
async fn ask_invokes_an_explicit_correlated_handler_and_original_call_is_denied()
-> anyhow::Result<()> {
    let mut config = manifest()?;
    config
        .permission_profiles
        .push(crate::config::PermissionProfile {
            id: "ask".into(),
            access: crate::config::Access::Autonomous,
            approval: Some(tinyhivemind_core::approval::ApprovalPolicy {
                enabled: true,
                default: tinyhivemind_core::approval::DefaultVerdict::Ask,
                rules: Vec::new(),
                approver: tinyhivemind_core::approval::ApproverRule::Person { id: "human".into() },
                allow_grants: false,
                max_grant_ttl: None,
            }),
            ..Default::default()
        });
    config.runtime.permission_profile = Some("ask".into());
    let handler = Arc::new(Handler(AtomicUsize::new(0)));
    let options = BuildOptions {
        approval_handler: Some(handler.clone()),
        approval: ApprovalSnapshot {
            people: vec![tinyhivemind_core::roster::Person {
                id: "human".into(),
                label: "Human".into(),
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = permissions(config, &options);
    let turn = scope("a");
    assert!(matches!(
        engine.decide("alice", Some(&turn), &context("write")).await,
        ToolHookDecision::Deny(_)
    ));
    let pending = engine
        .pending
        .lock()
        .map_err(|_| anyhow::anyhow!("poison"))?
        .values()
        .next()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing question"))?;
    assert_eq!(pending.scope, Some(turn));
    let changed = engine.changed.notified();
    if engine
        .pending
        .lock()
        .map_err(|_| anyhow::anyhow!("poison"))?
        .values()
        .any(|pending| pending.decision.is_none())
    {
        changed.await;
    }
    assert_eq!(handler.0.load(Ordering::Relaxed), 1);
    assert!(engine.pending.lock().map_err(|_| anyhow::anyhow!("poison"))?.values().all(|pending| pending.decision == Some(openhuman_embed::ApprovalDecision::ApproveOnce)));
    Ok(())
}
#[tokio::test]
async fn readonly_baseline_and_argument_rules_survive_turn_allows() -> anyhow::Result<()> {
    let mut config = manifest()?;
    config.runtime.autonomy = crate::config::Access::ReadOnly;
    let engine = permissions(config, &BuildOptions::default());
    assert!(matches!(
        engine
            .decide("alice", Some(&scope("a")), &context("write"))
            .await,
        ToolHookDecision::Deny(_)
    ));
    assert!(matches!(
        engine.decide("alice", None, &context("read")).await,
        ToolHookDecision::Proceed
    ));
    let mut config = manifest()?;
    config.runtime.tool_rules = Some(tinytools::ToolRules::deny_all());
    let engine = permissions(config, &BuildOptions::default());
    assert!(matches!(
        engine.decide("alice", None, &context("read")).await,
        ToolHookDecision::Deny(_)
    ));
    Ok(())
}
#[tokio::test]
async fn argument_rules_and_unknown_metadata_cannot_be_bypassed() -> anyhow::Result<()> {
    let mut config = manifest()?;
    config.runtime.tool_rules = Some(serde_json::from_value(
        serde_json::json!({"rules":[{"effect":"deny","match":{"name":"read","arg":{"pointer":"/path","value":"secret*"}}}]}),
    )?);
    let engine = permissions(config, &BuildOptions::default());
    assert!(matches!(
        engine
            .decide("alice", Some(&scope("a")), &context("read"))
            .await,
        ToolHookDecision::Deny(_)
    ));
    let mut allowed = context("read");
    allowed.arguments = serde_json::json!({"path":"public"});
    assert!(matches!(
        engine.decide("alice", Some(&scope("a")), &allowed).await,
        ToolHookDecision::Proceed
    ));
    for matcher in [
        serde_json::json!({"family":"private"}),
        serde_json::json!({"tags":"sensitive"}),
        serde_json::json!({"permission_at_least":"Write"}),
        serde_json::json!({"side_effects":["writes_files"]}),
    ] {
        let mut config = manifest()?;
        config.runtime.tool_rules = Some(serde_json::from_value(
            serde_json::json!({"rules":[{"effect":"deny","match":matcher}]}),
        )?);
        let engine = permissions(config, &BuildOptions::default());
        assert!(matches!(
            engine
                .decide("alice", Some(&scope("a")), &context("read"))
                .await,
            ToolHookDecision::Deny(_)
        ));
    }
    Ok(())
}
#[tokio::test]
async fn isolated_session_never_captures_active_hive_or_episode_authority() -> anyhow::Result<()> {
    let engine = permissions(manifest()?, &BuildOptions::default());
    engine
        .active
        .lock()
        .map_err(|_| anyhow::anyhow!("poison"))?
        .insert("alice".into(), scope("b"));
    assert!(matches!(
        engine.decide("alice", None, &context("write")).await,
        ToolHookDecision::Deny(_)
    ));
    let mut isolated = context("write");
    isolated.session_id = Some("cron-isolated".into());
    assert!(matches!(
        engine.decide("alice", None, &isolated).await,
        ToolHookDecision::Proceed
    ));
    isolated.tool_name = "hivemind_complete".into();
    assert!(matches!(
        engine.decide("alice", None, &isolated).await,
        ToolHookDecision::Deny(_)
    ));
    Ok(())
}
#[tokio::test]
async fn declared_delegates_are_internal_but_child_rules_still_narrow_effects() -> anyhow::Result<()>
{
    let mut config = manifest()?;
    config.profiles.push(crate::config::Profile {
        id: "helper".into(),
        deny_tools: vec!["write".into()],
        ..Default::default()
    });
    config.profiles[0].subagents.push("helper".into());
    let engine = permissions(config, &BuildOptions::default());
    assert!(matches!(
        engine
            .decide("alice", Some(&scope("a")), &context("delegate_helper"))
            .await,
        ToolHookDecision::Proceed
    ));
    assert!(matches!(
        engine
            .decide("alice", Some(&scope("a")), &context("delegate_unknown"))
            .await,
        ToolHookDecision::Deny(_)
    ));
    let mut child = context("write");
    child.agent_id = Some("helper".into());
    assert!(matches!(
        engine.decide("alice", Some(&scope("a")), &child).await,
        ToolHookDecision::Deny(_)
    ));
    child.tool_name = "read".into();
    assert!(matches!(
        engine.decide("alice", Some(&scope("a")), &child).await,
        ToolHookDecision::Proceed
    ));
    Ok(())
}
