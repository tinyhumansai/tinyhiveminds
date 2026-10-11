//! Every typed configuration failure and conservative authority dimension.
use super::*;
#[test]
fn malformed_json_and_unknown_fields_are_typed() {
    for input in ["{", r#"{"unrecognized":true}"#] {
        assert!(matches!(
            HiveConfig::from_json(input),
            Err(ConfigError::Json)
        ));
    }
}
#[test]
fn duplicate_ids_are_rejected_in_each_namespace() {
    let mut c = minimal();
    c.profiles.push(c.profiles[0].clone());
    assert!(matches!(c.validate(), Err(ConfigError::DuplicateId { .. })));
    let mut c = minimal();
    c.permission_profiles = vec![
        PermissionProfile {
            id: "p".into(),
            ..Default::default()
        };
        2
    ];
    assert!(matches!(c.validate(), Err(ConfigError::DuplicateId { .. })));
    let mut c = minimal();
    c.seats.push(c.seats[0].clone());
    assert!(matches!(c.validate(), Err(ConfigError::DuplicateId { .. })));
    let mut c = minimal();
    c.hives.push(c.hives[0].clone());
    assert!(matches!(c.validate(), Err(ConfigError::DuplicateId { .. })));
    let mut c = minimal();
    let repeated = c.hives[0].members[0].clone();
    c.hives[0].members.push(repeated);
    assert!(matches!(c.validate(), Err(ConfigError::DuplicateId { .. })));
    let mut c = minimal();
    c.workflows = vec![job(); 2];
    assert!(matches!(c.validate(), Err(ConfigError::DuplicateId { .. })));
}
fn job() -> Workflow {
    Workflow {
        id: "daily".into(),
        schedule: "0 8 * * *".into(),
        target: WorkflowTarget::Seat("alice".into()),
        prompt: "review".into(),
        enabled: true,
        retries: 0,
        single_flight: true,
    }
}
#[test]
fn missing_references_are_typed() {
    let mut c = minimal();
    c.profiles[0].permission_profile = Some("missing".into());
    assert!(matches!(
        c.validate(),
        Err(ConfigError::UnknownPermissionProfile(_))
    ));
    let mut c = minimal();
    c.profiles[0].subagents.push("missing".into());
    assert!(matches!(c.validate(), Err(ConfigError::UnknownProfile(_))));
    let mut c = minimal();
    c.hives[0].members[0].seat = "missing".into();
    assert!(matches!(c.validate(), Err(ConfigError::UnknownSeat(_))));
    let mut c = minimal();
    c.workflows = vec![job()];
    c.workflows[0].target = WorkflowTarget::Hive("missing".into());
    assert!(matches!(c.validate(), Err(ConfigError::UnknownHive(_))));
    let mut c = minimal();
    c.profiles[0].context = vec!["missing".into()];
    assert!(matches!(c.validate(), Err(ConfigError::UnknownContext(_))));
}
#[test]
fn empty_hives_bad_ids_limits_and_selectors_are_typed() {
    let mut c = minimal();
    c.hives[0].members.clear();
    assert!(matches!(c.validate(), Err(ConfigError::EmptyHive(_))));
    for id in ["", "UPPER", "../path", "bad.id"] {
        let mut c = minimal();
        c.seats[0].id = id.into();
        c.hives[0].members[0].seat = id.into();
        assert!(matches!(c.validate(), Err(ConfigError::InvalidId(_))));
    }
    let mut c = minimal();
    c.runtime.limits.timeout_ms = Some(0);
    assert!(matches!(c.validate(), Err(ConfigError::InvalidLimit(_))));
    let mut c = minimal();
    c.profiles[0].limits.iterations = Some(0);
    assert!(matches!(c.validate(), Err(ConfigError::InvalidLimit(_))));
    let mut c = minimal();
    c.seats[0].overrides.limits.budget_usd = Some(-1.0);
    assert!(matches!(c.validate(), Err(ConfigError::InvalidLimit(_))));
    let mut c = minimal();
    c.runtime.memory_engine = "invented".into();
    assert!(matches!(
        c.validate(),
        Err(ConfigError::UnsupportedSetting(_))
    ));
    let mut c = minimal();
    c.workflows = vec![job()];
    c.workflows[0].schedule = "90 90 * * *".into();
    assert!(matches!(c.validate(), Err(ConfigError::InvalidWorkflow(_))));
}
#[test]
fn permission_restrictions_allow_subsets_and_reject_each_widening() {
    let parent = PermissionProfile {
        id: "baseline".into(),
        access: Access::ReadOnly,
        sandbox: Sandbox::ReadOnly,
        allow_tools: Some(vec!["Read".into(), "List".into()]),
        deny_tools: vec!["Write".into()],
        send_destinations: Some(vec!["desk".into()]),
        send_operations: Some(vec!["send".into()]),
        ..Default::default()
    };
    let child = PermissionProfile {
        id: "narrow".into(),
        allow_tools: Some(vec!["Read".into()]),
        ..parent.clone()
    };
    let setup = |child: PermissionProfile| {
        let mut c = minimal();
        c.runtime.permission_profile = Some("baseline".into());
        c.hives[0].members[0].permission_profile = Some("narrow".into());
        c.permission_profiles = vec![parent.clone(), child];
        c
    };
    setup(child.clone())
        .validate()
        .unwrap_or_else(|error| unreachable!("valid fixture: {error}"));
    let mut variants = vec![];
    let mut c = child.clone();
    c.access = Access::Autonomous;
    variants.push(c);
    let mut c = child.clone();
    c.sandbox = Sandbox::None;
    variants.push(c);
    let mut c = child.clone();
    c.allow_tools = Some(vec!["Write".into()]);
    variants.push(c);
    let mut c = child.clone();
    c.deny_tools.clear();
    variants.push(c);
    let mut c = child.clone();
    c.send_destinations = Some(vec!["other".into()]);
    variants.push(c);
    let mut c = child;
    c.send_operations = Some(vec!["broadcast".into()]);
    variants.push(c);
    for variant in variants {
        assert!(matches!(
            setup(variant).validate(),
            Err(ConfigError::PermissionWidening(_))
        ));
    }
}
fn server(id: &str) -> McpServer {
    McpServer {
        id: id.into(),
        transport: "http".into(),
        endpoint: format!("https://{id}.example.test"),
        args: vec![],
        env: std::collections::BTreeMap::default(),
        headers: std::collections::BTreeMap::default(),
        tools: Some(vec!["Read".into(), "List".into()]),
        deny_tools: vec![],
    }
}
#[test]
fn immutable_mcp_normalizes_order_and_rejects_connection_changes() {
    let mut c = minimal();
    c.runtime.mcp = vec![server("a"), server("b")];
    c.hives[0].members[0].mcp = Some(vec![server("b"), server("a")]);
    c.validate()
        .unwrap_or_else(|error| unreachable!("valid fixture: {error}"));
    let mut c = minimal();
    c.runtime.mcp = vec![server("a")];
    let mut changed = server("a");
    changed.endpoint = "https://elsewhere.test".into();
    c.hives[0].members[0].mcp = Some(vec![changed]);
    assert!(matches!(c.validate(), Err(ConfigError::ConflictingMcp(_))));
}
#[test]
fn zero_temperature_valid_and_parent_limits_cannot_widen() {
    let mut c = minimal();
    c.profiles[0].model.temperature = Some(0.0);
    c.validate()
        .unwrap_or_else(|error| unreachable!("valid fixture: {error}"));
    let mut c = minimal();
    c.runtime.limits.iterations = Some(4);
    c.profiles[0].limits.iterations = Some(5);
    assert!(matches!(
        c.validate(),
        Err(ConfigError::PermissionWidening(_))
    ));
}

#[test]
fn approval_grants_and_rules_cannot_be_replaced_with_permissive_defaults() {
    use tinyhivemind_core::approval::{ApprovalPolicy, ApproverRule, DefaultVerdict};
    let policy = ApprovalPolicy {
        enabled: true,
        default: DefaultVerdict::Deny,
        rules: vec![],
        approver: ApproverRule::Absent,
        allow_grants: false,
        max_grant_ttl: None,
    };
    let parent = PermissionProfile {
        id: "parent".into(),
        approval: Some(policy.clone()),
        ..Default::default()
    };
    let child = PermissionProfile {
        id: "child".into(),
        approval: Some(policy),
        ..Default::default()
    };
    let setup = |child: PermissionProfile| {
        let mut c = minimal();
        c.runtime.permission_profile = Some("parent".into());
        c.hives[0].members[0].permission_profile = Some("child".into());
        c.permission_profiles = vec![parent.clone(), child];
        c
    };
    setup(child.clone())
        .validate()
        .unwrap_or_else(|error| unreachable!("valid fixture: {error}"));
    let mut grant = child.clone();
    grant
        .approval
        .as_mut()
        .unwrap_or_else(|| unreachable!("fixture approval is present"))
        .allow_grants = true;
    assert!(matches!(
        setup(grant).validate(),
        Err(ConfigError::PermissionWidening(_))
    ));
    let mut fallback = child.clone();
    fallback
        .approval
        .as_mut()
        .unwrap_or_else(|| unreachable!("fixture approval is present"))
        .default = DefaultVerdict::Ask;
    assert!(matches!(
        setup(fallback).validate(),
        Err(ConfigError::PermissionWidening(_))
    ));
    let mut approver = child;
    approver
        .approval
        .as_mut()
        .unwrap_or_else(|| unreachable!("fixture approval is present"))
        .approver = ApproverRule::Person { id: "other".into() };
    assert!(matches!(
        setup(approver).validate(),
        Err(ConfigError::PermissionWidening(_))
    ));
}
#[test]
fn omitted_intermediate_allowlist_does_not_erase_ancestor_restrictions() {
    let mut c = minimal();
    c.runtime.permission_profile = Some("parent".into());
    c.profiles[0].permission_profile = Some("middle".into());
    c.hives[0].members[0].permission_profile = Some("child".into());
    c.permission_profiles = vec![
        PermissionProfile {
            id: "parent".into(),
            allow_tools: Some(vec!["Read".into()]),
            ..Default::default()
        },
        PermissionProfile {
            id: "middle".into(),
            ..Default::default()
        },
        PermissionProfile {
            id: "child".into(),
            allow_tools: Some(vec!["Write".into()]),
            ..Default::default()
        },
    ];
    assert!(matches!(
        c.validate(),
        Err(ConfigError::PermissionWidening(_))
    ));
}

#[test]
fn host_only_cannot_replace_finite_names_or_connect_mcp() {
    let mut c = minimal();
    c.profiles[0].tool_scope = ToolScope::Named(vec!["Read".into()]);
    c.seats[0].overrides.tool_scope = Some(ToolScope::HostOnly);
    assert!(matches!(
        c.validate(),
        Err(ConfigError::PermissionWidening(_))
    ));
    let mut c = minimal();
    c.profiles[0].tool_scope = ToolScope::HostOnly;
    c.runtime.mcp = vec![server("a")];
    assert!(matches!(
        c.validate(),
        Err(ConfigError::UnsupportedSetting(_))
    ));
}

#[test]
fn scheduled_prompts_must_be_nonempty() {
    let mut c = minimal();
    let mut workflow = job();
    workflow.prompt = "  ".into();
    c.workflows = vec![workflow];
    assert!(matches!(c.validate(), Err(ConfigError::InvalidWorkflow(_))));
}

#[test]
fn blind_and_revealed_widths_are_independently_bounded() -> anyhow::Result<()> {
    let mut config = minimal();
    config.runtime.concurrency = 4;
    config.hives[0].episode = tinyhivemind_core::hive::EpisodePolicy::default();
    assert_eq!(config.hives[0].episode.round_width, 4);
    assert_eq!(config.hives[0].episode.revealed_width, 1);
    config.clone().validate()?;
    config.hives[0].episode.round_width = 5;
    assert!(matches!(
        config.clone().validate(),
        Err(ConfigError::InvalidWidth(_))
    ));
    config.hives[0].episode.round_width = 4;
    config.hives[0].episode.revealed_width = 5;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidWidth(_))
    ));
    Ok(())
}
