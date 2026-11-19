//! Read-only owner composition for one durable dual-frontier reconciliation.

use super::*;
use crate::alignment::host::AlignmentFrontierV1;

pub struct DualFrontierHostV1<J, F, D> {
    register: DualFrontierRegisterV1<J>,
    frontier: F,
    disclosure: D,
    limits: AlignmentLimitsV1,
}

impl<J, F, D> DualFrontierHostV1<J, F, D>
where
    J: DualFrontierJournalV1,
    F: AlignmentFrontierV1,
    D: DualFrontierDisclosureV1,
{
    pub fn new(
        register: DualFrontierRegisterV1<J>,
        frontier: F,
        disclosure: D,
        limits: AlignmentLimitsV1,
    ) -> Self {
        Self {
            register,
            frontier,
            disclosure,
            limits,
        }
    }

    pub fn dispatch(&mut self, request: public::Request) -> public::Response {
        let protocol_limits = public::Limits {
            max_field_bytes: self.limits.max_field_bytes,
            max_evidence: self.limits.max_evidence,
        };
        if request.validate(protocol_limits).is_err() {
            return refused(public::RefusalCode::InvalidRequest);
        }
        let retained_scope = self
            .register
            .record()
            .reconciliation()
            .basis()
            .chronos_scope();
        if request.chronos_scope() != retained_scope {
            return refused(public::RefusalCode::Unauthorized);
        }
        let current = match self.frontier.current(request.chronos_scope()) {
            Ok(current) => current,
            Err(_) => return refused(public::RefusalCode::Unavailable),
        };
        if current.chronos_scope() != request.chronos_scope() {
            return refused(public::RefusalCode::Unavailable);
        }
        match self
            .register
            .observe(&current, self.limits, &mut self.disclosure)
        {
            Ok(observation) => public::Response {
                version: 1,
                outcome: public::Outcome::Observed {
                    observation: Box::new(observation),
                },
            },
            Err(DualFrontierObservationErrorV1::Unauthorized) => {
                refused(public::RefusalCode::Unauthorized)
            }
            Err(DualFrontierObservationErrorV1::Invalid) => {
                refused(public::RefusalCode::Unavailable)
            }
        }
    }

    pub fn into_parts(self) -> (DualFrontierRegisterV1<J>, F, D) {
        (self.register, self.frontier, self.disclosure)
    }
}

fn refused(code: public::RefusalCode) -> public::Response {
    public::Response {
        version: 1,
        outcome: public::Outcome::Refused { code },
    }
}
