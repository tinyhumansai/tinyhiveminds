//! Provider selection resolves declared credentials and sanitizes host diagnostics.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Resolver(AtomicUsize);
impl SecretResolver for Resolver {
    fn resolve(&self, _: &crate::config::SecretRef) -> DeployResult<String> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok("private-fixture-credential".into())
    }
}
#[test]
fn known_provider_kinds_resolve_credentials_without_an_explicit_endpoint() -> anyhow::Result<()> {
    let resolver = Resolver(AtomicUsize::new(0));
    for kind in ["openai", "openrouter"] {
        let mut section = crate::config::RuntimeSection::default();
        section.provider.kind = kind.into();
        section.provider.credential = Some(crate::config::SecretRef::Store("key".into()));
        section.model.name = Some("configured-model".into());
        let _provider = provider(&section, &resolver, &BuildOptions::default())?;
    }
    assert_eq!(resolver.0.load(Ordering::Relaxed), 2);
    let options = BuildOptions {
        provider: Some(openhuman_embed::Provider::openai_compatible(
            "https://fixture",
            "private-fixture-credential",
        )),
        ..Default::default()
    };
    let _provider = provider(
        &crate::config::RuntimeSection::default(),
        &resolver,
        &options,
    )?;
    assert_eq!(resolver.0.load(Ordering::Relaxed), 2);
    assert!(!format!("{options:?}").contains("private-fixture-credential"));
    let _provider = provider(
        &crate::config::RuntimeSection::default(),
        &resolver,
        &BuildOptions::default(),
    )?;
    assert!(
        super::super::EnvironmentSecrets
            .resolve(&crate::config::SecretRef::Store("key".into()))
            .is_err()
    );
    assert!(
        super::super::EnvironmentSecrets
            .resolve(&crate::config::SecretRef::Env(
                "TINYHIVEMIND_MISSING_TEST_CREDENTIAL_8675309".into()
            ))
            .is_err()
    );
    Ok(())
}
#[test]
fn numeric_and_policy_mappings_preserve_declared_bounds() {
    assert_eq!(budget_micros(1.25), Some(1_250_000));
    assert_eq!(budget_micros(0.000_001_9), Some(1));
    assert_eq!(budget_micros(f64::MAX), None);
    for value in [Access::ReadOnly, Access::Supervised, Access::Autonomous] {
        let _native = access(value);
    }
    for value in [Sandbox::None, Sandbox::ReadOnly, Sandbox::Sandboxed] {
        let _native = sandbox(value);
    }
    let defaults = model(
        &Model {
            name: Some("fixture".into()),
            temperature: Some(0.25),
            max_tokens: Some(123),
        },
        Some(7),
    );
    assert_eq!(defaults.temperature, Some(0.25));
    assert_eq!(defaults.max_tokens, Some(123));
    assert_eq!(defaults.max_iterations, Some(7));
}
