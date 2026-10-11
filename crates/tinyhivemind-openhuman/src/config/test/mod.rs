//! Manifest wire and validation regressions.
use super::*;

fn minimal() -> HiveConfig {
    HiveConfig::from_json(
        r#"{"profiles":[{"id":"worker"}],"seats":[{"id":"alice","profile":"worker"}],"hives":[{"id":"desk","members":[{"seat":"alice","role":"reviewer"}]}]}"#,
    )
    .unwrap_or_else(|error| unreachable!("valid fixture: {error}"))
}

#[test]
fn canonical_manifest_round_trips_and_validates() -> anyhow::Result<()> {
    let config = minimal();
    let encoded = serde_json::to_value(&config)?;
    assert_eq!(encoded["runtime"]["concurrency"], 4);
    assert_eq!(encoded["hives"][0]["members"][0]["role"], "reviewer");
    HiveConfig::from_value(encoded)?.validate()?;
    Ok(())
}

#[test]
fn rejects_inline_provider_secrets_without_echoing_them() {
    let error = HiveConfig::from_json(r#"{"runtime":{"provider":{"credential":"private-value"}}}"#)
        .err()
        .unwrap_or_else(|| unreachable!("literal credentials must be rejected"));
    assert!(matches!(error, ConfigError::InlineSecret { .. }));
    assert!(!format!("{error:?}").contains("private-value"));
}

#[test]
fn resolves_profile_and_seat_references_before_permissions() {
    let mut config = minimal();
    config.seats[0].profile = "missing".into();
    assert!(matches!(
        config.validate(),
        Err(ConfigError::UnknownProfile(_))
    ));
}

#[test]
fn denies_empty_roles_and_width_above_runtime_cap() {
    let mut config = minimal();
    config.hives[0].members[0].role.clear();
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidRole(_))
    ));
    let mut config = minimal();
    config.runtime.concurrency = 1;
    config.hives[0].coordinator.round_width = 2;
    assert!(matches!(
        config.validate(),
        Err(ConfigError::InvalidWidth(_))
    ));
}

#[test]
fn policy_serde_preserves_defaults() -> anyhow::Result<()> {
    use tinyhivemind_core::{driver::ConductPolicy, hive::DivisionPolicy};
    let conduct: ConductPolicy = serde_json::from_str("{}")?;
    assert_eq!(
        serde_json::to_value(conduct)?,
        serde_json::json!({"child_turn_wall":6,"turn_wall":60})
    );
    let division: DivisionPolicy = serde_json::from_str("{}")?;
    assert_eq!(division, DivisionPolicy::default());
    let coordinator: tinyhivemind_hives::CoordinatorOptions = serde_json::from_str("{}")?;
    assert_eq!(coordinator.round_width, 1);
    let retention: tinyhivemind_hives::RetentionPolicy = serde_json::from_str("{}")?;
    assert_eq!(retention, tinyhivemind_hives::RetentionPolicy::default());
    Ok(())
}
mod directory;
mod validation;

#[test]
fn full_fixture_pins_every_configuration_section_and_round_trips() -> anyhow::Result<()> {
    let config = HiveConfig::from_json(include_str!("fixtures/full.json"))?;
    config.clone().validate()?;
    let first = serde_json::to_value(config)?;
    let second = serde_json::to_value(HiveConfig::from_value(first.clone())?)?;
    assert_eq!(first, second);
    assert_eq!(
        first["runtime"]["provider"]["credential"],
        serde_json::json!({"env":"MODEL_KEY"})
    );
    assert_eq!(
        first["runtime"]["mcp"][0]["headers"]["Authorization"],
        serde_json::json!({"store":"docs-key"})
    );
    assert_eq!(
        first["profiles"][0]["tool_scope"],
        serde_json::json!({"named":["Read"]})
    );
    assert_eq!(
        first["workflows"][0]["target"],
        serde_json::json!({"hive":"desk"})
    );
    assert_eq!(first["hives"][0]["episode"]["round_width"], 1);
    Ok(())
}
