//! Offline fixtures use one shared process runtime lock.
use crate::{
    config::HiveConfig,
    deploy::{BuildOptions, DeployResult, SecretResolver},
};
use std::sync::Arc;
pub(super) fn executor() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()?)
}
pub(super) fn run<T: Send + 'static>(
    future: impl std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
) -> anyhow::Result<T> {
    executor()?.block_on(async move { tokio::spawn(future).await? })
}
pub(super) fn manifest() -> anyhow::Result<HiveConfig> {
    Ok(HiveConfig::from_json(
        r#"{
 "runtime":{"autonomy":"autonomous","model":{"name":"fixture"}},
 "profiles":[{"id":"worker","system_prompt":"Act only using the current episode"}],
 "seats":[{"id":"alice","profile":"worker"},{"id":"bob","profile":"worker"}],
 "permission_profiles":[{"id":"narrow","access":"autonomous","deny_tools":["write"]}],
 "hives":[{"id":"a","members":[{"seat":"alice","role":"lead"},{"seat":"bob","role":"reviewer"}]},{"id":"b","members":[{"seat":"alice","role":"observer","permission_profile":"narrow"}]}]
}"#,
    )?)
}
pub(super) struct Secrets;
impl SecretResolver for Secrets {
    fn resolve(&self, _: &crate::config::SecretRef) -> DeployResult<String> {
        Ok("fixture".into())
    }
}
pub(super) async fn build_options() -> (BuildOptions, wiremock::MockServer, wiremock::MockServer) {
    let backend = crate::offline::backend().await;
    let model = wiremock::MockServer::start().await;
    let options = BuildOptions {
        config: Some(crate::offline::config()),
        backend_url: Some(backend.uri()),
        provider: Some(
            openhuman_embed::Provider::openai_compatible(format!("{}/v1", model.uri()), "fixture")
                .model("fixture"),
        ),
        ..Default::default()
    };
    (options, backend, model)
}
pub(super) fn scope(hive: &str) -> crate::TurnScope {
    crate::TurnScope {
        turn_id: "turn".into(),
        scheduled_job_id: None,
        session_id: Some("session".into()),
        agent_id: "alice".into(),
        episode: Some(tinyhivemind_hives::EpisodeContext {
            episode_id: "episode".into(),
            hive_id: hive.into(),
            thread: None,
            brief: String::new(),
        }),
        message_ids: vec!["message".into()],
        senders: vec!["hivemind:host".into()],
        destination: tinyhivemind_hives::Destination::Hive(hive.into()),
        thread: None,
    }
}
pub(super) fn context(name: &str) -> openhuman_embed::seams::ToolHookContext {
    openhuman_embed::seams::ToolHookContext {
        event: openhuman_core::agent::hooks::ToolHookEvent::PreToolUse,
        call_id: "call".into(),
        tool_name: name.into(),
        arguments: serde_json::json!({"path":"secret-user-payload"}),
        success: None,
        duration_ms: None,
        output: None,
        error: None,
        session_id: Some("session".into()),
        agent_id: Some("alice".into()),
        cwd: None,
    }
}
pub(super) fn permissions(
    config: HiveConfig,
    options: &BuildOptions,
) -> Arc<super::super::Permissions> {
    Arc::new(super::super::Permissions::new(config, options))
}
