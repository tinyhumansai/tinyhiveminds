//! `Recall` and `Remember` over the reference engine: what is written is
//! recalled, under the namespace `OpenHuman`'s seats use, and nowhere else.
// Test assertions deliberately panic on invalid fixture construction.
#![allow(clippy::unwrap_used)]
use super::HiveMemoryStore;
use crate::{Error, HiveMemory};
use openhuman_core::memory::{lifecycle::agent_memory_on, scope::MemoryIdentity};
use openhuman_embed::RuntimeConfig;
use std::{num::NonZeroU32, sync::Arc};
use tinyhivemind_core::runtime::{
    self, BriefingNote, EntryKind, Recall, RecallMoment, RecallRequest, Remember, RememberEntry,
    RememberRequest, Sequence, frame_recalled,
};
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{
    EngineDescriptor, EngineHealth, ForgetReport, ForgetTarget, ItemKind, LearningKind, ListPage,
    ListRequest, MemoryEngine, MetaFilter, StoreItem, StoreReceipt,
};
use tinymemory_tools::{PostTurn, RecallPolicy};

const RUN: &str = "run-1";

fn store(engine: &Arc<ReferenceEngine>, hive: &str) -> HiveMemoryStore {
    HiveMemoryStore::new(engine.clone(), HiveMemory::for_hive(hive).unwrap()).unwrap()
}

fn ask(seat: &str, moment: RecallMoment) -> RecallRequest {
    RecallRequest {
        seat: seat.into(),
        conversation: RUN.into(),
        focus: None,
        moment,
        budget_chars: 4_000,
    }
}

fn entries(seat: &str, entries: &[(EntryKind, &str)]) -> RememberRequest {
    RememberRequest {
        seat: seat.into(),
        conversation: RUN.into(),
        through: Some(Sequence(7)),
        entries: entries
            .iter()
            .map(|(kind, text)| RememberEntry {
                kind: *kind,
                text: (*text).into(),
            })
            .collect(),
    }
}

fn lines(notes: &[BriefingNote]) -> Vec<String> {
    notes.iter().flat_map(|note| note.lines.clone()).collect()
}

fn headings(notes: &[BriefingNote]) -> Vec<&str> {
    notes.iter().map(|note| note.heading.as_str()).collect()
}

/// A seat's turn as `OpenHuman`'s lifecycle logs it for a bound seat.
async fn openhuman_logs(
    engine: &Arc<ReferenceEngine>,
    hive: &str,
    seat: &str,
    thread: &str,
    text: &str,
) {
    let mut config = RuntimeConfig::default();
    config.memory.agent_id = Some(seat.into());
    config.memory.root = Some(format!("team:{hive}"));
    let identity = MemoryIdentity::agent(seat).resolve(&config);
    let memory = agent_memory_on(engine.clone(), &config, &identity).unwrap();
    memory
        .post_turn(PostTurn::new(thread, 1, text))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_remembered_entry_is_recalled_by_every_seat_of_the_hive() {
    let engine = Arc::new(ReferenceEngine::new());
    let hive = store(&engine, "h");
    let entry = entries(
        "builder",
        &[(EntryKind::Observation, "make test needs libssl-dev")],
    );
    hive.remember(&entry).await.unwrap();
    for seat in ["builder", "critic"] {
        let notes = hive
            .recall(&ask(seat, RecallMoment::SessionStart))
            .await
            .unwrap();
        let learnings = notes
            .iter()
            .find(|note| note.heading == "Learnings")
            .unwrap();
        assert_eq!(learnings.lines, ["Observation: make test needs libssl-dev"]);
    }
}

#[tokio::test]
async fn a_failed_attempt_is_marked_in_its_text_kind_and_tags() {
    let engine = Arc::new(ReferenceEngine::new());
    let hive = store(&engine, "h");
    let entry = entries(
        "builder",
        &[(
            EntryKind::FailedAttempt,
            "pip install without --user is denied",
        )],
    );
    hive.remember(&entry).await.unwrap();
    let notes = hive
        .recall(&ask("critic", RecallMoment::SessionStart))
        .await
        .unwrap();
    assert!(lines(&notes).contains(&"Failed attempt: pip install without --user is denied".into()));
    let page = engine
        .list(ListRequest::new(
            MetaFilter::kinds([ItemKind::Learning]),
            10,
        ))
        .await
        .unwrap();
    let stored = &page.items[0];
    assert_eq!(stored.meta.namespace.to_string(), "team:h");
    assert_eq!(stored.meta.agent_id.as_deref(), Some("builder"));
    assert_eq!(stored.meta.thread_id.as_deref(), Some(RUN));
    assert_eq!(
        stored.meta.tags,
        ["hive-entry:failed_attempt", "desk-through:7"]
    );
}

#[test]
fn entries_become_learnings_of_the_matching_kind() {
    use super::super::convert::{EntryContext, entry_item};
    let at = "team:h".parse().unwrap();
    let context = EntryContext {
        at: &at,
        agent_id: "builder",
        conversation: RUN,
        through: None,
    };
    let cases = [
        (EntryKind::Observation, LearningKind::Fact),
        (EntryKind::FailedAttempt, LearningKind::Correction),
        (EntryKind::Outcome, LearningKind::Fact),
        (EntryKind::Note, LearningKind::Other),
    ];
    for (entry_kind, learning) in cases {
        let entry = RememberEntry {
            kind: entry_kind,
            text: " text ".into(),
        };
        let item = entry_item(&context, &entry);
        assert!(
            matches!(
                &item,
                Some(StoreItem::Learning { kind, meta, .. })
                    if *kind == learning && meta.tags.len() == 1
            ),
            "{entry_kind:?} became {item:?}"
        );
    }
    let blank = RememberEntry {
        kind: EntryKind::Note,
        text: "\n ".into(),
    };
    assert!(entry_item(&context, &blank).is_none());
}

#[tokio::test]
async fn every_entry_kind_is_labelled() {
    let engine = Arc::new(ReferenceEngine::new());
    let hive = store(&engine, "h");
    let all = [
        (EntryKind::Observation, "one"),
        (EntryKind::FailedAttempt, "two"),
        (EntryKind::Outcome, "three"),
        (EntryKind::Note, "four"),
    ];
    hive.remember(&entries("builder", &all)).await.unwrap();
    hive.remember(&entries("builder", &[])).await.unwrap();
    let recalled = lines(
        &hive
            .recall(&ask("critic", RecallMoment::SessionStart))
            .await
            .unwrap(),
    );
    for line in [
        "Observation: one",
        "Failed attempt: two",
        "Outcome: three",
        "Note: four",
    ] {
        assert!(
            recalled.contains(&line.to_owned()),
            "{line} in {recalled:?}"
        );
    }
}

#[tokio::test]
async fn recall_reads_the_turns_openhuman_seats_log() {
    let engine = Arc::new(ReferenceEngine::new());
    openhuman_logs(
        &engine,
        "h",
        "builder",
        "session-9",
        "the fixture server listens on 8081",
    )
    .await;
    let hive = store(&engine, "h");
    let notes = hive
        .recall(&ask("critic", RecallMoment::SessionStart))
        .await
        .unwrap();
    let team = notes
        .iter()
        .find(|note| note.heading == "Team conversations")
        .unwrap();
    assert!(team.lines[0].contains("8081"), "{team:?}");
}

#[tokio::test]
async fn rejoin_leaves_out_the_seat_own_history_and_session_start_does_not() {
    let engine = Arc::new(ReferenceEngine::new());
    openhuman_logs(
        &engine,
        "h",
        "builder",
        "session-1",
        "builder ran the suite",
    )
    .await;
    let hive = store(&engine, "h");
    let start = hive
        .recall(&ask("builder", RecallMoment::SessionStart))
        .await
        .unwrap();
    assert!(
        headings(&start).contains(&"This agent's history"),
        "{start:?}"
    );
    hive.remember(&entries(
        "builder",
        &[(EntryKind::Note, "builder's own note")],
    ))
    .await
    .unwrap();
    let rejoin = hive
        .recall(&ask("builder", RecallMoment::Rejoin))
        .await
        .unwrap();
    assert!(
        rejoin.is_empty(),
        "nothing but its own work exists: {rejoin:?}"
    );
    let peer = hive
        .recall(&ask("critic", RecallMoment::Rejoin))
        .await
        .unwrap();
    assert!(headings(&peer).contains(&"Team conversations"), "{peer:?}");
}

#[tokio::test]
async fn the_compaction_moment_takes_the_compaction_path() {
    let engine = Arc::new(ReferenceEngine::new());
    openhuman_logs(&engine, "h", "builder", RUN, "the build needs cmake 3.28").await;
    let hive = store(&engine, "h");
    let start = hive
        .recall(&ask("builder", RecallMoment::SessionStart))
        .await
        .unwrap();
    assert!(
        headings(&start).contains(&"Earlier in this thread"),
        "{start:?}"
    );
    assert!(!headings(&start).contains(&"Earlier in this conversation"));
    let moment = RecallMoment::Compaction {
        dropped: vec!["configured the build with cmake".into()],
    };
    let compacted = hive.recall(&ask("builder", moment)).await.unwrap();
    let summary = compacted
        .iter()
        .find(|note| note.heading == "Earlier in this conversation")
        .unwrap();
    assert!(
        summary.lines.iter().any(|line| line.contains("cmake 3.28")),
        "{summary:?}"
    );
}

#[tokio::test]
async fn two_hive_roots_on_one_engine_do_not_share() {
    let engine = Arc::new(ReferenceEngine::new());
    let first = store(&engine, "hive-a");
    let second = store(&engine, "hive-b");
    first
        .remember(&entries(
            "builder",
            &[(EntryKind::Outcome, "tests pass in hive a")],
        ))
        .await
        .unwrap();
    openhuman_logs(&engine, "hive-a", "builder", "s", "hive a's turn").await;
    for moment in [RecallMoment::SessionStart, RecallMoment::Rejoin] {
        let notes = second.recall(&ask("builder", moment)).await.unwrap();
        assert!(notes.is_empty(), "{notes:?}");
    }
    let own = first
        .recall(&ask("critic", RecallMoment::Rejoin))
        .await
        .unwrap();
    assert_eq!(headings(&own), ["Learnings", "Team conversations"]);
}

#[tokio::test]
async fn recalled_notes_stay_within_the_character_budget() {
    let engine = Arc::new(ReferenceEngine::new());
    let hive = store(&engine, "h");
    let many: Vec<_> = (0..20)
        .map(|index| {
            (
                EntryKind::Note,
                format!("finding number {index} about the build"),
            )
        })
        .collect();
    let borrowed: Vec<_> = many
        .iter()
        .map(|(kind, text)| (*kind, text.as_str()))
        .collect();
    hive.remember(&entries("builder", &borrowed)).await.unwrap();
    let mut request = ask("critic", RecallMoment::SessionStart);
    request.budget_chars = 120;
    let notes = hive.recall(&request).await.unwrap();
    let spent: usize = lines(&notes).iter().map(|line| line.chars().count()).sum();
    assert!(spent > 0 && spent <= 120, "{spent}: {notes:?}");
    let framed = frame_recalled(&notes, 400).unwrap();
    assert!(framed.contains("### Learnings"));
}

#[tokio::test]
async fn recall_failures_are_recall_errors() {
    let engine = Arc::new(ReferenceEngine::new());
    let hive = store(&engine, "h");
    let mut blank = ask("critic", RecallMoment::SessionStart);
    blank.conversation = "  ".into();
    assert!(matches!(
        hive.recall(&blank).await,
        Err(runtime::Error::Recall { .. })
    ));
    let unusable = ask("Bad Seat", RecallMoment::Rejoin);
    let error = hive.recall(&unusable).await.unwrap_err();
    assert!(
        matches!(
            &error,
            runtime::Error::Recall { source }
                if matches!(source.downcast_ref::<Error>(), Some(Error::InvalidMemoryAgentId { .. }))
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn remember_failures_are_remember_errors() {
    let engine = Arc::new(ReferenceEngine::new());
    let hive = store(&engine, "h");
    let mut blank = entries("builder", &[(EntryKind::Note, "kept")]);
    blank.conversation = String::new();
    assert!(matches!(
        hive.remember(&blank).await,
        Err(runtime::Error::Remember { .. })
    ));
    let unusable = entries("Bad Seat", &[(EntryKind::Note, "kept")]);
    assert!(matches!(
        hive.remember(&unusable).await,
        Err(runtime::Error::Remember { .. })
    ));
    let empty = entries("builder", &[(EntryKind::Note, "   ")]);
    assert!(matches!(
        hive.remember(&empty).await,
        Err(runtime::Error::Remember { .. })
    ));

    let down = HiveMemoryStore::new(
        Arc::new(Down::default()),
        HiveMemory::for_hive("h").unwrap(),
    )
    .unwrap();
    let error = down
        .remember(&entries("builder", &[(EntryKind::Note, "kept")]))
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            runtime::Error::Remember { source } if source.to_string().contains("engine down")
        ),
        "{error:?}"
    );
}

#[test]
fn the_store_takes_the_hive_budget_and_reports_itself() {
    let engine = Arc::new(ReferenceEngine::new());
    let hive = HiveMemory::for_hive("h")
        .unwrap()
        .recall_budget_tokens(NonZeroU32::new(300).unwrap());
    let store = HiveMemoryStore::new(engine, hive.clone()).unwrap();
    assert_eq!(store.policy().budget_tokens, 300);
    assert_eq!(store.hive(), &hive);
    let debug = format!("{store:?}");
    assert!(debug.contains("team:h"), "{debug}");
    let custom = store.with_policy(RecallPolicy {
        team_limit: 0,
        ..RecallPolicy::default()
    });
    assert_eq!(custom.policy().team_limit, 0);
}

#[test]
fn from_config_needs_an_engine_openhuman_would_bind() {
    // Engine binding is process-global, shared with runtime-backed fixtures.
    let _guard = crate::RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    openhuman_core::memory::engine::clear_host_engine();
    let mut config = RuntimeConfig::default();
    config.memory.engine = String::new();
    let error =
        HiveMemoryStore::from_config(&config, HiveMemory::for_hive("h").unwrap()).unwrap_err();
    assert!(matches!(error, Error::Memory(_)), "{error}");
}

/// An engine that refuses every call.
#[derive(Default)]
struct Down(ReferenceEngine);

fn down<T>() -> tinymemory_api::Result<T> {
    Err(tinymemory_api::Error::Unavailable("engine down".into()))
}

#[tinymemory_api::async_trait]
impl MemoryEngine for Down {
    fn descriptor(&self) -> &EngineDescriptor {
        self.0.descriptor()
    }
    async fn health(&self) -> EngineHealth {
        EngineHealth::Down("engine down".into())
    }
    async fn recall(
        &self,
        _: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        down()
    }
    async fn fetch(
        &self,
        _: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        down()
    }
    async fn store(&self, _: StoreItem) -> tinymemory_api::Result<StoreReceipt> {
        down()
    }
    async fn forget(&self, _: ForgetTarget) -> tinymemory_api::Result<ForgetReport> {
        down()
    }
    async fn list(&self, _: ListRequest) -> tinymemory_api::Result<ListPage> {
        down()
    }
}
