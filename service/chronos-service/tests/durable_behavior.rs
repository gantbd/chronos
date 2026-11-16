use chronos_adapter_checkpoint::{DirectoryCheckpointStore, VolatileCheckpointStore};
use chronos_engine::{
    AcknowledgementStrength, Checkpoint, CommitReceipt, EngineLimits, RecoveryLimits,
    StateRevision, TaskId, TaskSpecV2,
};
use chronos_host::native::{RefusalV1, RequestV1, ResponseV1};
use chronos_host::{CheckpointStore, IoStage, LoadedCheckpoint, StoreError};
use chronos_service::{
    Authorizer, Limits,
    durable::{DurableError, DurableService},
};
use std::sync::{Arc, Mutex};

struct Policy(bool);
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        self.0
    }
}
impl chronos_service::coordination_v2::AuthorizerV2 for Policy {
    fn allows_v2(&mut self, _: &chronos_host::native_v2::RequestV2) -> bool {
        self.0
    }
}

#[test]
fn retirement_success_is_persistence_gated_and_lost_ack_is_recovered_without_replay() {
    use chronos_host::native_v2::{RequestV2, ResponseV2};
    for mode in [0, 1, 2, 3] {
        let mut store = Store::new(0);
        let mut service =
            DurableService::create_new(store.clone(), Policy(true), limits(), recovery()).unwrap();
        service.dispatch(submit()).unwrap().unwrap();
        let Ok(ResponseV2::Explain(Some(task))) = service
            .dispatch_v2(RequestV2::Explain {
                task: TaskId::from("a"),
            })
            .unwrap()
        else {
            panic!("task expected")
        };
        service
            .dispatch(RequestV1::Cancel {
                expected: task.snapshot.revision,
                task: task.snapshot.reference,
            })
            .unwrap()
            .unwrap();
        let Ok(ResponseV2::Explain(Some(task))) = service
            .dispatch_v2(RequestV2::Explain {
                task: TaskId::from("a"),
            })
            .unwrap()
        else {
            panic!("task expected")
        };
        drop(service);
        store.mode = mode;
        let mut service =
            DurableService::open_existing(store.clone(), Policy(true), limits(), recovery())
                .unwrap();
        let result = service.dispatch_v2(RequestV2::Retire {
            expected: task.snapshot.revision,
            task: task.snapshot.reference,
        });
        if mode == 0 {
            assert_eq!(result.unwrap(), Ok(ResponseV2::Retire(Ok(()))));
        } else {
            assert!(result.is_err());
            assert!(matches!(
                service.dispatch_v2(RequestV2::Explain {
                    task: TaskId::from("a")
                }),
                Err(DurableError::Fenced)
            ));
        }
        drop(service);
        store.mode = 0;
        let mut recovered =
            DurableService::open_existing(store, Policy(true), limits(), recovery()).unwrap();
        let Ok(ResponseV2::Explain(observed)) = recovered
            .dispatch_v2(RequestV2::Explain {
                task: TaskId::from("a"),
            })
            .unwrap()
        else {
            panic!("observation expected")
        };
        assert_eq!(
            observed.is_some(),
            mode == 1,
            "only prepublication refusal keeps the old task"
        );
    }
}

#[derive(Clone)]
struct Store {
    inner: Arc<Mutex<VolatileCheckpointStore>>,
    mode: u8,
}
impl Store {
    fn new(mode: u8) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VolatileCheckpointStore::new(1_000_000))),
            mode,
        }
    }
}
impl CheckpointStore for Store {
    fn load(&self) -> Result<Option<LoadedCheckpoint>, StoreError> {
        self.inner.lock().unwrap().load()
    }
    fn commit(&mut self, checkpoint: &Checkpoint) -> Result<CommitReceipt, StoreError> {
        if self.mode == 1 {
            return Err(StoreError::Io {
                stage: IoStage::WriteTemporary,
                kind: std::io::ErrorKind::Other,
            });
        }
        let receipt = self.inner.lock().unwrap().commit(checkpoint)?;
        if self.mode == 2 {
            return Err(StoreError::CommitIndeterminate {
                revision: receipt.revision,
                stage: IoStage::SyncDirectory,
                kind: std::io::ErrorKind::Other,
            });
        }
        Ok(CommitReceipt::new(
            receipt.revision,
            if self.mode == 3 {
                AcknowledgementStrength::Volatile
            } else {
                AcknowledgementStrength::Durable
            },
        ))
    }
}

impl chronos_host::CheckpointMaintenance for Store {
    fn maintain(&mut self) -> Result<chronos_host::MaintenanceSnapshot, StoreError> {
        if self.mode == 4 {
            return Err(StoreError::DirectoryEntryLimit);
        }
        Ok(chronos_host::MaintenanceSnapshot::default())
    }
}

#[test]
fn operator_maintenance_is_observed_separately_and_failure_fences_without_task_changes() {
    for mode in [0, 4] {
        let store = Store::new(mode);
        let mut service =
            DurableService::create_new(store.clone(), Policy(true), limits(), recovery()).unwrap();
        let before = store.load().unwrap().unwrap();
        let result = service.maintain();
        assert_eq!(store.load().unwrap().unwrap().bytes(), before.bytes());
        assert_eq!(service.signals().commits, 1);
        assert_eq!(service.signals().maintenance_runs, 1);
        if mode == 4 {
            assert!(result.is_err());
            assert_eq!(service.signals().maintenance_failures, 1);
            assert!(matches!(
                service.dispatch(RequestV1::Revision),
                Err(DurableError::Fenced)
            ));
        } else {
            assert_eq!(
                result.unwrap(),
                chronos_host::MaintenanceSnapshot::default()
            );
            assert_eq!(
                service.dispatch(RequestV1::Revision).unwrap(),
                Ok(ResponseV1::Revision(StateRevision::ZERO))
            );
        }
    }
}

fn limits() -> Limits {
    Limits {
        max_graph_tasks: 4,
        max_page_entries: 4,
    }
}
fn recovery() -> RecoveryLimits {
    RecoveryLimits::new(1_000_000, EngineLimits::default())
}
fn submit() -> RequestV1 {
    RequestV1::SubmitGraph {
        expected: StateRevision::ZERO,
        tasks: vec![TaskSpecV2::new("a")],
    }
}

#[test]
fn successful_commit_is_acknowledged_and_restart_recovers_state() {
    let store = Store::new(0);
    let mut service =
        DurableService::open(store.clone(), Policy(true), limits(), recovery()).unwrap();
    assert_eq!(
        service.dispatch(submit()).unwrap(),
        Ok(ResponseV1::SubmitGraph(Ok(())))
    );
    assert_eq!(service.signals().commits, 1);
    let mut restarted = DurableService::open(store, Policy(true), limits(), recovery()).unwrap();
    assert_eq!(
        restarted.dispatch(RequestV1::Revision).unwrap(),
        Ok(ResponseV1::Revision(StateRevision::Finite(1)))
    );
}

#[test]
fn prepublication_failure_fences_even_reads_and_never_returns_success() {
    let mut service =
        DurableService::open(Store::new(1), Policy(true), limits(), recovery()).unwrap();
    assert!(matches!(
        service.dispatch(submit()),
        Err(DurableError::Store(_))
    ));
    assert!(matches!(
        service.dispatch(RequestV1::Revision),
        Err(DurableError::Fenced)
    ));
    assert!(service.signals().fenced);
    assert_eq!(service.signals().failures, 1);
}

#[test]
fn lost_commit_ack_fences_and_recovery_observes_the_actual_commit() {
    let store = Store::new(2);
    let mut service =
        DurableService::open(store.clone(), Policy(true), limits(), recovery()).unwrap();
    assert!(matches!(
        service.dispatch(submit()),
        Err(DurableError::Store(StoreError::CommitIndeterminate { .. }))
    ));
    assert!(matches!(
        service.dispatch(submit()),
        Err(DurableError::Fenced)
    ));
    let mut restarted = DurableService::open(store, Policy(true), limits(), recovery()).unwrap();
    assert!(matches!(
        restarted
            .dispatch(RequestV1::Snapshot {
                task: TaskId::from("a")
            })
            .unwrap(),
        Ok(ResponseV1::Snapshot(Some(_)))
    ));
}

#[test]
fn weak_receipt_does_not_pass_for_durable_success() {
    let mut service =
        DurableService::open(Store::new(3), Policy(true), limits(), recovery()).unwrap();
    assert!(matches!(
        service.dispatch(submit()),
        Err(DurableError::Acknowledgement(_))
    ));
    assert!(service.signals().fenced);
}

#[test]
fn unauthorized_and_stale_commands_do_not_create_checkpoints() {
    let store = Store::new(0);
    let mut service =
        DurableService::open(store.clone(), Policy(false), limits(), recovery()).unwrap();
    assert_eq!(
        service.dispatch(submit()).unwrap(),
        Err(RefusalV1::Unauthorized)
    );
    assert!(store.load().unwrap().is_none());
    service.authorizer_mut().unwrap().0 = true;
    service.dispatch(submit()).unwrap().unwrap();
    service.dispatch(submit()).unwrap().unwrap();
    assert_eq!(service.signals().commits, 1);
}

#[test]
fn checkpoint_encoding_failure_fences_without_publishing_mutation() {
    let store = Store::new(0);
    let mut service = DurableService::open(
        store.clone(),
        Policy(true),
        limits(),
        RecoveryLimits::new(1, EngineLimits::default()),
    )
    .unwrap();
    assert!(matches!(
        service.dispatch(submit()),
        Err(DurableError::Checkpoint(_))
    ));
    assert!(store.load().unwrap().is_none());
    assert!(service.signals().fenced);
}

#[test]
fn real_directory_store_survives_drop_and_reopen() {
    let path = std::env::temp_dir().join(format!("chronos-native-service-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let store = DirectoryCheckpointStore::open(&path, 1_000_000, 32).unwrap();
    let mut service = DurableService::open(store, Policy(true), limits(), recovery()).unwrap();
    service.dispatch(submit()).unwrap().unwrap();
    drop(service);
    let store = DirectoryCheckpointStore::open(&path, 1_000_000, 32).unwrap();
    let mut restarted = DurableService::open(store, Policy(true), limits(), recovery()).unwrap();
    assert_eq!(
        restarted.dispatch(RequestV1::Revision).unwrap(),
        Ok(ResponseV1::Revision(StateRevision::Finite(1)))
    );
    drop(restarted);
    for entry in std::fs::read_dir(&path).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn explicit_creation_persists_empty_state_and_recovery_never_initializes_absence() {
    let store = Store::new(0);
    assert!(matches!(
        DurableService::open_existing(store.clone(), Policy(true), limits(), recovery()),
        Err(DurableError::MissingState)
    ));
    let mut created =
        DurableService::create_new(store.clone(), Policy(true), limits(), recovery()).unwrap();
    assert!(store.load().unwrap().is_some());
    assert_eq!(created.signals().commits, 1);
    assert_eq!(
        created.dispatch(RequestV1::Revision).unwrap(),
        Ok(ResponseV1::Revision(StateRevision::ZERO))
    );
    assert!(matches!(
        DurableService::create_new(store.clone(), Policy(true), limits(), recovery()),
        Err(DurableError::AlreadyInitialized)
    ));
    let mut recovered =
        DurableService::open_existing(store, Policy(true), limits(), recovery()).unwrap();
    assert_eq!(
        recovered.dispatch(RequestV1::Revision).unwrap(),
        Ok(ResponseV1::Revision(StateRevision::ZERO))
    );
}

#[test]
fn failed_initial_commit_never_returns_a_live_service() {
    for mode in [1, 2, 3] {
        let store = Store::new(mode);
        assert!(
            DurableService::create_new(store.clone(), Policy(true), limits(), recovery()).is_err()
        );
        if mode == 1 {
            assert!(store.load().unwrap().is_none());
        }
        if mode == 2 {
            let mut recovered =
                DurableService::open_existing(store, Policy(true), limits(), recovery()).unwrap();
            assert_eq!(
                recovered.dispatch(RequestV1::Revision).unwrap(),
                Ok(ResponseV1::Revision(StateRevision::ZERO))
            );
        }
    }
}
