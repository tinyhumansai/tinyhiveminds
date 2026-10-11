//! Replacing a registered agent's handle keeps its session and its tools.
use super::*;
use tinyhivemind_hives::{Destination, SendMessage};

async fn provider(reply: &str) -> wiremock::MockServer {
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":"fixture","object":"chat.completion","created":0,"model":"fixture",
            "choices":[{"index":0,"message":{"role":"assistant","content":reply},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        })))
        .mount(&provider)
        .await;
    provider
}
fn on(provider: &wiremock::MockServer) -> AgentSpec {
    AgentSpec::new("swap").provider(
        openhuman_embed::Provider::openai_compatible(format!("{}/v1", provider.uri()), "fixture")
            .model("fixture"),
    )
}
async fn ask(host: &OpenHumanHost, id: &str) {
    host.coordinator()
        .send_as_host(SendMessage {
            message_id: id.into(),
            sender: String::new(),
            destination: Destination::Agent("swap".into()),
            body: id.into(),
            thread: None,
            only_for: vec![],
            starters: vec![],
        })
        .await
        .unwrap();
    assert_eq!(
        host.coordinator().run_until_idle().await.unwrap().completed,
        1
    );
}
async fn bodies(provider: &wiremock::MockServer) -> Vec<String> {
    provider
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .map(|request| String::from_utf8(request.body).unwrap())
        .collect()
}
fn runner(host: &OpenHumanHost) -> Arc<SuppliedRunner> {
    host.inner.agents.lock().unwrap()["swap"].runner.clone()
}
#[test]
fn a_replaced_handle_continues_the_session_with_its_tools_attached() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {
        tokio::spawn(async {
            let (runtime, _backend, host) = Box::pin(fixture()).await;
            let first = provider("FIRST_HANDLE").await;
            let second = provider("SECOND_HANDLE").await;
            // The host keeps no clone of its own: OpenHuman ids are unique
            // while any clone of a handle is alive.
            host.register_agent(runtime.agent(on(&first)).unwrap())
                .await
                .unwrap();
            ask(&host, "before").await;
            let replacement = host
                .replace_agent("swap", || Ok(runtime.agent(on(&second))?))
                .await
                .unwrap();
            assert!(
                host.inner.agents.lock().unwrap()["swap"]
                    .agent
                    .as_ref()
                    .unwrap()
                    .same_agent(&replacement)
            );
            drop(replacement);
            ask(&host, "after").await;
            let replies: Vec<_> = host
                .coordinator()
                .read_transcript(None)
                .unwrap()
                .into_iter()
                .filter(|row| row.sender == "swap")
                .map(|row| row.body)
                .collect();
            assert_eq!(replies, ["FIRST_HANDLE", "SECOND_HANDLE"]);
            assert_eq!(bodies(&first).await.len(), 1);
            let sent = bodies(&second).await;
            assert_eq!(sent.len(), 1);
            // Same continuing session: the first handle's exchange is history.
            assert!(sent[0].contains("FIRST_HANDLE"));
            // The hivemind tools were attached to the new handle.
            assert!(sent[0].contains("hivemind_read"));
        })
        .await
        .unwrap();
    });
}
#[test]
fn replacement_waits_for_the_running_turn_and_a_failed_build_leaves_no_handle() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {
        tokio::spawn(async {
            let (runtime, _backend, host) = Box::pin(fixture()).await;
            let first = provider("FIRST_HANDLE").await;
            host.register_agent(runtime.agent(on(&first)).unwrap())
                .await
                .unwrap();
            // A running turn holds the handle; replacement waits for it.
            let running = runner(&host).agent.clone().read_owned().await;
            let built = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let witness = built.clone();
            let mut replacing = Box::pin(host.replace_agent("swap", move || {
                witness.store(true, std::sync::atomic::Ordering::SeqCst);
                Err(Error::Unauthorized("template withdrawn".into()))
            }));
            assert!(futures_poll(&mut replacing).await.is_none());
            assert!(!built.load(std::sync::atomic::Ordering::SeqCst));
            drop(running);
            assert!(matches!(replacing.await, Err(Error::Unauthorized(_))));
            assert!(built.load(std::sync::atomic::Ordering::SeqCst));
            // No handle: a claimed turn fails rather than using a stale one.
            let failed = tinyhivemind_hives::AgentRunner::run(
                runner(&host).as_ref(),
                tinyhivemind_hives::TurnRequest {
                    turn_id: String::new(),
                    scheduled_job_id: None,
                    teammates: Vec::new(),
                    agent_id: "swap".into(),
                    session_id: None,
                    messages: vec![],
                    memberships: vec![],
                    episode: None,
                    resumption: None,
                },
            )
            .await;
            assert!(failed.unwrap_err().to_string().contains("no live handle"));
            assert!(matches!(
                host.register_agent(runtime.agent(on(&first)).unwrap())
                    .await,
                Err(Error::AgentConflict(_))
            ));
            // A build returning another id is refused; a retry then succeeds.
            assert!(matches!(
                host.replace_agent("swap", || Ok(runtime.agent(AgentSpec::new("other"))?))
                    .await,
                Err(Error::AgentConflict(id)) if id == "other"
            ));
            host.replace_agent("swap", || Ok(runtime.agent(on(&first))?))
                .await
                .unwrap();
            ask(&host, "recovered").await;
        })
        .await
        .unwrap();
    });
}
/// Poll once; `None` while the future is still pending.
async fn futures_poll<F: std::future::Future + Unpin>(future: &mut F) -> Option<F::Output> {
    std::future::poll_fn(|cx| {
        std::task::Poll::Ready(match std::pin::Pin::new(&mut *future).poll(cx) {
            std::task::Poll::Ready(value) => Some(value),
            std::task::Poll::Pending => None,
        })
    })
    .await
}
#[test]
fn replacing_requires_an_agent_registered_with_this_host() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    executor().block_on(async {
        tokio::spawn(async {
            let (runtime, _backend, host) = Box::pin(fixture()).await;
            assert!(matches!(
                host.replace_agent("absent", || Ok(runtime.agent(AgentSpec::new("absent"))?))
                    .await,
                Err(Error::Coordinator(tinyhivemind_hives::Error::UnknownAgent(id))) if id == "absent"
            ));
        })
        .await
        .unwrap();
    });
}
