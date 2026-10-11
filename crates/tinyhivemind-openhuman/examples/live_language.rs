//! Load a language candidate and run two native tool-completing live seats.
//! Incidental backend fixtures are local; every inference call uses `OpenRouter`.
use openhuman_embed::{Access, AgentSpec, Provider, Runtime, Workspace};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tinyhivemind_hives::{Coordinator, Destination, HiveInfo, MemoryStorage, SendMessage, Storage};
use tinyhivemind_lang::{MemoryIdentity, MemoryKind, MemoryLifecycle, Package, SeatMemory, lower};
use tinyhivemind_openhuman::{
    AgentFactory, AgentFuture, Error, ManagementAuthorizer, ManagementRequest, OpenHumanHost,
    language,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
struct AllowHost;
impl ManagementAuthorizer for AllowHost {
    fn authorize(&self, actor: &str, _: &ManagementRequest) -> tinyhivemind_openhuman::Result<()> {
        if actor != "host" {
            return Err(Error::Unauthorized(
                "only the smoke host may manage seats".into(),
            ));
        }
        Ok(())
    }
}
struct Factory(Arc<Runtime>);
impl AgentFactory for Factory {
    fn create(&self, template: String, config: Value) -> AgentFuture {
        self.create_with_memory(template, config, None)
    }
    fn create_with_memory(
        &self,
        template: String,
        config: Value,
        memory: Option<tinyhivemind_lang::LoweredMemoryBinding>,
    ) -> AgentFuture {
        let runtime = self.0.clone();
        Box::pin(async move {
            if template != "live-invoice" {
                return Err(Error::MemoryBindingUnsupported);
            }
            let id = config["id"]
                .as_str()
                .ok_or(Error::MemoryBindingUnsupported)?;
            let prompt = config["prompt"]
                .as_str()
                .ok_or(Error::MemoryBindingUnsupported)?;
            let instructions = "For Incoming attributed Hivemind context, read the episode_id and task from the JSON. Calculate the task, then actually call the native hivemind_complete tool with that episode_id and body containing ONLY the numeric total as a string. Prose is not completion. After its accepted receipt, stop. Do not call any other tool.";
            let mut spec = AgentSpec::new(id).system_prompt(format!("{prompt}\n{instructions}"));
            if let Some(binding) = memory {
                // This smoke host supports inert read-only run identities only.
                // No engine is attached and no persistence/recall is claimed.
                if binding.identity.lifecycle != MemoryLifecycle::Run
                    || !binding.identity.read_only
                    || binding.identity.fork_parent.is_some()
                    || !binding.settings.reads.is_empty()
                    || !binding.settings.recall_at.is_empty()
                    || !binding.settings.remember.is_empty()
                {
                    return Err(Error::MemoryBindingUnsupported);
                }
                spec = spec.memory(language::memory_binding(&binding)?);
            }
            Ok(runtime.agent(spec)?)
        })
    }
}

/// Require live opt-in and boot the host on a suitably sized Tokio stack.
fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() != Some("--live") {
        return Err("explicit --live required".into());
    }
    let input = std::env::args()
        .nth(2)
        .ok_or("expected accepted-package.json path")?;
    let output = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "target/live-language-host.json".into());
    let mut config = tokio::runtime::Builder::new_multi_thread();
    config.enable_all().thread_stack_size(16 * 1024 * 1024);
    config
        .build()?
        .block_on(async { tokio::spawn(run(input, output)).await })??;
    Ok(())
}
/// Validate smoke-host actors and bind two seats to inert shared run memory.
fn host_package(input: &str) -> Result<Package> {
    let mut package: Package = serde_json::from_slice(&std::fs::read(input)?)?;
    evidence::validate_seat_actors(&package.manifest.seats)?;
    let source = package
        .manifest
        .seats
        .first()
        .ok_or("expected solver seat")?;
    if source.id != "solver" {
        return Err("expected solver as initial seat".into());
    }
    let mut peer = source.clone();
    peer.id = "auditor".into();
    peer.label = "Auditor".into();
    peer.prompt = "seats/auditor.md".into();
    package.documents.insert(
        peer.prompt.clone(),
        package.documents[&package.manifest.seats[0].prompt].clone(),
    );
    package.manifest.seats.push(peer);
    package.memory.root = "team:live-language".into();
    package.memory.identities.push(MemoryIdentity {
        id: "ledger".into(),
        namespace: "team:live-language/agent:ledger".into(),
        kind: MemoryKind::Agent,
        lifecycle: MemoryLifecycle::Run,
        read_only: true,
        fork_parent: None,
    });
    for seat in &package.manifest.seats {
        package.memory.bindings.push(SeatMemory {
            seat: seat.id.clone(),
            identity: "ledger".into(),
            reads: vec![],
            recall_at: vec![],
            budget_chars: 128,
            remember: vec![],
        });
    }
    Ok(package)
}

/// Use a live inference provider with a local incidental backend fixture.
async fn live_runtime(model: &str, key: String) -> Result<(Runtime, wiremock::MockServer)> {
    let backend = tinyhivemind_openhuman::offline::backend().await;
    let runtime = Box::pin(
        Runtime::builder()
            .config(tinyhivemind_openhuman::offline::config())
            .workspace(Workspace::Ephemeral)
            .backend_url(backend.uri())
            .provider(Provider::openai_compatible("https://openrouter.ai/api/v1", key).model(model))
            .access(Access::readonly())
            .build(),
    )
    .await?;
    Ok((runtime, backend))
}

/// Create package-defined agents and verify exact completed assignment rows.
async fn run(input: String, output: String) -> Result<()> {
    let model =
        std::env::var("OPENROUTER_MODEL").unwrap_or_else(|_| "openai/gpt-oss-120b:nitro".into());
    let key = std::env::var("OPENROUTER_API_KEY")?;
    let package = host_package(&input)?;
    let lowered = lower(&package)?;
    let (runtime, _backend) = live_runtime(&model, key).await?;
    let storage = Arc::new(MemoryStorage::new());
    let coordinator = Coordinator::new(
        runtime.runtime_id().into(),
        storage.clone(),
        language::coordinator_options(&lowered.coordinator),
    )
    .await?;
    let host = OpenHumanHost::new(runtime.runtime_id().into(), coordinator.clone())?
        .with_turn_timeout(Duration::from_secs(180))?
        .with_management(Arc::new(Factory(Arc::new(runtime))), Arc::new(AllowHost))?;
    for seat in &lowered.seats {
        host.manage("host", language::management_request(seat))
            .await?;
    }
    coordinator
        .create_hive(HiveInfo {
            hive_id: "invoices".into(),
            name: lowered.name,
            description: None,
            members: vec![],
        })
        .await?;
    for seat in &lowered.seats {
        coordinator.join_hive("invoices", &seat.id).await?;
    }
    let mut results = vec![];
    let mut private_tasks = vec![];
    for (id, task, expected) in [
        (
            "solver",
            "Calculate net paid revenue: [{\"amount\":22,\"status\":\"paid\"},{\"amount\":7,\"status\":\"canceled\"},{\"amount\":11,\"status\":\"paid\"}]",
            "33",
        ),
        (
            "auditor",
            "Calculate net paid revenue: [{\"amount\":14,\"status\":\"canceled\"},{\"amount\":5,\"status\":\"paid\"},{\"amount\":10,\"status\":\"pending\"}]",
            "5",
        ),
    ] {
        let receipt = coordinator
            .send_as_host(SendMessage {
                message_id: format!("live-{id}"),
                sender: String::new(),
                destination: Destination::Hive("invoices".into()),
                body: task.into(),
                thread: None,
                only_for: vec![id.into()],
                starters: vec![],
            })
            .await?;
        private_tasks.push((id.to_owned(), receipt.sequence));
        let report = coordinator.run_until_idle().await?;
        let state = storage.load().await?;
        if report.completed == 0 || report.failed != 0 {
            std::fs::write(&output, serde_json::to_vec_pretty(&state)?)?;
            return Err(format!("native episode failed for {id}: {report:?}").into());
        }
        let messages = coordinator.read_hive(id, "invoices", None, None)?;
        if let Err(error) =
            evidence::verify_episode(&state, receipt.sequence, &messages, id, expected)
        {
            std::fs::write(&output, serde_json::to_vec_pretty(&state)?)?;
            return Err(error);
        }
        results.push(json!({"seat":id,"expected":expected,"finished":true,"native_completion":true,"memory_agent_id":"ledger","messages":messages}));
        println!("{id}: native completion={expected}; shared memory binding=ledger");
    }
    finish(
        &coordinator,
        &storage,
        &model,
        results,
        &private_tasks,
        &output,
    )
    .await
}

/// Check both private audiences and revoke the departing seat before saving evidence.
async fn finish(
    coordinator: &Coordinator,
    storage: &MemoryStorage,
    model: &str,
    results: Vec<Value>,
    private_tasks: &[(String, u64)],
    output: &str,
) -> Result<()> {
    evidence::private_tasks_hidden(private_tasks, |seat| {
        Ok(coordinator.read_hive(seat, "invoices", None, None)?)
    })?;
    coordinator.leave_hive("invoices", "auditor").await?;
    if coordinator
        .read_hive("auditor", "invoices", None, None)
        .is_ok()
    {
        return Err("departed seat retained access".into());
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(
            &json!({"model":model,"results":results,"privacy_checked":true,"leave_access_revoked":true,"state":storage.load().await?}),
        )?,
    )?;
    println!("privacy and leave-access checks passed");
    Ok(())
}

#[path = "live_language/evidence.rs"]
mod evidence;
#[cfg(test)]
#[path = "live_language/test.rs"]
mod test;
