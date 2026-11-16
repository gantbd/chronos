//! Synthetic, volatile, single-scope embedding. No I/O, provider or credentials.
//! Storage qualification lives in the service's directory/recovery scenarios.
use chronos_sdk_embedded::{
    engine::{
        Agent, CommandOutcomeV2, Engine, EngineTime, LeaseToken, StateRevision, TaskAttemptRefV1,
        TaskPhase, TaskSpecV2, TaskState,
    },
    host::{
        native::{RequestV1, ResponseV1},
        worker::{WorkerClientV1, WorkerRequestV1 as Work},
    },
    service::{Authorizer, Limits, Service, worker::WorkerAuthorizer},
};
use std::time::{Duration, SystemTime};

// This policy is only for the private one-task fixture. It is not an example
// credential verifier, tenant policy, or safe default for an arbitrary host.
struct FixturePolicy;
impl Authorizer for FixturePolicy {
    fn allows(&mut self, request: &RequestV1) -> bool {
        match request {
            RequestV1::SubmitGraph { tasks, .. } => {
                tasks.len() == 1
                    && tasks[0].id.as_str() == "fixture"
                    && tasks[0].dependencies.is_empty()
                    && tasks[0].required_capabilities.is_empty()
            }
            RequestV1::Snapshot { task } => task.as_str() == "fixture",
            _ => false,
        }
    }
}
impl WorkerAuthorizer for FixturePolicy {
    fn allows_worker(&mut self, request: &Work) -> bool {
        match request {
            Work::AdvanceTime { .. } | Work::ClaimNext { .. } => true,
            Work::IssueGrant { agent, .. } => {
                agent.id == "fixture-worker" && agent.capabilities.is_empty()
            }
            Work::Complete { task_id, .. } | Work::FenceUncertain { task_id, .. } => {
                task_id.as_str() == "fixture"
            }
            _ => false,
        }
    }
    fn allows_worker_result(&mut self, _: &Work, outcome: &CommandOutcomeV2) -> bool {
        matches!(
            outcome,
            CommandOutcomeV2::AdvanceTime(_)
                | CommandOutcomeV2::IssueGrant(_)
                | CommandOutcomeV2::ClaimNext(_)
                | CommandOutcomeV2::Complete(_)
                | CommandOutcomeV2::FenceUncertain(_)
        )
    }
}
#[derive(Clone, Copy)]
enum SyntheticOutcome {
    Completed,
    Unknown,
}
struct SyntheticReceipt {
    attempt: TaskAttemptRefV1,
    outcome: SyntheticOutcome,
}

// The fake executor receives only correlation, never a grant, lease, service,
// principal, mutable engine or permission to issue its own reconciliation.
fn fake_executor(attempt: &TaskAttemptRefV1, outcome: SyntheticOutcome) -> SyntheticReceipt {
    SyntheticReceipt {
        attempt: attempt.clone(),
        outcome,
    }
}

fn report_command(
    expected: &TaskAttemptRefV1,
    lease: LeaseToken,
    time: EngineTime,
    receipt: SyntheticReceipt,
) -> Option<Work> {
    if &receipt.attempt != expected {
        return None;
    }
    let task_id = expected.task().task_id().clone();
    Some(match receipt.outcome {
        SyntheticOutcome::Completed => Work::Complete {
            task_id,
            lease,
            time,
        },
        // Unknown means fence now, not retry and not report a fabricated failure.
        SyntheticOutcome::Unknown => Work::FenceUncertain {
            task_id,
            lease,
            time,
        },
    })
}

fn fixture() -> (
    Service<FixturePolicy>,
    EngineTime,
    chronos_sdk_embedded::engine::Task,
    LeaseToken,
) {
    let mut host = Service::new(
        Engine::new(),
        FixturePolicy,
        Limits {
            max_graph_tasks: 1,
            max_page_entries: 1,
        },
    );
    assert!(matches!(
        host.dispatch(RequestV1::SubmitGraph {
            expected: StateRevision::ZERO,
            tasks: vec![TaskSpecV2::new("fixture")]
        }),
        Ok(ResponseV1::SubmitGraph(Ok(())))
    ));
    // Explicit fixture time; Chronos does not read a wall clock on its own.
    let CommandOutcomeV2::AdvanceTime(Ok(time)) = host
        .call_worker(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("time expected")
    };
    let CommandOutcomeV2::IssueGrant(Ok(grant)) = host
        .call_worker(Work::IssueGrant {
            agent: Agent::new("fixture-worker"),
            time: time.clone(),
            validity_duration: Duration::from_secs(60),
            claim_budget: Some(1),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("grant expected")
    };
    let CommandOutcomeV2::ClaimNext(Ok(Some(task))) = host
        .call_worker(Work::ClaimNext {
            grant,
            time: time.clone(),
            lease_duration: Duration::from_secs(10),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("claim expected")
    };
    let TaskState::Running(ref claim) = task.state else {
        panic!("running expected")
    };
    let lease = claim.lease.clone();
    (host, time, task, lease)
}

fn run(outcome: SyntheticOutcome) {
    let (mut host, time, task, lease) = fixture();
    let expected = task.attempt_reference().unwrap();
    let receipt = fake_executor(&expected, outcome);
    let command = report_command(&expected, lease, time, receipt).expect("matching receipt");
    assert!(matches!(
        host.call_worker(command).unwrap().unwrap(),
        CommandOutcomeV2::Complete(Ok(())) | CommandOutcomeV2::FenceUncertain(Ok(()))
    ));
    let Ok(ResponseV1::Snapshot(Some(snapshot))) = host.dispatch(RequestV1::Snapshot {
        task: "fixture".into(),
    }) else {
        panic!("snapshot expected")
    };
    assert_eq!(
        snapshot.task.phase,
        match outcome {
            SyntheticOutcome::Completed => TaskPhase::Succeeded,
            SyntheticOutcome::Unknown => TaskPhase::Uncertain,
        }
    );
    assert_eq!(host.worker_signals().executions, 4);
    assert_eq!(host.worker_usage(SystemTime::UNIX_EPOCH).engine.running, 0);
}
fn main() {
    run(SyntheticOutcome::Completed);
    run(SyntheticOutcome::Unknown);
    println!("Synthetic worker: completed receipt succeeds; unknown receipt fences without retry.");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_runs_both_completion_and_unknown_paths() {
        main();
    }
    #[test]
    fn mismatched_receipt_is_rejected_before_any_engine_command() {
        let (mut host, time, task, lease) = fixture();
        let expected = task.attempt_reference().unwrap();
        // A different task correlation is deliberately not this fixture's attempt.
        let mut other = Engine::new();
        other
            .submit(chronos_sdk_embedded::engine::Task::new("different", ""))
            .unwrap();
        let grant = other
            .issue_grant(
                Agent::new("worker"),
                SystemTime::UNIX_EPOCH,
                Duration::from_secs(60),
            )
            .unwrap();
        let other_task = other
            .claim_next(&grant, SystemTime::UNIX_EPOCH, Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let before = host.worker_signals().calls;
        assert!(
            report_command(
                &expected,
                lease,
                time,
                fake_executor(
                    &other_task.attempt_reference().unwrap(),
                    SyntheticOutcome::Completed
                )
            )
            .is_none()
        );
        assert_eq!(host.worker_signals().calls, before);
        assert!(
            matches!(host.dispatch(RequestV1::Snapshot { task: "fixture".into() }), Ok(ResponseV1::Snapshot(Some(value))) if value.task.phase == TaskPhase::Running)
        );
    }
}
