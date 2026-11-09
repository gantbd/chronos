use std::collections::{HashSet, VecDeque};

const MAX_ATTEMPTS: u8 = 2;
const MAX_TIME: u8 = 3;
const MAX_EPOCH: u8 = 2;
const MAX_TOKEN: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Phase {
    Ready,
    Running,
    Uncertain,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Model {
    phase: Phase,
    attempts: u8,
    token: Option<u8>,
    next_token: u8,
    expiry: u8,
    now: u8,
    grant_epoch: u8,
    grant: Option<(u8, u8)>,
}

impl Model {
    fn initial() -> Self {
        Self {
            phase: Phase::Ready,
            attempts: 0,
            token: None,
            next_token: 1,
            expiry: 0,
            now: 0,
            grant_epoch: 0,
            grant: None,
        }
    }

    fn successors(self) -> Vec<Self> {
        let mut next = Vec::new();
        for duration in [1, 2] {
            let mut issued = self;
            issued.grant = Some((self.grant_epoch, self.now + duration));
            next.push(issued);

            if self.phase == Phase::Ready
                && self.attempts < MAX_ATTEMPTS
                && self.next_token <= MAX_TOKEN
                && self
                    .grant
                    .is_some_and(|(epoch, expiry)| epoch == self.grant_epoch && self.now < expiry)
            {
                let mut claimed = self;
                claimed.phase = Phase::Running;
                claimed.attempts += 1;
                claimed.token = Some(self.next_token);
                claimed.next_token += 1;
                claimed.expiry = self.now + duration;
                next.push(claimed);
            }

            if self.phase == Phase::Running && self.now < self.expiry {
                let mut heartbeat = self;
                heartbeat.expiry = self.now + duration;
                next.push(heartbeat);
            }
        }

        if self.grant_epoch < MAX_EPOCH {
            let mut revoked = self;
            revoked.grant_epoch += 1;
            next.push(revoked);
        }
        if self.phase == Phase::Running && self.now < self.expiry {
            let mut completed = self;
            completed.phase = Phase::Succeeded;
            completed.token = None;
            next.push(completed);

            let mut failed = self;
            failed.phase = if self.attempts < MAX_ATTEMPTS {
                Phase::Ready
            } else {
                Phase::Failed
            };
            failed.token = None;
            next.push(failed);
        }
        if self.phase == Phase::Running && self.now >= self.expiry {
            let mut expired = self;
            expired.phase = Phase::Uncertain;
            expired.token = None;
            next.push(expired);
        }
        if matches!(self.phase, Phase::Ready | Phase::Running | Phase::Uncertain) {
            let mut cancelled = self;
            cancelled.phase = Phase::Cancelled;
            cancelled.token = None;
            next.push(cancelled);
        }
        if self.phase == Phase::Uncertain {
            let mut completed = self;
            completed.phase = Phase::Succeeded;
            next.push(completed);

            let mut failed = self;
            failed.phase = if self.attempts < MAX_ATTEMPTS {
                Phase::Ready
            } else {
                Phase::Failed
            };
            next.push(failed);
        }
        if self.now < MAX_TIME {
            let mut ticked = self;
            ticked.now += 1;
            next.push(ticked);
        }
        next
    }

    fn assert_invariants(self) {
        assert!(self.attempts <= MAX_ATTEMPTS);
        assert!(self.grant_epoch <= MAX_EPOCH);
        assert!(self.next_token <= MAX_TOKEN + 1);
        assert_eq!(self.token.is_some(), self.phase == Phase::Running);
        if let Some(token) = self.token {
            assert!(token < self.next_token);
        }
        if matches!(
            self.phase,
            Phase::Uncertain | Phase::Succeeded | Phase::Failed | Phase::Cancelled
        ) {
            assert!(self.token.is_none());
        }
    }
}

#[test]
fn every_reachable_bounded_protocol_state_preserves_authority_invariants() {
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

    eprintln!("exhaustive protocol states={}", seen.len());
    assert!(seen.len() > 1_000, "model bounds explored too little state");
}
