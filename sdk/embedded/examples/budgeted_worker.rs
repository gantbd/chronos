//! Private synchronous fixture: no real executor/provider, agent credentials or
//! remote worker transport. Host quiescence evidence is re-established outside
//! the engine; the service never derives it from a completion or timeout.
use chronos_adapter_checkpoint::DirectoryServiceEnvelopeStore;
use chronos_sdk_embedded::{
    engine::{
        Agent, CommandOutcomeV2 as O, EngineLimits, RecoveryLimits, SchedulingCommandV1 as C,
        StateRevision, TaskAttemptRefV1, TaskSpecV2, TaskState,
    },
    host::{
        native::{RequestV1, ResponseV1},
        native_v2::RequestV2,
        native_v3::*,
        scheduling::{SchedulingRequestV1 as Q, SchedulingResponseV1 as R},
        worker::WorkerRequestV1 as W,
    },
    service::{
        Authorizer, Limits,
        atomic::{AtomicLimits, AtomicService, InvocationAuthorizer},
        coordination_v2::AuthorizerV2,
        scheduling::SchedulingAuthorizerV1,
        worker::WorkerAuthorizer,
    },
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
type Stopped = Arc<Mutex<BTreeSet<TaskAttemptRefV1>>>;
struct Policy(Stopped);
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl AuthorizerV2 for Policy {
    fn allows_v2(&mut self, _: &RequestV2) -> bool {
        false
    }
}
impl InvocationAuthorizer for Policy {
    fn allows_invocation(&mut self, _: &InvocationRequestV1) -> bool {
        false
    }
    fn allows_retained_result(&mut self, _: &InvocationV1, _: &MutationV1) -> bool {
        false
    }
}
impl WorkerAuthorizer for Policy {
    fn allows_worker(&mut self, _: &W) -> bool {
        true
    }
    fn allows_worker_result(&mut self, _: &W, _: &O) -> bool {
        true
    }
}
impl SchedulingAuthorizerV1 for Policy {
    fn allows_scheduling(&mut self, request: &Q) -> bool {
        match request {
            Q::Control {
                command: C::ConfirmQuiescent { attempt },
                ..
            } => self.0.lock().unwrap().contains(attempt),
            _ => true,
        }
    }
    fn allows_scheduling_result(&mut self, _: &Q, _: &R) -> bool {
        true
    }
}
type Owner = AtomicService<Policy, DirectoryServiceEnvelopeStore>;
fn open(path: &std::path::Path, stopped: Stopped, recover: bool) -> Owner {
    let store = DirectoryServiceEnvelopeStore::open(path, 1_000_000, 16).unwrap();
    let limits = Limits {
        max_graph_tasks: 4,
        max_page_entries: 4,
    };
    let recovery = RecoveryLimits::new(900_000, EngineLimits::default());
    let atomic = AtomicLimits {
        max_envelope_bytes: 1_000_000,
        max_retained_bytes: 8192,
        max_command_bytes: 2048,
        max_key_bytes: 64,
        max_invocations: 4,
    };
    if recover {
        AtomicService::open_existing(store, Policy(stopped), limits, recovery, atomic).unwrap()
    } else {
        AtomicService::create_new(store, Policy(stopped), limits, recovery, atomic).unwrap()
    }
}
fn revision(owner: &mut Owner) -> StateRevision {
    let Ok(ResponseV1::Revision(value)) = owner.dispatch(RequestV1::Revision).unwrap() else {
        panic!("revision")
    };
    value
}
fn control(owner: &mut Owner, command: C) {
    let expected = revision(owner);
    assert!(matches!(
        owner
            .dispatch_scheduling(Q::Control { expected, command })
            .unwrap(),
        Ok(R::Outcome(Ok(())))
    ));
}
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "chronos-budgeted-worker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn run() {
    let directory = Directory::new();
    let stopped = Stopped::default();
    let mut owner = open(&directory.0, stopped.clone(), false);
    control(&mut owner, C::Enable);
    control(&mut owner, C::SetConcurrencyLimit { limit: 1 });
    let expected = revision(&mut owner);
    owner
        .dispatch(RequestV1::SubmitGraph {
            expected,
            tasks: vec![TaskSpecV2::new("fixture-a"), TaskSpecV2::new("fixture-b")],
        })
        .unwrap()
        .unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let O::AdvanceTime(Ok(time)) = owner
        .dispatch_worker(W::AdvanceTime { now })
        .unwrap()
        .unwrap()
    else {
        panic!("time")
    };
    let O::IssueGrant(Ok(grant)) = owner
        .dispatch_worker(W::IssueGrant {
            agent: Agent::new("fixture-worker"),
            time: time.clone(),
            validity_duration: Duration::from_secs(60),
            claim_budget: Some(2),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("grant")
    };
    let O::ClaimNext(Ok(Some(task))) = owner
        .dispatch_worker(W::ClaimNext {
            grant: grant.clone(),
            time: time.clone(),
            lease_duration: Duration::from_secs(30),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("claim")
    };
    let attempt = task.attempt_reference().unwrap();
    assert!(matches!(
        owner
            .dispatch_worker(W::ClaimNext {
                grant,
                time: time.clone(),
                lease_duration: Duration::from_secs(30)
            })
            .unwrap()
            .unwrap(),
        O::ClaimNext(Err(_))
    ));
    control(&mut owner, C::SetPaused { paused: true });
    let TaskState::Running(claim) = task.state else {
        panic!("lease")
    };
    // The private fake execution returns synchronously and cannot still run.
    // Only that host fact permits the separate ConfirmQuiescent operation.
    stopped.lock().unwrap().insert(attempt.clone());
    assert!(matches!(
        owner
            .dispatch_worker(W::Complete {
                task_id: attempt.task().task_id().clone(),
                lease: claim.lease,
                time
            })
            .unwrap()
            .unwrap(),
        O::Complete(Ok(()))
    ));
    assert!(matches!(
        owner
            .dispatch_scheduling(Q::Overview { now })
            .unwrap()
            .unwrap(),
        R::Overview {
            concurrency: Some(chronos_sdk_embedded::engine::ConcurrencyUsageV1 {
                held: 1,
                available: 0,
                ..
            }),
            ..
        }
    ));
    drop(owner);
    let mut owner = open(&directory.0, stopped, true);
    let expected = revision(&mut owner);
    let R::Reservations {
        attempts,
        next: None,
        ..
    } = owner
        .dispatch_scheduling(Q::Reservations {
            expected,
            after: None,
            limit: 1,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("reservations")
    };
    assert_eq!(attempts, vec![attempt.clone()]);
    control(&mut owner, C::ConfirmQuiescent { attempt });
    control(&mut owner, C::SetPaused { paused: false });
    assert!(matches!(
        owner
            .dispatch_scheduling(Q::Overview { now })
            .unwrap()
            .unwrap(),
        R::Overview {
            concurrency: Some(chronos_sdk_embedded::engine::ConcurrencyUsageV1 {
                reserved: 0,
                available: 1,
                ..
            }),
            ..
        }
    ));
    assert_eq!(owner.scheduling_signals().unwrap().controls, 2);
}
fn main() {
    run();
    println!(
        "Budgeted worker: pause preserved completion; capacity stayed reserved through restart until qualified release."
    );
}
#[cfg(test)]
mod tests {
    #[test]
    fn budgeted_native_worker_uses_durable_control_and_quiescence_ports() {
        super::run();
    }
}
