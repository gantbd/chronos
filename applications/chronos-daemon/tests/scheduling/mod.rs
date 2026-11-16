use super::*;
use chronos_client::SchedulingWireClientV5;
use chronos_wire::{Reference, scheduling_v5 as v5};
fn command(command: v5::Command) -> v5::Request {
    v5::Request {
        version: 5,
        command,
    }
}
fn revision(value: u64) -> Revision {
    Revision::Finite(value.to_string())
}
fn overview() -> v5::Request {
    command(v5::Command::Overview {
        now: v5::Timestamp {
            before_epoch: false,
            seconds: "0".into(),
            nanos: 0,
        },
    })
}
fn applied(reply: v5::Response) {
    assert!(matches!(
        reply.outcome,
        v5::Outcome::Control {
            result: v5::ControlResult::Applied {}
        }
    ));
}
#[test]
fn guarded_scheduling_controls_reconcile_discarded_reply_and_survive_readonly_process_restart() {
    use std::io::Write;
    let directory = Directory::new();
    let server = Running::start_flags(&directory.0, &["--atomic-state", "--allow-writes"]);
    let mut client = LocalClient::new(&directory.0);
    applied(
        client
            .request_scheduling_v5(command(v5::Command::Enable {
                expected: revision(0),
            }))
            .unwrap(),
    );
    applied(
        client
            .request_scheduling_v5(command(v5::Command::SetConcurrencyLimit {
                expected: revision(1),
                limit: 2,
            }))
            .unwrap(),
    );
    let mut submit = super::submit();
    let Command::SubmitGraph { expected, .. } = &mut submit.command else {
        panic!("graph");
    };
    *expected = revision(2);
    assert!(matches!(
        client.request(submit).unwrap().outcome,
        Outcome::Applied {}
    ));
    let update = command(v5::Command::SetEligibility {
        expected: revision(3),
        task: Reference {
            task: "a".into(),
            incarnation: "0".into(),
        },
        eligibility: v5::Eligibility {
            paused: true,
            not_before: Some(v5::Timestamp {
                before_epoch: false,
                seconds: "100000".into(),
                nanos: 1,
            }),
        },
    });
    let mut stream =
        std::os::unix::net::UnixStream::connect(chronos_local::socket_path(&directory.0)).unwrap();
    stream.shutdown(std::net::Shutdown::Read).unwrap();
    stream
        .write_all(&chronos_wire::encode(&update, chronos_local::MAX_REQUEST_BYTES).unwrap())
        .unwrap();
    stream.write_all(b"\n").unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    // Discard the reply while retaining the half-closed connection, avoiding
    // a connection reset racing away the intended delivered-request fault.
    let v5::Outcome::Overview {
        revision: observed,
        controls: Some(controls),
        concurrency: Some(usage),
    } = client.request_scheduling_v5(overview()).unwrap().outcome
    else {
        panic!("overview");
    };
    assert!(matches!(observed, Revision::Finite(value) if value == "4"));
    drop(stream);
    assert!(!controls.paused);
    assert_eq!(usage.capacity, 2);
    let request = || {
        command(v5::Command::Eligibility {
            task: Reference {
                task: "a".into(),
                incarnation: "0".into(),
            },
        })
    };
    let v5::Outcome::Eligibility {
        eligibility: Some(value),
        ..
    } = client.request_scheduling_v5(request()).unwrap().outcome
    else {
        panic!("eligibility");
    };
    assert!(value.paused);
    assert_eq!(value.not_before.unwrap().seconds, "100000");
    assert!(matches!(
        client.request_scheduling_v5(update).unwrap().outcome,
        v5::Outcome::Control {
            result: v5::ControlResult::RevisionMismatch { .. }
        }
    ));
    applied(
        client
            .request_scheduling_v5(command(v5::Command::SetPaused {
                expected: revision(4),
                paused: true,
            }))
            .unwrap(),
    );
    drop(server);
    let _readonly = Running::start_flags(&directory.0, &["--atomic-state"]);
    let v5::Outcome::Overview {
        revision: observed,
        controls: Some(controls),
        concurrency: Some(usage),
    } = client.request_scheduling_v5(overview()).unwrap().outcome
    else {
        panic!("recovered controls");
    };
    assert!(matches!(observed, Revision::Finite(value) if value == "5"));
    assert!(controls.paused);
    assert_eq!((usage.capacity, usage.reserved, usage.available), (2, 0, 2));
    let v5::Outcome::Eligibility {
        eligibility: Some(value),
        ..
    } = client.request_scheduling_v5(request()).unwrap().outcome
    else {
        panic!("recovered eligibility");
    };
    assert!(value.paused);
    assert_eq!(value.not_before.unwrap().nanos, 1);
    assert!(matches!(
        client
            .request_scheduling_v5(command(v5::Command::SetPaused {
                expected: revision(5),
                paused: false
            }))
            .unwrap()
            .outcome,
        v5::Outcome::Refused {
            code: v5::RefusalCode::Unauthorized
        }
    ));
}
#[test]
fn legacy_profile_refuses_scheduling_without_implicit_migration() {
    let directory = Directory::new();
    let _server = Running::start(&directory.0, true);
    let mut client = LocalClient::new(&directory.0);
    assert!(matches!(
        client.request_scheduling_v5(overview()).unwrap().outcome,
        v5::Outcome::Refused {
            code: v5::RefusalCode::Unavailable
        }
    ));
    assert!(!directory.0.join(".chronos-envelope-v1").exists());
}
#[test]
fn local_scheduling_exposes_existing_stage_signals_and_never_grants_stop_attestation() {
    use chronos_service::scheduling::SchedulingAuthorizerV1;
    let mut engine = chronos_engine::Engine::new();
    engine.submit(chronos_engine::Task::new("a", "")).unwrap();
    let now = std::time::SystemTime::UNIX_EPOCH;
    let grant = engine
        .issue_grant(
            chronos_engine::Agent::new("fixture"),
            now,
            Duration::from_secs(60),
        )
        .unwrap();
    let task = engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let attempt = task.attempt_reference().unwrap();
    let mut policy = chronos_standalone::OperatorPolicy { allow_writes: true };
    assert!(
        !policy.allows_scheduling(&chronos_host::scheduling::SchedulingRequestV1::Control {
            expected: chronos_engine::StateRevision::Finite(0),
            command: chronos_engine::SchedulingCommandV1::ConfirmQuiescent { attempt }
        })
    );
    let directory = Directory::new();
    let mut server = chronos_standalone::Server::bind_with_options(
        &directory.0,
        chronos_standalone::ServerOptions {
            atomic_state: true,
            allow_writes: true,
            ..Default::default()
        },
    )
    .unwrap();
    let path = directory.0.clone();
    let client = std::thread::spawn(move || {
        LocalClient::new(path)
            .request_scheduling_v5(overview())
            .unwrap()
    });
    server.serve_one().unwrap();
    assert!(matches!(
        client.join().unwrap().outcome,
        v5::Outcome::Overview { controls: None, .. }
    ));
    let signals = server.runtime_signals().scheduling.unwrap();
    assert_eq!(signals.calls, 1);
    assert_eq!(signals.queries, 1);
    assert_eq!(signals.residence_buckets.iter().sum::<u64>(), 1);
    assert_eq!(signals.stages[0].samples, 1);
    assert_eq!(signals.stages[1].samples, 1);
    assert_eq!(signals.stages[2].samples, 0); // no persistence stage for a query
}
