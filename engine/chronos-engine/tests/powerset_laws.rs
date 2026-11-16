use chronos_engine::{BoundedLattice, FinitePowerset};

fn domain() -> Vec<FinitePowerset<3>> {
    (0_u8..8)
        .map(|bits| FinitePowerset::from_membership([bits & 1 != 0, bits & 2 != 0, bits & 4 != 0]))
        .collect()
}

#[test]
fn finite_powerset_is_a_bounded_distributive_lattice() {
    let values = domain();
    for a in values.iter().copied() {
        assert_eq!(a.join(a), a);
        assert_eq!(a.meet(a), a);
        assert_eq!(a.join(FinitePowerset::bottom()), a);
        assert_eq!(a.meet(FinitePowerset::top()), a);
        assert_eq!(a.join(FinitePowerset::top()), FinitePowerset::top());
        assert_eq!(a.meet(FinitePowerset::bottom()), FinitePowerset::bottom());
        for b in values.iter().copied() {
            assert_eq!(a.join(b), b.join(a));
            assert_eq!(a.meet(b), b.meet(a));
            assert_eq!(a.join(a.meet(b)), a);
            assert_eq!(a.meet(a.join(b)), a);
            for c in values.iter().copied() {
                assert_eq!(a.join(b).join(c), a.join(b.join(c)));
                assert_eq!(a.meet(b).meet(c), a.meet(b.meet(c)));
                assert_eq!(a.join(b.meet(c)), a.join(b).meet(a.join(c)));
                assert_eq!(a.meet(b.join(c)), a.meet(b).join(a.meet(c)));
            }
        }
    }
}

#[test]
fn complement_and_membership_obey_boolean_algebra() {
    for value in domain() {
        assert_eq!(value.join(value.complement()), FinitePowerset::top());
        assert_eq!(value.meet(value.complement()), FinitePowerset::bottom());
        assert_eq!(value.complement().complement(), value);
    }

    let value = FinitePowerset::<3>::singleton(1).unwrap();
    assert!(!value.contains(0));
    assert!(value.contains(1));
    assert!(!value.contains(2));
    assert_eq!(FinitePowerset::<3>::singleton(3), None);
}
