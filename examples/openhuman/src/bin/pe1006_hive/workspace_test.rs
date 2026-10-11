//! Workspace initialization and exact prompt/reply snapshot regressions.
use super::*;

fn test_directory(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "tinyhivemind-openhuman-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

#[test]
fn workspace_templates_are_created_once_and_preserve_agent_memory() -> anyhow::Result<()> {
    let workspace = test_directory("workspace");
    initialize_workspace(&workspace)?;
    assert_eq!(
        std::fs::read_to_string(workspace.join("AGENTS.md"))?,
        AGENTS_TEMPLATE
    );
    std::fs::write(workspace.join("MEMORY.md"), "agent-authored memory\n")?;

    initialize_workspace(&workspace)?;

    assert_eq!(
        std::fs::read_to_string(workspace.join("MEMORY.md"))?,
        "agent-authored memory\n"
    );
    assert_eq!(std::fs::read_to_string(workspace.join("TASK.md"))?, TASK);
    std::fs::remove_dir_all(workspace)?;
    Ok(())
}

#[test]
fn turn_snapshots_capture_prompt_reply_session_and_index() -> anyhow::Result<()> {
    let run_dir = test_directory("snapshots");
    let mut snapshots = TurnSnapshots::new(&run_dir)?;
    let pending = snapshots.begin(
        "checker",
        "checker-pe1006-fixture",
        "company:agent:checker",
        "full prompt",
    )?;
    snapshots.complete(pending, "full reply")?;

    let turn = run_dir.join("turns/001-checker");
    assert_eq!(
        std::fs::read_to_string(turn.join("prompt.md"))?,
        "full prompt"
    );
    assert_eq!(
        std::fs::read_to_string(turn.join("reply.md"))?,
        "full reply"
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(turn.join("metadata.json"))?)?;
    assert_eq!(metadata["session_id"], "company:agent:checker");
    assert_eq!(metadata["status"], "completed");
    assert!(
        std::fs::read_to_string(run_dir.join("turns/README.md"))?.contains("001-checker/prompt.md")
    );
    std::fs::remove_dir_all(run_dir)?;
    Ok(())
}
