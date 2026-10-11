//! Dynamic templates retain one runtime and typed narrowing.
use super::support::*;
use crate::{AgentFactory, config::*, deploy::*};
use std::sync::{Arc, PoisonError};
struct AllowManagement;
#[test]
fn factory_refuses_unbounded_dynamic_budget_and_native_identity_without_mutating_catalog()
-> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (options, _backend, _model) = build_options().await;
        let config = manifest()?;
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        let diagnostic = format!("{:?}", deployment.factory());
        assert!(diagnostic.contains(deployment.runtime().runtime_id()));
        assert!(!diagnostic.contains("fixture"));
        let budget = SeatOverrides {
            limits: Limits {
                budget_usd: Some(0.1),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(deployment.factory().create_seat("budget", "worker", budget).await.is_err_and(|error| matches!(error, DeployError::Config(ConfigError::UnsupportedSetting(ref setting)) if setting.contains("call_budget"))));
        assert!(
            deployment
                .factory()
                .create_seat("critic", "worker", SeatOverrides::default())
                .await
                .is_err_and(|error| matches!(error, DeployError::Agent(ref id) if id=="critic"))
        );
        assert_eq!(
            deployment.host().coordinator().list_agents()?,
            ["alice", "bob"]
        );
        assert!(
            !deployment
                .factory()
                .permissions
                .config()
                .seats
                .iter()
                .any(|seat| seat.id == "budget" || seat.id == "critic")
        );
        Ok(())
    })
}
#[test]
fn model_management_requires_an_explicit_host_authorizer() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (options, _backend, model) = build_options().await;
        super::turns::mount(
            &model,
            super::turns::Script {
                action: Some("hivemind_create_agent".into()),
                arguments: Some(
                    serde_json::json!({"template":"worker","config":{"id":"unauthorized"}}),
                ),
                ..Default::default()
            },
        )
        .await;
        let deployment = Box::pin(HiveDeployment::build_with(
            manifest()?.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        deployment
            .host()
            .coordinator()
            .send_as_host(super::turns::message("default-management", "a"))
            .await?;
        deployment.host().coordinator().run_until_idle().await?;
        assert!(
            !deployment
                .host()
                .coordinator()
                .list_agents()?
                .contains(&"unauthorized".into())
        );
        assert!(
            !deployment
                .runtime()
                .agent_ids()
                .contains(&"unauthorized".into())
        );
        Ok(())
    })
}
impl crate::ManagementAuthorizer for AllowManagement {
    fn authorize(&self, _: &str, _: &crate::ManagementRequest) -> crate::Result<()> {
        Ok(())
    }
}
#[test]
fn model_management_creates_only_known_typed_profiles() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        for (template, overrides, allowed) in [
            ("worker", serde_json::json!({}), true),
            ("unknown", serde_json::json!({}), false),
            ("worker", serde_json::json!({"api_key":"literal"}), false),
        ] {
            let (mut options, _backend, model) = build_options().await;
            options.management = Some(Arc::new(AllowManagement));
            super::turns::mount(&model, super::turns::Script {
                action: Some("hivemind_create_agent".into()),
                arguments: Some(serde_json::json!({"template":template,"config":{"id":"dynamic","overrides":overrides}})),
                ..Default::default()
            }).await;
            let deployment = Box::pin(HiveDeployment::build_with(
                manifest()?.validate()?,
                Arc::new(Secrets),
                options,
            ))
            .await?;
            deployment
                .host()
                .coordinator()
                .send_as_host(super::turns::message("manage", "a"))
                .await?;
            let _report = deployment.host().coordinator().run_until_idle().await?;
            assert_eq!(
                deployment
                    .host()
                    .coordinator()
                    .list_agents()?
                    .contains(&"dynamic".into()),
                allowed
            );
        }
        Ok(())
    })
}
#[test]
fn dynamic_creation_uses_the_catalog_and_refuses_unknown_or_secret_overrides() -> anyhow::Result<()>
{
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (options, _backend, _model) = build_options().await;
        let deployment = Box::pin(HiveDeployment::build_with(
            manifest()?.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        assert!(
            deployment
                .factory()
                .create_seat("new", "unknown", SeatOverrides::default())
                .await
                .is_err()
        );
        assert!(
            deployment
                .factory()
                .create(
                    "worker".into(),
                    serde_json::json!({"id":"new","overrides":{"api_key":"literal"}})
                )
                .await
                .is_err()
        );
        let agent = deployment.factory().create("worker".into(),serde_json::json!({"id":"new","overrides":{"deny_tools":["write"],"limits":{"iterations":2}}})).await?;
        assert_eq!(agent.runtime_id(), deployment.runtime().runtime_id());
        deployment.host().register_agent(agent).await?;
        assert_eq!(deployment.host().coordinator().list_agents()?.len(), 3);
        assert!(
            deployment
                .factory()
                .create_seat("new", "worker", SeatOverrides::default())
                .await
                .is_err()
        );
        Ok::<_, anyhow::Error>(())
    })
}
