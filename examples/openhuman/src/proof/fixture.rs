//! Host-only agent configuration and loopback model fixtures.
use openhuman_embed::{
    Access, Agent, AgentDefinitionSpec, AgentSpec, McpServer, Provider, Runtime, ToolScopeSpec,
    Workspace,
};
use serde_json::{Value, json};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub const REPLY: &str = "configured-agent-reply";
pub const TOOLS: [&str; 9] = [
    "hivemind_list_hives",
    "hivemind_list_agents",
    "hivemind_read",
    "hivemind_send_hive",
    "hivemind_send_agent",
    "hivemind_post",
    "hivemind_ask",
    "hivemind_broadcast",
    "hivemind_complete",
];

pub(super) use super::types::Fixture;
use super::types::Script;
impl Respond for Script {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let last = body["messages"].as_array().unwrap().last().unwrap();
        let next = self.calls.lock().unwrap().pop_front().or_else(|| {
            if last["role"] == "tool"
                && body["messages"]
                    .as_array()?
                    .iter()
                    .rev()
                    .find(|m| m["role"] == "assistant" && m["tool_calls"].is_array())?["tool_calls"]
                    [0]["function"]["name"]
                    == "hivemind_complete"
            {
                return None;
            }
            let input = body["messages"]
                .as_array()?
                .iter()
                .rev()
                .find(|m| m["role"] == "user")?;
            let text = input["content"].as_str()?;
            let start = text.find('{')?;
            let turn: Value = serde_json::from_str(&text[start..]).ok()?;
            let episode_id = turn["episode"]["episode_id"].as_str()?;
            Some((
                "hivemind_complete",
                json!({"episode_id":episode_id,"body":"Fixture assignment complete"}),
            ))
        });
        let is_call = next.is_some();
        let phase = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|m| m["role"] == "user")
            .and_then(|m| m["content"].as_str())
            .and_then(|text| {
                text.split_whitespace()
                    .find(|word| word.starts_with("CAPABILITY_PHASE_"))
            })
            .unwrap_or("ordinary");
        let call_id = format!(
            "fixture-call-{phase}-{}",
            self.sequence
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );
        let message = match next {
            Some((name, args)) => json!({"role":"assistant","content":null,
                "tool_calls":[{"id":call_id,"type":"function",
                    "function":{"name":name,"arguments":args.to_string()}}]}),
            None => json!({"role":"assistant","content":REPLY}),
        };
        ResponseTemplate::new(200).set_body_json(json!({
            "id":"fixture-completion","object":"chat.completion","created":1700000000,
            "model":"fixture","choices":[{"index":0,"message":message,
                "finish_reason":if is_call {"tool_calls"} else {"stop"}}],
            "usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}
        }))
    }
}

impl Fixture {
    pub async fn new() -> anyhow::Result<Self> {
        let backend = tinyhivemind_openhuman::offline::backend().await;
        let provider = MockServer::start().await;
        let script = Script::default();
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(script.clone())
            .mount(&provider)
            .await;
        let runtime = Runtime::builder()
            .config(tinyhivemind_openhuman::offline::config())
            .workspace(Workspace::Ephemeral)
            .backend_url(backend.uri())
            .provider(
                Provider::openai_compatible(format!("{}/v1", provider.uri()), "fixture")
                    .model("fixture"),
            )
            .access(Access::readonly())
            .build()
            .await?;
        Ok(Self {
            runtime: Arc::new(runtime),
            provider,
            script,
            _backend: backend,
            files: tempfile::tempdir()?,
        })
    }

    /// Construct configured agents before they reach TinyHivemind.
    pub fn agent(&self, id: &str) -> anyhow::Result<Agent> {
        configured_agent(&self.runtime, self.files.path(), id)
    }
    pub async fn captures(&self) -> anyhow::Result<Vec<Value>> {
        self.provider
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/v1/chat/completions")
            .map(|r| serde_json::from_slice(&r.body).map_err(Into::into))
            .collect()
    }
}

pub fn configured_agent(
    runtime: &Runtime,
    root: &std::path::Path,
    id: &str,
) -> anyhow::Result<Agent> {
    // Validate before deriving any host filesystem path from model settings.
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= 64
            && id.as_bytes()[0].is_ascii_alphanumeric()
            && id.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'_'
                || byte == b'-'),
        "invalid host agent id"
    );
    anyhow::ensure!(
        !runtime.agent_ids().iter().any(|known| known == id),
        "host agent id already exists"
    );
    let workspace = root.join(id).join("workspace");
    let skills = workspace.join("skills");
    let bundle = skills.join(format!("skill-{id}"));
    std::fs::create_dir_all(&bundle)?;
    std::fs::write(
        bundle.join("SKILL.md"),
        format!(
            "---\nname: skill-{id}\ndescription: SKILL_MARKER_{id} is this agent's specialist skill.\n---\n# Specialist\nUse evidence.\n"
        ),
    )?;
    let server = root.join(id).join("mcp.py");
    std::fs::write(&server, MCP_SCRIPT)?;
    std::fs::create_dir_all(&workspace)?;
    // Inline prompts are host-authored: OpenHuman intentionally does not
    // add a skill/MCP catalogue around a custom body. This host describes its
    // installed resources and later verifies their actual native execution.
    let installed_skill = std::fs::read_to_string(bundle.join("SKILL.md"))?;
    let prompt = format!(
        "PROMPT_MARKER_{id}. Host-configured specialist.\nInstalled skill:\n{installed_skill}\nConnected MCP: MCP_MARKER_{id}"
    );
    let agent = runtime.agent(
        AgentSpec::new(id)
            // Native MCP dispatch is an acting operation even for this benign
            // evidence server. Restrict callable builtins and remote verbs.
            .access(Access::full())
            .definition(
                AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(
                    ["use_skill", "mcp_list_tools", "mcp_call_tool"]
                        .map(str::to_owned)
                        .to_vec(),
                )),
            )
            .system_prompt(prompt)
            .tool_groups(openhuman_core::tools::toolpacks::ToolGroups::packed())
            .config(move |config| config.workspace_dir = workspace)
            .mcp(
                McpServer::stdio(
                    format!("mcp-{id}"),
                    "python3",
                    vec![server.to_string_lossy().into_owned(), id.to_owned()],
                )
                .description(format!("MCP_MARKER_{id}"))
                .allow_tools([format!("evidence_{id}")]),
            ),
    )?;
    Ok(agent)
}

// Harmless, private MCP for each agent, configured by the host.
const MCP_SCRIPT: &str = r#"import json, sys
agent = sys.argv[1]
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req:
        continue
    method = req.get('method')
    result = {}
    if method == 'initialize':
        result = {'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'mcp-'+agent,'version':'1'}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'evidence_'+agent,'description':'MCP_MARKER_'+agent,'inputSchema':{'type':'object','properties':{}}}]}
    elif method == 'tools/call':
        result = {'content':[{'type':'text','text':'MCP_RESULT_'+agent}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}), flush=True)
"#;

/// Invoke packed installed skills and configured MCP actions natively.
pub fn queue_capabilities(fixture: &Fixture, id: &str) {
    fixture.script.calls.lock().unwrap().extend([
        (
            "use_skill",
            json!({"skill":"workflows","tool":"describe_workflow","args":{"workflow_id":format!("skill-{id}")}}),
        ),
        ("mcp_list_tools",json!({"server":format!("mcp-{id}")})),
        ("mcp_call_tool",json!({"server":format!("mcp-{id}"),"tool":format!("evidence_{id}"),"arguments":{}})),
    ]);
}
