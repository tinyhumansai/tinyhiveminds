//! Unit tests for the division of labour.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

use crate::runtime::aside::Audience;
use crate::runtime::{
    Sequence,
    desk::{Desk, DeskSet, ResponderMode},
    roster::{Roster, RosterMember},
};

use crate::hive::directory::{DirectoryPolicy, directory};
use crate::hive::trace::read;

const MEMBERS: [&str; 5] = ["planner", "critic", "scout", "auditor", "archivist"];

fn roster_members() -> Vec<RosterMember> {
    MEMBERS
        .iter()
        .map(|id| RosterMember {
            id: (*id).into(),
            name: Some((*id).into()),
        })
        .collect()
}

fn desks() -> Vec<Desk> {
    vec![Desk {
        id: "engineering".into(),
        name: "Engineering".into(),
        description: None,
        members: MEMBERS.iter().map(|id| (*id).to_owned()).collect(),
        responder_mode: ResponderMode::Auto,
    }]
}

/// `n` facets named `f0..fn`.
fn facets(count: usize) -> Vec<TopicId> {
    (0..count)
        .map(|index| TopicId::from(format!("f{index}").as_str()))
        .collect()
}

/// Divide `count` facets across the whole desk at `policy`.
fn divided(count: usize, policy: DivisionPolicy) -> Division {
    let members = roster_members();
    let roster = Roster::new(&members, &[], &[]);
    let held = desks();
    let desks = DeskSet::new(&held, &[], &[], &[], &[]);
    divide(&facets(count), "engineering", &roster, &desks, None, policy).unwrap()
}

/// One desk-visible row carrying one marker.
fn said(sequence: u64, author: &str, content: &str) -> crate::runtime::SessionMessage {
    crate::runtime::SessionMessage {
        sequence: Sequence(sequence),
        author: crate::runtime::SessionAuthor::Agent {
            id: author.into(),
            label: author.into(),
        },
        content: content.into(),
        audience: Audience::Desk,
        elided: None,
    }
}

/// The measured rule, and the reason it is not a special case in the code: a
/// task of one facet is one seat answering alone, in one round.
#[test]
fn one_facet_is_one_seat_answering_alone() {
    let division = divided(1, DivisionPolicy::DEFAULT);
    assert!(division.is_alone());
    assert_eq!(division.depth(), 1);
    assert_eq!(division.width(), 1);
    assert_eq!(division.owner_of(&TopicId::from("f0")), Some("planner"));
}

/// A task with no facets divides into nothing, and that is not an error.
#[test]
fn a_task_with_no_facets_divides_into_nothing() {
    let division = divided(0, DivisionPolicy::DEFAULT);
    assert!(division.is_empty(), "{division:?}");
    assert!(!division.is_alone());
    assert_eq!(division.depth(), 0);
    assert_eq!(division.width(), 0);
}

/// The depth a division buys, which is the whole cost argument: eight facets
/// across five seats at width four are two rounds, not eight.
#[test]
fn independent_facets_ride_the_same_round() {
    let division = divided(8, DivisionPolicy::DEFAULT);
    assert_eq!(division.assignments().len(), 8);
    assert_eq!(division.depth(), 2);
    assert_eq!(division.width(), 4);
    assert_eq!(division.round(0).len(), 4);
    assert_eq!(division.round(1).len(), 4);
}

/// One agent cannot answer two questions at once, so a seat never appears
/// twice in a round however wide the policy allows.
#[test]
fn a_seat_never_appears_twice_in_one_round() {
    let wide = DivisionPolicy {
        round_width: 64,
        ..DivisionPolicy::DEFAULT
    };
    let division = divided(12, wide);
    for round in 0..division.depth() {
        let mut owners: Vec<&str> = division
            .round(round)
            .iter()
            .map(|held| held.owner.as_str())
            .collect();
        let before = owners.len();
        owners.sort_unstable();
        owners.dedup();
        assert_eq!(owners.len(), before, "round {round} names a seat twice");
    }
    // Five seats, twelve facets: three rounds, and no round wider than the desk.
    assert_eq!(division.depth(), 3);
    assert_eq!(division.width(), 5);
}

/// The rotation is by the facet's own index, so a division is a fold rather
/// than a walk: which seat takes a facet does not depend on what came before.
#[test]
fn the_rotation_follows_the_facet_index() {
    let division = divided(7, DivisionPolicy::DEFAULT);
    for (index, held) in division.assignments().iter().enumerate() {
        assert_eq!(held.owner, MEMBERS[index % MEMBERS.len()]);
        assert_eq!(held.reason, OwnerReason::Rotation);
    }
}

/// A facet named twice is one facet, and keeps the position it was first
/// given.
#[test]
fn a_facet_named_twice_is_one_facet() {
    let members = roster_members();
    let roster = Roster::new(&members, &[], &[]);
    let held = desks();
    let desks = DeskSet::new(&held, &[], &[], &[], &[]);
    let repeated = [
        TopicId::from("stage"),
        TopicId::from("ship"),
        TopicId::from("stage"),
    ];
    let division = divide(
        &repeated,
        "engineering",
        &roster,
        &desks,
        None,
        DivisionPolicy::DEFAULT,
    )
    .unwrap();
    assert_eq!(division.assignments().len(), 2);
    assert_eq!(division.owner_of(&TopicId::from("stage")), Some("planner"));
    assert_eq!(division.owner_of(&TopicId::from("ship")), Some("critic"));
}

/// Competence picks the owner where the directory has an opinion, and says so.
#[test]
fn the_directory_names_an_owner_where_it_knows_one() {
    let transcript = [
        said(
            1,
            "auditor",
            "!evidence #stage The second environment was retired.",
        ),
        said(2, "critic", "!support #stage ^1 That settles it."),
        said(3, "scout", "!support #stage ^1 Agreed."),
    ];
    let known = directory(
        &read(&transcript),
        Sequence(3),
        &DirectoryPolicy {
            window: 100,
            ..DirectoryPolicy::DEFAULT
        },
        &[],
    )
    .unwrap();

    let members = roster_members();
    let roster = Roster::new(&members, &[], &[]);
    let held = desks();
    let desks = DeskSet::new(&held, &[], &[], &[], &[]);
    let asked = [TopicId::from("stage"), TopicId::from("ship")];
    let division = divide(
        &asked,
        "engineering",
        &roster,
        &desks,
        Some(&known),
        DivisionPolicy::DEFAULT,
    )
    .unwrap();

    // `stage` goes to the member the directory names, not to the rotation's
    // `planner`; `ship`, which nobody is known on, falls to the rotation.
    assert_eq!(division.owner_of(&TopicId::from("stage")), Some("auditor"));
    assert_eq!(division.assignments()[0].reason, OwnerReason::Knows);
    assert_eq!(division.assignments()[1].reason, OwnerReason::Rotation);

    // The control the benchmark runs: a division deliberately blind to
    // competence takes the rotation for everything.
    let blind = divide(
        &asked,
        "engineering",
        &roster,
        &desks,
        Some(&known),
        DivisionPolicy {
            follow_directory: false,
            ..DivisionPolicy::DEFAULT
        },
    )
    .unwrap();
    assert_eq!(blind.owner_of(&TopicId::from("stage")), Some("planner"));
    assert_eq!(blind.assignments()[0].reason, OwnerReason::Rotation);
}

/// The half of the mechanism that wins: an owner reads its own facet and none
/// of the others, and shared context reaches everybody.
#[test]
fn an_owner_reads_its_own_facet_and_not_its_peers() {
    let division = divided(2, DivisionPolicy::DEFAULT);
    let transcript = [
        said(1, "planner", "What should we do about the rollout?"),
        said(2, "planner", "!propose #f0 Stage it."),
        said(3, "critic", "!propose #f1 Rename the flag."),
        said(
            4,
            "scout",
            "!support #f0 ^2 Staging bounds the blast radius.",
        ),
        said(
            5,
            "auditor",
            "!propose #elsewhere Something nobody divided.",
        ),
    ];

    let mine = division.scoped(&TopicId::from("f0"), &transcript);
    let sequences: Vec<u64> = mine.iter().map(|row| row.sequence.0).collect();
    // The question, this facet's rows, and the undivided topic — but not the
    // other facet's proposal.
    assert_eq!(sequences, [1, 2, 4, 5]);

    let theirs = division.scoped(&TopicId::from("f1"), &transcript);
    let sequences: Vec<u64> = theirs.iter().map(|row| row.sequence.0).collect();
    assert_eq!(sequences, [1, 3, 5]);
}

/// A seat's own rows are its own, whatever facet they name.
#[test]
fn authorship_is_readable_off_a_row() {
    let row = said(1, "planner", "!propose #f0 Stage it.");
    assert!(Division::authored_by(&row, "planner"));
    assert!(!Division::authored_by(&row, "critic"));
}

/// A width of zero would authorize nobody, which is a configuration error
/// rather than a quieter way of saying one.
#[test]
fn a_zero_width_is_refused() {
    let members = roster_members();
    let roster = Roster::new(&members, &[], &[]);
    let held = desks();
    let desks = DeskSet::new(&held, &[], &[], &[], &[]);
    let outcome = divide(
        &facets(2),
        "engineering",
        &roster,
        &desks,
        None,
        DivisionPolicy {
            round_width: 0,
            ..DivisionPolicy::DEFAULT
        },
    );
    assert!(matches!(outcome, Err(Error::ZeroRoundWidth)));
}

/// A desk with facets and nobody to own them cannot answer the task, and says
/// so rather than returning an empty division that looks like success.
#[test]
fn a_desk_with_no_active_member_is_refused() {
    let roster = Roster::new(&[], &[], &[]);
    let held = desks();
    let desks = DeskSet::new(&held, &[], &[], &[], &[]);
    let outcome = divide(
        &facets(2),
        "engineering",
        &roster,
        &desks,
        None,
        DivisionPolicy::DEFAULT,
    );
    match outcome {
        Err(Error::NoSeats { desk_id }) => assert_eq!(desk_id, "engineering"),
        other => panic!("expected NoSeats, got {other:?}"),
    }
}

/// An unknown desk is the core snapshot's error to raise, not this module's.
#[test]
fn an_unknown_desk_is_refused() {
    let members = roster_members();
    let roster = Roster::new(&members, &[], &[]);
    let held = desks();
    let desks = DeskSet::new(&held, &[], &[], &[], &[]);
    let outcome = divide(
        &facets(1),
        "nowhere",
        &roster,
        &desks,
        None,
        DivisionPolicy::DEFAULT,
    );
    assert!(matches!(outcome, Err(Error::Core { .. })));
}

/// The same arguments produce the same division, every time.
#[test]
fn a_division_is_a_fold() {
    let first = divided(9, DivisionPolicy::DEFAULT);
    let second = divided(9, DivisionPolicy::DEFAULT);
    assert_eq!(first, second);
}

/// The facets one seat owns, which is the load it actually carries.
#[test]
fn a_seat_owns_the_facets_the_rotation_gave_it() {
    let division = divided(8, DivisionPolicy::DEFAULT);
    assert_eq!(
        division.facets_of("planner"),
        [&TopicId::from("f0"), &TopicId::from("f5")]
    );
    assert_eq!(division.facets_of("nobody"), Vec::<&TopicId>::new());
}

/// The default is on, which no other opt-in mechanism here is.
#[test]
fn the_default_policy_divides() {
    let policy = DivisionPolicy::default();
    assert_eq!(policy, DivisionPolicy::DEFAULT);
    assert!(policy.follow_directory);
    assert_eq!(
        policy.round_width,
        crate::hive::episode::DEFAULT_ROUND_WIDTH
    );
}

#[test]
fn division_policy_wire_preserves_width_and_directory_choice() {
    let policy = DivisionPolicy {
        round_width: 7,
        follow_directory: false,
    };
    let value = serde_json::to_value(policy).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"round_width":7,"follow_directory":false})
    );
    assert_eq!(
        serde_json::from_value::<DivisionPolicy>(value).unwrap(),
        policy
    );
}

#[test]
fn division_policy_wire_form_and_omitted_defaults_are_stable() {
    let value: DivisionPolicy = serde_json::from_str("{}").unwrap();
    assert_eq!(value, DivisionPolicy::default());
    assert_eq!(
        serde_json::to_value(value).unwrap(),
        serde_json::json!({"round_width":4,"follow_directory":true})
    );
}
