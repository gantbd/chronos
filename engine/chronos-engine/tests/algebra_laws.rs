use chronos_engine::{BoundedLattice, EffectKnowledge};

const VALUES: [EffectKnowledge; 4] = [
    EffectKnowledge::Unknown,
    EffectKnowledge::Failed,
    EffectKnowledge::Completed,
    EffectKnowledge::Conflict,
];

#[test]
fn effect_knowledge_is_a_bounded_lattice() {
    for a in VALUES {
        assert_eq!(a.join(a), a, "join idempotence");
        assert_eq!(a.meet(a), a, "meet idempotence");
        assert_eq!(a.join(EffectKnowledge::bottom()), a, "join identity");
        assert_eq!(a.meet(EffectKnowledge::top()), a, "meet identity");
        assert_eq!(a.join(EffectKnowledge::top()), EffectKnowledge::top());
        assert_eq!(a.meet(EffectKnowledge::bottom()), EffectKnowledge::bottom());

        for b in VALUES {
            assert_eq!(a.join(b), b.join(a), "join commutativity");
            assert_eq!(a.meet(b), b.meet(a), "meet commutativity");
            assert_eq!(a.join(a.meet(b)), a, "join absorption");
            assert_eq!(a.meet(a.join(b)), a, "meet absorption");

            for c in VALUES {
                assert_eq!(a.join(b).join(c), a.join(b.join(c)), "join associativity");
                assert_eq!(a.meet(b).meet(c), a.meet(b.meet(c)), "meet associativity");
            }
        }
    }
}

#[test]
fn contradictory_effect_evidence_never_collapses_to_success_or_failure() {
    assert_eq!(
        EffectKnowledge::Completed.join(EffectKnowledge::Failed),
        EffectKnowledge::Conflict
    );
    assert_eq!(
        EffectKnowledge::Unknown.join(EffectKnowledge::Completed),
        EffectKnowledge::Completed
    );
    assert_eq!(
        EffectKnowledge::Unknown.join(EffectKnowledge::Failed),
        EffectKnowledge::Failed
    );
}

#[test]
fn the_induced_order_has_explicit_bottom_and_top() {
    for value in VALUES {
        assert!(EffectKnowledge::bottom().leq(value));
        assert!(value.leq(EffectKnowledge::top()));
    }
    assert!(!EffectKnowledge::Completed.leq(EffectKnowledge::Failed));
    assert!(!EffectKnowledge::Failed.leq(EffectKnowledge::Completed));
}
