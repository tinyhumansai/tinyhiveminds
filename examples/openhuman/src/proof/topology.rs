//! Captured requests prove configuration and one conversation across hives.
use super::fixture::{Fixture, REPLY, TOOLS, queue_capabilities};
use serde_json::{Value, json};
use std::sync::Arc;
use tinyhivemind_hives::{
    Coordinator, CoordinatorOptions, Destination, HiveInfo, MemoryStorage, SendMessage,
};
use tinyhivemind_openhuman::OpenHumanHost;

pub async fn run(agent_count: usize, hive_count: usize) -> anyhow::Result<()> {
    let fixture = Fixture::new().await?;
    let agents = (0..agent_count)
        .map(|n| fixture.agent(&format!("agent{n}")))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let coordinator = Coordinator::new(
        agents[0].runtime_id().into(),
        Arc::new(MemoryStorage::new()),
        CoordinatorOptions::default(),
    )
    .await?;
    let host = OpenHumanHost::new(agents[0].runtime_id().into(), coordinator.clone())?;
    for agent in &agents {
        // Ordinary host conversations exist before the adapter is registered.
        let session = format!("continuing:{}", agent.id());
        queue_capabilities(&fixture, agent.id());
        agent
            .turn(format!(
                "HOST_ORIGINAL_INPUT CAPABILITY_PHASE_BEFORE_{}",
                agent.id()
            ))
            .session(&session)
            .send()
            .await?;
        host.register_agent_in_session(agent.clone(), &session)
            .await?;
        host.register_agent(agent.clone()).await?; // The identical handle is idempotent.
    }
    for hive in 0..hive_count {
        let id = format!("hive{hive}");
        coordinator
            .create_hive(HiveInfo {
                hive_id: id.clone(),
                name: id.clone(),
                description: None,
                members: Vec::new(),
            })
            .await?;
        for agent in &agents {
            coordinator.join_hive(&id, agent.id()).await?;
        }
    }
    // Exercise the original native capabilities again after handoff, on
    // every bound continuing session. Fresh phase-tagged call IDs make old
    // carried receipts insufficient to prove this execution succeeded.
    for agent in &agents {
        queue_capabilities(&fixture, agent.id());
        agent
            .turn(format!("CAPABILITY_PHASE_AFTER_{}", agent.id()))
            .session(format!("continuing:{}", agent.id()))
            .send()
            .await?;
    }
    // A supplied agent may call its permanent tools in an ordinary host turn.
    fixture
        .script
        .calls
        .lock()
        .unwrap()
        .push_back(("hivemind_list_hives", json!({})));
    agents[0]
        .turn("ORDINARY_HOST_DISCOVERY")
        .session("continuing:agent0")
        .send()
        .await?;
    // Ordinary host turns can send to any joined hive without an episode.
    fixture.script.calls.lock().unwrap().push_back((
        "hivemind_send_hive",
        json!({
            "message_id":"ordinary-cross-hive", "hive_id":format!("hive{}",hive_count-1),
            "body":"ORDINARY_HIVE_INPUT"
        }),
    ));
    agents[0]
        .turn("ORDINARY_HOST_SEND")
        .session("continuing:agent0")
        .send()
        .await?;
    let report = coordinator.run_until_idle().await?;
    anyhow::ensure!(
        report.completed == agent_count && report.failed == 0,
        "ordinary hive send failed: {report:?}"
    );
    for hive in 0..hive_count - 1 {
        coordinator
            .send_as_host(SendMessage {
                message_id: format!("input:{hive}"),
                sender: String::new(),
                destination: Destination::Hive(format!("hive{hive}")),
                body: format!("HOST_HIVE_INPUT_{hive}; retain all earlier conversation."),
                thread: None,
                only_for: Vec::new(),
                starters: Vec::new(),
            })
            .await?;
        let report = coordinator.run_until_idle().await?;
        anyhow::ensure!(
            report.completed == agent_count && report.failed == 0,
            "topology failed: {report:?}"
        );
    }
    // Direct messages use global runtime agent IDs, independent of a hive.
    let recipient = agents.last().unwrap();
    fixture.script.calls.lock().unwrap().push_back((
        "hivemind_send_agent",
        json!({
            "message_id":"ordinary-direct", "agent_id":recipient.id(), "body":"GLOBAL_DIRECT_INPUT"
        }),
    ));
    agents[0]
        .turn("ORDINARY_HOST_DIRECT")
        .session("continuing:agent0")
        .send()
        .await?;
    let report = coordinator.run_until_idle().await?;
    anyhow::ensure!(
        report.completed == 1 && report.failed == 0,
        "direct delivery failed: {report:?}"
    );
    let captures = fixture.captures().await?;
    let mut counts = vec![0usize; agent_count];
    let mut originals = vec![None; agent_count];
    let mut skill_before = vec![false; agent_count];
    let mut mcp_before = vec![false; agent_count];
    let mut skill_results = vec![false; agent_count];
    let mut mcp_results = vec![false; agent_count];
    for capture in captures {
        let system = system_text(&capture);
        let agent_index = (0..agent_count)
            .find(|n| system.contains(&format!("PROMPT_MARKER_agent{n}")))
            .ok_or_else(|| anyhow::anyhow!("request lacks its host system prompt"))?;
        let id = format!("agent{agent_index}");
        let original = host_system(&capture);
        anyhow::ensure!(
            original.len() == 1,
            "missing or duplicated host-authored prompt"
        );
        if let Some(before) = &originals[agent_index] {
            anyhow::ensure!(before == &original, "adapter changed original host prompt");
        } else {
            originals[agent_index] = Some(original);
        }
        for message in capture["messages"].as_array().unwrap() {
            if message["role"] != "tool" {
                continue;
            }
            let result = message["content"].as_str().unwrap_or("");
            let call_id = message["tool_call_id"].as_str().unwrap_or("");
            let before = call_id.contains(&format!("CAPABILITY_PHASE_BEFORE_{id}-"));
            let after = call_id.contains(&format!("CAPABILITY_PHASE_AFTER_{id}-"));
            if before || after {
                let name = capture["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|m| m["tool_calls"].as_array())
                    .flatten()
                    .find(|call| call["id"] == call_id)
                    .and_then(|call| call["function"]["name"].as_str())
                    .unwrap_or("");
                let skill = name == "use_skill" && result.contains(&format!("SKILL_MARKER_{id}"));
                let mcp = name == "mcp_call_tool" && result.contains(&format!("MCP_RESULT_{id}"));
                if before {
                    skill_before[agent_index] |= skill;
                    mcp_before[agent_index] |= mcp;
                }
                if after {
                    skill_results[agent_index] |= skill;
                    mcp_results[agent_index] |= mcp;
                }
            }
            for peer in 0..agent_count {
                if peer == agent_index {
                    continue;
                }
                for marker in ["SKILL_MARKER", "MCP_MARKER", "MCP_RESULT"] {
                    anyhow::ensure!(
                        !result.contains(&format!("{marker}_agent{peer}")),
                        "native capability result leaked agent{peer}'s metadata"
                    );
                }
            }
        }
        for marker in [format!("SKILL_MARKER_{id}"), format!("MCP_MARKER_{id}")] {
            anyhow::ensure!(
                system.contains(&marker),
                "missing {marker} in captured prompt: {system}"
            );
        }
        for peer in 0..agent_count {
            if peer == agent_index {
                continue;
            }
            for marker in ["PROMPT", "SKILL", "MCP"] {
                anyhow::ensure!(
                    !system.contains(&format!("{marker}_MARKER_agent{peer}")),
                    "agent {id} sees agent{peer}'s {marker}"
                );
            }
        }
        // Initial ordinary turn precedes attachment. Every later request has
        // all tools once, including tool continuations and resumed sessions.
        if counts[agent_index] < 4 {
            anyhow::ensure!(
                TOOLS.iter().all(|name| !system.contains(name)),
                "registration affected the earlier host request"
            );
        }
        if counts[agent_index] >= 4 {
            assert_catalogue(&capture)?;
            let messages = capture["messages"].as_array().unwrap();
            anyhow::ensure!(
                messages.iter().any(|m| m["role"] == "user"
                    && m["content"]
                        .as_str()
                        .unwrap_or("")
                        .contains("HOST_ORIGINAL_INPUT")),
                "resumed turn lost host input"
            );
            anyhow::ensure!(
                messages
                    .iter()
                    .any(|m| m["role"] == "assistant" && m["content"] == REPLY),
                "resumed turn lost prior assistant reply"
            );
        }
        counts[agent_index] += 1;
    }
    for (index, count) in counts.into_iter().enumerate() {
        anyhow::ensure!(
            skill_before[index] && mcp_before[index],
            "agent{index}: pre-registration native baseline was not proven"
        );
        anyhow::ensure!(
            skill_results[index] && mcp_results[index],
            "agent{index}: post-registration native skill/MCP execution was not proven"
        );
        let expected = 8
            + 2 * hive_count
            + if index == 0 { 6 } else { 0 }
            + usize::from(index == agent_count - 1);
        anyhow::ensure!(
            count == expected,
            "agent{index}: expected {expected} requests, got {count}"
        );
    }
    Ok(())
}

pub fn system_text(capture: &Value) -> String {
    capture["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "system")
        .filter_map(|m| m["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn assert_catalogue(capture: &Value) -> anyhow::Result<()> {
    let system = system_text(capture);
    let tools = capture["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no native schemas"))?;
    anyhow::ensure!(
        !tools
            .iter()
            .any(|tool| tool["function"]["name"] == "describe_workflow"),
        "adapter widened the host's packed workflow group"
    );
    for name in TOOLS {
        anyhow::ensure!(
            system.matches(name).count() == 1,
            "{name} prompt count != 1: {system}"
        );
        anyhow::ensure!(
            tools
                .iter()
                .filter(|tool| tool["function"]["name"] == name)
                .count()
                == 1,
            "{name} native schema count != 1"
        );
    }
    Ok(())
}

pub fn host_system(capture: &Value) -> Vec<String> {
    capture["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "system")
        .filter_map(|m| m["content"].as_str())
        .filter(|s| s.starts_with("PROMPT_MARKER_"))
        .map(str::to_owned)
        .collect()
}
