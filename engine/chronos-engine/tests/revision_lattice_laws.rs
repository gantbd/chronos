use chronos_engine::{BoundedLattice, StateRevision};

const VALUES: [StateRevision; 5] = [
    StateRevision::Finite(0),
    StateRevision::Finite(1),
    StateRevision::Finite(7),
    StateRevision::Finite(u64::MAX - 1),
    StateRevision::Exhausted,
];

#[test]
fn state_revisions_form_the_bounded_total_order_lattice() {
    for a in VALUES {
        assert_eq!(a.join(a), a);
        assert_eq!(a.meet(a), a);
        assert_eq!(a.join(StateRevision::bottom()), a);
        assert_eq!(a.meet(StateRevision::top()), a);
        for b in VALUES {
            assert_eq!(a.join(b), b.join(a));
            assert_eq!(a.meet(b), b.meet(a));
            assert_eq!(a.join(a.meet(b)), a);
            assert_eq!(a.meet(a.join(b)), a);
            for c in VALUES {
                assert_eq!(a.join(b).join(c), a.join(b.join(c)));
                assert_eq!(a.meet(b).meet(c), a.meet(b.meet(c)));
            }
        }
    }
}

#[test]
fn exhausted_is_explicit_top_and_never_aliases_a_finite_revision() {
    for value in VALUES {
        assert!(StateRevision::bottom().leq(value));
        assert!(value.leq(StateRevision::top()));
    }
    assert_eq!(StateRevision::finite(u64::MAX), StateRevision::Exhausted);
}
