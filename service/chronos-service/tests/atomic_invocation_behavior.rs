use chronos_engine::{
    AcknowledgementStrength, EngineLimits, RecoveryLimits, ScheduledTaskSpecV1,
    SchedulingEligibilityV1, StateRevision, TaskSpecV2,
};
use chronos_host::native::{RequestV1, ResponseV1};
use chronos_host::native_v2::RequestV2;
use chronos_host::native_v3::*;
use chronos_host::service_envelope::{EnvelopeReceipt, LoadedEnvelope, ServiceEnvelopeStore};
use chronos_host::{IoStage, StoreError};
use chronos_service::atomic::{AtomicError, AtomicLimits, AtomicService, InvocationAuthorizer};
use chronos_service::{Authorizer, Limits, coordination_v2::AuthorizerV2};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};
type Register = Arc<Mutex<Option<(u64, Vec<u8>)>>>;
#[derive(Clone, Default)]
struct Store {
    state: Register,
    mode: Arc<AtomicU8>,
}
impl ServiceEnvelopeStore for Store {
    fn load(&self) -> Result<Option<LoadedEnvelope>, StoreError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .as_ref()
            .map(|(revision, bytes)| LoadedEnvelope::from_storage(*revision, bytes.clone())))
    }
    fn commit(
        &mut self,
        expected: u64,
        revision: u64,
        bytes: &[u8],
    ) -> Result<EnvelopeReceipt, StoreError> {
        let mode = self.mode.load(Ordering::Relaxed);
        if mode == 1 {
            return Err(StoreError::Io {
                stage: IoStage::WriteTemporary,
                kind: std::io::ErrorKind::Other,
            });
        }
        let mut state = self.state.lock().unwrap();
        assert_eq!(expected, state.as_ref().map_or(0, |value| value.0));
        *state = Some((revision, bytes.to_vec()));
        if mode == 2 {
            return Err(StoreError::CommitIndeterminate {
                revision: chronos_engine::SnapshotRevision::new(revision),
                stage: IoStage::SyncDirectory,
                kind: std::io::ErrorKind::Other,
            });
        }
        Ok(EnvelopeReceipt {
            revision: if mode == 4 { revision + 1 } else { revision },
            strength: if mode == 3 {
                AcknowledgementStrength::Volatile
            } else {
                AcknowledgementStrength::Durable
            },
        })
    }
}
struct Policy {
    release: bool,
}
impl chronos_service::scheduling::SchedulingAuthorizerV1 for Policy {
    fn allows_scheduling(&mut self, _: &chronos_host::scheduling::SchedulingRequestV1) -> bool {
        true
    }
    fn allows_scheduling_result(
        &mut self,
        _: &chronos_host::scheduling::SchedulingRequestV1,
        _: &chronos_host::scheduling::SchedulingResponseV1,
    ) -> bool {
        self.release
    }
}

#[test]
fn scheduling_controls_publish_before_release_and_queries_do_not_commit() {
    use chronos_engine::SchedulingCommandV1 as C;
    use chronos_host::scheduling::{
        SchedulingRefusalV1 as F, SchedulingRequestV1 as Q, SchedulingResponseV1 as R,
    };
    let store = Store::default();
    let mut service = service(store.clone());
    for (revision, command) in [(0, C::Enable), (1, C::SetConcurrencyLimit { limit: 1 })] {
        assert!(matches!(
            service
                .dispatch_scheduling(Q::Control {
                    expected: StateRevision::finite(revision),
                    command
                })
                .unwrap(),
            Ok(R::Outcome(Ok(())))
        ));
    }
    let committed = store.state.lock().unwrap().as_ref().unwrap().0;
    assert!(matches!(
        service
            .dispatch_scheduling(Q::Overview {
                now: std::time::SystemTime::UNIX_EPOCH
            })
            .unwrap(),
        Ok(R::Overview {
            concurrency: Some(chronos_engine::ConcurrencyUsageV1 {
                capacity: 1,
                reserved: 0,
                ..
            }),
            ..
        })
    ));
    assert_eq!(store.state.lock().unwrap().as_ref().unwrap().0, committed);
    drop(service);
    let reopen = |release| {
        AtomicService::open_existing(
            store.clone(),
            Policy { release },
            Limits {
                max_graph_tasks: 4,
                max_page_entries: 4,
            },
            recovery(),
            limits(),
        )
        .unwrap()
    };
    let mut withheld = reopen(false);
    assert!(matches!(
        withheld
            .dispatch_scheduling(Q::Control {
                expected: StateRevision::finite(2),
                command: C::SetPaused { paused: true }
            })
            .unwrap(),
        Err(F::ResultWithheld)
    ));
    assert_eq!(withheld.scheduling_signals().unwrap().withheld, 1);
    drop(withheld);
    let mut recovered = reopen(true);
    assert!(matches!(
        recovered
            .dispatch_scheduling(Q::Overview {
                now: std::time::SystemTime::UNIX_EPOCH
            })
            .unwrap(),
        Ok(R::Overview {
            controls: Some(chronos_engine::SchedulingControlsV1 { paused: true, .. }),
            ..
        })
    ));
    assert!(matches!(
        recovered
            .dispatch_scheduling(Q::Reservations {
                expected: StateRevision::ZERO,
                after: None,
                limit: 1
            })
            .unwrap(),
        Err(F::StaleView)
    ));
    assert!(matches!(
        recovered
            .dispatch_scheduling(Q::Reservations {
                expected: StateRevision::finite(3),
                after: None,
                limit: 0
            })
            .unwrap(),
        Err(F::Limit)
    ));
}

#[test]
fn scheduling_control_storage_faults_fence_and_recover_actual_committed_profile() {
    use chronos_host::scheduling::{SchedulingRequestV1 as Q, SchedulingResponseV1 as R};
    for mode in 1..=4 {
        let store = Store::default();
        let mut service = service(store.clone());
        store.mode.store(mode, Ordering::Relaxed);
        assert!(
            service
                .dispatch_scheduling(Q::Control {
                    expected: StateRevision::ZERO,
                    command: chronos_engine::SchedulingCommandV1::Enable
                })
                .is_err()
        );
        assert!(matches!(
            service.dispatch_scheduling(Q::Overview {
                now: std::time::SystemTime::UNIX_EPOCH
            }),
            Err(AtomicError::Fenced)
        ));
        drop(service);
        store.mode.store(0, Ordering::Relaxed);
        let mut recovered = AtomicService::open_existing(
            store,
            Policy { release: true },
            Limits {
                max_graph_tasks: 4,
                max_page_entries: 4,
            },
            recovery(),
            limits(),
        )
        .unwrap();
        let Ok(R::Overview { controls, .. }) = recovered
            .dispatch_scheduling(Q::Overview {
                now: std::time::SystemTime::UNIX_EPOCH,
            })
            .unwrap()
        else {
            panic!("overview")
        };
        assert_eq!(controls.is_some(), mode != 1);
    }
}
impl chronos_service::atomic::AllocationAuthorizer for Policy {
    fn allows_allocation(&mut self, _: &InvocationV1) -> bool {
        true
    }
    fn allows_allocation_result(
        &mut self,
        _: &InvocationV1,
        _: &chronos_host::native_allocation::AllocationReceiptV1,
    ) -> bool {
        self.release
    }
}

#[test]
fn retained_allocation_survives_retirement_name_reuse_and_restart_without_retargeting() {
    use chronos_host::native_allocation::AllocationResultV1;
    let store = Store::default();
    let mut service = service(store.clone());
    service.enable_allocation_receipts().unwrap();
    service.invoke(execute(1, submit())).unwrap().unwrap();
    let AllocationResultV1::Created(original) = service.allocation(id(1)).unwrap().unwrap() else {
        panic!("original allocation expected")
    };
    let first = original.tasks[0].clone();
    let revision = |service: &mut AtomicService<Policy, Store>| {
        let Ok(ResponseV1::Revision(value)) = service.dispatch(RequestV1::Revision).unwrap() else {
            panic!("revision expected")
        };
        value
    };
    let expected = revision(&mut service);
    service
        .dispatch(RequestV1::Cancel {
            expected,
            task: first.clone(),
        })
        .unwrap()
        .unwrap();
    let expected = revision(&mut service);
    service
        .dispatch_v2(chronos_host::native_v2::RequestV2::Retire {
            expected,
            task: first.clone(),
        })
        .unwrap()
        .unwrap();
    let expected = revision(&mut service);
    service
        .dispatch(RequestV1::SubmitGraph {
            expected,
            tasks: vec![TaskSpecV2::new("a")],
        })
        .unwrap()
        .unwrap();
    let Ok(ResponseV1::Snapshot(Some(current))) = service
        .dispatch(RequestV1::Snapshot { task: "a".into() })
        .unwrap()
    else {
        panic!("current snapshot expected")
    };
    assert_ne!(current.reference, first);
    assert_eq!(
        service.allocation(id(1)).unwrap(),
        Ok(AllocationResultV1::Created(original.clone()))
    );
    drop(service);
    let mut reopened = AtomicService::open_existing(
        store,
        Policy { release: true },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap();
    assert_eq!(
        reopened.allocation(id(1)).unwrap(),
        Ok(AllocationResultV1::Created(original))
    );
    reopened.authorizer_mut().unwrap().release = false;
    assert_eq!(
        reopened.allocation(id(1)).unwrap(),
        Err(InvocationRefusalV1::Unauthorized)
    );
}

#[test]
fn allocation_upgrade_preserves_v1_outcomes_without_inventing_old_receipts() {
    use chronos_host::native_allocation::AllocationResultV1;
    let store = Store::default();
    let mut service = service(store.clone());
    service.invoke(execute(1, submit())).unwrap().unwrap();
    assert_eq!(
        service.allocation(id(1)).unwrap(),
        Ok(AllocationResultV1::Unavailable)
    );
    service.enable_allocation_receipts().unwrap();
    assert_eq!(
        service.allocation(id(1)).unwrap(),
        Ok(AllocationResultV1::Unavailable)
    );
    assert_eq!(
        service
            .invoke(InvocationRequestV1::Lookup { invocation: id(1) })
            .unwrap(),
        Ok(InvocationResponseV1::Outcome {
            serial: 1,
            result: MutationOutcomeV1::Applied
        })
    );
    let before = service.signals().commits;
    service.enable_allocation_receipts().unwrap();
    assert_eq!(service.signals().commits, before);
    drop(service);
    let mut reopened = AtomicService::open_existing(
        store,
        Policy { release: true },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap();
    assert_eq!(
        reopened.allocation(id(1)).unwrap(),
        Ok(AllocationResultV1::Unavailable)
    );
}

#[test]
fn allocation_candidate_never_escapes_failed_publication_and_recovery_reads_the_actual_winner() {
    use chronos_host::native_allocation::AllocationResultV1;
    for mode in 1..=4 {
        let store = Store::default();
        let mut service = service(store.clone());
        service.enable_allocation_receipts().unwrap();
        store.mode.store(mode, Ordering::Relaxed);
        assert!(service.invoke(execute(1, submit())).is_err());
        assert!(matches!(
            service.allocation(id(1)),
            Err(AtomicError::Fenced)
        ));
        store.mode.store(0, Ordering::Relaxed);
        let mut recovered = AtomicService::open_existing(
            store,
            Policy { release: true },
            Limits {
                max_graph_tasks: 4,
                max_page_entries: 4,
            },
            recovery(),
            limits(),
        )
        .unwrap();
        if mode == 1 {
            assert_eq!(
                recovered.allocation(id(1)).unwrap(),
                Err(InvocationRefusalV1::OutOfSequence)
            );
        } else {
            assert!(matches!(
                recovered.allocation(id(1)).unwrap(),
                Ok(AllocationResultV1::Created(_))
            ));
        }
    }
}

#[test]
fn denied_allocation_scope_precedes_validation_and_presence_without_a_commit() {
    struct Denied;
    impl Authorizer for Denied {
        fn allows(&mut self, _: &RequestV1) -> bool {
            true
        }
    }
    impl AuthorizerV2 for Denied {
        fn allows_v2(&mut self, _: &RequestV2) -> bool {
            true
        }
    }
    impl InvocationAuthorizer for Denied {
        fn allows_invocation(&mut self, _: &InvocationRequestV1) -> bool {
            true
        }
        fn allows_retained_result(&mut self, _: &InvocationV1, _: &MutationV1) -> bool {
            true
        }
    }
    impl chronos_service::atomic::AllocationAuthorizer for Denied {
        fn allows_allocation(&mut self, _: &InvocationV1) -> bool {
            false
        }
        fn allows_allocation_result(
            &mut self,
            _: &InvocationV1,
            _: &chronos_host::native_allocation::AllocationReceiptV1,
        ) -> bool {
            false
        }
    }
    let mut service = AtomicService::create_new(
        Store::default(),
        Denied,
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap();
    let commits = service.signals().commits;
    assert_eq!(
        service
            .allocation(InvocationV1 {
                serial: 0,
                key: "x".repeat(1024)
            })
            .unwrap(),
        Err(InvocationRefusalV1::Unauthorized)
    );
    assert_eq!(service.signals().commits, commits);
    assert_eq!(service.signals().limited, 0);
}

#[test]
fn v2_allocation_recovery_refuses_all_truncations_and_eviction_never_retargets() {
    use chronos_host::native_allocation::AllocationResultV1;
    let store = Store::default();
    let mut service = service(store.clone());
    service.enable_allocation_receipts().unwrap();
    service.invoke(execute(1, submit())).unwrap().unwrap();
    let (revision, bytes) = store.state.lock().unwrap().clone().unwrap();
    assert_eq!(&bytes[8..10], &[0, 2]);
    for end in 0..bytes.len() {
        let damaged = Store::default();
        *damaged.state.lock().unwrap() = Some((revision, bytes[..end].to_vec()));
        assert!(
            AtomicService::open_existing(
                damaged,
                Policy { release: true },
                Limits {
                    max_graph_tasks: 4,
                    max_page_entries: 4
                },
                recovery(),
                limits()
            )
            .is_err()
        );
    }
    let before = service.signals().commits;
    assert!(matches!(
        service.allocation(id(1)).unwrap(),
        Ok(AllocationResultV1::Created(_))
    ));
    assert_eq!(
        service.signals().commits,
        before,
        "facet lookup is not a mutation"
    );
    assert_eq!(
        service
            .allocation(InvocationV1 {
                serial: 1,
                key: "changed".into()
            })
            .unwrap(),
        Err(InvocationRefusalV1::Conflict)
    );
    for serial in [2, 3] {
        // Retain typed domain refusals as before, evicting the original serial.
        service.invoke(execute(serial, submit())).unwrap().unwrap();
    }
    assert_eq!(
        service.allocation(id(1)).unwrap(),
        Err(InvocationRefusalV1::Expired)
    );
    assert_eq!(
        service.invoke(execute(1, submit())).unwrap(),
        Err(InvocationRefusalV1::Expired)
    );
}
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl chronos_service::worker::WorkerAuthorizer for Policy {
    fn allows_worker(&mut self, _: &chronos_host::worker::WorkerRequestV1) -> bool {
        true
    }
    fn allows_worker_result(
        &mut self,
        _: &chronos_host::worker::WorkerRequestV1,
        _: &chronos_engine::CommandOutcomeV2,
    ) -> bool {
        self.release
    }
}

#[test]
fn worker_envelope_commits_preserve_retained_results_and_fence_failed_claim_delivery() {
    use chronos_engine::{Agent, CommandOutcomeV2};
    use chronos_host::worker::WorkerRequestV1 as Work;
    use std::time::{Duration, SystemTime};
    for mode in 0..=4 {
        let store = Store::default();
        let mut service = service(store.clone());
        let expected = service.invoke(execute(1, submit())).unwrap();
        let CommandOutcomeV2::AdvanceTime(Ok(time)) = service
            .dispatch_worker(Work::AdvanceTime {
                now: SystemTime::UNIX_EPOCH,
            })
            .unwrap()
            .unwrap()
        else {
            panic!("time expected")
        };
        let CommandOutcomeV2::IssueGrant(Ok(grant)) = service
            .dispatch_worker(Work::IssueGrant {
                agent: Agent::new("worker"),
                time: time.clone(),
                validity_duration: Duration::from_secs(60),
                claim_budget: None,
            })
            .unwrap()
            .unwrap()
        else {
            panic!("grant expected")
        };
        store.mode.store(mode, Ordering::Relaxed);
        let result = service.dispatch_worker(Work::ClaimNext {
            grant,
            time,
            lease_duration: Duration::from_secs(10),
        });
        if mode == 0 {
            assert!(matches!(
                result,
                Ok(Ok(CommandOutcomeV2::ClaimNext(Ok(Some(_)))))
            ));
            assert_eq!(service.worker_signals().unwrap().executions, 3);
            assert_eq!(
                service
                    .invoke(InvocationRequestV1::Lookup { invocation: id(1) })
                    .unwrap(),
                expected
            );
        } else {
            assert!(result.is_err());
            assert!(service.worker_signals().is_none());
            assert!(service.worker_usage(SystemTime::UNIX_EPOCH).is_none());
            assert!(matches!(
                service.invoke(InvocationRequestV1::Next),
                Err(AtomicError::Fenced)
            ));
        }
        drop(service);
        store.mode.store(0, Ordering::Relaxed);
        let mut recovered = AtomicService::open_existing(
            store,
            Policy { release: true },
            Limits {
                max_graph_tasks: 4,
                max_page_entries: 4,
            },
            recovery(),
            limits(),
        )
        .unwrap();
        assert_eq!(
            recovered
                .invoke(InvocationRequestV1::Lookup { invocation: id(1) })
                .unwrap(),
            expected
        );
        let Ok(ResponseV1::Snapshot(Some(value))) = recovered
            .dispatch(RequestV1::Snapshot { task: "a".into() })
            .unwrap()
        else {
            panic!("snapshot expected")
        };
        assert_eq!(
            value.task.phase,
            if mode == 1 {
                chronos_engine::TaskPhase::Ready
            } else {
                chronos_engine::TaskPhase::Uncertain
            }
        );
    }
}
impl AuthorizerV2 for Policy {
    fn allows_v2(&mut self, _: &RequestV2) -> bool {
        true
    }
}
impl InvocationAuthorizer for Policy {
    fn allows_invocation(&mut self, _: &InvocationRequestV1) -> bool {
        true
    }
    fn allows_retained_result(&mut self, _: &InvocationV1, _: &MutationV1) -> bool {
        self.release
    }
}
impl chronos_service::observer::ObserverAuthorizer for Policy {
    fn allows_observer(&mut self, _: &chronos_host::native_observer::ObserverRequestV1) -> bool {
        true
    }
    fn allows_observer_changes(
        &mut self,
        _: &[chronos_host::native_observer::ChangeSetV1],
    ) -> bool {
        self.release
    }
}
fn limits() -> AtomicLimits {
    AtomicLimits {
        max_envelope_bytes: 1_000_000,
        max_retained_bytes: 4096,
        max_command_bytes: 2048,
        max_key_bytes: 64,
        max_invocations: 2,
    }
}
fn recovery() -> RecoveryLimits {
    RecoveryLimits::new(900_000, EngineLimits::default())
}
fn service(store: Store) -> AtomicService<Policy, Store> {
    AtomicService::create_new(
        store,
        Policy { release: true },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap()
}

fn id(serial: u64) -> InvocationV1 {
    InvocationV1 {
        serial,
        key: format!("intent-{serial}"),
    }
}
fn submit() -> MutationV1 {
    MutationV1::SubmitGraph {
        expected: StateRevision::ZERO,
        tasks: vec![TaskSpecV2::new("a")],
    }
}
fn execute(serial: u64, command: MutationV1) -> InvocationRequestV1 {
    InvocationRequestV1::Execute {
        invocation: id(serial),
        command,
    }
}
#[test]
fn lost_reply_lookup_and_identical_replay_survive_restart_without_a_second_commit() {
    let store = Store::default();
    let mut service = service(store.clone());
    let expected = Ok(InvocationResponseV1::Outcome {
        serial: 1,
        result: MutationOutcomeV1::Applied,
    });
    assert_eq!(service.invoke(execute(1, submit())).unwrap(), expected);
    let committed = store.state.lock().unwrap().clone();
    assert_eq!(
        service
            .invoke(InvocationRequestV1::LookupCommand {
                invocation: id(1),
                command: submit(),
            })
            .unwrap(),
        expected
    );
    assert_eq!(service.invoke(execute(1, submit())).unwrap(), expected);
    assert_eq!(*store.state.lock().unwrap(), committed);
    assert_eq!(
        service
            .invoke(InvocationRequestV1::Execute {
                invocation: InvocationV1 {
                    serial: 1,
                    key: "different-intent".into()
                },
                command: submit()
            })
            .unwrap(),
        Err(InvocationRefusalV1::Conflict)
    );
    assert_eq!(
        service
            .invoke(execute(
                1,
                MutationV1::SubmitGraph {
                    expected: StateRevision::ZERO,
                    tasks: vec![TaskSpecV2::new("changed")]
                }
            ))
            .unwrap(),
        Err(InvocationRefusalV1::Conflict)
    );
    drop(service);
    let mut reopened = AtomicService::open_existing(
        store,
        Policy { release: true },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap();
    assert_eq!(
        reopened
            .invoke(InvocationRequestV1::Lookup { invocation: id(1) })
            .unwrap(),
        expected
    );
    assert_eq!(
        reopened.invoke(InvocationRequestV1::Next).unwrap(),
        Ok(InvocationResponseV1::Next { serial: Some(2) })
    );
}

#[test]
fn command_scoped_lookup_never_admits_a_missing_invocation() {
    let mut service = service(Store::default());
    assert_eq!(
        service
            .invoke(InvocationRequestV1::LookupCommand {
                invocation: id(1),
                command: submit(),
            })
            .unwrap(),
        Err(InvocationRefusalV1::OutOfSequence)
    );
    assert!(matches!(
        service
            .dispatch(RequestV1::Snapshot { task: "a".into() })
            .unwrap(),
        Ok(ResponseV1::Snapshot(None))
    ));
}

#[test]
fn scheduled_admission_is_atomic_retained_and_recovers_its_exact_allocation() {
    use chronos_host::native_allocation::AllocationResultV1;
    use chronos_host::scheduling::{SchedulingRequestV1 as Q, SchedulingResponseV1 as R};

    let not_before = std::time::SystemTime::UNIX_EPOCH
        .checked_add(std::time::Duration::new(42, 7))
        .unwrap();
    let command = MutationV1::SubmitScheduledGraph {
        expected: StateRevision::finite(1),
        tasks: vec![ScheduledTaskSpecV1 {
            task: TaskSpecV2::new("scheduled"),
            eligibility: SchedulingEligibilityV1 {
                paused: true,
                not_before: Some(not_before),
            },
        }],
    };

    let mut disabled = service(Store::default());
    assert_eq!(
        disabled
            .invoke(execute(
                1,
                MutationV1::SubmitScheduledGraph {
                    expected: StateRevision::ZERO,
                    tasks: match &command {
                        MutationV1::SubmitScheduledGraph { tasks, .. } => tasks.clone(),
                        _ => unreachable!(),
                    },
                },
            ))
            .unwrap(),
        Ok(InvocationResponseV1::Outcome {
            serial: 1,
            result: MutationOutcomeV1::Refused(MutationCodeV1::SchedulingNotEnabled),
        })
    );
    assert!(matches!(
        disabled
            .dispatch(RequestV1::Snapshot {
                task: "scheduled".into()
            })
            .unwrap(),
        Ok(ResponseV1::Snapshot(None))
    ));

    let store = Store::default();
    let mut service = service(store.clone());
    service.enable_allocation_receipts().unwrap();
    assert!(matches!(
        service
            .dispatch_scheduling(Q::Control {
                expected: StateRevision::ZERO,
                command: chronos_engine::SchedulingCommandV1::Enable,
            })
            .unwrap(),
        Ok(R::Outcome(Ok(())))
    ));
    let expected = Ok(InvocationResponseV1::Outcome {
        serial: 1,
        result: MutationOutcomeV1::Applied,
    });
    assert_eq!(
        service.invoke(execute(1, command.clone())).unwrap(),
        expected
    );
    let committed = store.state.lock().unwrap().clone();
    assert_eq!(service.invoke(execute(1, command)).unwrap(), expected);
    assert_eq!(*store.state.lock().unwrap(), committed);

    let AllocationResultV1::Created(allocation) = service.allocation(id(1)).unwrap().unwrap()
    else {
        panic!("scheduled allocation expected")
    };
    assert_eq!(allocation.tasks.len(), 1);
    assert_eq!(allocation.tasks[0].task_id().as_str(), "scheduled");
    assert!(matches!(
        service
            .dispatch_scheduling(Q::Eligibility {
                task: allocation.tasks[0].clone(),
            })
            .unwrap(),
        Ok(R::Eligibility {
            eligibility: Some(SchedulingEligibilityV1 {
                paused: true,
                not_before: Some(value),
            }),
            ..
        }) if value == not_before
    ));

    drop(service);
    let mut reopened = AtomicService::open_existing(
        store,
        Policy { release: true },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap();
    assert_eq!(
        reopened
            .invoke(InvocationRequestV1::Lookup { invocation: id(1) })
            .unwrap(),
        expected
    );
    assert_eq!(
        reopened.allocation(id(1)).unwrap(),
        Ok(AllocationResultV1::Created(allocation.clone()))
    );
    assert!(matches!(
        reopened
            .dispatch_scheduling(Q::Eligibility {
                task: allocation.tasks[0].clone(),
            })
            .unwrap(),
        Ok(R::Eligibility {
            eligibility: Some(SchedulingEligibilityV1 {
                paused: true,
                not_before: Some(value),
            }),
            ..
        }) if value == not_before
    ));
}
#[test]
fn eviction_never_reopens_a_serial_and_result_disclosure_is_currently_authorized() {
    let mut service = service(Store::default());
    service.invoke(execute(1, submit())).unwrap().unwrap();
    let Ok(ResponseV1::Snapshot(Some(snapshot))) = service
        .dispatch(RequestV1::Snapshot { task: "a".into() })
        .unwrap()
    else {
        panic!("task expected")
    };
    service
        .invoke(execute(
            2,
            MutationV1::Cancel {
                expected: snapshot.revision,
                task: snapshot.reference.clone(),
            },
        ))
        .unwrap()
        .unwrap();
    service
        .invoke(execute(
            3,
            MutationV1::Retire {
                expected: StateRevision::Finite(2),
                task: snapshot.reference,
            },
        ))
        .unwrap()
        .unwrap();
    assert_eq!(
        service.invoke(execute(1, submit())).unwrap(),
        Err(InvocationRefusalV1::Expired)
    );
    service.authorizer_mut().unwrap().release = false;
    assert_eq!(
        service
            .invoke(InvocationRequestV1::Lookup { invocation: id(3) })
            .unwrap(),
        Err(InvocationRefusalV1::Unauthorized)
    );
    service.authorizer_mut().unwrap().release = true;
    assert_eq!(
        service
            .invoke(InvocationRequestV1::Lookup { invocation: id(3) })
            .unwrap(),
        Ok(InvocationResponseV1::Outcome {
            serial: 3,
            result: MutationOutcomeV1::Applied
        })
    );
    assert_eq!(
        service.invoke(execute(5, submit())).unwrap(),
        Err(InvocationRefusalV1::OutOfSequence)
    );
}
#[test]
fn unknown_or_bad_receipts_fence_all_access_and_recovery_reads_the_atomic_record() {
    for mode in 1..=4 {
        let store = Store::default();
        let mut service = service(store.clone());
        store.mode.store(mode, Ordering::Relaxed);
        assert!(service.invoke(execute(1, submit())).is_err());
        assert!(matches!(
            service.invoke(InvocationRequestV1::Next),
            Err(AtomicError::Fenced)
        ));
        store.mode.store(0, Ordering::Relaxed);
        let mut recovered = AtomicService::open_existing(
            store,
            Policy { release: true },
            Limits {
                max_graph_tasks: 4,
                max_page_entries: 4,
            },
            recovery(),
            limits(),
        )
        .unwrap();
        let result = recovered
            .invoke(InvocationRequestV1::Lookup { invocation: id(1) })
            .unwrap();
        if mode == 1 {
            assert_eq!(result, Err(InvocationRefusalV1::OutOfSequence));
        } else {
            assert_eq!(
                result,
                Ok(InvocationResponseV1::Outcome {
                    serial: 1,
                    result: MutationOutcomeV1::Applied
                })
            );
        }
    }
}

#[test]
fn canonical_envelope_recovery_refuses_every_truncation_and_changed_version_or_revision() {
    let store = Store::default();
    let mut service = service(store.clone());
    service.invoke(execute(1, submit())).unwrap().unwrap();
    let (revision, bytes) = store.state.lock().unwrap().clone().unwrap();
    for end in 0..bytes.len() {
        let damaged = Store::default();
        *damaged.state.lock().unwrap() = Some((revision, bytes[..end].to_vec()));
        assert!(
            AtomicService::open_existing(
                damaged,
                Policy { release: true },
                Limits {
                    max_graph_tasks: 4,
                    max_page_entries: 4
                },
                recovery(),
                limits()
            )
            .is_err()
        );
    }
    for offset in [8, 10] {
        let mut changed = bytes.clone();
        changed[offset] ^= 1;
        let damaged = Store::default();
        *damaged.state.lock().unwrap() = Some((revision, changed));
        assert!(
            AtomicService::open_existing(
                damaged,
                Policy { release: true },
                Limits {
                    max_graph_tasks: 4,
                    max_page_entries: 4
                },
                recovery(),
                limits()
            )
            .is_err()
        );
    }
    let mut smaller = limits();
    smaller.max_retained_bytes = 1;
    assert!(
        AtomicService::open_existing(
            store,
            Policy { release: true },
            Limits {
                max_graph_tasks: 4,
                max_page_entries: 4
            },
            recovery(),
            smaller
        )
        .is_err()
    );
}

#[test]
fn explicit_migration_preserves_the_legacy_checkpoint_and_real_store_reopens_results() {
    use chronos_adapter_checkpoint::{DirectoryCheckpointStore, DirectoryServiceEnvelopeStore};
    use chronos_host::CheckpointStore;
    let path = std::env::temp_dir().join(format!(
        "chronos-explicit-envelope-migration-{}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    let mut legacy = DirectoryCheckpointStore::open(&path, 1_000_000, 32).unwrap();
    let mut engine = chronos_engine::Engine::new();
    engine
        .submit(chronos_engine::Task::new("a", "private legacy content"))
        .unwrap();
    legacy
        .commit(&engine.prepare_checkpoint(900_000).unwrap())
        .unwrap();
    let original = legacy.load().unwrap().unwrap();
    let destination = DirectoryServiceEnvelopeStore::open(&path, 1_000_000, 32).unwrap();
    assert!(matches!(
        AtomicService::open_existing(
            DirectoryServiceEnvelopeStore::open(&path, 1_000_000, 32).unwrap(),
            Policy { release: true },
            Limits {
                max_graph_tasks: 4,
                max_page_entries: 4
            },
            recovery(),
            limits()
        ),
        Err(AtomicError::MissingState)
    ));
    let mut migrated = AtomicService::migrate_from_checkpoint(
        &legacy,
        destination,
        Policy { release: true },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap();
    assert_eq!(legacy.load().unwrap().unwrap().bytes(), original.bytes());
    let Ok(ResponseV1::Snapshot(Some(snapshot))) = migrated
        .dispatch(RequestV1::Snapshot { task: "a".into() })
        .unwrap()
    else {
        panic!("migrated task expected")
    };
    migrated
        .invoke(execute(
            1,
            MutationV1::Cancel {
                expected: snapshot.revision,
                task: snapshot.reference,
            },
        ))
        .unwrap()
        .unwrap();
    drop(migrated);
    let mut reopened = AtomicService::open_existing(
        DirectoryServiceEnvelopeStore::open(&path, 1_000_000, 32).unwrap(),
        Policy { release: true },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap();
    assert_eq!(
        reopened
            .invoke(InvocationRequestV1::Lookup { invocation: id(1) })
            .unwrap(),
        Ok(InvocationResponseV1::Outcome {
            serial: 1,
            result: MutationOutcomeV1::Applied
        })
    );
    assert_eq!(legacy.load().unwrap().unwrap().bytes(), original.bytes());
    drop(reopened);
    drop(legacy);
    for entry in std::fs::read_dir(&path).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn result_permission_is_rechecked_after_the_atomic_commit() {
    use std::sync::atomic::AtomicBool;
    struct PolicyAfterCommit(Arc<AtomicBool>);
    impl Authorizer for PolicyAfterCommit {
        fn allows(&mut self, _: &RequestV1) -> bool {
            true
        }
    }
    impl AuthorizerV2 for PolicyAfterCommit {
        fn allows_v2(&mut self, _: &RequestV2) -> bool {
            true
        }
    }
    impl InvocationAuthorizer for PolicyAfterCommit {
        fn allows_invocation(&mut self, _: &InvocationRequestV1) -> bool {
            true
        }
        fn allows_retained_result(&mut self, _: &InvocationV1, _: &MutationV1) -> bool {
            !self.0.load(Ordering::Relaxed)
        }
    }
    impl chronos_service::worker::WorkerAuthorizer for PolicyAfterCommit {
        fn allows_worker(&mut self, _: &chronos_host::worker::WorkerRequestV1) -> bool {
            true
        }
        fn allows_worker_result(
            &mut self,
            _: &chronos_host::worker::WorkerRequestV1,
            _: &chronos_engine::CommandOutcomeV2,
        ) -> bool {
            !self.0.load(Ordering::Relaxed)
        }
    }
    struct RevokingStore {
        store: Store,
        revoked: Arc<AtomicBool>,
    }
    impl ServiceEnvelopeStore for RevokingStore {
        fn load(&self) -> Result<Option<LoadedEnvelope>, StoreError> {
            self.store.load()
        }
        fn commit(
            &mut self,
            expected: u64,
            revision: u64,
            bytes: &[u8],
        ) -> Result<EnvelopeReceipt, StoreError> {
            let receipt = self.store.commit(expected, revision, bytes)?;
            self.revoked.store(true, Ordering::Relaxed);
            Ok(receipt)
        }
    }
    let revoked = Arc::new(AtomicBool::new(false));
    let store = Store::default();
    let mut service = AtomicService::create_new(
        RevokingStore {
            store: store.clone(),
            revoked: revoked.clone(),
        },
        PolicyAfterCommit(revoked.clone()),
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap();
    revoked.store(false, Ordering::Relaxed);
    assert_eq!(
        service.invoke(execute(1, submit())).unwrap(),
        Err(InvocationRefusalV1::Unauthorized)
    );
    assert_eq!(
        store.state.lock().unwrap().as_ref().unwrap().0,
        2,
        "mutation and result committed even though delivery was revoked"
    );
    revoked.store(false, Ordering::Relaxed);
    assert_eq!(
        service
            .invoke(InvocationRequestV1::Lookup { invocation: id(1) })
            .unwrap(),
        Ok(InvocationResponseV1::Outcome {
            serial: 1,
            result: MutationOutcomeV1::Applied
        })
    );
    assert_eq!(service.signals().commits, 2);
    assert_eq!(service.signals().unauthorized, 1);
    revoked.store(false, Ordering::Relaxed);
    assert_eq!(
        service
            .dispatch_worker(chronos_host::worker::WorkerRequestV1::AdvanceTime {
                now: std::time::SystemTime::UNIX_EPOCH
            })
            .unwrap(),
        Err(chronos_host::worker::WorkerRefusalV1::ResultWithheld)
    );
    assert_eq!(service.signals().commits, 3);
    assert_eq!(service.worker_signals().unwrap().withheld, 1);
}

#[test]
fn durable_feed_advances_only_for_committed_changes_and_recovery_invalidates_old_cursors() {
    use chronos_host::native_observer::*;
    use chronos_service::observer::FeedLimits;
    let feed_limits = FeedLimits {
        max_cached_tasks: 4,
        max_cached_bytes: 4096,
        max_transitions: 4,
        max_history_bytes: 4096,
        max_changes_per_transition: 4,
        max_page_transitions: 4,
        max_response_bytes: 4096,
    };
    let store = Store::default();
    let mut service = service(store.clone());
    service.enable_observer(feed_limits).unwrap().unwrap();
    let ObserverResponseV1::ResyncRequired { head } = service
        .observe(ObserverRequestV1 {
            after: None,
            limit: 4,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("head expected")
    };
    service.invoke(execute(1, submit())).unwrap().unwrap();
    let ObserverResponseV1::Changes {
        cursor, changes, ..
    } = service
        .observe(ObserverRequestV1 {
            after: Some(head),
            limit: 4,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("changes expected")
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(service.signals().commits, 2);
    service.invoke(execute(1, submit())).unwrap().unwrap();
    assert_eq!(service.signals().commits, 2);
    assert_eq!(service.observer_signals().unwrap().published, 1);
    assert_eq!(service.observer_usage().unwrap().history_transitions, 1);
    assert_eq!(service.usage().retained_entries, 1);
    let Ok(ResponseV1::Snapshot(Some(snapshot))) = service
        .dispatch(RequestV1::Snapshot { task: "a".into() })
        .unwrap()
    else {
        panic!("task expected")
    };
    store.mode.store(1, Ordering::Relaxed);
    assert!(
        service
            .invoke(execute(
                2,
                MutationV1::Cancel {
                    expected: snapshot.revision,
                    task: snapshot.reference
                }
            ))
            .is_err()
    );
    assert!(matches!(
        service.observe(ObserverRequestV1 {
            after: Some(cursor.clone()),
            limit: 4
        }),
        Err(AtomicError::Fenced)
    ));
    drop(service);
    store.mode.store(0, Ordering::Relaxed);
    let mut recovered = AtomicService::open_existing(
        store,
        Policy { release: true },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
        recovery(),
        limits(),
    )
    .unwrap();
    recovered.enable_observer(feed_limits).unwrap().unwrap();
    assert!(matches!(
        recovered
            .observe(ObserverRequestV1 {
                after: Some(cursor),
                limit: 4
            })
            .unwrap()
            .unwrap(),
        ObserverResponseV1::ResyncRequired { .. }
    ));
    assert_eq!(recovered.observer_signals().unwrap().published, 0);
}
