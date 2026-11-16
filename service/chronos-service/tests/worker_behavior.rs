use chronos_engine::{
    Agent, CommandOutcomeV2, Engine, EngineLimits, RecoveryLimits, StateRevision, TaskId,
    TaskSpecV2, TaskState,
};
use chronos_host::{
    native::{RequestV1, ResponseV1},
    worker::{WorkerRefusalV1, WorkerRequestV1 as Work},
};
use chronos_service::{Authorizer, Limits, Service, worker::WorkerAuthorizer};
use std::time::{Duration, SystemTime};
mod lifecycle;

struct Policy {
    admit: bool,
    release: bool,
}
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl WorkerAuthorizer for Policy {
    fn allows_worker(&mut self, _: &Work) -> bool {
        self.admit
    }
    fn allows_worker_result(&mut self, _: &Work, _: &CommandOutcomeV2) -> bool {
        self.release
    }
}
fn limits() -> Limits {
    Limits {
        max_graph_tasks: 8,
        max_page_entries: 8,
    }
}
fn recovery() -> RecoveryLimits {
    RecoveryLimits::new(1_000_000, EngineLimits::default())
}
fn service() -> Service<Policy> {
    let mut service = Service::new(
        Engine::new(),
        Policy {
            admit: true,
            release: true,
        },
        limits(),
    );
    assert!(matches!(
        service.dispatch(RequestV1::SubmitGraph {
            expected: StateRevision::ZERO,
            tasks: vec![TaskSpecV2::new("private-task")]
        }),
        Ok(ResponseV1::SubmitGraph(Ok(())))
    ));
    service
}
fn time(service: &mut Service<Policy>) -> chronos_engine::EngineTime {
    let CommandOutcomeV2::AdvanceTime(Ok(time)) = service
        .dispatch_worker(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH,
        })
        .unwrap()
    else {
        panic!("time expected")
    };
    time
}

#[test]
fn two_trusted_workers_contend_without_duplicate_dispatch_and_completion_uses_the_live_lease() {
    let mut service = service();
    let time = time(&mut service);
    let grant = |service: &mut Service<Policy>, name: &str| {
        let CommandOutcomeV2::IssueGrant(Ok(grant)) = service
            .dispatch_worker(Work::IssueGrant {
                agent: Agent::new(name),
                time: time.clone(),
                validity_duration: Duration::from_secs(60),
                claim_budget: Some(1),
            })
            .unwrap()
        else {
            panic!("grant expected")
        };
        grant
    };
    let first = grant(&mut service, "private-worker-one");
    let second = grant(&mut service, "private-worker-two");
    let CommandOutcomeV2::ClaimNext(Ok(Some(task))) = service
        .dispatch_worker(Work::ClaimNext {
            grant: first,
            time: time.clone(),
            lease_duration: Duration::from_secs(10),
        })
        .unwrap()
    else {
        panic!("one claim expected")
    };
    assert!(task.attempt_reference().is_some());
    let TaskState::Running(claim) = task.state else {
        panic!("running expected")
    };
    assert!(matches!(
        service.dispatch_worker(Work::ClaimNext {
            grant: second,
            time: time.clone(),
            lease_duration: Duration::from_secs(10)
        }),
        Ok(CommandOutcomeV2::ClaimNext(Ok(None)))
    ));
    assert!(matches!(
        service.dispatch_worker(Work::Complete {
            task_id: task.id.clone(),
            lease: claim.lease.clone(),
            time: time.clone()
        }),
        Ok(CommandOutcomeV2::Complete(Ok(())))
    ));
    assert!(matches!(
        service.dispatch_worker(Work::Complete {
            task_id: task.id,
            lease: claim.lease,
            time
        }),
        Ok(CommandOutcomeV2::Complete(Err(_)))
    ));
}

#[test]
fn denial_is_identity_but_withheld_result_does_not_undo_execution_or_offer_replay() {
    let mut service = service();
    service.authorizer_mut().admit = false;
    assert_eq!(
        service.dispatch_worker(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH
        }),
        Err(WorkerRefusalV1::Unauthorized)
    );
    assert_eq!(service.worker_signals().executions, 0);
    service.authorizer_mut().admit = true;
    service.authorizer_mut().release = false;
    assert_eq!(
        service.dispatch_worker(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH
        }),
        Err(WorkerRefusalV1::ResultWithheld)
    );
    assert_eq!(service.worker_signals().executions, 1);
    assert_eq!(service.worker_signals().withheld, 1);
    service.authorizer_mut().release = true;
    let observed = time(&mut service);
    let CommandOutcomeV2::IssueGrant(Ok(grant)) = service
        .dispatch_worker(Work::IssueGrant {
            agent: Agent::new("private-worker"),
            time: observed.clone(),
            validity_duration: Duration::from_secs(60),
            claim_budget: None,
        })
        .unwrap()
    else {
        panic!("grant expected")
    };
    assert!(
        !format!(
            "{:?}",
            Work::ClaimTask {
                expected: StateRevision::ZERO,
                grant,
                task_id: TaskId::from("private-task"),
                time: observed,
                lease_duration: Duration::from_secs(1)
            }
        )
        .contains("private")
    );
}

#[test]
fn oversized_native_scope_refuses_before_execution_and_policy_does_not_fall_back_to_v1() {
    let mut service = service();
    let time = time(&mut service);
    let before = service.worker_signals().executions;
    assert_eq!(
        service.dispatch_worker(Work::IssueGrant {
            agent: Agent::new("x".repeat(257)),
            time,
            validity_duration: Duration::from_secs(1),
            claim_budget: None
        }),
        Err(WorkerRefusalV1::Limit)
    );
    assert_eq!(service.worker_signals().executions, before);
}

#[test]
fn worker_observations_are_passive_bounded_and_keep_admission_and_release_distinct() {
    let mut service = service();
    let before = service.worker_usage(SystemTime::UNIX_EPOCH);
    assert_eq!(before.engine.ready, 1);
    assert_eq!(before.engine.running, 0);
    assert!(before.resources.fits_within(before.resource_limit));
    service.authorizer_mut().admit = false;
    // No bounds/presence oracle before authorization, even on an oversized scope.
    let time = {
        service.authorizer_mut().admit = true;
        let time = time(&mut service);
        service.authorizer_mut().admit = false;
        time
    };
    assert_eq!(
        service.dispatch_worker(Work::IssueGrant {
            agent: Agent::new("x".repeat(257)),
            time,
            validity_duration: Duration::from_secs(1),
            claim_budget: None
        }),
        Err(WorkerRefusalV1::Unauthorized)
    );
    let signals = service.worker_signals();
    assert_eq!(signals.calls, 2);
    assert_eq!(signals.unauthorized, 1);
    assert_eq!(signals.limited, 0);
    assert_eq!(signals.residence_buckets.iter().sum::<u64>(), signals.calls);
    assert_eq!(signals.stages[0].samples, 2);
    assert_eq!(signals.stages[1].samples, 1);
    assert_eq!(
        signals.stages[2].samples, 0,
        "volatile execution fabricates no storage samples"
    );
    assert_eq!(signals.stages[3].samples, 1);
    let debug = format!(
        "{signals:?} {:?}",
        service.worker_usage(SystemTime::UNIX_EPOCH)
    );
    assert!(!debug.contains("private-task"));
    assert!(!debug.contains("private-worker"));
    assert_eq!(
        before.engine.ready,
        service.worker_usage(SystemTime::UNIX_EPOCH).engine.ready
    );
}

#[test]
fn grant_saturation_is_observable_and_revocation_releases_capacity() {
    let engine_limits = EngineLimits {
        max_active_grants: 1,
        ..EngineLimits::default()
    };
    let mut service = Service::new(
        Engine::with_limits(engine_limits),
        Policy {
            admit: true,
            release: true,
        },
        limits(),
    );
    let time = time(&mut service);
    let request = || Work::IssueGrant {
        agent: Agent::new("private-worker"),
        time: time.clone(),
        validity_duration: Duration::from_secs(60),
        claim_budget: None,
    };
    let CommandOutcomeV2::IssueGrant(Ok(grant)) = service.dispatch_worker(request()).unwrap()
    else {
        panic!("grant expected")
    };
    assert!(matches!(
        service.dispatch_worker(request()),
        Ok(CommandOutcomeV2::IssueGrant(Err(_)))
    ));
    let full = service.worker_usage(SystemTime::UNIX_EPOCH);
    assert_eq!(full.engine.active_grants, 1);
    assert_eq!(full.engine.active_grant_capacity, 1);
    assert_eq!(service.worker_signals().domain_refused, 1);
    service.dispatch_worker(Work::RevokeGrant(grant)).unwrap();
    assert_eq!(
        service
            .worker_usage(SystemTime::UNIX_EPOCH)
            .engine
            .active_grants,
        0
    );
    assert!(matches!(
        service.dispatch_worker(request()),
        Ok(CommandOutcomeV2::IssueGrant(Ok(_)))
    ));
}

// Receipt protocol model only: this test double is not physical durability.
#[derive(Clone)]
struct ReceiptStore {
    state:
        std::sync::Arc<std::sync::Mutex<(chronos_adapter_checkpoint::VolatileCheckpointStore, u8)>>,
}

#[test]
fn durable_denial_does_not_commit_and_commit_time_revocation_withholds_time_authority() {
    use chronos_host::CheckpointStore;
    use chronos_service::durable::DurableService;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct LivePolicy {
        admission: bool,
        revoked: Arc<AtomicBool>,
    }
    impl Authorizer for LivePolicy {
        fn allows(&mut self, _: &RequestV1) -> bool {
            true
        }
    }
    impl WorkerAuthorizer for LivePolicy {
        fn allows_worker(&mut self, _: &Work) -> bool {
            self.admission
        }
        fn allows_worker_result(&mut self, _: &Work, _: &CommandOutcomeV2) -> bool {
            !self.revoked.load(Ordering::Relaxed)
        }
    }
    struct RevokingStore {
        inner: ReceiptStore,
        revoked: Arc<AtomicBool>,
    }
    impl CheckpointStore for RevokingStore {
        fn load(&self) -> Result<Option<chronos_host::LoadedCheckpoint>, chronos_host::StoreError> {
            self.inner.load()
        }
        fn commit(
            &mut self,
            cp: &chronos_engine::Checkpoint,
        ) -> Result<chronos_engine::CommitReceipt, chronos_host::StoreError> {
            let receipt = self.inner.commit(cp)?;
            self.revoked.store(true, Ordering::Relaxed);
            Ok(receipt)
        }
    }
    let store = ReceiptStore::new();
    let revoked = Arc::new(AtomicBool::new(false));
    let mut service = DurableService::create_new(
        RevokingStore {
            inner: store.clone(),
            revoked: revoked.clone(),
        },
        LivePolicy {
            admission: false,
            revoked: revoked.clone(),
        },
        limits(),
        recovery(),
    )
    .unwrap();
    let before = store.load().unwrap().unwrap();
    assert_eq!(
        service
            .dispatch_worker(Work::AdvanceTime {
                now: SystemTime::UNIX_EPOCH
            })
            .unwrap(),
        Err(WorkerRefusalV1::Unauthorized)
    );
    assert_eq!(before, store.load().unwrap().unwrap());
    service.authorizer_mut().unwrap().admission = true;
    // Time observations are authority too: current release policy runs after
    // the store callback, never from a cached pre-commit authorization decision.
    let result = service
        .dispatch_worker(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH,
        })
        .unwrap();
    assert_eq!(result, Err(WorkerRefusalV1::ResultWithheld));
    assert_eq!(service.worker_signals().unwrap().withheld, 1);
    assert_ne!(before, store.load().unwrap().unwrap());
    assert!(!service.signals().fenced);
}
impl ReceiptStore {
    fn new() -> Self {
        Self {
            state: std::sync::Arc::new(std::sync::Mutex::new((
                chronos_adapter_checkpoint::VolatileCheckpointStore::new(1_000_000),
                0,
            ))),
        }
    }
    fn mode(&self, mode: u8) {
        self.state.lock().unwrap().1 = mode;
    }
}
impl chronos_host::CheckpointStore for ReceiptStore {
    fn load(&self) -> Result<Option<chronos_host::LoadedCheckpoint>, chronos_host::StoreError> {
        self.state.lock().unwrap().0.load()
    }
    fn commit(
        &mut self,
        checkpoint: &chronos_engine::Checkpoint,
    ) -> Result<chronos_engine::CommitReceipt, chronos_host::StoreError> {
        use chronos_engine::{AcknowledgementStrength, CommitReceipt, SnapshotRevision};
        use chronos_host::{IoStage, StoreError};
        let mut state = self.state.lock().unwrap();
        if state.1 == 1 {
            return Err(StoreError::SnapshotTooLarge);
        }
        let receipt = state.0.commit(checkpoint)?;
        if state.1 == 2 {
            return Err(StoreError::CommitIndeterminate {
                revision: receipt.revision,
                stage: IoStage::SyncDirectory,
                kind: std::io::ErrorKind::Other,
            });
        }
        Ok(CommitReceipt::new(
            if state.1 == 4 {
                SnapshotRevision::ZERO
            } else {
                receipt.revision
            },
            if state.1 == 3 {
                AcknowledgementStrength::Volatile
            } else {
                AcknowledgementStrength::Durable
            },
        ))
    }
}

#[test]
fn durable_claim_never_releases_authority_on_failed_or_weak_or_misattributed_commit() {
    use chronos_service::durable::{DurableError, DurableService};
    for mode in 0..=4 {
        let store = ReceiptStore::new();
        let mut service = DurableService::create_new(
            store.clone(),
            Policy {
                admit: true,
                release: true,
            },
            limits(),
            recovery(),
        )
        .unwrap();
        service
            .dispatch(RequestV1::SubmitGraph {
                expected: StateRevision::ZERO,
                tasks: vec![TaskSpecV2::new("private-task")],
            })
            .unwrap()
            .unwrap();
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
                agent: Agent::new("private-worker"),
                time: time.clone(),
                validity_duration: Duration::from_secs(60),
                claim_budget: None,
            })
            .unwrap()
            .unwrap()
        else {
            panic!("grant expected")
        };
        store.mode(mode);
        let result = service.dispatch_worker(Work::ClaimNext {
            grant: grant.clone(),
            time: time.clone(),
            lease_duration: Duration::from_secs(10),
        });
        if mode == 0 {
            assert!(matches!(
                result,
                Ok(Ok(CommandOutcomeV2::ClaimNext(Ok(Some(_)))))
            ));
        } else {
            assert!(result.is_err());
            assert!(service.signals().fenced);
            assert!(service.worker_signals().is_none());
            assert!(service.worker_usage(SystemTime::UNIX_EPOCH).is_none());
            assert!(matches!(
                service.dispatch(RequestV1::Revision),
                Err(DurableError::Fenced)
            ));
            assert!(matches!(
                service.dispatch_worker(Work::AdvanceTime {
                    now: SystemTime::UNIX_EPOCH
                }),
                Err(DurableError::Fenced)
            ));
        }
        drop(service);
        store.mode(0);
        let mut recovered = DurableService::open_existing(
            store,
            Policy {
                admit: true,
                release: true,
            },
            limits(),
            recovery(),
        )
        .unwrap();
        let Ok(ResponseV1::Snapshot(Some(snapshot))) = recovered
            .dispatch(RequestV1::Snapshot {
                task: TaskId::from("private-task"),
            })
            .unwrap()
        else {
            panic!("snapshot expected")
        };
        assert_eq!(
            snapshot.task.phase,
            if mode == 1 {
                chronos_engine::TaskPhase::Ready
            } else {
                chronos_engine::TaskPhase::Uncertain
            }
        );
        assert!(matches!(
            recovered.dispatch_worker(Work::ClaimNext {
                grant,
                time,
                lease_duration: Duration::from_secs(10)
            }),
            Ok(Ok(CommandOutcomeV2::ClaimNext(Err(_))))
        ));
    }
}
