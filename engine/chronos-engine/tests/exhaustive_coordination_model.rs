use std::collections::{HashSet, VecDeque};

const MAX_INCARNATION: u8 = 3;
const MAX_REVISION: u8 = 6;
const MAX_TIME: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Phase {
    Ready,
    Running,
    Uncertain,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Model {
    active: Option<(u8, Phase)>,
    next_incarnation: u8,
    retired: Option<u8>,
    revision: u8,
    now: u8,
    grant_budget: u8,
    grant_revoked: bool,
}

impl Model {
    fn initial() -> Self {
        Self {
            active: None,
            next_incarnation: 0,
            retired: None,
            revision: 0,
            now: 0,
            grant_budget: 0,
            grant_revoked: false,
        }
    }

    fn successors(self) -> Vec<Self> {
        let mut next = Vec::new();
        if self.active.is_none()
            && self.next_incarnation < MAX_INCARNATION
            && self.revision < MAX_REVISION
        {
            let mut submitted = self;
            submitted.active = Some((self.next_incarnation, Phase::Ready));
            submitted.next_incarnation += 1;
            submitted.revision += 1;
            next.push(submitted);
        }
        if let Some((incarnation, phase)) = self.active {
            if phase == Phase::Ready
                && self.grant_budget > 0
                && !self.grant_revoked
                && self.revision < MAX_REVISION
            {
                let mut claimed = self;
                claimed.active = Some((incarnation, Phase::Running));
                claimed.grant_budget -= 1;
                claimed.revision += 1;
                next.push(claimed);
            }
            if phase == Phase::Running && self.revision < MAX_REVISION {
                let mut uncertain = self;
                uncertain.active = Some((incarnation, Phase::Uncertain));
                uncertain.revision += 1;
                next.push(uncertain);

                let mut completed = self;
                completed.active = Some((incarnation, Phase::Terminal));
                completed.revision += 1;
                next.push(completed);
            }
            if phase == Phase::Uncertain && self.revision < MAX_REVISION {
                let mut reconciled = self;
                reconciled.active = Some((incarnation, Phase::Terminal));
                reconciled.revision += 1;
                next.push(reconciled);
            }
            if phase == Phase::Terminal && self.revision < MAX_REVISION {
                let mut retired = self;
                retired.active = None;
                retired.retired = Some(incarnation);
                retired.revision += 1;
                next.push(retired);
            }
        }
        for budget in [1, 2] {
            if self.grant_budget != budget || self.grant_revoked {
                let mut issued = self;
                issued.grant_budget = budget;
                issued.grant_revoked = false;
                next.push(issued);
            }
        }
        if !self.grant_revoked && self.grant_budget > 0 {
            let mut revoked = self;
            revoked.grant_revoked = true;
            next.push(revoked);
        }
        if self.now < MAX_TIME {
            let mut ticked = self;
            ticked.now += 1;
            next.push(ticked);
        }
        next
    }

    fn assert_invariants(self) {
        assert!(self.next_incarnation <= MAX_INCARNATION);
        assert!(self.revision <= MAX_REVISION);
        assert!(self.now <= MAX_TIME);
        assert!(self.grant_budget <= 2);
        if let Some((incarnation, _)) = self.active {
            assert!(incarnation < self.next_incarnation);
            assert_ne!(Some(incarnation), self.retired);
        }
        if let Some(retired) = self.retired {
            assert!(retired < self.next_incarnation);
        }
    }
}

#[test]
fn every_bounded_coordination_state_preserves_lifecycle_time_and_budget_invariants() {
    let initial = Model::initial();
    let mut seen = HashSet::from([initial]);
    let mut frontier = VecDeque::from([initial]);

    while let Some(state) = frontier.pop_front() {
        state.assert_invariants();
        for successor in state.successors() {
            successor.assert_invariants();
            if seen.insert(successor) {
                frontier.push_back(successor);
            }
        }
    }

    eprintln!("exhaustive v0.2 coordination states={}", seen.len());
    assert!(seen.len() > 100);
}
