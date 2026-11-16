//! Current authorization and bounded native observations over one engine owner.
use crate::{Authorizer, Service, increment};
use chronos_host::scheduling::*;
use std::time::{Duration, Instant};
pub trait SchedulingAuthorizerV1: Authorizer {
    /// In particular ConfirmQuiescent requires host-established evidence that
    /// the exact execution has stopped, not merely permission to cancel a task.
    fn allows_scheduling(&mut self, request: &SchedulingRequestV1) -> bool;
    fn allows_scheduling_result(
        &mut self,
        request: &SchedulingRequestV1,
        response: &SchedulingResponseV1,
    ) -> bool;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SchedulingStageSignalsV1 {
    pub samples: u64,
    pub total: Duration,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SchedulingSignalsV1 {
    pub calls: u64,
    pub controls: u64,
    pub queries: u64,
    pub unauthorized: u64,
    pub limited: u64,
    pub domain_refused: u64,
    pub withheld: u64,
    pub residence: Duration,
    /// Admission, engine, publication and release durations.
    pub stages: [SchedulingStageSignalsV1; 4],
    /// <=50us, <=250us, <=1ms, <=5ms, <=20ms, greater. Measurement buckets, not SLOs.
    pub residence_buckets: [u64; 6],
    pub saturated: bool,
}
impl<A: SchedulingAuthorizerV1> Service<A> {
    pub fn scheduling_signals(&self) -> SchedulingSignalsV1 {
        self.scheduling_signals
    }
    pub(crate) fn scheduling_stage(&mut self, index: usize, duration: Duration) {
        let signals = &mut self.scheduling_signals;
        increment(&mut signals.stages[index].samples, &mut signals.saturated);
        match signals.stages[index].total.checked_add(duration) {
            Some(next) => signals.stages[index].total = next,
            None => {
                signals.stages[index].total = Duration::MAX;
                signals.saturated = true;
            }
        }
    }
    pub(crate) fn finish_scheduling(&mut self, duration: Duration) {
        let signals = &mut self.scheduling_signals;
        match signals.residence.checked_add(duration) {
            Some(next) => signals.residence = next,
            None => {
                signals.residence = Duration::MAX;
                signals.saturated = true;
            }
        }
        let bucket = [50, 250, 1000, 5000, 20000]
            .into_iter()
            .position(|us| duration.as_micros() <= us)
            .unwrap_or(5);
        increment(
            &mut signals.residence_buckets[bucket],
            &mut signals.saturated,
        );
    }
    pub(crate) fn admit_scheduling(
        &mut self,
        request: &SchedulingRequestV1,
    ) -> Result<(), SchedulingRefusalV1> {
        let started = Instant::now();
        increment(
            &mut self.scheduling_signals.calls,
            &mut self.scheduling_signals.saturated,
        );
        let result = if !self.authorizer.allows_scheduling(request) {
            increment(
                &mut self.scheduling_signals.unauthorized,
                &mut self.scheduling_signals.saturated,
            );
            Err(SchedulingRefusalV1::Unauthorized)
        } else if !request.within_limits(self.engine.limits(), self.limits.max_page_entries) {
            increment(
                &mut self.scheduling_signals.limited,
                &mut self.scheduling_signals.saturated,
            );
            Err(SchedulingRefusalV1::Limit)
        } else {
            Ok(())
        };
        self.scheduling_stage(0, started.elapsed());
        result
    }
    pub(crate) fn execute_scheduling(
        &mut self,
        request: &SchedulingRequestV1,
    ) -> SchedulingReplyV1 {
        let started = Instant::now();
        let result = match request {
            SchedulingRequestV1::Control { expected, command } => {
                increment(
                    &mut self.scheduling_signals.controls,
                    &mut self.scheduling_signals.saturated,
                );
                let result = self
                    .engine
                    .execute_scheduling_control_v1(*expected, command.clone());
                if result.is_err() {
                    increment(
                        &mut self.scheduling_signals.domain_refused,
                        &mut self.scheduling_signals.saturated,
                    );
                }
                Ok(SchedulingResponseV1::Outcome(result))
            }
            _ => {
                increment(
                    &mut self.scheduling_signals.queries,
                    &mut self.scheduling_signals.saturated,
                );
                match request {
                    SchedulingRequestV1::Overview { now } => Ok(SchedulingResponseV1::Overview {
                        revision: self.engine.task_state_revision(),
                        controls: self.engine.scheduling_controls_v1(),
                        concurrency: self.engine.concurrency_usage_v1(*now),
                    }),
                    SchedulingRequestV1::Eligibility { task } => {
                        Ok(SchedulingResponseV1::Eligibility {
                            revision: self.engine.task_state_revision(),
                            eligibility: self.engine.scheduling_eligibility_v1(task),
                        })
                    }
                    SchedulingRequestV1::Reservations {
                        expected,
                        after,
                        limit,
                    } => {
                        let revision = self.engine.task_state_revision();
                        if revision != *expected
                            || revision == chronos_engine::StateRevision::Exhausted
                        {
                            Err(SchedulingRefusalV1::StaleView)
                        } else {
                            let mut attempts = Vec::new();
                            let mut more = false;
                            if let Some(reservations) = self.engine.concurrency_reservations_v1() {
                                for attempt in reservations
                                    .filter(|item| after.as_ref().is_none_or(|after| *item > after))
                                {
                                    if attempts.len() == *limit {
                                        more = true;
                                        break;
                                    }
                                    attempts.push(attempt.clone());
                                }
                            }
                            let next = if more { attempts.last().cloned() } else { None };
                            Ok(SchedulingResponseV1::Reservations {
                                revision,
                                attempts,
                                next,
                            })
                        }
                    }
                    SchedulingRequestV1::Control { .. } => unreachable!("control handled above"),
                }
            }
        };
        self.scheduling_stage(1, started.elapsed());
        result
    }
    pub(crate) fn release_scheduling(
        &mut self,
        request: &SchedulingRequestV1,
        response: SchedulingResponseV1,
    ) -> SchedulingReplyV1 {
        let started = Instant::now();
        let allowed = self.authorizer.allows_scheduling_result(request, &response);
        self.scheduling_stage(3, started.elapsed());
        if !allowed {
            increment(
                &mut self.scheduling_signals.withheld,
                &mut self.scheduling_signals.saturated,
            );
            return Err(SchedulingRefusalV1::ResultWithheld);
        }
        Ok(response)
    }
    pub fn dispatch_scheduling(&mut self, request: SchedulingRequestV1) -> SchedulingReplyV1 {
        let started = Instant::now();
        let result = self.admit_scheduling(&request).and_then(|()| {
            let response = self.execute_scheduling(&request)?;
            if matches!(request, SchedulingRequestV1::Control { .. }) && !self.gated_observer {
                self.publish_observer();
            }
            self.release_scheduling(&request, response)
        });
        self.finish_scheduling(started.elapsed());
        result
    }
}
impl<A: SchedulingAuthorizerV1> SchedulingClientV1 for Service<A> {
    type TransportError = std::convert::Infallible;
    fn call_scheduling(
        &mut self,
        request: SchedulingRequestV1,
    ) -> Result<SchedulingReplyV1, Self::TransportError> {
        Ok(self.dispatch_scheduling(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Deny;
    impl Authorizer for Deny {
        fn allows(&mut self, _: &chronos_host::native::RequestV1) -> bool {
            false
        }
    }
    impl SchedulingAuthorizerV1 for Deny {
        fn allows_scheduling(&mut self, _: &SchedulingRequestV1) -> bool {
            false
        }
        fn allows_scheduling_result(
            &mut self,
            _: &SchedulingRequestV1,
            _: &SchedulingResponseV1,
        ) -> bool {
            panic!("denied before lookup")
        }
    }
    #[test]
    fn denied_scope_precedes_oversized_target_validation_and_presence() {
        let mut owner = Service::new(
            chronos_engine::Engine::new(),
            Deny,
            crate::Limits {
                max_graph_tasks: 1,
                max_page_entries: 1,
            },
        );
        let request = SchedulingRequestV1::Eligibility {
            task: chronos_engine::TaskRefV1::from_observation(
                chronos_engine::TaskId::new("x".repeat(4096)),
                0,
            ),
        };
        assert!(matches!(
            owner.dispatch_scheduling(request),
            Err(SchedulingRefusalV1::Unauthorized)
        ));
        assert_eq!(owner.scheduling_signals().unauthorized, 1);
        assert_eq!(owner.scheduling_signals().limited, 0);
    }

    struct Reader;
    impl Authorizer for Reader {
        fn allows(&mut self, _: &chronos_host::native::RequestV1) -> bool {
            true
        }
    }
    impl SchedulingAuthorizerV1 for Reader {
        fn allows_scheduling(&mut self, request: &SchedulingRequestV1) -> bool {
            !matches!(request, SchedulingRequestV1::Control { .. })
        }
        fn allows_scheduling_result(
            &mut self,
            _: &SchedulingRequestV1,
            _: &SchedulingResponseV1,
        ) -> bool {
            true
        }
    }
    #[test]
    fn reservation_pages_are_bounded_revision_consistent_and_read_rights_do_not_release_slots() {
        use chronos_engine::{Agent, Engine, SchedulingCommandV1 as C, Task};
        let mut engine = Engine::new();
        for id in ["a", "b"] {
            engine.submit(Task::new(id, "")).unwrap();
        }
        for command in [C::Enable, C::SetConcurrencyLimit { limit: 2 }] {
            engine
                .execute_scheduling_control_v1(engine.task_state_revision(), command)
                .unwrap();
        }
        let now = std::time::SystemTime::UNIX_EPOCH;
        let grant = engine
            .issue_grant(Agent::new("fixture"), now, Duration::from_secs(60))
            .unwrap();
        for id in ["a", "b"] {
            engine
                .claim_task(&grant, &id.into(), now, Duration::from_secs(10))
                .unwrap();
            engine.cancel(&id.into()).unwrap();
        }
        let revision = engine.task_state_revision();
        let mut owner = Service::new(
            engine,
            Reader,
            crate::Limits {
                max_graph_tasks: 2,
                max_page_entries: 1,
            },
        );
        let SchedulingResponseV1::Reservations {
            attempts,
            next: Some(next),
            ..
        } = owner
            .dispatch_scheduling(SchedulingRequestV1::Reservations {
                expected: revision,
                after: None,
                limit: 1,
            })
            .unwrap()
        else {
            panic!("page")
        };
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].task().task_id().as_str(), "a");
        let SchedulingResponseV1::Reservations {
            attempts,
            next: None,
            ..
        } = owner
            .dispatch_scheduling(SchedulingRequestV1::Reservations {
                expected: revision,
                after: Some(next.clone()),
                limit: 1,
            })
            .unwrap()
        else {
            panic!("page")
        };
        assert_eq!(attempts[0].task().task_id().as_str(), "b");
        assert!(matches!(
            owner.dispatch_scheduling(SchedulingRequestV1::Control {
                expected: revision,
                command: C::ConfirmQuiescent {
                    attempt: next.clone()
                }
            }),
            Err(SchedulingRefusalV1::Unauthorized)
        ));
        assert_eq!(owner.engine.concurrency_usage_v1(now).unwrap().reserved, 2);
        owner
            .engine
            .execute_scheduling_control_v1(revision, C::SetPaused { paused: true })
            .unwrap();
        assert!(matches!(
            owner.dispatch_scheduling(SchedulingRequestV1::Reservations {
                expected: revision,
                after: Some(next),
                limit: 1
            }),
            Err(SchedulingRefusalV1::StaleView)
        ));
        assert_eq!(owner.scheduling_signals().stages[2].samples, 0);
        assert_eq!(owner.scheduling_signals().stages[3].samples, 2);
    }
}
