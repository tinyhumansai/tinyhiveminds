//! Direct job triggering pins scheduled authority and native isolated sessions.
use super::{
    support::*,
    turns::{Script, message, mount, source},
};
use crate::{config::*, deploy::*};
use std::future::Future;
use std::sync::{
    Arc, PoisonError,
    atomic::{AtomicUsize, Ordering},
};
#[test]
fn hive_job_provenance_reaches_first_and_private_child_turns() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (mut options, _backend, model) = build_options().await;
        let script = Script {
            action: Some("write".into()),
            child: true,
            ..Default::default()
        };
        mount(&model, script.clone()).await;
        let writes = Arc::new(AtomicUsize::new(0));
        options.tools = Some(source(writes.clone()));
        let mut config = manifest()?;
        config.workflows.push(Workflow {
            id: "hive-job".into(),
            schedule: "0 8 * * *".into(),
            target: WorkflowTarget::Hive("a".into()),
            prompt: "work".into(),
            enabled: true,
            retries: 0,
            single_flight: true,
        });
        let storage = Arc::new(tinyhivemind_hives::MemoryStorage::new());
        options.coordinator_storage = Some(storage.clone());
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        let job = deployment
            .runtime()
            .cron()
            .list()?
            .into_iter()
            .find(|job| job.name == "hive-job")
            .ok_or_else(|| anyhow::anyhow!("missing job"))?;
        assert_eq!(job.retries, Some(0));
        assert!(job.enabled && job.single_flight);
        let run = deployment.runtime().cron().run_now("hive-job").await?;
        assert!(!run.success, "{:?}", run.output);
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        let state = tinyhivemind_hives::Storage::load(storage.as_ref()).await?;
        assert!(
            deployment
                .host()
                .coordinator()
                .read_transcript(None)?
                .iter()
                .any(|message| !message.only_for.is_empty())
        );
        assert!(
            state
                .episodes
                .iter()
                .all(|episode| episode.scheduled_job_id.as_deref() == Some(job.id.as_str()))
        );
        assert!(
            script
                .seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .any(|text| text.contains("\"agent_id\":\"bob\""))
        );
        assert!(state.episodes.iter().all(|episode| episode.finished));
        Ok::<_, anyhow::Error>(())
    })
}
#[test]
fn native_seat_job_keeps_baseline_denials_and_isolates_session() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (mut options, _backend, model) = build_options().await;
        mount(
            &model,
            Script {
                action: Some("write".into()),
                ..Default::default()
            },
        )
        .await;
        let writes = Arc::new(AtomicUsize::new(0));
        options.tools = Some(source(writes.clone()));
        let mut config = manifest()?;
        config.profiles[0].limits.iterations = Some(2);
        config.workflows.push(Workflow {
            id: "seat-job".into(),
            schedule: "0 8 * * *".into(),
            target: WorkflowTarget::Seat("alice".into()),
            prompt: "native isolated work".into(),
            enabled: true,
            retries: 0,
            single_flight: false,
        });
        let storage = Arc::new(tinyhivemind_hives::MemoryStorage::new());
        options.coordinator_storage = Some(storage.clone());
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        deployment
            .host()
            .coordinator()
            .bind_session("alice", "continuing-session")
            .await?;
        let job = deployment
            .runtime()
            .cron()
            .list()?
            .into_iter()
            .find(|job| job.name == "seat-job")
            .ok_or_else(|| anyhow::anyhow!("missing job"))?;
        assert!(!job.single_flight);
        assert!(
            matches!(job.target,openhuman_embed::JobTarget::Agent { ref agent_id,.. } if agent_id == "alice")
        );
        assert!(
            !deployment
                .runtime()
                .cron()
                .run_now("seat-job")
                .await?
                .success
        );
        assert_eq!(writes.load(Ordering::Relaxed), 0);
        let state = tinyhivemind_hives::Storage::load(storage.as_ref()).await?;
        assert_eq!(
            state
                .agents
                .get("alice")
                .and_then(|agent| agent.session_id.as_deref()),
            Some("continuing-session")
        );
        assert!(state.episodes.is_empty());
        // The same native agent still admits the ordinary coordinator operation.
        deployment
            .host()
            .coordinator()
            .send_as_host(message("ordinary", "a"))
            .await?;
        let report = deployment.host().coordinator().run_until_idle().await?;
        assert_eq!(report.failed, 0);
        assert_eq!(writes.load(Ordering::Relaxed), 1);
        Ok::<_, anyhow::Error>(())
    })
}
#[test]
fn isolated_native_cron_cannot_forward_work_with_interactive_origin() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (options, _backend, model) = build_options().await;
        mount(
            &model,
            Script {
                action: Some("hivemind_send_hive".into()),
                arguments: Some(serde_json::json!({"hive_id":"a","body":"forward"})),
                ..Default::default()
            },
        )
        .await;
        let mut config = manifest()?;
        config.workflows.push(Workflow {
            id: "forward".into(),
            schedule: "0 8 * * *".into(),
            target: WorkflowTarget::Seat("alice".into()),
            prompt: "try forwarding".into(),
            enabled: true,
            retries: 0,
            single_flight: true,
        });
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        assert!(
            !deployment
                .runtime()
                .cron()
                .run_now("forward")
                .await?
                .success
        );
        assert_eq!(
            deployment.host().coordinator().read_transcript(None)?.len(),
            0
        );
        let report = deployment.host().coordinator().run_until_idle().await?;
        assert_eq!(report.completed, 0);
        Ok(())
    })
}

struct PausedAppend {
    inner: tinyhivemind_hives::MemoryStorage,
    pause: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl tinyhivemind_hives::Storage for PausedAppend {
    fn load(&self) -> tinyhivemind_hives::StorageFuture<'_, tinyhivemind_hives::StoredState> {
        tinyhivemind_hives::Storage::load(&self.inner)
    }
    fn commit<'a>(
        &'a self,
        commit: tinyhivemind_hives::Commit<'a>,
    ) -> tinyhivemind_hives::StorageFuture<'a, ()> {
        Box::pin(async move {
            if !commit.appended.is_empty() && self.pause.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.resume.notified().await;
            }
            tinyhivemind_hives::Storage::commit(&self.inner, commit).await
        })
    }
}
#[test]
fn overlapping_hive_job_invocations_append_distinct_messages() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (mut options, _backend, model) = build_options().await;
        mount(&model, Script::default()).await;
        let storage = Arc::new(PausedAppend {
            inner: tinyhivemind_hives::MemoryStorage::new(),
            pause: std::sync::atomic::AtomicBool::new(false),
            entered: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
        });
        options.coordinator_storage = Some(storage.clone());
        let mut config = manifest()?;
        config.workflows.push(Workflow {
            id: "overlap".into(),
            schedule: "0 8 * * *".into(),
            target: WorkflowTarget::Hive("a".into()),
            prompt: "work".into(),
            enabled: true,
            retries: 0,
            single_flight: false,
        });
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        storage.pause.store(true, Ordering::SeqCst);
        let job_id = deployment
            .runtime()
            .cron()
            .list()?
            .into_iter()
            .find(|job| job.name == "overlap")
            .ok_or_else(|| anyhow::anyhow!("missing overlap job"))?
            .id;
        let prefix = format!("cron:{job_id}:");
        let context = openhuman_embed::SystemJobContext {
            job_id,
            name: "overlap".into(),
        };
        let first = tokio::spawn(openhuman_core::cron::system_job_handlers::dispatch(
            context.clone(),
        ));
        storage.entered.notified().await;
        let mut second = Box::pin(openhuman_core::cron::system_job_handlers::dispatch(context));
        // Poll through message-ID allocation while the first durable append is held.
        std::future::poll_fn(|cx| {
            assert!(second.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        storage.resume.notify_one();
        let (first, second) = tokio::join!(first, second);
        assert!(first?.is_some_and(|result| result.is_ok()));
        assert!(second.is_some_and(|result| result.is_ok()));
        let transcript = deployment.host().coordinator().read_transcript(None)?;
        let messages: Vec<_> = transcript
            .iter()
            .filter(|message| message.message_id.starts_with(&prefix))
            .collect();
        assert_eq!(
            messages.len(),
            2,
            "each invocation must be durably accepted"
        );
        assert_ne!(messages[0].message_id, messages[1].message_id);
        Ok(())
    })
}

#[test]
fn invocation_identity_exhaustion_never_wraps_or_reuses_a_number() {
    let counter = std::sync::atomic::AtomicU64::new(u64::MAX - 1);
    assert_eq!(
        super::super::workflow::next_invocation(&counter),
        Ok(u64::MAX - 1)
    );
    assert!(super::super::workflow::next_invocation(&counter).is_err());
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}
