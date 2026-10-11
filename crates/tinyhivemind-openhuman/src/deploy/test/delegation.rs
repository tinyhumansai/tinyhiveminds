//! Native delegation preserves the child's declared restrictions.
use super::{
    support::*,
    turns::{message, source},
};
use crate::{config::Profile, deploy::HiveDeployment};
use serde_json::{Value, json};
use std::sync::{
    Arc, PoisonError,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{Request, Respond, ResponseTemplate};

#[derive(Clone)]
struct DelegateScript(Arc<AtomicUsize>);
impl Respond for DelegateScript {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
            return ResponseTemplate::new(400);
        };
        let Some(messages) = body["messages"].as_array() else {
            return ResponseTemplate::new(400);
        };
        let Some(last) = messages.last() else {
            return ResponseTemplate::new(400);
        };
        let episode = messages
            .iter()
            .filter_map(|message| message["content"].as_str())
            .filter_map(|text| {
                text.find('{')
                    .and_then(|start| serde_json::from_str::<Value>(&text[start..]).ok())
            })
            .find_map(|turn| turn["episode"]["episode_id"].as_str().map(str::to_owned));
        let completed = messages.iter().any(|message| {
            message["tool_calls"].as_array().is_some_and(|calls| {
                calls
                    .iter()
                    .any(|call| call["function"]["name"] == "hivemind_complete")
            })
        });
        let call = if completed {
            None
        } else if last["role"] == "user" {
            if episode.is_some() {
                Some((
                    "delegate_helper",
                    json!({"prompt":"Attempt the restricted write", "blocking":true}),
                ))
            } else {
                self.0.fetch_add(1, Ordering::Relaxed);
                Some(("write", json!({})))
            }
        } else {
            episode
                .as_ref()
                .map(|id| ("hivemind_complete", json!({"episode_id":id,"body":"done"})))
        };
        let message = call.as_ref().map_or_else(
            || json!({"role":"assistant","content":"done"}),
            |(name, arguments)| json!({"role":"assistant","content":null,"tool_calls":[{"id":"delegate-fixture","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]}));
        ResponseTemplate::new(200).set_body_json(json!({"id":"fixture","object":"chat.completion","created":0,"model":"fixture","choices":[{"index":0,"message":message,"finish_reason":if call.is_some(){"tool_calls"}else{"stop"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))
    }
}

#[test]
fn native_delegate_cannot_execute_a_child_denied_effect() -> anyhow::Result<()> {
    let _lock = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    run(async {
        let (mut options, _backend, model) = build_options().await;
        let child_requests = Arc::new(AtomicUsize::new(0));
        // `mount` accepts the shared episode script; delegation needs a distinct
        // response for the native child's plain-text task.
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(DelegateScript(child_requests.clone()))
            .mount(&model)
            .await;
        let count = Arc::new(AtomicUsize::new(0));
        options.tools = Some(source(count.clone()));
        let mut config = manifest()?;
        config.profiles.push(Profile {
            id: "helper".into(),
            deny_tools: vec!["write".into()],
            ..Default::default()
        });
        config.profiles[0].subagents.push("helper".into());
        let deployment = Box::pin(HiveDeployment::build_with(
            config.validate()?,
            Arc::new(Secrets),
            options,
        ))
        .await?;
        deployment
            .host()
            .coordinator()
            .send_as_host(message("delegate", "a"))
            .await?;
        let _report = deployment.host().coordinator().run_until_idle().await?;
        assert!(
            child_requests.load(Ordering::Relaxed) > 0,
            "native child never ran"
        );
        assert_eq!(count.load(Ordering::Relaxed), 0);
        Ok(())
    })
}
