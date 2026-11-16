use chronos_engine::{
    BoundedLattice, Monoid, ResourceAccount, ResourceResidualError, ResourceVector,
};

fn domain() -> Vec<ResourceAccount> {
    let mut values = Vec::new();
    for bits in 0_u8..64 {
        values.push(ResourceAccount::Finite(ResourceVector {
            tasks: (bits & 1) as usize,
            dependency_edges: ((bits >> 1) & 1) as usize,
            capability_entries: ((bits >> 2) & 1) as usize,
            payload_bytes: ((bits >> 3) & 1) as usize,
            retained_outcomes: ((bits >> 4) & 1) as usize,
            active_grants: ((bits >> 5) & 1) as usize,
        }));
    }
    values.push(ResourceAccount::Overflow);
    values
}

#[test]
fn resource_accounts_form_an_ordered_commutative_monoid() {
    let values = domain();
    for a in &values {
        assert_eq!(a.combine(&ResourceAccount::identity()), *a);
        assert_eq!(ResourceAccount::identity().combine(a), *a);
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
fn resource_accounts_form_a_bounded_lattice() {
    let values = domain();
    for a in &values {
        assert_eq!(a.join(*a), *a);
        assert_eq!(a.meet(*a), *a);
        assert_eq!(a.join(ResourceAccount::bottom()), *a);
        assert_eq!(a.meet(ResourceAccount::top()), *a);
        assert_eq!(a.join(ResourceAccount::top()), ResourceAccount::top());
        assert_eq!(a.meet(ResourceAccount::bottom()), ResourceAccount::bottom());
        for b in &values {
            assert_eq!(a.join(*b), b.join(*a));
            assert_eq!(a.meet(*b), b.meet(*a));
            assert_eq!(a.join(a.meet(*b)), *a);
            assert_eq!(a.meet(a.join(*b)), *a);
            for c in &values {
                assert_eq!(a.join(*b).join(*c), a.join(b.join(*c)));
                assert_eq!(a.meet(*b).meet(*c), a.meet(b.meet(*c)));
                if a.leq(*b) {
                    assert!(a.combine(c).leq(b.combine(c)), "addition monotonicity");
                }
            }
        }
    }
}

#[test]
fn checked_residual_never_creates_negative_or_indeterminate_resources() {
    let available = ResourceAccount::Finite(ResourceVector {
        tasks: 3,
        dependency_edges: 5,
        capability_entries: 7,
        payload_bytes: 11,
        retained_outcomes: 13,
        active_grants: 17,
    });
    let used = ResourceAccount::Finite(ResourceVector {
        tasks: 1,
        dependency_edges: 2,
        capability_entries: 3,
        payload_bytes: 4,
        retained_outcomes: 5,
        active_grants: 6,
    });
    assert_eq!(
        available.residual(used),
        Ok(ResourceAccount::Finite(ResourceVector {
            tasks: 2,
            dependency_edges: 3,
            capability_entries: 4,
            payload_bytes: 7,
            retained_outcomes: 8,
            active_grants: 11,
        }))
    );
    assert_eq!(
        used.residual(available),
        Err(ResourceResidualError::Underflow)
    );
    assert_eq!(
        ResourceAccount::Overflow.residual(used),
        Err(ResourceResidualError::Indeterminate)
    );
}
