//! Durable shared-workspace initialization and per-turn prompt snapshots.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::{MODEL, TASK};

const AGENTS_TEMPLATE: &str = include_str!("../../../hives/pe1006_hive/context/agents.md");

const MEMORY_TEMPLATE: &str = include_str!("../../../hives/pe1006_hive/context/memory.md");

pub(super) struct TurnSnapshots {
    directory: PathBuf,
    index: PathBuf,
    next: usize,
}

pub(super) struct PendingSnapshot {
    directory: PathBuf,
    turn: usize,
    route_id: String,
    agent_id: String,
    session_id: String,
    prompt_chars: usize,
}

impl TurnSnapshots {
    pub(super) fn new(run_dir: &Path) -> anyhow::Result<Self> {
        let directory = run_dir.join("turns");
        std::fs::create_dir_all(&directory)?;
        let index = directory.join("README.md");
        std::fs::write(
            &index,
            "# Agent turn snapshots\n\n| Turn | Route | OpenHuman agent | Session | Status | Files |\n| ---: | --- | --- | --- | --- | --- |\n",
        )?;
        Ok(Self {
            directory,
            index,
            next: 1,
        })
    }

    pub(super) fn begin(
        &mut self,
        route_id: &str,
        agent_id: &str,
        session_id: &str,
        prompt: &str,
    ) -> anyhow::Result<PendingSnapshot> {
        let turn = self.next;
        self.next += 1;
        let directory = self.directory.join(format!("{turn:03}-{route_id}"));
        std::fs::create_dir_all(&directory)?;
        std::fs::write(directory.join("prompt.md"), prompt)?;
        let pending = PendingSnapshot {
            directory,
            turn,
            route_id: route_id.to_string(),
            agent_id: agent_id.to_string(),
            session_id: session_id.to_string(),
            prompt_chars: prompt.chars().count(),
        };
        pending.write_metadata("started", None)?;
        Ok(pending)
    }

    pub(super) fn complete(&self, pending: PendingSnapshot, reply: &str) -> anyhow::Result<()> {
        std::fs::write(pending.directory.join("reply.md"), reply)?;
        pending.write_metadata("completed", Some(reply.chars().count()))?;
        let mut index = std::fs::OpenOptions::new().append(true).open(&self.index)?;
        writeln!(
            index,
            "| {} | @{} | `{}` | `{}` | completed | [prompt](./{:03}-{}/prompt.md), [reply](./{:03}-{}/reply.md), [metadata](./{:03}-{}/metadata.json) |",
            pending.turn,
            pending.route_id,
            pending.agent_id,
            pending.session_id,
            pending.turn,
            pending.route_id,
            pending.turn,
            pending.route_id,
            pending.turn,
            pending.route_id,
        )?;
        Ok(())
    }
}

impl PendingSnapshot {
    fn write_metadata(&self, status: &str, reply_chars: Option<usize>) -> anyhow::Result<()> {
        std::fs::write(
            self.directory.join("metadata.json"),
            serde_json::to_vec_pretty(&json!({
                "turn": self.turn,
                "route_id": self.route_id,
                "openhuman_agent_id": self.agent_id,
                "session_id": self.session_id,
                "model": MODEL,
                "status": status,
                "prompt_chars": self.prompt_chars,
                "reply_chars": reply_chars,
            }))?,
        )?;
        Ok(())
    }
}

pub(super) fn hive_workspace() -> PathBuf {
    std::env::var_os("OPENHUMAN_HIVE_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("workspace/pe1006"))
}

pub(super) fn initialize_workspace(workspace: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(workspace.join("runs"))?;
    write_if_missing(&workspace.join("AGENTS.md"), AGENTS_TEMPLATE)?;
    write_if_missing(&workspace.join("MEMORY.md"), MEMORY_TEMPLATE)?;
    write_if_missing(&workspace.join("TASK.md"), TASK)?;
    Ok(())
}

fn write_if_missing(path: &Path, content: &str) -> anyhow::Result<()> {
    if !path.exists() {
        std::fs::write(path, content)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "workspace_test.rs"]
mod test;
