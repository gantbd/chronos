#![cfg(unix)]

use chronos_client::WireClientV1;
use chronos_engine::{Agent, CommandOutcomeV2, TaskState};
use chronos_host::worker::{WorkerClientV1, WorkerRequestV1 as Work};
use chronos_local::LocalClient;
use chronos_wire::{Command, Outcome, Phase, Request, RetrySafety, Revision, TaskSpec};
use std::{
    io::Write,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = PathBuf::from(format!(
            "/private/tmp/ch-worker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        chronos_standalone::ownership::create_private_directory(&path).unwrap();
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

fn snapshot(client: &mut LocalClient, task: &str) -> Phase {
    let response = client
        .request(Request {
            version: 1,
            command: Command::Snapshot { task: task.into() },
        })
        .unwrap();
    let Outcome::Snapshot {
        value: Some(entry), ..
    } = response.outcome
    else {
        panic!("snapshot expected")
    };
    entry.snapshot.phase
}

fn digest(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

#[test]
fn typed_worker_handle_claims_and_completes_the_same_durable_owner_task() {
    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind_with_options(
        &directory.0,
        chronos_standalone::ServerOptions {
            allow_writes: true,
            atomic_state: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut worker = server.trusted_worker().unwrap();
    assert!(server.trusted_worker().is_err());
    let (shutdown_read, mut shutdown_write) = UnixStream::pair().unwrap();
    let owner = std::thread::spawn(move || {
        server.run_with_shutdown(&shutdown_read).unwrap();
        server
    });

    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client
            .request(Request {
                version: 1,
                command: Command::SubmitGraph {
                    expected: Revision::Finite("0".into()),
                    tasks: vec![TaskSpec {
                        id: "j1-agent-task".into(),
                        priority: 0,
                        dependencies: vec![],
                        capabilities: vec![],
                        max_attempts: 1,
                        retry_safety: RetrySafety::FenceUnknown,
                    }],
                },
            })
            .unwrap()
            .outcome,
        Outcome::Applied {}
    ));
    let CommandOutcomeV2::AdvanceTime(Ok(time)) = worker
        .call_worker(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("time expected")
    };
    let CommandOutcomeV2::IssueGrant(Ok(grant)) = worker
        .call_worker(Work::IssueGrant {
            agent: Agent::new("moira-j1-worker"),
            time: time.clone(),
            validity_duration: Duration::from_secs(60),
            claim_budget: Some(1),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("grant expected")
    };
    let CommandOutcomeV2::ClaimNext(Ok(Some(task))) = worker
        .call_worker(Work::ClaimNext {
            grant,
            time: time.clone(),
            lease_duration: Duration::from_secs(30),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("claim expected")
    };
    let TaskState::Running(claim) = &task.state else {
        panic!("running task expected")
    };
    assert!(matches!(
        snapshot(&mut client, "j1-agent-task"),
        Phase::Running
    ));
    assert!(matches!(
        worker
            .call_worker(Work::Complete {
                task_id: task.id.clone(),
                lease: claim.lease.clone(),
                time,
            })
            .unwrap()
            .unwrap(),
        CommandOutcomeV2::Complete(Ok(()))
    ));
    assert!(matches!(
        snapshot(&mut client, "j1-agent-task"),
        Phase::Succeeded
    ));

    shutdown_write.write_all(&[1]).unwrap();
    let server = owner.join().unwrap();
    let signals = server.signals();
    assert_eq!(signals.worker_requests, 4);
    assert_eq!(signals.worker_refused, 0);
    assert_eq!(signals.worker_delivery_failed, 0);
}

#[test]
fn accepted_review_lost_reply_replays_after_real_socket_restart() {
    use chronos_client::{ReviewWireClientV10, WorkflowWireClientV9};
    use chronos_wire::{review_v10 as v10, workflow_v9 as v9};

    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind_with_options(
        &directory.0,
        chronos_standalone::ServerOptions {
            allow_writes: true,
            atomic_state: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut worker = server.trusted_worker().unwrap();
    let (shutdown_read, mut shutdown_write) = UnixStream::pair().unwrap();
    let owner = std::thread::spawn(move || {
        server.run_with_shutdown(&shutdown_read).unwrap();
        server
    });
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client
            .request(Request {
                version: 1,
                command: Command::SubmitGraph {
                    expected: Revision::Finite("0".into()),
                    tasks: vec![
                        TaskSpec {
                            id: "review-root".into(),
                            priority: 0,
                            dependencies: vec![],
                            capabilities: vec![],
                            max_attempts: 2,
                            retry_safety: RetrySafety::FenceUnknown,
                        },
                        TaskSpec {
                            id: "review-child".into(),
                            priority: 0,
                            dependencies: vec!["review-root".into()],
                            capabilities: vec![],
                            max_attempts: 1,
                            retry_safety: RetrySafety::FenceUnknown,
                        },
                    ],
                },
            })
            .unwrap()
            .outcome,
        Outcome::Applied {}
    ));
    let CommandOutcomeV2::AdvanceTime(Ok(time)) = worker
        .call_worker(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("time expected")
    };
    let CommandOutcomeV2::IssueGrant(Ok(grant)) = worker
        .call_worker(Work::IssueGrant {
            agent: Agent::new("review-worker"),
            time: time.clone(),
            validity_duration: Duration::from_secs(60),
            claim_budget: Some(1),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("grant expected")
    };
    let CommandOutcomeV2::ClaimNext(Ok(Some(task))) = worker
        .call_worker(Work::ClaimNext {
            grant,
            time: time.clone(),
            lease_duration: Duration::from_secs(30),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("claim expected")
    };
    assert_eq!(task.id.as_str(), "review-root");
    let TaskState::Running(claim) = &task.state else {
        panic!("running task expected")
    };
    assert!(matches!(
        worker
            .call_worker(Work::Complete {
                task_id: task.id.clone(),
                lease: claim.lease.clone(),
                time,
            })
            .unwrap()
            .unwrap(),
        CommandOutcomeV2::Complete(Ok(()))
    ));

    let root = client
        .request_workflow_v9(v9::Request {
            version: 9,
            command: v9::Command::Snapshot {
                task: "review-root".into(),
            },
        })
        .unwrap();
    let v9::Outcome::Snapshot {
        value: Some(root),
        revision: Some(expected),
    } = root.outcome
    else {
        panic!("root workflow snapshot expected")
    };
    assert_eq!(root.snapshot.status, v9::Status::ReadyForReview);
    assert_eq!(root.snapshot.attempt_number, Some(1));
    let child = client
        .request_workflow_v9(v9::Request {
            version: 9,
            command: v9::Command::Snapshot {
                task: "review-child".into(),
            },
        })
        .unwrap();
    assert!(matches!(
        child.outcome,
        v9::Outcome::Snapshot {
            value: Some(v9::Entry {
                snapshot: v9::Snapshot {
                    status: v9::Status::Blocked,
                    ..
                },
                ..
            }),
            ..
        }
    ));

    let evidence = v10::Evidence {
        decision_sha256: digest(1),
        reviewer_sha256: digest(2),
        authority_sha256: digest(3),
        challenged_sha256: digest(4),
        criteria_sha256: digest(5),
        findings_sha256: digest(6),
        feedback_sha256: None,
    };
    let execute = v10::Request {
        version: 10,
        command: v10::Command::ExecuteReview {
            invocation: v10::Invocation {
                serial: "1".into(),
                key: "accepted-review".into(),
            },
            expected: expected.clone(),
            task: root.reference.clone(),
            attempt: 1,
            disposition: v10::Disposition::Accepted,
            evidence: evidence.clone(),
        },
    };
    let lookup = || v10::Request {
        version: 10,
        command: v10::Command::LookupReview {
            invocation: v10::Invocation {
                serial: "1".into(),
                key: "accepted-review".into(),
            },
            expected: expected.clone(),
            task: root.reference.clone(),
            attempt: 1,
            disposition: v10::Disposition::Accepted,
            evidence: evidence.clone(),
        },
    };
    let mut lost = UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
    lost.shutdown(std::net::Shutdown::Read).unwrap();
    lost.write_all(&chronos_wire::encode(&execute, chronos_local::MAX_REQUEST_BYTES).unwrap())
        .unwrap();
    lost.write_all(b"\n").unwrap();
    lost.shutdown(std::net::Shutdown::Write).unwrap();
    drop(lost);
    let applied = (0..50)
        .find_map(|_| {
            let response = client.request_review_v10(lookup()).unwrap();
            if matches!(
                &response.outcome,
                v10::Outcome::Refused {
                    code: v10::RefusalCode::OutOfSequence
                }
            ) {
                std::thread::sleep(Duration::from_millis(1));
                None
            } else {
                Some(response)
            }
        })
        .expect("lost review command must become observable within the bounded wait");
    assert!(matches!(
        applied.outcome,
        v10::Outcome::ReviewResult {
            result: v10::ReviewResult::Applied {
                attempt: 1,
                disposition: v10::Disposition::Accepted,
                ..
            },
            ..
        }
    ));
    let child = client
        .request_workflow_v9(v9::Request {
            version: 9,
            command: v9::Command::Snapshot {
                task: "review-child".into(),
            },
        })
        .unwrap();
    assert!(matches!(
        child.outcome,
        v9::Outcome::Snapshot {
            value: Some(v9::Entry {
                snapshot: v9::Snapshot {
                    status: v9::Status::Open,
                    ..
                },
                ..
            }),
            ..
        }
    ));

    shutdown_write.write_all(&[1]).unwrap();
    drop(owner.join().unwrap());
    let mut restarted = chronos_standalone::Server::bind_with_options(
        &directory.0,
        chronos_standalone::ServerOptions {
            allow_writes: true,
            atomic_state: true,
            ..Default::default()
        },
    )
    .unwrap();
    let (restart_read, mut restart_write) = UnixStream::pair().unwrap();
    let owner = std::thread::spawn(move || {
        restarted.run_with_shutdown(&restart_read).unwrap();
        restarted
    });
    assert!(matches!(
        client.request_review_v10(lookup()).unwrap().outcome,
        v10::Outcome::ReviewResult {
            result: v10::ReviewResult::Applied {
                attempt: 1,
                disposition: v10::Disposition::Accepted,
                ..
            },
            ..
        }
    ));
    restart_write.write_all(&[1]).unwrap();
    drop(owner.join().unwrap());
}

#[test]
fn legacy_or_read_only_owner_never_installs_worker_authority() {
    let legacy = Directory::new();
    let mut server = chronos_standalone::Server::bind(&legacy.0, true).unwrap();
    assert!(server.trusted_worker().is_err());

    let read_only = Directory::new();
    let mut server = chronos_standalone::Server::bind_with_options(
        &read_only.0,
        chronos_standalone::ServerOptions {
            atomic_state: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(server.trusted_worker().is_err());
}

#[test]
fn dropped_handle_keeps_coordination_live_and_restart_fences_the_claim() {
    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind_with_options(
        &directory.0,
        chronos_standalone::ServerOptions {
            allow_writes: true,
            atomic_state: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut worker = server.trusted_worker().unwrap();
    let (shutdown_read, mut shutdown_write) = UnixStream::pair().unwrap();
    let owner = std::thread::spawn(move || {
        server.run_with_shutdown(&shutdown_read).unwrap();
        server
    });
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client
            .request(Request {
                version: 1,
                command: Command::SubmitGraph {
                    expected: Revision::Finite("0".into()),
                    tasks: vec![TaskSpec {
                        id: "restart-fence".into(),
                        priority: 0,
                        dependencies: vec![],
                        capabilities: vec![],
                        max_attempts: 2,
                        retry_safety: RetrySafety::FenceUnknown,
                    }],
                },
            })
            .unwrap()
            .outcome,
        Outcome::Applied {}
    ));
    let CommandOutcomeV2::AdvanceTime(Ok(time)) = worker
        .call_worker(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("time expected")
    };
    let CommandOutcomeV2::IssueGrant(Ok(grant)) = worker
        .call_worker(Work::IssueGrant {
            agent: Agent::new("moira-j1-worker"),
            time: time.clone(),
            validity_duration: Duration::from_secs(60),
            claim_budget: Some(1),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("grant expected")
    };
    assert!(matches!(
        worker
            .call_worker(Work::ClaimNext {
                grant,
                time,
                lease_duration: Duration::from_secs(30),
            })
            .unwrap()
            .unwrap(),
        CommandOutcomeV2::ClaimNext(Ok(Some(_)))
    ));
    drop(worker);
    assert!(matches!(
        snapshot(&mut client, "restart-fence"),
        Phase::Running
    ));
    shutdown_write.write_all(&[1]).unwrap();
    drop(owner.join().unwrap());

    let mut server = chronos_standalone::Server::bind_with_options(
        &directory.0,
        chronos_standalone::ServerOptions {
            allow_writes: true,
            atomic_state: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut worker = server.trusted_worker().unwrap();
    let (shutdown_read, mut shutdown_write) = UnixStream::pair().unwrap();
    let owner = std::thread::spawn(move || {
        server.run_with_shutdown(&shutdown_read).unwrap();
        server
    });
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        snapshot(&mut client, "restart-fence"),
        Phase::Uncertain
    ));
    let CommandOutcomeV2::AdvanceTime(Ok(time)) = worker
        .call_worker(Work::AdvanceTime {
            now: SystemTime::UNIX_EPOCH + Duration::from_secs(31),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("time expected")
    };
    let CommandOutcomeV2::IssueGrant(Ok(grant)) = worker
        .call_worker(Work::IssueGrant {
            agent: Agent::new("moira-j1-worker"),
            time: time.clone(),
            validity_duration: Duration::from_secs(60),
            claim_budget: Some(1),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("grant expected")
    };
    assert!(matches!(
        worker
            .call_worker(Work::ClaimNext {
                grant,
                time,
                lease_duration: Duration::from_secs(30),
            })
            .unwrap()
            .unwrap(),
        CommandOutcomeV2::ClaimNext(Ok(None))
    ));
    shutdown_write.write_all(&[1]).unwrap();
    drop(owner.join().unwrap());
}
