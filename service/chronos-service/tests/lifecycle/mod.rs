//! Same worker contract exercised through all three service interpretations.
use super::*;
use chronos_adapter_checkpoint::{DirectoryCheckpointStore, DirectoryServiceEnvelopeStore};
use chronos_engine::{
    CapabilityGrant, CompletionError, ConditionalError, EffectKnowledge, EngineTime,
    FailureClassV1, FailureDisposition, LeaseToken, RevisionedTaskSnapshotV1, Task, TaskPhase,
    TemporalError,
};
use chronos_host::{ClientV1, worker::WorkerClientV1};
use chronos_service::{
    atomic::{AtomicLimits, AtomicService, InvocationAuthorizer},
    coordination_v2::AuthorizerV2,
    durable::DurableService,
};

impl AuthorizerV2 for Policy {
    fn allows_v2(&mut self, _: &chronos_host::native_v2::RequestV2) -> bool {
        self.admit
    }
}
impl InvocationAuthorizer for Policy {
    fn allows_invocation(&mut self, _: &chronos_host::native_v3::InvocationRequestV1) -> bool {
        self.admit
    }
    fn allows_retained_result(
        &mut self,
        _: &chronos_host::native_v3::InvocationV1,
        _: &chronos_host::native_v3::MutationV1,
    ) -> bool {
        self.release
    }
}
fn policy() -> Policy {
    Policy {
        admit: true,
        release: true,
    }
}
fn atomic_limits() -> AtomicLimits {
    AtomicLimits {
        max_envelope_bytes: 1_100_000,
        max_retained_bytes: 4096,
        max_command_bytes: 2048,
        max_key_bytes: 64,
        max_invocations: 8,
    }
}

struct Harness<C>(C);
impl<C: ClientV1 + WorkerClientV1> Harness<C>
where
    <C as ClientV1>::TransportError: std::fmt::Debug,
    <C as WorkerClientV1>::TransportError: std::fmt::Debug,
{
    fn work(&mut self, request: Work) -> CommandOutcomeV2 {
        self.0.call_worker(request).unwrap().unwrap()
    }
    fn read(&mut self, request: RequestV1) -> ResponseV1 {
        self.0.call(request).unwrap().unwrap()
    }
    fn revision(&mut self) -> StateRevision {
        let ResponseV1::Revision(revision) = self.read(RequestV1::Revision) else {
            panic!("revision expected")
        };
        revision
    }
    fn snapshot(&mut self, id: &str) -> RevisionedTaskSnapshotV1 {
        let ResponseV1::Snapshot(Some(snapshot)) =
            self.read(RequestV1::Snapshot { task: id.into() })
        else {
            panic!("snapshot expected")
        };
        snapshot
    }
    fn submit(&mut self, id: &str) {
        let expected = self.revision();
        assert!(matches!(
            self.read(RequestV1::SubmitGraph {
                expected,
                tasks: vec![TaskSpecV2::new(id)]
            }),
            ResponseV1::SubmitGraph(Ok(()))
        ));
    }
    fn advance(&mut self, seconds: u64) -> EngineTime {
        let CommandOutcomeV2::AdvanceTime(Ok(time)) = self.work(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
        }) else {
            panic!("time expected")
        };
        time
    }
    fn grant(&mut self, time: &EngineTime, duration: u64) -> CapabilityGrant {
        let CommandOutcomeV2::IssueGrant(Ok(grant)) = self.work(Work::IssueGrant {
            agent: Agent::new("trusted-worker"),
            time: time.clone(),
            validity_duration: Duration::from_secs(duration),
            claim_budget: None,
        }) else {
            panic!("grant expected")
        };
        grant
    }
    fn claim(
        &mut self,
        id: &str,
        grant: &CapabilityGrant,
        time: &EngineTime,
    ) -> (Task, LeaseToken) {
        let expected = self.revision();
        let CommandOutcomeV2::ClaimTask(Ok(task)) = self.work(Work::ClaimTask {
            expected,
            grant: grant.clone(),
            task_id: id.into(),
            time: time.clone(),
            lease_duration: Duration::from_secs(10),
        }) else {
            panic!("claim expected")
        };
        let TaskState::Running(ref claim) = task.state else {
            panic!("running expected")
        };
        let lease = claim.lease.clone();
        (task, lease)
    }
    fn resolve(
        &mut self,
        id: &str,
        knowledge: EffectKnowledge,
        disposition: FailureDisposition,
    ) -> CommandOutcomeV2 {
        let snapshot = self.snapshot(id);
        self.work(Work::ResolveUncertain {
            expected: snapshot.revision,
            task: snapshot.reference,
            knowledge,
            failure_class: FailureClassV1::Execution,
            failure_disposition: disposition,
        })
    }
}

fn qualify<C: ClientV1 + WorkerClientV1>(client: C)
where
    <C as ClientV1>::TransportError: std::fmt::Debug,
    <C as WorkerClientV1>::TransportError: std::fmt::Debug,
{
    let mut h = Harness(client);
    h.submit("renew");
    let original_time = h.advance(0);
    let grant = h.grant(&original_time, 60);
    let spare = h.grant(&original_time, 60);
    // Guarded targeting refuses the stale revision before selecting work.
    assert!(matches!(
        h.work(Work::ClaimTask {
            expected: StateRevision::ZERO,
            grant: grant.clone(),
            task_id: "renew".into(),
            time: original_time.clone(),
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::ClaimTask(Err(TemporalError::Domain(
            ConditionalError::RevisionMismatch { .. }
        )))
    ));
    let (_, lease) = h.claim("renew", &grant, &original_time);
    assert!(matches!(
        h.work(Work::RevokeGrant(grant.clone())),
        CommandOutcomeV2::RevokeGrant(Ok(()))
    ));
    assert!(matches!(
        h.work(Work::ClaimNext {
            grant,
            time: original_time.clone(),
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::ClaimNext(Err(_))
    ));
    let at5 = h.advance(5);
    assert!(matches!(
        h.work(Work::Heartbeat {
            task_id: "renew".into(),
            lease: lease.clone(),
            time: at5.clone(),
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::Heartbeat(Ok(()))
    ));
    assert_eq!(h.snapshot("renew").task.phase, TaskPhase::Running);
    assert!(matches!(
        h.work(Work::RevokeAllGrants),
        CommandOutcomeV2::RevokeAllGrants(Ok(()))
    ));
    assert!(matches!(
        h.work(Work::ClaimNext {
            grant: spare,
            time: at5.clone(),
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::ClaimNext(Err(_))
    ));
    let at14 = h.advance(14);
    assert!(matches!(
        h.work(Work::Complete {
            task_id: "renew".into(),
            lease: lease.clone(),
            time: at5
        }),
        CommandOutcomeV2::Complete(Err(TemporalError::Time(_)))
    ));
    // Revocation blocks claims, not a previously issued live lease. Renewal
    // extended the original expiry from 10 to 15; it never implied completion.
    assert!(matches!(
        h.work(Work::Complete {
            task_id: "renew".into(),
            lease,
            time: at14.clone()
        }),
        CommandOutcomeV2::Complete(Ok(()))
    ));
    h.submit("expiry");
    let grant = h.grant(&at14, 60);
    let (_, expired_lease) = h.claim("expiry", &grant, &at14);
    let at24 = h.advance(24);
    assert!(matches!(
        h.work(Work::Complete {
            task_id: "expiry".into(),
            lease: expired_lease.clone(),
            time: at24.clone()
        }),
        CommandOutcomeV2::Complete(Err(TemporalError::Domain(CompletionError::LeaseExpired(_))))
    ));
    assert!(matches!(
        h.work(Work::Heartbeat {
            task_id: "expiry".into(),
            lease: expired_lease,
            time: at24.clone(),
            lease_duration: Duration::from_secs(1)
        }),
        CommandOutcomeV2::Heartbeat(Err(_))
    ));
    assert!(matches!(
        h.work(Work::ReleaseExpired { time: at24.clone() }),
        CommandOutcomeV2::ReleaseExpired(Ok(1))
    ));
    let fenced = h.snapshot("expiry");
    assert_eq!(fenced.task.phase, TaskPhase::Uncertain);
    for knowledge in [EffectKnowledge::Unknown, EffectKnowledge::Conflict] {
        assert!(matches!(
            h.resolve("expiry", knowledge, FailureDisposition::Retry),
            CommandOutcomeV2::ResolveUncertain(Err(_))
        ));
        assert_eq!(h.snapshot("expiry"), fenced);
    }
    assert!(matches!(
        h.work(Work::ClaimNext {
            grant: grant.clone(),
            time: at24.clone(),
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::ClaimNext(Ok(None))
    ));
    assert!(matches!(
        h.resolve(
            "expiry",
            EffectKnowledge::Completed,
            FailureDisposition::Terminal
        ),
        CommandOutcomeV2::ResolveUncertain(Ok(()))
    ));
    assert!(matches!(
        h.resolve("expiry", EffectKnowledge::Failed, FailureDisposition::Retry),
        CommandOutcomeV2::ResolveUncertain(Err(_))
    ));
    h.submit("retry");
    let (first, first_lease) = h.claim("retry", &grant, &at24);
    assert!(matches!(
        h.work(Work::Fail {
            task_id: "retry".into(),
            lease: first_lease.clone(),
            failure_class: FailureClassV1::Execution,
            disposition: FailureDisposition::Retry,
            time: at24.clone()
        }),
        CommandOutcomeV2::Fail(Ok(()))
    ));
    let (second, second_lease) = h.claim("retry", &grant, &at24);
    assert_ne!(first.attempt_reference(), second.attempt_reference());
    assert!(matches!(
        h.work(Work::Complete {
            task_id: "retry".into(),
            lease: first_lease,
            time: at24.clone()
        }),
        CommandOutcomeV2::Complete(Err(TemporalError::Domain(CompletionError::WrongLease(_))))
    ));
    assert!(matches!(
        h.work(Work::FenceUncertain {
            task_id: "retry".into(),
            lease: second_lease,
            time: at24.clone()
        }),
        CommandOutcomeV2::FenceUncertain(Ok(()))
    ));
    assert_eq!(h.snapshot("retry").task.phase, TaskPhase::Uncertain);
    assert!(matches!(
        h.resolve("retry", EffectKnowledge::Failed, FailureDisposition::Retry),
        CommandOutcomeV2::ResolveUncertain(Ok(()))
    ));
    let (_, final_lease) = h.claim("retry", &grant, &at24);
    assert!(matches!(
        h.work(Work::Fail {
            task_id: "retry".into(),
            lease: final_lease,
            failure_class: FailureClassV1::Execution,
            disposition: FailureDisposition::Retry,
            time: at24.clone()
        }),
        CommandOutcomeV2::Fail(Ok(()))
    ));
    assert_eq!(
        h.snapshot("retry").task.phase,
        TaskPhase::Failed,
        "attempt cap stops retry"
    );
    h.submit("cancel");
    let (_, cancelled_lease) = h.claim("cancel", &grant, &at24);
    let snapshot = h.snapshot("cancel");
    assert!(matches!(
        h.read(RequestV1::Cancel {
            expected: snapshot.revision,
            task: snapshot.reference
        }),
        ResponseV1::Cancel(Ok(()))
    ));
    assert!(matches!(
        h.work(Work::Complete {
            task_id: "cancel".into(),
            lease: cancelled_lease,
            time: at24.clone()
        }),
        CommandOutcomeV2::Complete(Err(_))
    ));
    assert_eq!(h.snapshot("cancel").task.phase, TaskPhase::Cancelled);
    let expiring = h.grant(&at24, 1);
    let at25 = h.advance(25);
    assert!(matches!(
        h.work(Work::ReleaseExpiredGrants { time: at25.clone() }),
        CommandOutcomeV2::ReleaseExpiredGrants(Ok(1))
    ));
    assert!(matches!(
        h.work(Work::ClaimNext {
            grant: expiring,
            time: at25,
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::ClaimNext(Err(_))
    ));
}

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "chronos-workers-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        for entry in std::fs::read_dir(&self.0).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}

#[test]
fn volatile_worker_lifecycle_preserves_existing_semantics() {
    qualify(Service::new(Engine::new(), policy(), limits()));
}
#[test]
fn durable_directory_worker_lifecycle_preserves_existing_semantics() {
    let directory = Directory::new();
    qualify(
        DurableService::create_new(
            DirectoryCheckpointStore::open(&directory.0, 1_000_000, 128).unwrap(),
            policy(),
            limits(),
            recovery(),
        )
        .unwrap(),
    );
}
#[test]
fn atomic_directory_worker_lifecycle_preserves_existing_semantics() {
    let directory = Directory::new();
    qualify(
        AtomicService::create_new(
            DirectoryServiceEnvelopeStore::open(&directory.0, 1_100_000, 128).unwrap(),
            policy(),
            limits(),
            recovery(),
            atomic_limits(),
        )
        .unwrap(),
    );
}

#[test]
fn recovery_invalidates_worker_authority_and_lost_completion_does_not_repeat_the_effect() {
    let directory = Directory::new();
    let open = || DirectoryCheckpointStore::open(&directory.0, 1_000_000, 128).unwrap();
    let mut h =
        Harness(DurableService::create_new(open(), policy(), limits(), recovery()).unwrap());
    h.submit("effect");
    let time = h.advance(0);
    let grant = h.grant(&time, 60);
    let (claimed, lease) = h.claim("effect", &grant, &time);
    let attempt = claimed.attempt_reference().unwrap();
    // The synthetic executor receives only this authority-free correlation.
    let effect_count = std::sync::atomic::AtomicUsize::new(0);
    let execute = |_: &chronos_engine::TaskAttemptRefV1| {
        effect_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    };
    execute(&attempt);
    drop(h); // committed claim, missing completion report
    let mut h =
        Harness(DurableService::open_existing(open(), policy(), limits(), recovery()).unwrap());
    assert_eq!(h.snapshot("effect").task.phase, TaskPhase::Uncertain);
    assert_eq!(
        h.snapshot("effect").task.attempt_number,
        Some(attempt.attempt_number())
    );
    let time = h.advance(0);
    assert!(matches!(
        h.work(Work::Complete {
            task_id: "effect".into(),
            lease,
            time: time.clone()
        }),
        CommandOutcomeV2::Complete(Err(_))
    ));
    assert!(matches!(
        h.work(Work::ClaimNext {
            grant,
            time: time.clone(),
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::ClaimNext(Err(TemporalError::Domain(
            chronos_engine::ClaimError::ForeignGrant
        )))
    ));
    let grant = h.grant(&time, 60);
    assert!(matches!(
        h.work(Work::ClaimNext {
            grant,
            time,
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::ClaimNext(Ok(None))
    ));
    // Qualified external receipt, not timeout, permits reconciliation. Discard
    // its reply to model delivery loss after commit, then recover and inspect.
    let _lost = h.resolve(
        "effect",
        EffectKnowledge::Completed,
        FailureDisposition::Terminal,
    );
    drop(h);
    let mut h =
        Harness(DurableService::open_existing(open(), policy(), limits(), recovery()).unwrap());
    assert_eq!(h.snapshot("effect").task.phase, TaskPhase::Succeeded);
    assert!(matches!(
        h.resolve(
            "effect",
            EffectKnowledge::Completed,
            FailureDisposition::Terminal
        ),
        CommandOutcomeV2::ResolveUncertain(Err(_))
    ));
    assert_eq!(effect_count.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn concurrent_durable_workers_share_one_owner_and_exactly_one_claim() {
    use std::sync::{Arc, Barrier, Mutex};
    let directory = Directory::new();
    let mut h = Harness(
        DurableService::create_new(
            DirectoryCheckpointStore::open(&directory.0, 1_000_000, 128).unwrap(),
            policy(),
            limits(),
            recovery(),
        )
        .unwrap(),
    );
    h.submit("contended");
    let time = h.advance(0);
    let grants = [h.grant(&time, 60), h.grant(&time, 60)];
    let shared = Arc::new(Mutex::new(h));
    let barrier = Barrier::new(2);
    let claims = std::thread::scope(|scope| {
        let handles: Vec<_> = grants
            .into_iter()
            .map(|grant| {
                let shared = &shared;
                let barrier = &barrier;
                let time = time.clone();
                scope.spawn(move || {
                    barrier.wait();
                    shared.lock().unwrap().work(Work::ClaimNext {
                        grant,
                        time,
                        lease_duration: Duration::from_secs(10),
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        claims
            .iter()
            .filter(|claim| matches!(claim, CommandOutcomeV2::ClaimNext(Ok(Some(_)))))
            .count(),
        1
    );
    assert_eq!(
        claims
            .iter()
            .filter(|claim| matches!(claim, CommandOutcomeV2::ClaimNext(Ok(None))))
            .count(),
        1
    );
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .snapshot("contended")
            .task
            .attempt_number,
        Some(1)
    );
}

#[test]
fn withheld_claim_expires_to_uncertainty_instead_of_redispatching() {
    let directory = Directory::new();
    let mut h = Harness(
        DurableService::create_new(
            DirectoryCheckpointStore::open(&directory.0, 1_000_000, 128).unwrap(),
            policy(),
            limits(),
            recovery(),
        )
        .unwrap(),
    );
    h.submit("withheld");
    let time = h.advance(0);
    let grant = h.grant(&time, 60);
    h.0.authorizer_mut().unwrap().release = false;
    assert_eq!(
        h.0.call_worker(Work::ClaimNext {
            grant: grant.clone(),
            time,
            lease_duration: Duration::from_secs(10)
        })
        .unwrap(),
        Err(WorkerRefusalV1::ResultWithheld)
    );
    assert_eq!(h.snapshot("withheld").task.phase, TaskPhase::Running);
    h.0.authorizer_mut().unwrap().release = true;
    let time = h.advance(10);
    assert!(matches!(
        h.work(Work::ReleaseExpired { time: time.clone() }),
        CommandOutcomeV2::ReleaseExpired(Ok(1))
    ));
    assert_eq!(h.snapshot("withheld").task.phase, TaskPhase::Uncertain);
    assert!(matches!(
        h.work(Work::ClaimNext {
            grant,
            time,
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::ClaimNext(Ok(None))
    ));
    assert_eq!(h.snapshot("withheld").task.attempt_number, Some(1));
}

fn exit_after_synthetic_effect<C: ClientV1 + WorkerClientV1>(
    client: C,
    directory: &std::path::Path,
) -> !
where
    <C as ClientV1>::TransportError: std::fmt::Debug,
    <C as WorkerClientV1>::TransportError: std::fmt::Debug,
{
    use std::io::Write;
    let mut h = Harness(client);
    h.submit("process-effect");
    let time = h.advance(0);
    let grant = h.grant(&time, 60);
    let (task, _lease) = h.claim("process-effect", &grant, &time);
    let execute = |_: &chronos_engine::TaskAttemptRefV1| {
        let mut file = std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-effect"))
            .unwrap();
        file.write_all(b"1").unwrap();
        file.sync_all().unwrap();
    };
    execute(&task.attempt_reference().unwrap());
    // Abrupt process exit bypasses Drop and every completion/report path.
    // It is not a machine-power-loss simulation.
    std::process::exit(73)
}

#[test]
fn worker_process_exit_probe() {
    let Some(mode) = std::env::var_os("CHRONOS_WORKER_EXIT_PROFILE") else {
        return;
    };
    let path = std::path::PathBuf::from(
        std::env::var_os("CHRONOS_WORKER_EXIT_DIRECTORY").expect("private fixture directory"),
    );
    match mode.to_str().unwrap() {
        "checkpoint" => exit_after_synthetic_effect(
            DurableService::create_new(
                DirectoryCheckpointStore::open(&path, 1_000_000, 128).unwrap(),
                policy(),
                limits(),
                recovery(),
            )
            .unwrap(),
            &path,
        ),
        "envelope" => exit_after_synthetic_effect(
            AtomicService::create_new(
                DirectoryServiceEnvelopeStore::open(&path, 1_100_000, 128).unwrap(),
                policy(),
                limits(),
                recovery(),
                atomic_limits(),
            )
            .unwrap(),
            &path,
        ),
        _ => panic!("unknown fixture profile"),
    }
}

fn inspect_process_recovery<C: ClientV1 + WorkerClientV1>(client: C)
where
    <C as ClientV1>::TransportError: std::fmt::Debug,
    <C as WorkerClientV1>::TransportError: std::fmt::Debug,
{
    let mut h = Harness(client);
    assert_eq!(
        h.snapshot("process-effect").task.phase,
        TaskPhase::Uncertain
    );
    assert_eq!(h.snapshot("process-effect").task.attempt_number, Some(1));
    let time = h.advance(0);
    let grant = h.grant(&time, 60);
    assert!(matches!(
        h.work(Work::ClaimNext {
            grant,
            time,
            lease_duration: Duration::from_secs(10)
        }),
        CommandOutcomeV2::ClaimNext(Ok(None))
    ));
    assert!(matches!(
        h.resolve(
            "process-effect",
            EffectKnowledge::Completed,
            FailureDisposition::Terminal
        ),
        CommandOutcomeV2::ResolveUncertain(Ok(()))
    ));
    assert_eq!(
        h.snapshot("process-effect").task.phase,
        TaskPhase::Succeeded
    );
}

#[test]
fn abrupt_worker_process_loss_recovers_without_redispatch_in_both_stores() {
    for profile in ["checkpoint", "envelope"] {
        let directory = Directory::new();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lifecycle::worker_process_exit_probe",
                "--nocapture",
            ])
            .env("CHRONOS_WORKER_EXIT_PROFILE", profile)
            .env("CHRONOS_WORKER_EXIT_DIRECTORY", &directory.0)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(73),
            "fixture child did not reach committed claim/effect"
        );
        assert_eq!(
            std::fs::read(directory.0.join("synthetic-effect")).unwrap(),
            b"1"
        );
        match profile {
            "checkpoint" => inspect_process_recovery(
                DurableService::open_existing(
                    DirectoryCheckpointStore::open(&directory.0, 1_000_000, 128).unwrap(),
                    policy(),
                    limits(),
                    recovery(),
                )
                .unwrap(),
            ),
            "envelope" => inspect_process_recovery(
                AtomicService::open_existing(
                    DirectoryServiceEnvelopeStore::open(&directory.0, 1_100_000, 128).unwrap(),
                    policy(),
                    limits(),
                    recovery(),
                    atomic_limits(),
                )
                .unwrap(),
            ),
            _ => unreachable!(),
        }
        assert_eq!(
            std::fs::read(directory.0.join("synthetic-effect")).unwrap(),
            b"1"
        );
    }
}
