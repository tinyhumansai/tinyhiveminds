//! One-runtime registration, limits, native storage and cycle-free ownership.
use super::support::*;
use crate::deploy::*;
use std::sync::{Arc, PoisonError};
#[test]
fn ordinary_host_registration_preserves_supplied_agent_tools() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (options, _backend, model) = build_options().await;
        super::turns::mount(
            &model,
            super::turns::Script {
                action: Some("write".into()),
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
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let supplied = super::turns::source(count.clone());
        let agent = deployment.runtime().agent(
            openhuman_embed::AgentSpec::new("legacy")
                .access(openhuman_embed::Access::full())
                .tools(move |context| supplied(context)),
        )?;
        let coordinator = tinyhivemind_hives::Coordinator::new(
            deployment.runtime().runtime_id().into(),
            Arc::new(tinyhivemind_hives::MemoryStorage::new()),
            tinyhivemind_hives::CoordinatorOptions::default(),
        )
        .await?;
        let host =
            crate::OpenHumanHost::new(deployment.runtime().runtime_id().into(), coordinator)?;
        host.register_agent(agent).await?;
        let mut message = super::turns::message("legacy", "a");
        message.destination = tinyhivemind_hives::Destination::Agent("legacy".into());
        message.starters.clear();
        host.coordinator().send_as_host(message).await?;
        let report = host.coordinator().run_until_idle().await?;
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 1);
        Ok(())
    })
}
#[test]
fn builds_two_hives_and_fixed_memory_seats_then_releases_runtime() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (options, _backend, _model) = build_options().await;
        let mut config = manifest()?;
        config.runtime.memory_engine = "memory".into();
        config.profiles[0].limits.iterations = Some(7);
        config.seats[0].overrides.memory = Some(crate::config::Memory {
            root: Some("explicit".into()),
        });
        config.hives[0].memory_root = Some("shared".into());
        config.hives[0].name = "Shared desk".into();
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        assert_eq!(deployment.seats().len(), 2);
        deployment.update_approval(ApprovalSnapshot::default());
        deployment.start_scheduler()?;
        let scheduler_id = deployment
            .scheduler
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("scheduler did not start"))?
            .id();
        deployment.start_scheduler()?;
        assert_eq!(
            deployment
                .scheduler
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("scheduler disappeared"))?
                .id(),
            scheduler_id
        );

        assert!(
            deployment
                .seats()
                .values()
                .all(|agent| agent.runtime_id() == deployment.runtime().runtime_id())
        );
        assert_eq!(deployment.host().coordinator().list_hives()?.len(), 2);
        assert_eq!(
            deployment
                .host()
                .coordinator()
                .hive_settings("a")?
                .ok_or_else(|| anyhow::anyhow!("missing settings"))?
                .roles
                .get("alice")
                .map(String::as_str),
            Some("lead")
        );
        assert_eq!(
            deployment
                .policy("a")
                .and_then(|policy| policy.memory_root.as_deref()),
            Some("shared")
        );
        assert_eq!(
            deployment.memory("a")?.root(),
            native_memory_root("shared")?
        );
        assert_eq!(
            deployment.seats()["alice"].config().memory.root.as_deref(),
            Some(native_memory_root("explicit")?.as_str())
        );
        assert_eq!(
            deployment.seats()["bob"].config().memory.root.as_deref(),
            Some(native_memory_root("seat:bob")?.as_str())
        );
        assert!(deployment.memory("b").is_err());
        assert!(format!("{deployment:?}").contains("alice"));
        assert!(!format!("{:?}", deployment.factory()).contains("fixture"));
        drop(deployment);
        let (options, _backend2, _model2) = build_options().await;
        let deployment = Box::pin(HiveDeployment::build_with(
            manifest()?.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        assert_eq!(deployment.runtime().agent_ids().len(), 2);
        Ok::<_, anyhow::Error>(())
    })
}
#[test]
fn sqlite_and_host_storage_lower_to_native_ports() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let directory = tempfile::tempdir()?;
        let mut config = manifest()?;
        config.runtime.workspace = Some(directory.path().to_string_lossy().into_owned());
        config.runtime.session_store = "sqlite".into();
        let (options, _backend, _model) = build_options().await;
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        assert!(deployment.runtime().storage().is_some());
        assert!(directory.path().join("sessions.sqlite").exists());
        drop(deployment);
        let mut config = manifest()?;
        config.runtime.session_store = "host".into();
        config.runtime.memory_engine = "host".into();
        let (mut options, _backend2, _model2) = build_options().await;
        options.session_store = Some(Arc::new(openhuman_embed::InMemorySessionStores::new()));
        options.memory_engine = Some(Arc::new(
            tinymemory_api::conformance::reference::ReferenceEngine::new(),
        ));
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        assert!(
            deployment
                .runtime()
                .memory(&native_memory_root("host-shared")?)
                .is_ok()
        );
        Ok::<_, anyhow::Error>(())
    })
}
