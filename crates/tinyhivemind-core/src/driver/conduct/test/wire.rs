//! The wire forms: what a host journals and streams, pinned exactly.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use crate::driver::ConductPolicy;
use crate::runtime::Sequence;
use crate::runtime::speech::Utterance;
use serde_json::{Value, json};

use crate::driver::conduct::child::Child;
use crate::driver::conduct::steps::Kind;
use crate::driver::conduct::{Commit, Event, Note, Refusal, Step, Turn};
use crate::driver::engine::Channel;

/// Every field in `required` must be present: a payload missing one fails to
/// decode rather than decoding to something the conductor never said.
fn rejects_missing<T: serde::de::DeserializeOwned>(value: &Value, required: &[&str]) {
    for field in required {
        let mut payload = value.as_object().expect("an object").clone();
        payload.remove(*field);
        assert!(
            serde_json::from_value::<T>(payload.into()).is_err(),
            "missing {field} must be rejected"
        );
    }
}

fn round_trips<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let wire = serde_json::to_value(value).expect("serializes");
    let back: T = serde_json::from_value(wire).expect("deserializes");
    assert_eq!(&back, value);
}

#[test]
fn a_turn_names_its_seat_channel_and_watermark() {
    let thread = Turn {
        seat: "two".into(),
        channel: Channel::Thread {
            root: Sequence(4),
            others: vec!["one".into()],
            opened_it: false,
        },
        since: Some(Sequence(4)),
    };
    let wire = serde_json::to_value(&thread).expect("serializes");
    assert_eq!(
        wire,
        json!({
            "seat": "two",
            "channel": {"kind": "thread", "root": 4, "others": ["one"], "opened_it": false},
            "since": 4
        })
    );
    rejects_missing::<Turn>(&wire, &["seat", "channel", "since"]);
    round_trips(&thread);

    let desk = Turn {
        seat: "one".into(),
        channel: Channel::Desk,
        since: None,
    };
    let wire = serde_json::to_value(&desk).expect("serializes");
    assert_eq!(wire["channel"], json!({"kind": "desk"}));
    assert_eq!(
        wire["since"],
        json!(null),
        "nothing shown yet is null on the wire, never a sequence"
    );
    round_trips(&desk);
}

#[test]
fn a_note_is_tagged_as_a_step_with_its_fields_beside_the_tag() {
    let note = Step::Note(Note {
        body: "you hold open work".into(),
        thread: None,
        only_for: Some("one".into()),
    });
    assert_eq!(
        serde_json::to_value(&note).expect("serializes"),
        json!({
            "step": "note",
            "body": "you hold open work",
            "thread": null,
            "only_for": "one"
        })
    );
    round_trips(&note);
}

#[test]
fn a_commit_carries_its_conversation_and_an_opaque_purpose() {
    let lifted = Commit {
        author: "two".into(),
        utterance: Utterance::Broadcast {
            message: "three should check the logs".into(),
        },
        thread: None,
        only_for: Vec::new(),
        conversation: Some(Sequence(4)),
        kind: Kind::Desk,
    };
    let wire = serde_json::to_value(Step::Commit(lifted.clone())).expect("serializes");
    assert_eq!(
        wire,
        json!({
            "step": "commit",
            "author": "two",
            "utterance": {"kind": "broadcast", "message": "three should check the logs"},
            "thread": null,
            "only_for": [],
            "conversation": 4,
            "purpose": {"kind": "desk"}
        })
    );
    let bare = serde_json::to_value(&lifted).expect("serializes");
    rejects_missing::<Commit>(&bare, &["author", "utterance", "purpose"]);
    round_trips(&Step::Commit(lifted));

    for kind in [
        Kind::Thread { root: Sequence(4) },
        Kind::Desk,
        Kind::Conclusion {
            root: Sequence(4),
            forced: true,
        },
        Kind::Discharge,
    ] {
        round_trips(&Commit {
            author: "one".into(),
            utterance: Utterance::CompleteEpisode {
                message: "done".into(),
            },
            thread: Some(Sequence(4)),
            only_for: Vec::new(),
            conversation: Some(Sequence(4)),
            kind,
        });
    }
}

#[test]
fn an_event_is_tagged_by_kind_inside_its_step() {
    let asked = Step::Event(Event::Asked {
        seat: "one".into(),
        askees: vec!["two".into(), "three".into()],
        root: Sequence(4),
    });
    assert_eq!(
        serde_json::to_value(&asked).expect("serializes"),
        json!({
            "step": "event",
            "kind": "asked",
            "seat": "one",
            "askees": ["two", "three"],
            "root": 4,
        })
    );
    let refused = Event::Refused {
        seat: "one".into(),
        thread: None,
        why: Refusal::AwaitingReply {
            waiting_on: vec!["two".into()],
        },
        at: Sequence(5),
    };
    let wire = serde_json::to_value(&refused).expect("serializes");
    assert_eq!(
        wire,
        json!({
            "kind": "refused",
            "seat": "one",
            "thread": null,
            "why": {"kind": "awaiting_reply", "waiting_on": ["two"]},
            "at": 5
        })
    );
    rejects_missing::<Event>(&wire, &["kind", "seat", "why", "at"]);
}

#[test]
fn every_event_and_refusal_survives_the_wire() {
    let events = [
        Event::Nudged {
            seat: "one".into(),
            thread: Some(Sequence(4)),
        },
        Event::Parked {
            seat: "one".into(),
            thread: None,
        },
        Event::Resumed {
            seat: "one".into(),
            thread: Some(Sequence(4)),
        },
        Event::Broadcast {
            seat: "one".into(),
            to: vec!["two".into()],
            at: Sequence(3),
        },
        Event::Unplaced {
            seat: "one".into(),
            at: Sequence(3),
        },
        Event::CompletedByBroadcast {
            seat: "one".into(),
            at: Sequence(3),
        },
        Event::Asked {
            seat: "one".into(),
            askees: vec!["two".into()],
            root: Sequence(4),
        },
        Event::Handoff {
            to: "two".into(),
            from: "one".into(),
            origin: Sequence(3),
        },
        Event::Refused {
            seat: "two".into(),
            thread: Some(Sequence(4)),
            why: Refusal::NotYetShown,
            at: Sequence(6),
        },
        Event::Refused {
            seat: "two".into(),
            thread: None,
            why: Refusal::Undelivered {
                assigned_at: Sequence(5),
            },
            at: Sequence(6),
        },
        Event::Discharged {
            seat: "one".into(),
            at: Sequence(7),
        },
        Event::Concluded {
            root: Sequence(4),
            asker: "one".into(),
            askees: vec!["two".into()],
            forced: false,
            at: Sequence(8),
        },
    ];
    for event in events {
        round_trips(&Step::Event(event.clone()));
        round_trips(&event);
    }
    assert_eq!(
        serde_json::to_value(Refusal::NotYetShown).expect("serializes"),
        json!({"kind": "not_yet_shown"})
    );
}

/// **A snapshot written before a conversation could hold a group still
/// resumes.**
///
/// It carried one `askee`, and `nudged`/`turned` as flags over that one
/// seat. A host checkpoints after every committed row, so an episode in
/// flight across this change has one of these on disk; failing to decode it
/// would lose the episode the checkpoint exists to save.
#[test]
fn a_conversation_snapshotted_before_groups_reads_forward() {
    let hive = super::support::hive(&["one", "two"]);
    let driver = crate::driver::CompletionDriver::new(&hive, 4).expect("driver");
    let route_policy = super::support::policy(1);
    let routing = crate::driver::engine::BroadcastRouting {
        primary: None,
        reasoning: None,
        policy: &route_policy,
        roster_version: 1,
        thread_context: &[],
    };
    let journal = super::support::Journal::default();
    let mut conductor = super::support::two_seat(
        &driver,
        routing,
        crate::driver::ConductPolicy::default(),
        &journal,
    );
    super::support::wave(
        &mut conductor,
        &journal,
        &[("one", vec![super::support::ask("two", "which port?")])],
    )
    .expect("wave");
    let snapshot = conductor.snapshot().expect("recordable");
    let wire = serde_json::to_value(&snapshot).expect("serializes");

    // Rewrite the one conversation into the shape the build before this one
    // wrote, and read it back.
    let mut child = wire["children"][0][1].clone();
    let held = child.as_object_mut().expect("a conversation");
    let askee = held["askees"][0].clone();
    held.remove("askees");
    held.remove("answered");
    held.insert("askee".into(), askee);
    held.insert("nudged".into(), json!(true));
    held.insert("turned".into(), json!(false));

    let older: Child = serde_json::from_value(child).expect("an older conversation reads");
    assert_eq!(
        older.askees,
        ["two".to_string()],
        "one askee is a group of one"
    );
    assert_eq!(
        older.nudged,
        ["two".to_string()].into_iter().collect::<BTreeSet<_>>(),
        "the flag stood for that seat, and now names it"
    );
    assert!(older.turned.is_empty(), "and `false` stood for nobody");
    assert!(
        older.answered.is_empty(),
        "a conversation snapshotted then had carried nothing back yet"
    );

    // And the shape written today reads as itself.
    let current = serde_json::to_value(&older).expect("serializes");
    assert_eq!(current["askees"], json!(["two"]));
    assert_eq!(current["nudged"], json!(["two"]));
    let again: Child = serde_json::from_value(current).expect("round trips");
    assert_eq!(again.askees, older.askees);
    assert_eq!(again.nudged, older.nudged);
}

#[test]
fn a_conductor_snapshot_names_every_field_a_restart_reads_back() {
    let hive = super::support::hive(&["one", "two"]);
    let driver = crate::driver::CompletionDriver::new(&hive, 4).expect("driver");
    let route_policy = super::support::policy(1);
    let routing = crate::driver::engine::BroadcastRouting {
        primary: None,
        reasoning: None,
        policy: &route_policy,
        roster_version: 1,
        thread_context: &[],
    };
    let journal = super::support::Journal::default();
    let mut conductor = super::support::two_seat(
        &driver,
        routing,
        crate::driver::ConductPolicy::default(),
        &journal,
    );
    // A conversation open and a seat held: the two pieces of state a
    // restart most needs, so both are on the wire.
    super::support::wave(
        &mut conductor,
        &journal,
        &[("one", vec![super::support::ask("two", "which port?")])],
    )
    .expect("wave");
    let snapshot = conductor.snapshot().expect("recordable");
    let wire = serde_json::to_value(&snapshot).expect("serializes");
    let object = wire.as_object().expect("an object");

    // The spelling a stored snapshot is read back by. Renaming any of these
    // orphans every snapshot written by the build before it, which is the
    // failure the whole checkpoint exists to prevent.
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "chat",
            "children",
            "concluded",
            "desk_name",
            "desk_nudged",
            "discharged",
            "parked",
            "shown",
            "state",
            "turns",
            "wave",
            "waves",
        ]
    );
    assert_eq!(object["chat"], json!("engineering"));

    // A conversation on the wire is its root paired with its record: the
    // key is the root a host files it under, and `resume` refuses a pair
    // whose two halves disagree.
    let pair = &wire["children"][0];
    let child = &pair[1];
    assert_eq!(
        pair[0], child["root"],
        "the key a conversation is filed under is its own root"
    );
    for field in [
        "root", "asker", "askees", "state", "turns", "nudged", "turned", "answered",
    ] {
        assert!(
            child.get(field).is_some(),
            "a conversation must carry `{field}`: {child}"
        );
    }
    assert_eq!(child["asker"], json!("one"));
    assert_eq!(child["askees"], json!(["two"]));

    // And it decodes back to the same thing.
    let back: crate::driver::ConductorState = serde_json::from_value(wire).expect("deserializes");
    assert_eq!(back.chat, snapshot.chat);
    assert_eq!(back.mid_wave_is_empty(), snapshot.mid_wave_is_empty());
}

#[test]
fn conduct_policy_wire_preserves_conversation_and_episode_walls() {
    let policy = ConductPolicy {
        child_turn_wall: 5,
        turn_wall: 40,
    };
    let value = serde_json::to_value(policy).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"child_turn_wall":5,"turn_wall":40})
    );
    assert_eq!(
        serde_json::from_value::<ConductPolicy>(value).unwrap(),
        policy
    );
}

#[test]
fn conduct_policy_wire_form_and_omitted_defaults_are_stable() {
    let value: crate::driver::ConductPolicy = serde_json::from_str("{}").unwrap();
    assert_eq!(value, crate::driver::ConductPolicy::default());
    assert_eq!(
        serde_json::to_value(value).unwrap(),
        serde_json::json!({"child_turn_wall":6,"turn_wall":60})
    );
}
