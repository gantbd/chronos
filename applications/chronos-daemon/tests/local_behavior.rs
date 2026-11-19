#![cfg(unix)]
use chronos_client::WireClientV1;
use chronos_local::LocalClient;
use chronos_standalone::ownership::WriterOwnership;
use chronos_wire::{
    Command, Failure, FailureCode, Outcome, Request, RetrySafety, Revision, TaskSpec,
};
use std::{
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command as Process, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static NEXT: AtomicU64 = AtomicU64::new(0);
mod allocation;
mod qualification;
mod scheduling;
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        // Short socket paths also work on macOS's sockaddr_un ceiling.
        let path = PathBuf::from(format!(
            "/private/tmp/ch-{}-{}",
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
struct Running(Child);
impl Running {
    fn start(path: &std::path::Path, writes: bool) -> Self {
        Self::start_flags(path, if writes { &["--allow-writes"] } else { &[] })
    }
    fn start_flags(path: &std::path::Path, flags: &[&str]) -> Self {
        let mut command = Process::new(env!("CARGO_BIN_EXE_chronos-service"));
        command.arg(path).stderr(Stdio::piped());
        command.args(flags);
        let mut child = Self(command.spawn().unwrap());
        let stderr = child.0.stderr.take().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stderr).read_line(&mut line);
            let _ = send.send(line);
        });
        assert!(
            receive
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .contains("ready")
        );
        child
    }
}

#[test]
fn workflow_v9_exposes_authoritative_status_over_the_real_local_transport() {
    use chronos_client::WorkflowWireClientV9;
    use chronos_wire::workflow_v9 as v9;

    let directory = Directory::new();
    let _server = Running::start(&directory.0, true);
    let mut client = LocalClient::new(&directory.0);
    let submitted = client
        .request(Request {
            version: 1,
            command: Command::SubmitGraph {
                expected: Revision::Finite("0".into()),
                tasks: vec![
                    TaskSpec {
                        id: "root".into(),
                        priority: 0,
                        dependencies: vec![],
                        capabilities: vec![],
                        max_attempts: 3,
                        retry_safety: RetrySafety::FenceUnknown,
                    },
                    TaskSpec {
                        id: "dependent".into(),
                        priority: 0,
                        dependencies: vec!["root".into()],
                        capabilities: vec![],
                        max_attempts: 3,
                        retry_safety: RetrySafety::FenceUnknown,
                    },
                ],
            },
        })
        .unwrap();
    assert!(matches!(submitted.outcome, Outcome::Applied {}));

    let response = client
        .request_workflow_v9(v9::Request {
            version: 9,
            command: v9::Command::Page {
                cursor: None,
                limit: 4,
            },
        })
        .unwrap();
    let v9::Outcome::Page { entries, .. } = response.outcome else {
        panic!("expected workflow page")
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].snapshot.status, v9::Status::Blocked);
    assert_eq!(
        entries[0].snapshot.blocker.as_ref().unwrap().kind,
        v9::BlockerKind::Dependency
    );
    assert_eq!(entries[1].snapshot.status, v9::Status::Open);
    assert_eq!(
        entries[1].snapshot.execution_phase,
        v9::ExecutionPhase::Ready
    );
}

#[test]
fn atomic_wire_lost_reply_lookup_replay_and_read_only_restart_use_one_committed_invocation() {
    use chronos_client::WireClientV3;
    use chronos_wire::coordination_v3 as v3;
    use std::io::Write;
    let directory = Directory::new();
    let server = Running::start_flags(&directory.0, &["--atomic-state", "--allow-writes"]);
    let execute = || v3::Request {
        version: 3,
        command: v3::Command::ExecuteInvocation {
            invocation: v3::Invocation {
                serial: "1".into(),
                key: "stable-private-intent".into(),
            },
            command: v3::Mutation::SubmitGraph {
                expected: Revision::Finite("0".into()),
                tasks: vec![TaskSpec {
                    id: "a".into(),
                    priority: 0,
                    dependencies: vec![],
                    capabilities: vec![],
                    max_attempts: 3,
                    retry_safety: RetrySafety::FenceUnknown,
                }],
            },
        },
    };
    let mut lost =
        std::os::unix::net::UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
    lost.shutdown(std::net::Shutdown::Read).unwrap();
    lost.write_all(&chronos_wire::encode(&execute(), chronos_local::MAX_REQUEST_BYTES).unwrap())
        .unwrap();
    lost.write_all(b"\n").unwrap();
    lost.shutdown(std::net::Shutdown::Write).unwrap();
    let mut client = LocalClient::new(&directory.0);
    let lookup = || v3::Request {
        version: 3,
        command: v3::Command::LookupInvocation {
            invocation: v3::Invocation {
                serial: "1".into(),
                key: "stable-private-intent".into(),
            },
        },
    };
    assert!(matches!(
        client.request_v3(lookup()).unwrap().outcome,
        v3::Outcome::InvocationResult {
            result: v3::MutationResult::Applied {},
            ..
        }
    ));
    drop(lost);
    let files = || {
        let mut names: Vec<_> = std::fs::read_dir(&directory.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().starts_with("service-"))
            .collect();
        names.sort();
        names
    };
    let before = files();
    assert!(matches!(
        client.request_v3(execute()).unwrap().outcome,
        v3::Outcome::InvocationResult {
            result: v3::MutationResult::Applied {},
            ..
        }
    ));
    assert_eq!(files(), before, "replay did not publish another envelope");
    drop(server);
    let _server = Running::start(&directory.0, false);
    assert!(matches!(
        client.request_v3(lookup()).unwrap().outcome,
        v3::Outcome::InvocationResult {
            result: v3::MutationResult::Applied {},
            ..
        }
    ));
    assert!(matches!(
        client.request_v3(execute()).unwrap().outcome,
        v3::Outcome::Refused {
            code: v3::RefusalCode::Unauthorized
        }
    ));
    assert!(
        matches!(client.request_v3(v3::Request {version:3,command:v3::Command::NextInvocation {}}).unwrap().outcome,v3::Outcome::NextInvocation {serial:Some(value)} if value=="2")
    );
}

#[test]
fn scheduled_v7_lost_reply_reconciles_without_partial_claimability_or_reexecution() {
    use chronos_client::{ScheduledAdmissionWireClientV7, SchedulingWireClientV5, WireClientV4};
    use chronos_wire::{coordination_v4 as v4, coordination_v7 as v7, scheduling_v5 as v5};
    use std::io::Write;

    let directory = Directory::new();
    let server = Running::start_flags(
        &directory.0,
        &["--atomic-state", "--allocation-receipts", "--allow-writes"],
    );
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client
            .request_scheduling_v5(v5::Request {
                version: 5,
                command: v5::Command::Enable {
                    expected: Revision::Finite("0".into()),
                },
            })
            .unwrap()
            .outcome,
        v5::Outcome::Control {
            result: v5::ControlResult::Applied {}
        }
    ));

    let scheduled = || v7::Request {
        version: 7,
        command: v7::Command::ExecuteScheduledGraph {
            invocation: v7::Invocation {
                serial: "1".into(),
                key: "scheduled-private-intent".into(),
            },
            expected: Revision::Finite("1".into()),
            tasks: vec![v7::ScheduledTask {
                task: TaskSpec {
                    id: "scheduled".into(),
                    priority: 0,
                    dependencies: vec![],
                    capabilities: vec![],
                    max_attempts: 1,
                    retry_safety: RetrySafety::FenceUnknown,
                },
                eligibility: v5::Eligibility {
                    paused: true,
                    not_before: Some(v5::Timestamp {
                        before_epoch: false,
                        seconds: "42".into(),
                        nanos: 7,
                    }),
                },
            }],
        },
    };
    let lookup = || {
        let request = scheduled();
        let v7::Command::ExecuteScheduledGraph {
            invocation,
            expected,
            tasks,
        } = request.command
        else {
            unreachable!()
        };
        v7::Request {
            version: 7,
            command: v7::Command::LookupScheduledGraph {
                invocation,
                expected,
                tasks,
            },
        }
    };

    let mut lost =
        std::os::unix::net::UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
    lost.shutdown(std::net::Shutdown::Read).unwrap();
    lost.write_all(&chronos_wire::encode(&scheduled(), chronos_local::MAX_REQUEST_BYTES).unwrap())
        .unwrap();
    lost.write_all(b"\n").unwrap();
    lost.shutdown(std::net::Shutdown::Write).unwrap();
    assert!(matches!(
        client
            .request_scheduled_admission_v7(lookup())
            .unwrap()
            .outcome,
        v7::Outcome::InvocationResult {
            result: v7::MutationResult::Applied {},
            ..
        }
    ));

    let allocation = client
        .request_v4(v4::Request {
            version: 4,
            command: v4::Command::LookupAllocation {
                serial: "1".into(),
                key: "scheduled-private-intent".into(),
            },
        })
        .unwrap();
    let v4::Outcome::Allocation { tasks, .. } = allocation.outcome else {
        panic!("exact scheduled allocation expected")
    };
    assert_eq!(tasks.len(), 1);
    assert!(matches!(
        client
            .request_scheduling_v5(v5::Request {
                version: 5,
                command: v5::Command::Eligibility {
                    task: tasks[0].clone(),
                },
            })
            .unwrap()
            .outcome,
        v5::Outcome::Eligibility {
            eligibility: Some(v5::Eligibility {
                paused: true,
                not_before: Some(v5::Timestamp {
                    before_epoch: false,
                    seconds,
                    nanos: 7,
                }),
            }),
            ..
        } if seconds == "42"
    ));

    drop(lost);
    drop(server);
    let _readonly = Running::start(&directory.0, false);
    assert!(matches!(
        client
            .request_scheduled_admission_v7(lookup())
            .unwrap()
            .outcome,
        v7::Outcome::InvocationResult {
            result: v7::MutationResult::Applied {},
            ..
        }
    ));
    assert!(matches!(
        client
            .request_scheduled_admission_v7(scheduled())
            .unwrap()
            .outcome,
        v7::Outcome::Refused {
            code: v7::RefusalCode::Unauthorized
        }
    ));
}

#[test]
fn scheduled_v8_cancellation_lost_reply_reconciles_exactly_without_reexecution() {
    use chronos_client::{
        ScheduledAdmissionWireClientV7, ScheduledCancellationWireClientV8, SchedulingWireClientV5,
        WireClientV4,
    };
    use chronos_wire::{
        Phase, coordination_v4 as v4, coordination_v7 as v7, coordination_v8 as v8,
        scheduling_v5 as v5,
    };
    use std::io::Write;

    let directory = Directory::new();
    let server = Running::start_flags(
        &directory.0,
        &["--atomic-state", "--allocation-receipts", "--allow-writes"],
    );
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client
            .request_scheduling_v5(v5::Request {
                version: 5,
                command: v5::Command::Enable {
                    expected: Revision::Finite("0".into()),
                },
            })
            .unwrap()
            .outcome,
        v5::Outcome::Control {
            result: v5::ControlResult::Applied {}
        }
    ));
    assert!(matches!(
        client
            .request_scheduled_admission_v7(v7::Request {
                version: 7,
                command: v7::Command::ExecuteScheduledGraph {
                    invocation: v7::Invocation {
                        serial: "1".into(),
                        key: "schedule-intent".into(),
                    },
                    expected: Revision::Finite("1".into()),
                    tasks: vec![v7::ScheduledTask {
                        task: TaskSpec {
                            id: "scheduled-cancel".into(),
                            priority: 0,
                            dependencies: vec![],
                            capabilities: vec![],
                            max_attempts: 1,
                            retry_safety: RetrySafety::FenceUnknown,
                        },
                        eligibility: v5::Eligibility {
                            paused: true,
                            not_before: None,
                        },
                    }],
                },
            })
            .unwrap()
            .outcome,
        v7::Outcome::InvocationResult {
            result: v7::MutationResult::Applied {},
            ..
        }
    ));
    let allocation = client
        .request_v4(v4::Request {
            version: 4,
            command: v4::Command::LookupAllocation {
                serial: "1".into(),
                key: "schedule-intent".into(),
            },
        })
        .unwrap();
    let v4::Outcome::Allocation {
        revision,
        mut tasks,
        ..
    } = allocation.outcome
    else {
        panic!("allocation expected")
    };
    assert_eq!(tasks.len(), 1);
    let task = tasks.pop().unwrap();
    let execute = || v8::Request {
        version: 8,
        command: v8::Command::ExecuteCancellation {
            invocation: v8::Invocation {
                serial: "2".into(),
                key: "cancel-intent".into(),
            },
            expected: revision.clone(),
            task: task.clone(),
        },
    };
    let lookup = || v8::Request {
        version: 8,
        command: v8::Command::LookupCancellation {
            invocation: v8::Invocation {
                serial: "2".into(),
                key: "cancel-intent".into(),
            },
            expected: revision.clone(),
            task: task.clone(),
        },
    };

    let mut stream =
        std::os::unix::net::UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
    stream
        .write_all(&chronos_wire::encode(&execute(), chronos_local::MAX_REQUEST_BYTES).unwrap())
        .unwrap();
    stream.write_all(b"\n").unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    chronos_wire::read_frame(
        &mut BufReader::new(stream),
        chronos_local::MAX_RESPONSE_BYTES,
    )
    .unwrap()
    .unwrap();
    let files = || {
        std::fs::read_dir(&directory.0)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("service-")
            })
            .count()
    };
    let before = files();
    assert!(matches!(
        client
            .request_scheduled_cancellation_v8(lookup())
            .unwrap()
            .outcome,
        v8::Outcome::CancellationResult {
            result: v8::CancellationResult::Applied {},
            ..
        }
    ));
    assert_eq!(
        files(),
        before,
        "scoped lookup did not republish cancellation"
    );
    assert!(matches!(
        client
            .request(Request {
                version: 1,
                command: Command::Snapshot {
                    task: "scheduled-cancel".into(),
                },
            })
            .unwrap()
            .outcome,
        Outcome::Snapshot {
            value: Some(chronos_wire::Entry {
                reference,
                snapshot: chronos_wire::Snapshot {
                    phase: Phase::Cancelled,
                    ..
                },
            }),
            ..
        } if reference.task == task.task && reference.incarnation == task.incarnation
    ));

    drop(server);
    let _readonly = Running::start(&directory.0, false);
    assert!(matches!(
        client
            .request_scheduled_cancellation_v8(lookup())
            .unwrap()
            .outcome,
        v8::Outcome::CancellationResult {
            result: v8::CancellationResult::Applied {},
            ..
        }
    ));
    assert!(matches!(
        client
            .request_scheduled_cancellation_v8(execute())
            .unwrap()
            .outcome,
        v8::Outcome::Refused {
            code: v8::RefusalCode::Unauthorized
        }
    ));
}

#[test]
fn exact_cancellation_receipt_recovers_lost_reply_without_another_effect() {
    use chronos_client::WireClientV3;
    use chronos_wire::{Phase, coordination_v3 as v3};
    use std::io::Write;

    let directory = Directory::new();
    let server = Running::start_flags(&directory.0, &["--atomic-state", "--allow-writes"]);
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client.request(submit()).unwrap().outcome,
        Outcome::Applied {}
    ));
    let snapshot = |client: &mut LocalClient| {
        let reply = client
            .request(Request {
                version: 1,
                command: Command::Snapshot { task: "a".into() },
            })
            .unwrap();
        let Outcome::Snapshot {
            value: Some(entry),
            revision: Some(Revision::Finite(revision)),
        } = reply.outcome
        else {
            panic!("expected task");
        };
        (revision, entry)
    };
    let (revision, entry) = snapshot(&mut client);
    let invocation = || v3::Invocation {
        serial: "1".into(),
        key: "exact-cancellation-test".into(),
    };
    let execute = || v3::Request {
        version: 3,
        command: v3::Command::ExecuteInvocation {
            invocation: invocation(),
            command: v3::Mutation::Cancel {
                expected: Revision::Finite(revision.clone()),
                task: entry.reference.clone(),
            },
        },
    };
    let lookup = || v3::Request {
        version: 3,
        command: v3::Command::LookupInvocation {
            invocation: invocation(),
        },
    };
    let mut lost =
        std::os::unix::net::UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
    lost.shutdown(std::net::Shutdown::Read).unwrap();
    lost.write_all(&chronos_wire::encode(&execute(), chronos_local::MAX_REQUEST_BYTES).unwrap())
        .unwrap();
    lost.write_all(b"\n").unwrap();
    lost.shutdown(std::net::Shutdown::Write).unwrap();

    let applied = |reply: v3::Response| {
        matches!(
            reply.outcome,
            v3::Outcome::InvocationResult { serial, result: v3::MutationResult::Applied {} }
                if serial == "1"
        )
    };
    assert!(applied(client.request_v3(lookup()).unwrap()));
    let (cancelled_revision, cancelled) = snapshot(&mut client);
    assert!(matches!(cancelled.snapshot.phase, Phase::Cancelled));
    assert_eq!(cancelled.reference.incarnation, entry.reference.incarnation);
    assert!(applied(client.request_v3(execute()).unwrap()));
    assert_eq!(snapshot(&mut client).0, cancelled_revision);

    drop(server);
    let _reader = Running::start(&directory.0, false);
    assert!(applied(client.request_v3(lookup()).unwrap()));
    assert!(matches!(
        client.request_v3(execute()).unwrap().outcome,
        v3::Outcome::Refused {
            code: v3::RefusalCode::Unauthorized
        }
    ));
    assert_eq!(snapshot(&mut client).0, cancelled_revision);
}

#[test]
fn envelope_migration_is_explicit_preserves_source_and_missing_profile_evidence_never_downgrades() {
    let directory = Directory::new();
    let server = Running::start(&directory.0, true);
    LocalClient::new(&directory.0).request(submit()).unwrap();
    drop(server);
    let legacy: Vec<_> = std::fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("checkpoint-")
        })
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    assert!(
        chronos_standalone::Server::bind_with_options(
            &directory.0,
            chronos_standalone::ServerOptions {
                atomic_state: true,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(!directory.0.join(".chronos-envelope-v1").exists());
    let migrated = Process::new(env!("CARGO_BIN_EXE_chronos-service"))
        .arg("--migrate-envelope")
        .arg(&directory.0)
        .output()
        .unwrap();
    assert!(migrated.status.success(), "{:?}", migrated.stderr);
    for (path, bytes) in legacy {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    let server = Running::start(&directory.0, false);
    assert!(
        matches!(LocalClient::new(&directory.0).request(revision()).unwrap().outcome,Outcome::Revision {revision:Revision::Finite(value)} if value=="1")
    );
    drop(server);
    let marker = directory.0.join(".chronos-envelope-v1");
    let bytes = std::fs::read(&marker).unwrap();
    std::fs::remove_file(&marker).unwrap();
    assert!(
        chronos_standalone::Server::bind(&directory.0, false).is_err(),
        "service records without marker cannot fall back to legacy"
    );
    std::fs::write(&marker, bytes).unwrap();
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();
    for entry in std::fs::read_dir(&directory.0).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_string_lossy().starts_with("service-") {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
    assert!(
        chronos_standalone::Server::bind(&directory.0, false).is_err(),
        "marker without records cannot fall back to legacy"
    );
}

#[test]
fn two_wire_observers_apply_committed_deltas_and_resynchronize_after_eviction_and_restart() {
    use chronos_client::{
        WireClientV3,
        observer::{Observer, ObserverLimits, RefreshError},
    };
    use chronos_wire::coordination_v3 as v3;
    let directory = Directory::new();
    let server = Running::start_flags(
        &directory.0,
        &["--atomic-state", "--observer-history", "--allow-writes"],
    );
    let limits = ObserverLimits {
        page_size: 64,
        max_pages: 4,
        max_entries: 256,
        max_id_bytes: 256,
    };
    let mut first = Observer::new(limits).unwrap();
    let mut second = Observer::new(limits).unwrap();
    let mut first_client = LocalClient::new(&directory.0);
    let mut second_client = LocalClient::new(&directory.0);
    let mut writer = LocalClient::new(&directory.0);
    assert!(!first.resync_feed(&mut first_client).unwrap());
    assert!(!second.resync_feed(&mut second_client).unwrap());
    let generation = first.feed_cursor().unwrap().generation.clone();
    writer.request(submit()).unwrap();
    first.poll_changes(&mut first_client).unwrap();
    assert_eq!(first.view().unwrap().entries.len(), 1);
    assert!(second.view().unwrap().entries.is_empty());
    second.poll_changes(&mut second_client).unwrap();
    let old = first.view().unwrap().entries[0].reference.clone();
    writer
        .request(Request {
            version: 1,
            command: Command::Cancel {
                expected: first.view().unwrap().revision.clone(),
                task: old.clone(),
            },
        })
        .unwrap();
    let response = writer
        .request_v3(v3::Request {
            version: 3,
            command: v3::Command::ExecuteInvocation {
                invocation: v3::Invocation {
                    serial: "1".into(),
                    key: "retire-original-a".into(),
                },
                command: v3::Mutation::Retire {
                    expected: Revision::Finite("2".into()),
                    task: old.clone(),
                },
            },
        })
        .unwrap();
    assert!(matches!(
        response.outcome,
        v3::Outcome::InvocationResult {
            result: v3::MutationResult::Applied {},
            ..
        }
    ));
    first.poll_changes(&mut first_client).unwrap();
    second.poll_changes(&mut second_client).unwrap();
    assert!(first.view().unwrap().entries.is_empty());
    assert!(second.view().unwrap().entries.is_empty());
    let mut recreate = submit();
    let Command::SubmitGraph { expected, .. } = &mut recreate.command else {
        unreachable!()
    };
    *expected = Revision::Finite("3".into());
    writer.request(recreate).unwrap();
    first.poll_changes(&mut first_client).unwrap();
    second.poll_changes(&mut second_client).unwrap();
    assert_ne!(
        first.view().unwrap().entries[0].reference.incarnation,
        old.incarnation
    );
    // One more change than the funded 128-transition history causes an explicit
    // retention gap, without asking the server for unbounded history.
    for index in 0..129 {
        let mut request = submit();
        let Command::SubmitGraph { expected, tasks } = &mut request.command else {
            unreachable!()
        };
        *expected = Revision::Finite((4 + index).to_string());
        tasks[0].id = format!("noise-{index:03}");
        assert!(matches!(
            writer.request(request).unwrap().outcome,
            Outcome::Applied {}
        ));
    }
    assert!(matches!(
        first.poll_changes(&mut first_client),
        Err(RefreshError::ResyncRequired)
    ));
    assert!(first.view().is_none());
    assert_eq!(
        second.view().unwrap().entries.len(),
        1,
        "peer state is not silently shared"
    );
    assert!(matches!(
        second.poll_changes(&mut second_client),
        Err(RefreshError::ResyncRequired)
    ));
    first.resync_feed(&mut first_client).unwrap();
    second.resync_feed(&mut second_client).unwrap();
    assert_eq!(first.view().unwrap().entries.len(), 130);
    assert_eq!(second.view().unwrap().entries.len(), 130);
    drop(server);
    assert!(first.poll_changes(&mut first_client).is_err());
    assert!(first.view().is_none());
    let _server = Running::start_flags(&directory.0, &["--observer-history"]);
    assert!(matches!(
        second.poll_changes(&mut second_client),
        Err(RefreshError::ResyncRequired)
    ));
    first.resync_feed(&mut first_client).unwrap();
    second.resync_feed(&mut second_client).unwrap();
    assert_ne!(first.feed_cursor().unwrap().generation, generation);
    for observer in [&first, &second] {
        assert_eq!(observer.view().unwrap().entries.len(), 130);
        assert!(
            matches!(&observer.view().unwrap().revision,Revision::Finite(value) if value=="133")
        );
    }
}

#[test]
fn selected_profile_marker_replacement_fences_the_current_server_and_preserves_replacement() {
    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind_with_options(
        &directory.0,
        chronos_standalone::ServerOptions {
            atomic_state: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(server.runtime_signals().atomic.unwrap().commits, 1);
    assert_eq!(
        server
            .runtime_signals()
            .invocation_usage
            .unwrap()
            .retained_entries,
        0
    );
    assert!(server.runtime_signals().legacy_persistence.is_none());
    let path = directory.0.join(".chronos-envelope-v1");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(server.serve_one().is_err());
    assert!(server.signals().fenced);
    assert_eq!(server.signals().ownership_failures, 1);
    drop(server);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn malformed_v3_frame_gets_a_v3_refusal_without_exposing_input_or_changing_state() {
    use chronos_wire::coordination_v3 as v3;
    use std::io::Write;
    let directory = Directory::new();
    let _server = Running::start_flags(&directory.0, &["--atomic-state", "--allow-writes"]);
    let mut stream =
        std::os::unix::net::UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
    stream
        .set_read_timeout(Some(chronos_local::IO_TIMEOUT))
        .unwrap();
    stream.write_all(br#"{"version":3,"command":{"operation":"execute_invocation","invocation":{"serial":"1","key":"private"},"command":{"operation":"surprise"}}}"#).unwrap();
    stream.write_all(b"\n").unwrap();
    let frame = chronos_wire::read_frame(
        &mut BufReader::new(stream),
        chronos_local::MAX_RESPONSE_BYTES,
    )
    .unwrap()
    .unwrap();
    assert!(!String::from_utf8_lossy(&frame).contains("private"));
    assert!(matches!(
        chronos_wire::decode_response_v3(
            &frame,
            chronos_local::MAX_RESPONSE_BYTES,
            chronos_local::wire_limits()
        )
        .unwrap()
        .outcome,
        v3::Outcome::Refused {
            code: v3::RefusalCode::InvalidRequest
        }
    ));
    assert!(
        matches!(LocalClient::new(&directory.0).request(revision()).unwrap().outcome,Outcome::Revision {revision:Revision::Finite(value)} if value=="0")
    );
}

#[test]
fn observer_profile_on_legacy_storage_does_not_implicitly_migrate() {
    use chronos_client::{
        WireClientV3,
        observer::{Observer, ObserverLimits},
    };
    use chronos_wire::coordination_v3 as v3;
    let directory = Directory::new();
    let _server = Running::start_flags(&directory.0, &["--observer-history", "--allow-writes"]);
    let mut client = LocalClient::new(&directory.0);
    let mut observer = Observer::new(ObserverLimits {
        page_size: 2,
        max_pages: 2,
        max_entries: 4,
        max_id_bytes: 256,
    })
    .unwrap();
    observer.resync_feed(&mut client).unwrap();
    client.request(submit()).unwrap();
    observer.poll_changes(&mut client).unwrap();
    assert_eq!(observer.view().unwrap().entries.len(), 1);
    assert!(matches!(
        client
            .request_v3(v3::Request {
                version: 3,
                command: v3::Command::NextInvocation {}
            })
            .unwrap()
            .outcome,
        v3::Outcome::Refused {
            code: v3::RefusalCode::Unavailable
        }
    ));
    assert!(!directory.0.join(".chronos-envelope-v1").exists());
    assert!(!std::fs::read_dir(&directory.0).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("service-")
    }));
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn revision() -> Request {
    Request {
        version: 1,
        command: Command::Revision {},
    }
}

/// Opt-in calibration workload, never a normative product latency budget.
#[test]
#[ignore = "opt-in optimized local latency calibration; no product SLO asserted"]
fn local_request_latency_measurement() {
    use chronos_client::WireClientV2;
    use chronos_wire::coordination_v2 as v2;
    use std::time::Instant;
    let directory = Directory::new();
    let _server = Running::start(&directory.0, true);
    let mut client = LocalClient::new(&directory.0);
    let mut timings: [Vec<Duration>; 5] = std::array::from_fn(|_| Vec::with_capacity(128));
    // Two complete warmup cycles, then 64 bounded measured cycles. Each cycle
    // reuses a task name only after retirement; incarnation identity is fresh.
    for iteration in 0..66 {
        let started = Instant::now();
        let response = client.request(revision()).unwrap();
        let elapsed = started.elapsed();
        if iteration >= 2 {
            timings[0].push(elapsed);
        }
        let Outcome::Revision { revision } = response.outcome else {
            panic!("revision expected")
        };
        let mut admission = submit();
        let Command::SubmitGraph { expected, .. } = &mut admission.command else {
            unreachable!()
        };
        *expected = revision;
        let started = Instant::now();
        let response = client.request(admission).unwrap();
        let elapsed = started.elapsed();
        if iteration >= 2 {
            timings[1].push(elapsed);
        }
        assert!(matches!(response.outcome, Outcome::Applied {}));
        let started = Instant::now();
        let response = client
            .request_v2(v2::Request {
                version: 2,
                command: v2::Command::Explain { task: "a".into() },
            })
            .unwrap();
        let elapsed = started.elapsed();
        if iteration >= 2 {
            timings[2].push(elapsed);
        }
        let v2::Outcome::Explanation { value: Some(value) } = response.outcome else {
            panic!("explanation expected")
        };
        let started = Instant::now();
        let response = client
            .request(Request {
                version: 1,
                command: Command::Cancel {
                    expected: value.revision,
                    task: value.entry.reference,
                },
            })
            .unwrap();
        let elapsed = started.elapsed();
        if iteration >= 2 {
            timings[3].push(elapsed);
        }
        assert!(matches!(response.outcome, Outcome::Applied {}));
        let started = Instant::now();
        let response = client
            .request_v2(v2::Request {
                version: 2,
                command: v2::Command::Explain { task: "a".into() },
            })
            .unwrap();
        let elapsed = started.elapsed();
        if iteration >= 2 {
            timings[2].push(elapsed);
        }
        let v2::Outcome::Explanation { value: Some(value) } = response.outcome else {
            panic!("explanation expected")
        };
        assert!(value.retirement_eligible);
        let started = Instant::now();
        let response = client
            .request_v2(v2::Request {
                version: 2,
                command: v2::Command::Retire {
                    expected: value.revision,
                    task: value.entry.reference,
                },
            })
            .unwrap();
        let elapsed = started.elapsed();
        if iteration >= 2 {
            timings[4].push(elapsed);
        }
        assert!(matches!(response.outcome, v2::Outcome::Applied {}));
    }
    println!(
        "profile=synthetic_private_local cycles=64 warmup_cycles=2 os={} arch={} optimized={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        !cfg!(debug_assertions)
    );
    for (operation, samples) in ["revision", "submit_graph", "explain", "cancel", "retire"]
        .into_iter()
        .zip(&mut timings)
    {
        samples.sort_unstable();
        let percentile =
            |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1].as_nanos();
        println!(
            "operation={operation} count={} min_ns={} p50_ns={} p95_ns={} p99_ns={} max_ns={}",
            samples.len(),
            samples[0].as_nanos(),
            percentile(50),
            percentile(95),
            percentile(99),
            samples.last().unwrap().as_nanos()
        );
    }
}

#[test]
fn malformed_v2_request_returns_a_v2_refusal_without_mutating_or_stopping_service() {
    use chronos_wire::coordination_v2 as v2;
    use std::io::Write;
    let directory = Directory::new();
    let _server = Running::start(&directory.0, true);
    for frame in [
        br#"{"version":2,"command":{"operation":"retire","expected":{"kind":"finite","value":"00"},"task":{"task":"private","incarnation":"0"}}}"#.as_slice(),
        br#"{"version":2,"command":{"operation":"execute","task":"private"}}"#.as_slice(),
    ] {
        let mut stream=std::os::unix::net::UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
        stream.set_read_timeout(Some(chronos_local::IO_TIMEOUT)).unwrap();
        stream.write_all(frame).unwrap();stream.write_all(b"\n").unwrap();
        let bytes=chronos_wire::read_frame(&mut BufReader::new(stream),chronos_local::MAX_RESPONSE_BYTES).unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("private"));
        let response=chronos_wire::decode_response_v2(&bytes,chronos_local::MAX_RESPONSE_BYTES).unwrap();
        assert!(matches!(response.outcome,v2::Outcome::Refused {failure:v2::RetirementFailure::Common {failure:Failure::Refused {code:FailureCode::InvalidRequest}}}));
    }
    let mut client = LocalClient::new(&directory.0);
    assert!(
        matches!(client.request(revision()).unwrap().outcome,Outcome::Revision {revision:Revision::Finite(value)} if value=="0")
    );
}

#[test]
fn independent_observers_resynchronize_after_disconnect_restart_and_task_recreation() {
    use chronos_client::{
        WireClientV2,
        observer::{Observer, ObserverLimits},
    };
    use chronos_wire::coordination_v2 as v2;
    let directory = Directory::new();
    let server = Running::start(&directory.0, true);
    let mut client = LocalClient::new(&directory.0);
    let limits = ObserverLimits {
        page_size: 2,
        max_pages: 2,
        max_entries: 4,
        max_id_bytes: 256,
    };
    let mut first = Observer::new(limits).unwrap();
    let mut second = Observer::new(limits).unwrap();
    first.refresh(&mut client).unwrap();
    assert!(first.view().unwrap().entries.is_empty());
    client.request(submit()).unwrap();
    second.refresh(&mut client).unwrap();
    assert_eq!(second.view().unwrap().entries.len(), 1);
    assert!(
        first.view().unwrap().entries.is_empty(),
        "views are not shared mutable state"
    );
    let old = second.view().unwrap().entries[0].reference.clone();
    drop(server);
    assert!(first.refresh(&mut client).is_err());
    assert!(first.view().is_none());
    let _server = Running::start(&directory.0, true);
    assert!(matches!(
        client
            .request(Request {
                version: 1,
                command: Command::Cancel {
                    expected: second.view().unwrap().revision.clone(),
                    task: old.clone()
                }
            })
            .unwrap()
            .outcome,
        Outcome::Applied {}
    ));
    let v2::Outcome::Explanation { value: Some(value) } = client
        .request_v2(v2::Request {
            version: 2,
            command: v2::Command::Explain { task: "a".into() },
        })
        .unwrap()
        .outcome
    else {
        panic!("explanation expected")
    };
    assert!(matches!(
        client
            .request_v2(v2::Request {
                version: 2,
                command: v2::Command::Retire {
                    expected: value.revision,
                    task: value.entry.reference
                }
            })
            .unwrap()
            .outcome,
        v2::Outcome::Applied {}
    ));
    let mut recreate = submit();
    let Command::SubmitGraph { expected, .. } = &mut recreate.command else {
        unreachable!()
    };
    *expected = Revision::Finite("3".into());
    assert!(matches!(
        client.request(recreate).unwrap().outcome,
        Outcome::Applied {}
    ));
    first.refresh(&mut client).unwrap();
    second.refresh(&mut client).unwrap();
    for observer in [&first, &second] {
        let view = observer.view().unwrap();
        assert!(matches!(&view.revision,Revision::Finite(value) if value=="4"));
        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.entries[0].reference.task, old.task);
        assert_ne!(view.entries[0].reference.incarnation, old.incarnation);
    }
}

#[test]
fn opt_in_v2_explains_and_retires_durably_without_weakening_read_only_policy() {
    use chronos_client::WireClientV2;
    use chronos_wire::coordination_v2 as v2;
    let directory = Directory::new();
    let server = Running::start(&directory.0, true);
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client.request(submit()).unwrap().outcome,
        Outcome::Applied {}
    ));
    let v2::Outcome::Explanation { value: Some(value) } = client
        .request_v2(v2::Request {
            version: 2,
            command: v2::Command::Explain { task: "a".into() },
        })
        .unwrap()
        .outcome
    else {
        panic!("explanation expected")
    };
    assert!(matches!(
        value.readiness,
        v2::Readiness::ReadyForAuthorizedClaim {}
    ));
    assert!(!value.retirement_eligible);
    assert!(matches!(
        client
            .request(Request {
                version: 1,
                command: Command::Cancel {
                    expected: value.revision,
                    task: value.entry.reference
                }
            })
            .unwrap()
            .outcome,
        Outcome::Applied {}
    ));
    drop(server);
    let server = Running::start(&directory.0, false);
    let v2::Outcome::Explanation { value: Some(value) } = client
        .request_v2(v2::Request {
            version: 2,
            command: v2::Command::Explain { task: "a".into() },
        })
        .unwrap()
        .outcome
    else {
        panic!("explanation expected")
    };
    assert!(value.retirement_eligible);
    assert!(matches!(
        client
            .request_v2(v2::Request {
                version: 2,
                command: v2::Command::Retire {
                    expected: value.revision.clone(),
                    task: value.entry.reference.clone()
                }
            })
            .unwrap()
            .outcome,
        v2::Outcome::Refused {
            failure: v2::RetirementFailure::Common {
                failure: Failure::Refused {
                    code: FailureCode::Unauthorized
                }
            }
        }
    ));
    drop(server);
    let server = Running::start(&directory.0, true);
    assert!(matches!(
        client
            .request_v2(v2::Request {
                version: 2,
                command: v2::Command::Retire {
                    expected: value.revision,
                    task: value.entry.reference
                }
            })
            .unwrap()
            .outcome,
        v2::Outcome::Applied {}
    ));
    drop(server);
    let _server = Running::start(&directory.0, false);
    assert!(matches!(
        client
            .request_v2(v2::Request {
                version: 2,
                command: v2::Command::Explain { task: "a".into() }
            })
            .unwrap()
            .outcome,
        v2::Outcome::Explanation { value: None }
    ));
}
fn submit() -> Request {
    Request {
        version: 1,
        command: Command::SubmitGraph {
            expected: Revision::Finite("0".into()),
            tasks: vec![TaskSpec {
                id: "a".into(),
                priority: 1,
                dependencies: vec![],
                capabilities: vec![],
                max_attempts: 2,
                retry_safety: RetrySafety::FenceUnknown,
            }],
        },
    }
}

#[test]
fn actual_binary_round_trip_restart_and_stale_mutation() {
    let directory = Directory::new();
    let process = Running::start(&directory.0, true);
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client.request(submit()).unwrap().outcome,
        Outcome::Applied {}
    ));
    drop(process);
    let _restarted = Running::start(&directory.0, true);
    assert!(
        matches!(client.request(revision()).unwrap().outcome, Outcome::Revision { revision: Revision::Finite(value) } if value == "1")
    );
    assert!(matches!(
        client.request(submit()).unwrap().outcome,
        Outcome::Refused {
            failure: Failure::RevisionMismatch { .. }
        }
    ));
}

#[test]
fn process_defaults_to_read_only_and_callers_cannot_elevate() {
    let directory = Directory::new();
    let _process = Running::start(&directory.0, false);
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client.request(submit()).unwrap().outcome,
        Outcome::Refused {
            failure: Failure::Refused {
                code: FailureCode::Unauthorized
            }
        }
    ));
    assert!(
        matches!(client.request(revision()).unwrap().outcome, Outcome::Revision { revision: Revision::Finite(value) } if value == "0")
    );
}

#[test]
fn second_writer_refuses_without_removing_first_socket() {
    let directory = Directory::new();
    let _process = Running::start(&directory.0, true);
    assert!(WriterOwnership::acquire(&directory.0).is_err());
    assert!(chronos_standalone::Server::bind(&directory.0, true).is_err());
    assert!(LocalClient::new(&directory.0).request(revision()).is_ok());
}

#[test]
fn public_directory_and_socket_destination_symlink_are_refused() {
    let directory = Directory::new();
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(chronos_standalone::Server::bind(&directory.0, true).is_err());
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink("absent", chronos_local::socket_path(&directory.0)).unwrap();
    assert!(chronos_standalone::Server::bind(&directory.0, true).is_err());
}

#[test]
fn lock_symlink_refuses_without_touching_destination() {
    let directory = Directory::new();
    let target = directory.0.join("original");
    std::fs::write(&target, b"unchanged").unwrap();
    std::os::unix::fs::symlink(&target, directory.0.join("writer.lock")).unwrap();
    assert!(WriterOwnership::acquire(&directory.0).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"unchanged");
}

#[test]
fn malformed_and_oversized_connection_does_not_stop_service() {
    use std::io::Write;
    let directory = Directory::new();
    let _process = Running::start(&directory.0, false);
    let mut stream =
        std::os::unix::net::UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
    stream.write_all(b"not-json\n").unwrap();
    let frame = chronos_wire::read_frame(
        &mut BufReader::new(stream),
        chronos_local::MAX_RESPONSE_BYTES,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        chronos_wire::decode_response(&frame, chronos_local::MAX_RESPONSE_BYTES)
            .unwrap()
            .outcome,
        Outcome::Refused {
            failure: Failure::Refused {
                code: FailureCode::InvalidRequest
            }
        }
    ));
    let mut stream =
        std::os::unix::net::UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
    let _ = stream.write_all(&vec![b'x'; chronos_local::MAX_REQUEST_BYTES + 1]);
    drop(stream);
    assert!(LocalClient::new(&directory.0).request(revision()).is_ok());
}

#[test]
fn startup_persists_initial_state_and_missing_initialized_state_refuses_reopen() {
    use chronos_host::CheckpointStore;
    let directory = Directory::new();
    let server = chronos_standalone::Server::bind(&directory.0, false).unwrap();
    let store = chronos_adapter_checkpoint::DirectoryCheckpointStore::open(
        &directory.0,
        8 * 1024 * 1024,
        4096,
    )
    .unwrap();
    assert!(
        store.load().unwrap().is_some(),
        "readiness requires an initialized checkpoint"
    );
    drop(server);
    // Deliberately remove only known checkpoint fixtures, retaining writer.lock.
    for entry in std::fs::read_dir(&directory.0).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_str().unwrap();
        if name.starts_with("checkpoint-") && name.ends_with(".bin") {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
    assert!(chronos_standalone::Server::bind(&directory.0, false).is_err());
    assert!(!chronos_local::socket_path(&directory.0).exists());
}

#[test]
fn dropped_server_does_not_unlink_a_replacement_socket() {
    let directory = Directory::new();
    let server = chronos_standalone::Server::bind(&directory.0, false).unwrap();
    let path = chronos_local::socket_path(&directory.0);
    std::fs::remove_file(&path).unwrap();
    let _replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    drop(server);
    assert!(
        path.exists(),
        "Drop must not unlink a different socket inode"
    );
}

#[test]
fn fresh_lock_does_not_authorize_removing_a_preexisting_socket() {
    let directory = Directory::new();
    let path = chronos_local::socket_path(&directory.0);
    let _other = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(chronos_standalone::Server::bind(&directory.0, false).is_err());
    assert!(path.exists());
}

#[test]
fn lost_writer_identity_fences_before_accept_or_dispatch_and_does_not_clean_up() {
    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind(&directory.0, true).unwrap();
    std::fs::rename(
        directory.0.join("writer.lock"),
        directory.0.join("held.lock"),
    )
    .unwrap();
    let _other = WriterOwnership::acquire(&directory.0).unwrap();
    assert!(server.serve_one().is_err());
    assert!(server.signals().fenced);
    assert_eq!(server.signals().ownership_failures, 1);
    let mut observed = server.signals();
    observed.fenced = false;
    observed.ownership_failures = 0;
    assert!(!observed.fenced);
    assert!(
        server.signals().fenced,
        "editing an observation cannot clear the operational fence"
    );
    assert!(server.serve_one().is_err());
    drop(server);
    assert!(chronos_local::socket_path(&directory.0).exists());
}

#[test]
fn replaced_socket_fences_before_accept_and_is_not_removed() {
    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind(&directory.0, false).unwrap();
    let path = chronos_local::socket_path(&directory.0);
    std::fs::rename(&path, directory.0.join("previous.sock")).unwrap();
    let _replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(server.serve_one().is_err());
    assert!(server.signals().fenced);
    assert_eq!(server.signals().accepted, 0);
    drop(server);
    assert!(path.exists());
}

#[test]
fn renamed_directory_is_not_retargeted_to_its_replacement() {
    let directory = Directory::new();
    let root = directory.0.join("state");
    let moved = directory.0.join("moved");
    let mut server = chronos_standalone::Server::bind(&root, false).unwrap();
    std::fs::rename(&root, &moved).unwrap();
    chronos_standalone::ownership::create_private_directory(&root).unwrap();
    let path = chronos_local::socket_path(&root);
    let replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(server.serve_one().is_err());
    assert!(server.signals().fenced);
    drop(server);
    assert!(path.exists());
    drop(replacement);
    for fixture_root in [&root, &moved] {
        for entry in std::fs::read_dir(fixture_root).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(fixture_root).unwrap();
    }
}

fn wait_clean_exit(process: &mut Running) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = process.0.try_wait().unwrap() {
            assert!(
                status.success(),
                "service did not drain cleanly: {status:?}"
            );
            return;
        }
        assert!(std::time::Instant::now() < deadline, "service did not stop");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn operator_status_maintenance_and_drain_preserve_domain_state_and_unknown_files() {
    use chronos_local::control::{CommandV1, LifecycleV1, MaintenanceV1, request};
    let directory = Directory::new();
    let mut process = Running::start(&directory.0, true);
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client.request(submit()).unwrap().outcome,
        Outcome::Applied {}
    ));
    let status = request(&directory.0, CommandV1::Status).unwrap();
    assert_eq!(status.lifecycle, LifecycleV1::Running);
    assert!(status.ready);
    assert_eq!(status.obligations, 0);
    std::fs::write(directory.0.join(".chronos-tmp-unknown"), b"retain this").unwrap();
    let maintained = request(&directory.0, CommandV1::Maintain).unwrap();
    assert_eq!(maintained.maintenance, MaintenanceV1::Inspected);
    assert_eq!(maintained.temporary, 1);
    assert_eq!(
        std::fs::read(directory.0.join(".chronos-tmp-unknown")).unwrap(),
        b"retain this"
    );
    let receipt = request(&directory.0, CommandV1::Drain).unwrap();
    assert_eq!(receipt.lifecycle, LifecycleV1::Draining);
    assert!(!receipt.ready);
    wait_clean_exit(&mut process);
    assert!(!chronos_local::socket_path(&directory.0).exists());
    assert!(!chronos_local::control_path(&directory.0).exists());
    let mut restarted = Running::start(&directory.0, false);
    assert!(
        matches!(client.request(revision()).unwrap().outcome,Outcome::Revision {revision:Revision::Finite(value)} if value=="1")
    );
    request(&directory.0, CommandV1::Drain).unwrap();
    wait_clean_exit(&mut restarted);
}

#[test]
fn malformed_control_prefix_or_version_cannot_request_a_drain() {
    use chronos_local::control::{CommandV1, LifecycleV1, request};
    use std::{
        io::{Read, Write},
        net::Shutdown,
    };
    let directory = Directory::new();
    let mut process = Running::start(&directory.0, false);
    for bytes in [b"CC\x01\x01extra".as_slice(), b"CC\x02\x01", b"CC\x01\xff"] {
        let mut stream =
            std::os::unix::net::UnixStream::connect(chronos_local::control_path(&directory.0))
                .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(bytes).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut response = [0; 40];
        let _ = stream.read(&mut response);
        let status = request(&directory.0, CommandV1::Status).unwrap();
        assert_eq!(status.lifecycle, LifecycleV1::Running);
        assert!(status.ready);
    }
    request(&directory.0, CommandV1::Drain).unwrap();
    wait_clean_exit(&mut process);
}

#[test]
fn lost_drain_reply_still_stops_without_replaying_task_work() {
    use std::{io::Write, net::Shutdown};
    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind(&directory.0, false).unwrap();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = server.run();
        let signals = server.signals();
        drop(server);
        send.send((result, signals)).unwrap();
    });
    let mut stream =
        std::os::unix::net::UnixStream::connect(chronos_local::control_path(&directory.0)).unwrap();
    // Retain the connection for intake, but make its response unreadable.
    stream.shutdown(Shutdown::Read).unwrap();
    stream
        .write_all(&chronos_local::control::CommandV1::Drain.encode())
        .unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let (result, signals) = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(result, Ok(()));
    assert_eq!(signals.drain_requests, 1);
    assert!(!signals.fenced);
    worker.join().unwrap();
}

#[test]
fn disconnected_data_and_control_peers_do_not_fence_the_service() {
    use chronos_local::control::{CommandV1, request};
    let directory = Directory::new();
    let mut process = Running::start(&directory.0, false);
    for path in [
        chronos_local::socket_path(&directory.0),
        chronos_local::control_path(&directory.0),
    ] {
        drop(std::os::unix::net::UnixStream::connect(path).unwrap());
    }
    assert!(request(&directory.0, CommandV1::Status).unwrap().ready);
    assert!(LocalClient::new(&directory.0).request(revision()).is_ok());
    request(&directory.0, CommandV1::Drain).unwrap();
    wait_clean_exit(&mut process);
}

#[test]
fn sigterm_wakes_idle_os_wait_and_performs_graceful_cleanup() {
    let directory = Directory::new();
    let mut process = Running::start(&directory.0, false);
    assert!(process.0.try_wait().unwrap().is_none());
    let pid = rustix::process::Pid::from_raw(i32::try_from(process.0.id()).unwrap()).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
    wait_clean_exit(&mut process);
    assert!(!chronos_local::socket_path(&directory.0).exists());
    assert!(!chronos_local::control_path(&directory.0).exists());
    assert!(directory.0.join("writer.lock").exists());
}
