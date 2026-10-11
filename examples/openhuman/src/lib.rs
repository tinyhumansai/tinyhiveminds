//! Manifest-backed example construction, keeping native ports outside checked-in JSON.
//! The deployment owns the runtime and persistent seats; these helpers supply
//! paths and classify the experiments' application-owned MCP tools.
use std::{path::Path, sync::Arc};
use tinyhivemind_core::approval::Effect;
use tinyhivemind_openhuman::{
    config::HiveConfig,
    deploy::{BuildOptions, EffectClassifier, EnvironmentSecrets, HiveDeployment},
};

/// Load a checked-in example, without resolving credentials or starting a runtime.
/// # Errors
/// Returns parsing, filesystem or manifest validation failures.
pub fn manifest(name: &str) -> anyhow::Result<HiveConfig> {
    Ok(HiveConfig::load_dir(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("hives")
            .join(name),
    )?)
}

/// Classify the exact application MCP verbs served by these examples.
/// Unknown servers and tools remain refused.
pub struct ExperimentEffects;
impl EffectClassifier for ExperimentEffects {
    fn classify(&self, context: &openhuman_embed::seams::ToolHookContext) -> Effect {
        if context.tool_name != "mcp_call_tool" {
            return Effect::Unclassified;
        }
        match (
            context.arguments["server"].as_str(),
            context.arguments["tool"].as_str(),
        ) {
            (Some("tinyhive"), Some("broadcast" | "complete_episode" | "hive_memory_recall")) => {
                Effect::ReadOnly
            }
            (Some("tinyhive"), Some("hive_memory_note" | "hive_memory_forget")) => Effect::Mutating,
            (Some("deepswe"), Some("file_read")) => Effect::ReadOnly,
            (Some("deepswe"), Some("file_write" | "file_edit" | "shell" | "test")) => {
                Effect::Mutating
            }
            _ => Effect::Unclassified,
        }
    }
}

/// Build from an editable manifest after applying runtime-only paths and native ports.
/// # Errors
/// Returns structural validation or native deployment failures.
pub async fn deploy(
    config: HiveConfig,
    mut options: BuildOptions,
) -> anyhow::Result<HiveDeployment> {
    options.classifier = Some(Arc::new(ExperimentEffects));
    Ok(Box::pin(HiveDeployment::build_with(
        config.validate()?,
        Arc::new(EnvironmentSecrets),
        options,
    ))
    .await?)
}

#[cfg(test)]
mod test;
