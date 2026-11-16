//! Domain adapter over Keystone's qualified post-offer receipt gate. Candidate,
//! serialization, recovery, retention and the absorbing service fence stay local.
use crate::atomic::AtomicError;
use chronos_host::{StoreError, service_envelope::EnvelopeReceipt};
use keystone_storage::{
    AcknowledgementStrength as Strength, CommitGate, CommitGateFailure, CommitOutcome,
    CommitReceipt, Revision,
};

pub(crate) fn validate(
    previous: u64,
    proposed: u64,
    outcome: Result<EnvelopeReceipt, StoreError>,
) -> Result<(), AtomicError> {
    let mut gate = CommitGate::new(Some(Revision(previous)));
    let offered = outcome.map(|receipt| {
        CommitOutcome::Committed(CommitReceipt {
            revision: Revision(receipt.revision),
            mechanism: "chronos-envelope-cas",
            // The shared law only distinguishes Durable from weaker evidence.
            // Chronos keeps its full public strength/error vocabulary unchanged.
            strength: match receipt.strength {
                chronos_engine::AcknowledgementStrength::Durable => Strength::Durable,
                chronos_engine::AcknowledgementStrength::Process
                | chronos_engine::AcknowledgementStrength::Volatile => Strength::Volatile,
            },
        })
    });
    gate.resolve(Revision(proposed), offered)
        .map_err(|error| match error {
            CommitGateFailure::Store(error) => AtomicError::Store(error),
            CommitGateFailure::InvalidReceipt
            | CommitGateFailure::Refused(_)
            | CommitGateFailure::Indeterminate => AtomicError::Receipt,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_gate_matches_chronos_exact_successor_and_all_three_strengths() {
        use chronos_engine::AcknowledgementStrength::{Durable, Process, Volatile};
        for previous in [0, 41, u64::MAX - 1] {
            let proposed = previous + 1;
            for actual in [previous, proposed] {
                for strength in [Volatile, Process, Durable] {
                    assert_eq!(
                        validate(
                            previous,
                            proposed,
                            Ok(EnvelopeReceipt {
                                revision: actual,
                                strength
                            })
                        )
                        .is_ok(),
                        actual == proposed && strength == Durable
                    );
                }
            }
        }
        assert!(matches!(
            validate(0, 1, Err(StoreError::Capacity)),
            Err(AtomicError::Store(StoreError::Capacity))
        ));
        assert!(
            validate(
                0,
                2,
                Ok(EnvelopeReceipt {
                    revision: 2,
                    strength: Durable
                })
            )
            .is_err()
        );
    }
}
