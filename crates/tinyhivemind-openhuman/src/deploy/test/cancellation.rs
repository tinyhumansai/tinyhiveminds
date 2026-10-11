//! Native deadlines return only after owned subprocess cleanup has finished.
#[cfg(unix)]
use super::{
    support::*,
    turns::{Script, message, mount},
};
#[cfg(unix)]
use crate::deploy::*;
#[cfg(unix)]
use std::{
    path::PathBuf,
    sync::{Arc, PoisonError},
    time::Duration,
};
#[cfg(unix)]
struct SlowTool(PathBuf);
#[cfg(unix)]
#[async_trait::async_trait]
impl tinytools::Tool for SlowTool {
    fn name(&self) -> &'static str {
        "slow"
    }
    fn description(&self) -> &'static str {
        "Run a registered cancellable subprocess"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    async fn execute(&self, _: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        let mut command = tokio::process::Command::new("sh");
        command
            .args(["-c", "printf %s $$ > \"$1\"; exec sleep 600", "sh"])
            .arg(&self.0);
        let output = openhuman_embed::process::command_output(
            &mut command,
            Vec::new(),
            Duration::from_secs(7200),
        )
        .await?;
        Ok(tinytools::ToolResult::success(String::from_utf8_lossy(
            &output.stdout,
        )))
    }
    fn policy(&self) -> tinytools::ToolPolicy {
        tinytools::ToolPolicy::classified().with_side_effects(tinytools::ToolSideEffects {
            destructive: true,
            ..Default::default()
        })
    }
}
#[cfg(unix)]
struct Classifier;
#[cfg(unix)]
impl EffectClassifier for Classifier {
    fn classify(
        &self,
        context: &openhuman_embed::seams::ToolHookContext,
    ) -> tinyhivemind_core::approval::Effect {
        if context.tool_name == "slow" {
            tinyhivemind_core::approval::Effect::Mutating
        } else {
            tinyhivemind_core::approval::Effect::Unclassified
        }
    }
}
#[cfg(unix)]
#[test]
fn expired_turn_reaps_a_registered_command_before_returning() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| -> anyhow::Result<()> {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(async {
                    let directory = tempfile::tempdir()?;
                    let marker = directory.path().join("pid");
                    let (mut options, _backend, model) = build_options().await;
                    mount(
                        &model,
                        Script {
                            action: Some("slow".into()),
                            ..Default::default()
                        },
                    )
                    .await;
                    let tool_marker = marker.clone();
                    options.tools = Some(Arc::new(move |_| {
                        openhuman_embed::HostTurnTools::advertised(vec![Box::new(SlowTool(
                            tool_marker.clone(),
                        ))])
                    }));
                    options.classifier = Some(Arc::new(Classifier));
                    let mut config = manifest()?;
                    config.profiles[0].limits.timeout_ms = Some(3_600_000);
                    let deployment = Box::pin(HiveDeployment::build_with(
                        config.validate()?,
                        Arc::new(Secrets),
                        options,
                    ))
                    .await?;
                    deployment
                        .host()
                        .coordinator()
                        .send_as_host(message("slow", "a"))
                        .await?;
                    let coordinator = deployment.host().coordinator().clone();
                    let task = tokio::spawn(async move { coordinator.run_until_idle().await });
                    let pid = tokio::time::timeout(Duration::from_secs(60), async {
                        loop {
                            if let Ok(pid) = std::fs::read_to_string(&marker)
                                && !pid.is_empty()
                            {
                                break pid;
                            }
                            tokio::task::yield_now().await;
                        }
                    })
                    .await?;
                    tokio::time::pause();
                    tokio::time::advance(Duration::from_secs(3601)).await;
                    let report = task.await??;
                    tokio::time::resume();
                    assert!(report.failed > 0);
                    let alive = std::process::Command::new("kill")
                        .args(["-0", pid.trim()])
                        .output()?
                        .status
                        .success();
                    assert!(!alive, "runner returned before the child was reaped");
                    Ok::<_, anyhow::Error>(())
                })
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("fixture thread failed"))??;
    Ok(())
}
