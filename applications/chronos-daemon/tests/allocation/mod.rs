use super::*;
use chronos_client::{WireClientV2, WireClientV3, WireClientV4};
use chronos_wire::{coordination_v3 as v3, coordination_v4 as v4};
fn lookup() -> v4::Request {
    v4::Request {
        version: 4,
        command: v4::Command::LookupAllocation {
            serial: "1".into(),
            key: "original-intent".into(),
        },
    }
}
fn submit_original(client: &mut LocalClient) {
    let v1 = super::submit();
    let Command::SubmitGraph { expected, tasks } = v1.command else {
        panic!("graph")
    };
    let reply = client
        .request_v3(v3::Request {
            version: 3,
            command: v3::Command::ExecuteInvocation {
                invocation: v3::Invocation {
                    serial: "1".into(),
                    key: "original-intent".into(),
                },
                command: v3::Mutation::SubmitGraph { expected, tasks },
            },
        })
        .unwrap();
    assert!(matches!(
        reply.outcome,
        v3::Outcome::InvocationResult {
            result: v3::MutationResult::Applied {},
            ..
        }
    ));
}
#[test]
fn original_allocation_survives_retirement_name_reuse_and_readonly_process_restart() {
    let directory = Directory::new();
    let server = Running::start_flags(
        &directory.0,
        &["--atomic-state", "--allocation-receipts", "--allow-writes"],
    );
    let mut client = LocalClient::new(&directory.0);
    submit_original(&mut client);
    let v4::Outcome::Allocation {
        revision, tasks, ..
    } = client.request_v4(lookup()).unwrap().outcome
    else {
        panic!("allocation")
    };
    assert!(matches!(revision, Revision::Finite(ref value) if value == "1"));
    assert_eq!(tasks.len(), 1);
    let original = tasks[0].clone();
    client
        .request(Request {
            version: 1,
            command: Command::Cancel {
                expected: Revision::Finite("1".into()),
                task: original.clone(),
            },
        })
        .unwrap();
    let retired = client
        .request_v2(chronos_wire::coordination_v2::Request {
            version: 2,
            command: chronos_wire::coordination_v2::Command::Retire {
                expected: Revision::Finite("2".into()),
                task: original.clone(),
            },
        })
        .unwrap();
    assert!(matches!(
        retired.outcome,
        chronos_wire::coordination_v2::Outcome::Applied {}
    ));
    let mut recreate = super::submit();
    let Command::SubmitGraph { expected, .. } = &mut recreate.command else {
        panic!("graph")
    };
    *expected = Revision::Finite("3".into());
    assert!(matches!(
        client.request(recreate).unwrap().outcome,
        Outcome::Applied {}
    ));
    let current = client
        .request(Request {
            version: 1,
            command: Command::Snapshot {
                task: original.task.clone(),
            },
        })
        .unwrap();
    let Outcome::Snapshot {
        value: Some(current),
        ..
    } = current.outcome
    else {
        panic!("current")
    };
    assert_ne!(current.reference.incarnation, original.incarnation);
    drop(server);
    let _server = Running::start_flags(&directory.0, &["--atomic-state"]);
    let before = std::fs::read_dir(&directory.0).unwrap().count();
    let v4::Outcome::Allocation {
        revision, tasks, ..
    } = client.request_v4(lookup()).unwrap().outcome
    else {
        panic!("original retained allocation")
    };
    assert_eq!(tasks[0].incarnation, original.incarnation);
    assert_eq!(tasks[0].task, original.task);
    assert!(matches!(revision, Revision::Finite(ref value) if value == "1"));
    assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), before);
    assert!(matches!(
        client
            .request_v4(v4::Request {
                version: 4,
                command: v4::Command::LookupAllocation {
                    serial: "1".into(),
                    key: "changed-intent".into()
                }
            })
            .unwrap()
            .outcome,
        v4::Outcome::Refused {
            code: v4::RefusalCode::Conflict
        }
    ));
}
#[test]
fn legacy_outcomes_remain_uncaptured_after_explicit_upgrade() {
    let directory = Directory::new();
    let server = Running::start_flags(&directory.0, &["--atomic-state", "--allow-writes"]);
    let mut client = LocalClient::new(&directory.0);
    submit_original(&mut client);
    assert!(matches!(
        client.request_v4(lookup()).unwrap().outcome,
        v4::Outcome::Unavailable { .. }
    ));
    drop(server);
    let _server = Running::start_flags(&directory.0, &["--atomic-state", "--allocation-receipts"]);
    assert!(matches!(
        client.request_v4(lookup()).unwrap().outcome,
        v4::Outcome::Unavailable { .. }
    ));
}
