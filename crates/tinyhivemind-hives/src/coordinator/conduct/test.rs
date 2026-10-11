//! Native routing configuration and membership role propagation.
use super::*;

#[test]
fn configured_roles_and_routing_thresholds_reach_the_native_routing_request() -> Result<()> {
    let hive = HiveInfo {
        hive_id: "one".into(),
        name: "One".into(),
        description: None,
        members: vec!["agent".into()],
    };
    let mut settings = crate::HiveSettings::default();
    settings.roles.insert("agent".into(), "reviewer".into());
    settings.routing.minimum_confidence = Probability::ONE;
    settings.routing.choice_option_limit = 3;
    settings.routing.round_width = 4;
    let environment = Environment::new(&hive, &settings.options, Some(&settings))?;
    let request =
        environment
            .hive
            .desk_request("work", Vec::new(), None, 0, environment.routing.clone());
    assert_eq!(request.candidates[0].role.as_deref(), Some("reviewer"));
    assert_eq!(request.policy.minimum_confidence, Probability::ONE);
    assert_eq!(request.policy.choice_option_limit, 3);
    assert_eq!(request.policy.round_width, 1);
    Ok(())
}
