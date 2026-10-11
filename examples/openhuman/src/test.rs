//! Trusted application MCP classification remains finite and fails closed.
use super::*;
use serde_json::json;

#[test]
fn exact_known_remote_verbs_keep_their_declared_effects() {
    for (server, tool, effect) in [
        ("tinyhive", "broadcast", Effect::ReadOnly),
        ("tinyhive", "complete_episode", Effect::ReadOnly),
        ("tinyhive", "hive_memory_recall", Effect::ReadOnly),
        ("tinyhive", "hive_memory_note", Effect::Mutating),
        ("tinyhive", "hive_memory_forget", Effect::Mutating),
        ("deepswe", "file_read", Effect::ReadOnly),
        ("deepswe", "file_write", Effect::Mutating),
        ("deepswe", "file_edit", Effect::Mutating),
        ("deepswe", "shell", Effect::Mutating),
        ("deepswe", "test", Effect::Mutating),
        ("unknown", "file_read", Effect::Unclassified),
        ("tinyhive", "shell", Effect::Unclassified),
        ("deepswe", "broadcast", Effect::Unclassified),
    ] {
        let context = context(
            "mcp_call_tool",
            json!({"server":server,"tool":tool,"arguments":{}}),
        );
        assert_eq!(ExperimentEffects.classify(&context), effect);
    }
    assert_eq!(
        ExperimentEffects.classify(&context("custom", json!({}))),
        Effect::Unclassified
    );
    assert_eq!(
        ExperimentEffects.classify(&context(
            "mcp_call_tool",
            json!({"server_name":"tinyhive","tool_name":"broadcast"})
        )),
        Effect::Unclassified
    );
}
fn context(name: &str, arguments: serde_json::Value) -> openhuman_embed::seams::ToolHookContext {
    openhuman_embed::seams::ToolHookContext {
        event: openhuman_core::agent::hooks::ToolHookEvent::PreToolUse,
        call_id: "fixture".into(),
        tool_name: name.into(),
        arguments,
        success: None,
        duration_ms: None,
        output: None,
        error: None,
        session_id: Some("fixture".into()),
        agent_id: Some("seat".into()),
        cwd: None,
    }
}
