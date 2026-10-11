//! Native isolated seat cron and durable coordinator hive jobs.
use super::{DeployError, DeployResult, HiveDeployment};
use crate::config::WorkflowTarget;
use openhuman_embed::{JobSchedule, JobSpec};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicU64, Ordering},
};
use tinyhivemind_hives::{Destination, SendMessage};
pub(super) fn register(deployment: &HiveDeployment) -> DeployResult<()> {
    for workflow in &deployment.config.workflows {
        let schedule = JobSchedule::Cron {
            expr: workflow.schedule.clone(),
            tz: None,
        };
        let mut spec = match &workflow.target {
            WorkflowTarget::Seat(seat) => {
                JobSpec::agent(&workflow.id, seat, &workflow.prompt, schedule)
            }
            WorkflowTarget::Hive(hive) => {
                let weak: Weak<crate::host::Inner> = Arc::downgrade(&deployment.host.inner);
                let hive = hive.clone();
                let prompt = workflow.prompt.clone();
                // Native system-job contexts expose no run ID. A runtime UUID
                // namespaces this checked counter across deployment restarts;
                // allocation is independent of asynchronous transcript commits.
                let runtime_id = deployment.runtime.runtime_id().to_owned();
                let invocations = AtomicU64::new(0);
                deployment
                    .runtime
                    .on_system_job(&workflow.id, move |context| {
                        let weak = weak.clone();
                        let hive = hive.clone();
                        let prompt = prompt.clone();
                        let message_id = next_invocation(&invocations).map(|invocation| {
                            format!("cron:{}:{runtime_id}:{invocation}", context.job_id)
                        });
                        async move {
                            let host = weak
                                .upgrade()
                                .ok_or_else(|| "deployment unavailable".to_string())?;
                            host.coordinator
                                .send_scheduled_as_host(
                                    &context.job_id,
                                    SendMessage {
                                        message_id: message_id?,
                                        sender: String::new(),
                                        destination: Destination::Hive(hive),
                                        body: prompt,
                                        thread: None,
                                        only_for: Vec::new(),
                                        starters: Vec::new(),
                                    },
                                )
                                .await
                                .map_err(|_| "scheduled enqueue failed".to_string())?;
                            let report = host
                                .coordinator
                                .run_until_idle()
                                .await
                                .map_err(|_| "scheduled drain failed".to_string())?;
                            if report.failed > 0 {
                                return Err("scheduled turn failed".to_string());
                            }
                            Ok(())
                        }
                    })
                    .map_err(|_| DeployError::Workflow(workflow.id.clone()))?;
                JobSpec::system(&workflow.id, &workflow.id, schedule)
            }
        };
        spec.enabled = workflow.enabled;
        spec.retries = Some(workflow.retries);
        spec.single_flight = workflow.single_flight;
        deployment
            .runtime
            .cron()
            .upsert(spec)
            .map_err(|_| DeployError::Workflow(workflow.id.clone()))?;
    }
    Ok(())
}

// Compare-and-swap keeps checked allocation compatible with the workspace MSRV.
pub(super) fn next_invocation(counter: &AtomicU64) -> Result<u64, String> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current
            .checked_add(1)
            .ok_or_else(|| "workflow invocation identities exhausted".to_string())?;
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Ok(current),
            Err(actual) => current = actual,
        }
    }
}
