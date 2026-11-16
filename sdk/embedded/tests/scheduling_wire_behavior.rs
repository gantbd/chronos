use chronos_adapter_native::EmbeddedClient;
use chronos_client::SchedulingWireClientV5;
use chronos_engine::{Engine, Task};
use chronos_host::{
    native::RequestV1,
    scheduling::{SchedulingRequestV1, SchedulingResponseV1},
};
use chronos_protocol::{Limits, Reference, Revision, scheduling_v5 as v5};
use chronos_service::{Authorizer, Service, scheduling::SchedulingAuthorizerV1};
struct Policy {
    writes: bool,
    release: bool,
}
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl SchedulingAuthorizerV1 for Policy {
    fn allows_scheduling(&mut self, request: &SchedulingRequestV1) -> bool {
        !matches!(request, SchedulingRequestV1::Control { .. }) || self.writes
    }
    fn allows_scheduling_result(
        &mut self,
        _: &SchedulingRequestV1,
        _: &SchedulingResponseV1,
    ) -> bool {
        self.release
    }
}
fn fixture(policy: Policy) -> EmbeddedClient<Service<Policy>> {
    let mut engine = Engine::new();
    engine.submit(Task::new("a", "")).unwrap();
    let service = Service::new(
        engine,
        policy,
        chronos_service::Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
    );
    EmbeddedClient::new(
        service,
        Limits {
            max_frame_bytes: 4096,
            max_id_bytes: 64,
            max_graph_tasks: 4,
            max_dependencies_per_task: 4,
            max_capabilities_per_task: 4,
            max_capability_bytes: 64,
            max_page_entries: 4,
        },
    )
}
fn call(client: &mut EmbeddedClient<Service<Policy>>, command: v5::Command) -> v5::Response {
    client
        .request_scheduling_v5(v5::Request {
            version: 5,
            command,
        })
        .unwrap()
}
fn revision(value: u64) -> Revision {
    Revision::Finite(value.to_string())
}
#[test]
fn native_service_preserves_control_guards_time_and_content_free_observations() {
    let mut client = fixture(Policy {
        writes: true,
        release: true,
    });
    assert!(matches!(
        call(
            &mut client,
            v5::Command::Enable {
                expected: revision(1)
            }
        )
        .outcome,
        v5::Outcome::Control {
            result: v5::ControlResult::Applied {}
        }
    ));
    assert!(matches!(
        call(
            &mut client,
            v5::Command::SetPaused {
                expected: revision(1),
                paused: true
            }
        )
        .outcome,
        v5::Outcome::Control {
            result: v5::ControlResult::RevisionMismatch { .. }
        }
    ));
    assert!(matches!(
        call(
            &mut client,
            v5::Command::SetConcurrencyLimit {
                expected: revision(2),
                limit: 2
            }
        )
        .outcome,
        v5::Outcome::Control {
            result: v5::ControlResult::Applied {}
        }
    ));
    assert!(matches!(
        call(
            &mut client,
            v5::Command::SetEligibility {
                expected: revision(3),
                task: Reference {
                    task: "a".into(),
                    incarnation: "0".into()
                },
                eligibility: v5::Eligibility {
                    paused: true,
                    not_before: Some(v5::Timestamp {
                        before_epoch: true,
                        seconds: "0".into(),
                        nanos: 1
                    })
                }
            }
        )
        .outcome,
        v5::Outcome::Control {
            result: v5::ControlResult::Applied {}
        }
    ));
    let response = call(
        &mut client,
        v5::Command::Eligibility {
            task: Reference {
                task: "a".into(),
                incarnation: "0".into(),
            },
        },
    );
    let v5::Outcome::Eligibility {
        eligibility: Some(eligibility),
        ..
    } = response.outcome
    else {
        panic!("missing eligibility");
    };
    let time = eligibility.not_before.unwrap();
    assert!(time.before_epoch);
    assert_eq!(time.seconds, "0");
    assert_eq!(time.nanos, 1);
    let response = call(
        &mut client,
        v5::Command::Overview {
            now: v5::Timestamp {
                before_epoch: false,
                seconds: "0".into(),
                nanos: 0,
            },
        },
    );
    let v5::Outcome::Overview {
        concurrency: Some(usage),
        ..
    } = response.outcome
    else {
        panic!("missing usage");
    };
    assert_eq!((usage.capacity, usage.reserved, usage.available), (2, 0, 2));
    let signals = client.host_mut().scheduling_signals();
    assert_eq!(signals.calls, 6);
    assert_eq!(signals.controls, 4);
    assert_eq!(signals.queries, 2);
}
#[test]
fn public_control_does_not_bypass_current_admission_or_release_authority() {
    let mut denied = fixture(Policy {
        writes: false,
        release: true,
    });
    assert!(matches!(
        call(
            &mut denied,
            v5::Command::Enable {
                expected: revision(1)
            }
        )
        .outcome,
        v5::Outcome::Refused {
            code: v5::RefusalCode::Unauthorized
        }
    ));
    let mut withheld = fixture(Policy {
        writes: true,
        release: false,
    });
    assert!(matches!(
        call(
            &mut withheld,
            v5::Command::Enable {
                expected: revision(1)
            }
        )
        .outcome,
        v5::Outcome::Refused {
            code: v5::RefusalCode::ResultWithheld
        }
    ));
    // The accepted write can commit despite withheld delivery. Repeating with
    // the old guard fails; callers must inspect instead of automatically retry.
    assert!(matches!(
        call(
            &mut withheld,
            v5::Command::Enable {
                expected: revision(1)
            }
        )
        .outcome,
        v5::Outcome::Refused {
            code: v5::RefusalCode::ResultWithheld
        }
    ));
    assert_eq!(withheld.host_mut().scheduling_signals().withheld, 2);
    withheld.host_mut().authorizer_mut().release = true;
    assert!(matches!(
        call(
            &mut withheld,
            v5::Command::Enable {
                expected: revision(1)
            }
        )
        .outcome,
        v5::Outcome::Control {
            result: v5::ControlResult::RevisionMismatch { .. }
        }
    ));
}
