use chronos_engine::{Best, Engine, Monoid, ScheduleKey, SchedulingProfile, TaskId};

fn domain() -> Vec<Best<ScheduleKey>> {
    vec![
        Best::identity(),
        Best::candidate(ScheduleKey::new(0, TaskId::from("a"))),
        Best::candidate(ScheduleKey::new(0, TaskId::from("b"))),
        Best::candidate(ScheduleKey::new(1, TaskId::from("a"))),
        Best::candidate(ScheduleKey::new(-1, TaskId::from("z"))),
    ]
}

#[test]
fn best_choice_is_a_commutative_idempotent_monoid() {
    let values = domain();
    for a in &values {
        assert_eq!(a.combine(&Best::identity()), *a);
        assert_eq!(Best::identity().combine(a), *a);
        assert_eq!(a.combine(a), *a, "idempotence");
        for b in &values {
            assert_eq!(a.combine(b), b.combine(a), "commutativity");
            for c in &values {
                assert_eq!(
                    a.combine(b).combine(c),
                    a.combine(&b.combine(c)),
                    "associativity"
                );
            }
        }
    }
}

#[test]
fn strict_priority_then_ascending_task_id_is_the_closed_order() {
    let low = Best::candidate(ScheduleKey::new(0, TaskId::from("a")));
    let high = Best::candidate(ScheduleKey::new(1, TaskId::from("z")));
    assert_eq!(low.combine(&high), high);

    let a = Best::candidate(ScheduleKey::new(1, TaskId::from("a")));
    let b = Best::candidate(ScheduleKey::new(1, TaskId::from("b")));
    assert_eq!(a.combine(&b), a);
}

#[test]
fn folding_any_permutation_selects_the_same_candidate() {
    let candidates = [
        ScheduleKey::new(1, TaskId::from("b")),
        ScheduleKey::new(2, TaskId::from("z")),
        ScheduleKey::new(2, TaskId::from("a")),
    ];
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];

    for permutation in permutations {
        let selected = permutation.iter().fold(Best::identity(), |best, index| {
            best.combine(&Best::candidate(candidates[*index].clone()))
        });
        assert_eq!(
            selected,
            Best::candidate(ScheduleKey::new(2, TaskId::from("a")))
        );
    }
}

#[test]
fn the_default_engine_names_its_exact_behavioral_profile() {
    assert_eq!(
        Engine::new().scheduling_profile(),
        SchedulingProfile::StrictPriorityV1
    );
}
