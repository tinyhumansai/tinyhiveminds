//! Real MCP envelopes retain scheduled denials before remote effects.
use super::{
    support::*,
    turns::{Script, message, mount},
};
use crate::{config::*, deploy::*};
use serde_json::{Value, json};
use std::sync::{
    Arc, PoisonError,
    atomic::{AtomicUsize, Ordering},
};
struct Remote(Arc<AtomicUsize>);
impl wiremock::Respond for Remote {
    fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let result = match body["method"].as_str().unwrap_or_default() {
            "initialize" => {
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"remote","version":"1"}})
            }
            "notifications/initialized" => return wiremock::ResponseTemplate::new(202),
            "tools/list" => {
                json!({"tools":[{"name":"mutate","description":"Write fixture","inputSchema":{"type":"object"}}]})
            }
            "tools/call" => {
                self.0.fetch_add(1, Ordering::Relaxed);
                json!({"content":[{"type":"text","text":"written"}]})
            }
            _ => json!({}),
        };
        wiremock::ResponseTemplate::new(200)
            .set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
    }
}
struct RemoteEffects;
impl EffectClassifier for RemoteEffects {
    fn classify(
        &self,
        context: &openhuman_embed::seams::ToolHookContext,
    ) -> tinyhivemind_core::approval::Effect {
        if context.tool_name == "mcp_call_tool"
            && context.arguments["server"] == "remote"
            && context.arguments["tool"] == "mutate"
            || context.tool_name == "mcp_remote_mutate"
        {
            tinyhivemind_core::approval::Effect::Mutating
        } else {
            tinyhivemind_core::approval::Effect::Unclassified
        }
    }
}
#[test]
fn scheduled_mcp_envelope_and_private_children_never_reach_remote_effect() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let remote = wiremock::MockServer::start().await;
        let count = Arc::new(AtomicUsize::new(0));
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(Remote(count.clone()))
            .mount(&remote)
            .await;
        let (mut options, _backend, model) = build_options().await;
        options.classifier = Some(Arc::new(RemoteEffects));
        let script = Script {
            action: Some("mcp_call_tool".into()),
            arguments: Some(json!({"server":"remote","tool":"mutate","arguments":{}})),
            child: true,
            ..Default::default()
        };
        mount(&model, script).await;
        let mut config = manifest()?;
        config.runtime.mcp.push(McpServer {
            id: "remote".into(),
            transport: "http".into(),
            endpoint: remote.uri(),
            args: Vec::new(),
            env: std::collections::BTreeMap::default(),
            headers: std::collections::BTreeMap::default(),
            tools: None,
            deny_tools: Vec::new(),
        });
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        deployment
            .host()
            .coordinator()
            .send_scheduled_as_host("trusted-job", message("mcp-scheduled", "a"))
            .await?;
        let report = deployment.host().coordinator().run_until_idle().await?;
        assert!(report.failed > 0, "{report:?}");
        assert_eq!(count.load(Ordering::Relaxed), 0);
        // The first queued message retains origin after the enqueue call returned.
        assert!(
            deployment
                .host()
                .coordinator()
                .read_transcript(None)?
                .iter()
                .any(|row| !row.only_for.is_empty())
        );
        deployment
            .host()
            .coordinator()
            .send_as_host(message("mcp-interactive", "a"))
            .await?;
        let report = deployment.host().coordinator().run_until_idle().await?;
        assert_eq!(report.failed, 0, "{report:?}");
        assert!(
            count.load(Ordering::Relaxed) > 0,
            "interactive MCP call must reach the fixture"
        );
        Ok(())
    })
}
