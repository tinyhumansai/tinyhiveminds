//! Authoritative deadline inheritance and host narrowing.
use super::support::*;
use crate::{TurnHooks, TurnScope, deploy::BuildOptions};
use std::{sync::Arc, time::Duration};
struct Deadline(Duration);
impl TurnHooks for Deadline {
    fn turn_timeout(&self, _: &TurnScope) -> Option<Duration> {
        Some(self.0)
    }
}
#[test]
fn timeout_inherits_the_closest_limit_and_host_hooks_only_narrow() -> anyhow::Result<()> {
    let mut config = manifest()?;
    config.runtime.limits.timeout_ms = Some(1000);
    config.profiles[0].limits.timeout_ms = Some(800);
    config.seats[0].overrides.limits.timeout_ms = Some(600);
    let scope = scope("a");
    for (extra, expected) in [
        (None, 600),
        (Some(900), 600),
        (Some(400), 400),
        (Some(0), 0),
    ] {
        let options = BuildOptions {
            hooks: extra
                .map(|ms| Arc::new(Deadline(Duration::from_millis(ms))) as Arc<dyn TurnHooks>),
            ..Default::default()
        };
        let hooks =
            super::super::hooks::Hooks::new(permissions(config.clone(), &options), &options);
        assert_eq!(
            hooks.turn_timeout(&scope),
            Some(Duration::from_millis(expected))
        );
    }
    config.seats[0].overrides.limits.timeout_ms = None;
    assert_eq!(
        super::super::profile::limits(&config, &config.seats[0])?.timeout_ms,
        Some(800)
    );
    config.profiles[0].limits.timeout_ms = None;
    assert_eq!(
        super::super::profile::limits(&config, &config.seats[0])?.timeout_ms,
        Some(1000)
    );
    config.runtime.limits.timeout_ms = None;
    let options = BuildOptions::default();
    let hooks = super::super::hooks::Hooks::new(permissions(config, &options), &options);
    assert_eq!(hooks.turn_timeout(&scope), None);
    Ok(())
}
