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
