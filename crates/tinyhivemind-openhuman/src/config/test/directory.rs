//! Relative Markdown/frontmatter loading and safe credential references.
use super::*;
use std::fs;
#[test]
fn directory_loads_body_context_and_yaml_with_relative_paths() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join("profiles"))?;
    fs::create_dir(dir.path().join("context"))?;
    fs::write(dir.path().join("hive.json"), "{}")?;
    fs::write(
        dir.path().join("profiles/reader.md"),
        "---\ncontext: [task]\nmodel:\n  temperature: 0\n---\nReview carefully.\n---\nOrdinary separator.",
    )?;
    fs::write(
        dir.path().join("context/task.md"),
        "---\ntitle: Task\n---\nTask details.",
    )?;
    let c = HiveConfig::load_dir(dir.path())?;
    assert_eq!(c.profiles[0].id, "reader");
    assert!(c.profiles[0].system_prompt.contains("\n---\n"));
    assert_eq!(c.contexts["task"], "Task details.");
    c.validate()?;
    Ok(())
}
#[test]
fn missing_files_malformed_frontmatter_and_duplicate_profiles_are_typed() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    assert!(matches!(
        HiveConfig::load_dir(dir.path()),
        Err(ConfigError::Io { .. })
    ));
    fs::write(dir.path().join("hive.json"), "{}")?;
    fs::create_dir(dir.path().join("profiles"))?;
    for body in ["---\nid: a", "---\ninvalid: [\n---\nbody"] {
        fs::write(dir.path().join("profiles/a.md"), body)?;
        assert!(matches!(
            HiveConfig::load_dir(dir.path()),
            Err(ConfigError::Frontmatter(_))
        ));
    }
    fs::write(dir.path().join("profiles/a.md"), "body")?;
    fs::write(dir.path().join("hive.json"), r#"{"profiles":[{"id":"a"}]}"#)?;
    assert!(matches!(
        HiveConfig::load_dir(dir.path()),
        Err(ConfigError::DuplicateId { .. })
    ));
    Ok(())
}
#[test]
fn credentials_have_exact_reference_wire_forms() -> anyhow::Result<()> {
    assert_eq!(
        serde_json::to_value(SecretRef::Env("KEY".into()))?,
        serde_json::json!({"env":"KEY"})
    );
    assert_eq!(
        serde_json::to_value(SecretRef::Store("key".into()))?,
        serde_json::json!({"store":"key"})
    );
    HiveConfig::from_json(
        r#"{"runtime":{"provider":{"credential":{"env":"KEY"}},"mcp":[{"id":"a","transport":"http","endpoint":"http://localhost","headers":{"Authorization":{"store":"token"}}}]}}"#,
    )?;
    assert!(matches!(
        HiveConfig::from_json(r#"{"runtime":{"mcp":[{"env":{"SECRET":"literal"}}]}}"#),
        Err(ConfigError::InlineSecret { .. })
    ));
    assert!(matches!(
        HiveConfig::from_json(r#"{"seats":[{"overrides":{"api_key":"literal"}}]}"#),
        Err(ConfigError::InlineSecret { .. })
    ));
    Ok(())
}

#[test]
fn provider_urls_cannot_smuggle_literal_credentials() {
    for endpoint in [
        "https://user:password@example.test/v1",
        "https://example.test?api_key=literal",
    ] {
        let value = serde_json::json!({"runtime":{"provider":{"endpoint":endpoint}}});
        assert!(matches!(
            HiveConfig::from_value(value),
            Err(ConfigError::InlineSecret { .. })
        ));
    }
}

#[test]
fn native_policy_unknown_fields_are_rejected_only_at_manifest_boundary() -> anyhow::Result<()> {
    for policy in ["episode", "routing", "conduct", "division", "coordinator"] {
        let mut value = serde_json::to_value(minimal())?;
        value["hives"][0][policy]["unknown_setting"] = serde_json::json!(true);
        assert!(matches!(
            HiveConfig::from_value(value),
            Err(ConfigError::Json)
        ));
    }
    Ok(())
}
#[test]
fn empty_secret_reference_and_mutated_credential_url_are_rejected() {
    assert!(matches!(
        HiveConfig::from_json(r#"{"runtime":{"provider":{"credential":{"env":""}}}}"#),
        Err(ConfigError::InlineSecret { .. })
    ));
    let mut c = minimal();
    c.runtime.provider.endpoint = Some("https://a:b@example.test".into());
    assert!(matches!(
        c.validate(),
        Err(ConfigError::InlineSecret { .. })
    ));
}

#[test]
fn unknown_tool_rules_key_and_approval_fields_cannot_bypass_strict_parsing() -> anyhow::Result<()> {
    let base = serde_json::to_value(minimal())?;
    for policy in ["episode", "routing"] {
        let mut value = base.clone();
        value["hives"][0][policy]["tool_rules"] = serde_json::json!({});
        assert!(matches!(
            HiveConfig::from_value(value),
            Err(ConfigError::Json)
        ));
    }
    let approval = serde_json::json!({"enabled":true,"default":"deny","rules":[],"approver":{"kind":"absent"},"allow_grants":false,"max_grant_ttl":null});
    let mut value = base.clone();
    value["permission_profiles"] = serde_json::json!([{"id":"p","approval":approval.clone()}]);
    value["permission_profiles"][0]["approval"]["unknown"] = serde_json::json!(true);
    assert!(matches!(
        HiveConfig::from_value(value),
        Err(ConfigError::Json)
    ));
    let mut value = base;
    value["permission_profiles"] = serde_json::json!([{"id":"p","approval":approval}]);
    value["permission_profiles"][0]["approval"]["rules"] = serde_json::json!([{"effect":null,"verb":null,"target":null,"verdict":"deny","tool_rules":{}}]);
    assert!(matches!(
        HiveConfig::from_value(value),
        Err(ConfigError::Json)
    ));
    Ok(())
}

#[test]
fn directory_readmes_do_not_become_profiles_or_context() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join("profiles"))?;
    fs::create_dir(dir.path().join("context"))?;
    fs::write(dir.path().join("hive.json"), "{}")?;
    for name in ["README.md", "readme.md", "ReadMe.md"] {
        fs::write(dir.path().join("profiles").join(name), "---\ninvalid: [")?;
        fs::write(dir.path().join("context").join(name), "---\ninvalid: [")?;
    }
    fs::write(dir.path().join("profiles/reader.md"), "Read carefully.")?;
    fs::write(
        dir.path().join("context/readme-example.md"),
        "Task details.",
    )?;
    let config = crate::config::load_dir(dir.path())?;
    assert_eq!(config.profiles.len(), 1);
    assert_eq!(config.profiles[0].id, "reader");
    assert_eq!(config.contexts.len(), 1);
    assert_eq!(config.contexts["readme-example"], "Task details.");
    Ok(())
}

#[test]
fn directory_rejects_typed_frontmatter_and_context_collisions() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("hive.json"), "{}")?;
    fs::create_dir(dir.path().join("profiles"))?;
    for body in [
        "---",
        "---\niterations: 1\n---\nbody",
        "---\nid: []\n---\nbody",
    ] {
        fs::write(dir.path().join("profiles/a.md"), body)?;
        assert!(matches!(
            crate::config::load_dir(dir.path()),
            Err(ConfigError::Frontmatter(_))
        ));
    }
    fs::write(
        dir.path().join("profiles/a.md"),
        "---\napi_key: literal\n---\nbody",
    )?;
    assert!(matches!(
        crate::config::load_dir(dir.path()),
        Err(ConfigError::InlineSecret { .. })
    ));
    fs::write(dir.path().join("profiles/a.md"), "")?;
    fs::create_dir(dir.path().join("context"))?;
    fs::write(
        dir.path().join("context/task.md"),
        "---\ninvalid: [\n---\nbody",
    )?;
    assert!(matches!(
        crate::config::load_dir(dir.path()),
        Err(ConfigError::Frontmatter(_))
    ));
    fs::write(dir.path().join("context/task.md"), "body")?;
    fs::write(
        dir.path().join("hive.json"),
        r#"{"contexts":{"task":"inline"}}"#,
    )?;
    assert!(
        matches!(crate::config::load_dir(dir.path()), Err(ConfigError::DuplicateId {section, ..}) if section == "context")
    );
    fs::remove_file(dir.path().join("profiles/a.md"))?;
    fs::remove_dir(dir.path().join("profiles"))?;
    fs::write(dir.path().join("profiles"), "a file is not a directory")?;
    assert!(matches!(
        crate::config::load_dir(dir.path()),
        Err(ConfigError::Io { .. })
    ));
    Ok(())
}

#[test]
fn workflow_defaults_and_validation_view_preserve_canonical_settings() -> anyhow::Result<()> {
    let config = HiveConfig::from_json(r#"{"profiles":[{"id":"worker"}],"seats":[{"id":"reader","profile":"worker"}],"workflows":[{"id":"daily","schedule":"0 0 * * *","target":{"seat":"reader"},"prompt":"Review"}]}"#)?.validate()?;
    let workflow = &config.config().workflows[0];
    assert!(workflow.enabled);
    assert!(workflow.single_flight);
    assert_eq!(workflow.retries, 0);
    Ok(())
}
