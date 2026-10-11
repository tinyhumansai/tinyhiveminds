//! Opt-in management delegates configured agent creation to the host.
use super::{
    fixture::{Fixture, configured_agent, queue_capabilities},
    topology::{assert_catalogue, system_text},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tinyhivemind_hives::{
    Coordinator, CoordinatorOptions, Destination, MemoryStorage, SendMessage,
};
use tinyhivemind_openhuman::{
    AgentFactory, AgentFuture, Error, ManagementAuthorizer, ManagementRequest, OpenHumanHost,
    Result,
};

use super::types::{Authorizer, Factory};
impl AgentFactory for Factory {
    fn create(&self, template: String, config: Value) -> AgentFuture {
        let runtime = self.runtime.clone();
        let root = self.root.clone();
        let calls = self.calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            if template != "specialist" {
                return Err(Error::Harness(anyhow::anyhow!("unknown host template")));
            }
            if config
                .as_object()
                .is_none_or(|fields| fields.len() != 1 || !fields.contains_key("id"))
            {
                return Err(Error::Harness(anyhow::anyhow!(
                    "configuration accepts only an agent id"
                )));
            }
            let id = config["id"].as_str().ok_or_else(|| {
                Error::Harness(anyhow::anyhow!("configuration requires an agent id"))
            })?;
            configured_agent(&runtime, &root, id).map_err(Error::Harness)
        })
    }
}
impl ManagementAuthorizer for Authorizer {
    fn authorize(&self, actor: &str, request: &ManagementRequest) -> Result<()> {
        if actor != "manager"
            || matches!(request,
            ManagementRequest::CreateAgent {template,..} if template == "denied")
        {
            return Err(Error::Unauthorized(
                "template or actor is not authorized".into(),
            ));
        }
        Ok(())
    }
}

pub async fn run() -> anyhow::Result<()> {
    let fixture = Fixture::new().await?;
    let manager = fixture.agent("manager")?;
    let coordinator = Coordinator::new(
        manager.runtime_id().into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions::default(),
    )
    .await?;
    let calls = Arc::new(AtomicUsize::new(0));
    let host = OpenHumanHost::new(manager.runtime_id().into(), coordinator.clone())?
        .with_management(
            Arc::new(Factory {
                runtime: fixture.runtime.clone(),
                root: fixture.files.path().into(),
                calls: calls.clone(),
            }),
            Arc::new(Authorizer),
        )?;
    host.register_agent(manager.clone()).await?;
    for (name, args) in [
        (
            "hivemind_create_hive",
            json!({"hive_id":"dynamic","name":"Dynamic hive"}),
        ),
        (
            "hivemind_create_agent",
            json!({"template":"denied","config":{"id":"unauthorized"}}),
        ),
        (
            "hivemind_create_agent",
            json!({"template":"unknown","config":{"id":"failed"}}),
        ),
        (
            "hivemind_create_agent",
            json!({"template":"specialist","config":{"id":"../outside"}}),
        ),
        (
            "hivemind_create_agent",
            json!({"template":"specialist","config":{"id":"specialist"}}),
        ),
        (
            "hivemind_join_hive",
            json!({"hive_id":"dynamic","agent_id":"specialist"}),
        ),
    ] {
        fixture.script.calls.lock().unwrap().push_back((name, args));
        manager
            .turn(format!("Manage using {name}"))
            .session("management")
            .send()
            .await?;
    }
    anyhow::ensure!(
        calls.load(Ordering::SeqCst) == 3,
        "expected three authorized factory calls, observed {}",
        calls.load(Ordering::SeqCst)
    );
    anyhow::ensure!(
        coordinator.list_agents()? == ["manager", "specialist"],
        "factory failure or denial left a partial registration"
    );
    anyhow::ensure!(
        coordinator.list_hives()?[0].members == ["specialist"],
        "management tool did not register and join supplied instance"
    );
    coordinator
        .send_as_host(SendMessage {
            message_id: "dynamic-input".into(),
            sender: String::new(),
            destination: Destination::Hive("dynamic".into()),
            body: "Work from dynamically created hive".into(),
            thread: None,
            only_for: Vec::new(),
            starters: Vec::new(),
        })
        .await?;
    queue_capabilities(&fixture, "specialist");
    let report = coordinator.run_until_idle().await?;
    anyhow::ensure!(
        report.completed == 1 && report.failed == 0,
        "dynamic agent turn failed: {report:?}"
    );
    let captures = fixture.captures().await?;
    anyhow::ensure!(
        captures.len() == 17,
        "expected native call plus continuation per turn"
    );
    let mut denied_receipt = false;
    let mut failed_receipt = false;
    let mut invalid_receipt = false;
    let mut skill_result = false;
    let mut mcp_result = false;
    for capture in &captures {
        assert_catalogue(capture)?;
        let system = system_text(capture);
        let own = if system.contains("PROMPT_MARKER_manager") {
            "manager"
        } else {
            "specialist"
        };
        // OpenHuman no longer renders a workspace `MEMORY.md` into the
        // prompt; memory arrives as a recalled pack from the memory engine.
        for marker in ["PROMPT", "SKILL", "MCP"] {
            anyhow::ensure!(
                system.contains(&format!("{marker}_MARKER_{own}")),
                "factory supplied agent lost its {marker}"
            );
        }
        for name in [
            "hivemind_create_hive",
            "hivemind_create_agent",
            "hivemind_join_hive",
            "hivemind_leave_hive",
        ] {
            anyhow::ensure!(
                system.matches(name).count() == 1,
                "management prompt count != 1 for {own}: {name}"
            );
            let schemas = capture["tools"].as_array().unwrap();
            anyhow::ensure!(
                schemas
                    .iter()
                    .filter(|tool| tool["function"]["name"] == name)
                    .count()
                    == 1,
                "management native schema count != 1 for {own}: {name}"
            );
        }
        for message in capture["messages"].as_array().unwrap() {
            if message["role"] == "tool" {
                let text = message["content"].as_str().unwrap_or("");
                skill_result |= own == "specialist" && text.contains("SKILL_MARKER_specialist");
                mcp_result |= own == "specialist" && text.contains("MCP_RESULT_specialist");
                let other = if own == "manager" {
                    "specialist"
                } else {
                    "manager"
                };
                anyhow::ensure!(
                    !text.contains(&format!("MCP_RESULT_{other}"))
                        && !text.contains(&format!("SKILL_MARKER_{other}")),
                    "private native capability leaked"
                );
                denied_receipt |= text.contains("management denied");
                failed_receipt |= text.contains("unknown host template");
                invalid_receipt |= text.contains("invalid host agent id");
            }
        }
    }
    anyhow::ensure!(
        skill_result && mcp_result,
        "dynamic factory lost installed native capabilities: skill={skill_result}, mcp={mcp_result}"
    );
    anyhow::ensure!(
        denied_receipt && failed_receipt && invalid_receipt,
        "model did not receive explicit failure receipts"
    );
    println!("dynamic hive and configured agent created through authorized native tools");
    Ok(())
}
