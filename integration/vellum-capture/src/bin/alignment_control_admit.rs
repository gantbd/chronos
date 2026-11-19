#![forbid(unsafe_code)]

#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    eprintln!("Local Chronos admission requires macOS.");
    std::process::ExitCode::FAILURE
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use chronos_client::{WireClientV1, WireClientV3};
    use chronos_local::LocalClient;
    use chronos_protocol::{
        Command, Outcome, Request, RetrySafety, Revision, TaskSpec, coordination_v3 as v3,
    };
    use serde_json::json;
    use std::path::PathBuf;

    let mut args = std::env::args_os().skip(1);
    let directory = PathBuf::from(args.next().ok_or("Chronos directory required")?);
    let task = args
        .next()
        .ok_or("control task required")?
        .into_string()
        .map_err(|_| "control task must be UTF-8")?;
    if args.next().is_some() || !directory.is_absolute() || task.is_empty() {
        return Err("usage: chronos-alignment-control-admit CHRONOS_DIR CONTROL_TASK".into());
    }

    let mut client = LocalClient::new(&directory);
    let reply = client
        .request_v3(v3::Request {
            version: 3,
            command: v3::Command::ExecuteInvocation {
                invocation: v3::Invocation {
                    serial: "1".into(),
                    key: "product-alignment-control-admission".into(),
                },
                command: v3::Mutation::SubmitGraph {
                    expected: Revision::Finite("0".into()),
                    tasks: vec![TaskSpec {
                        id: task.clone(),
                        priority: 10,
                        dependencies: Vec::new(),
                        capabilities: vec!["product-alignment".into()],
                        max_attempts: 1,
                        retry_safety: RetrySafety::FenceUnknown,
                    }],
                },
            },
        })
        .map_err(|_| "Chronos control-task admission unavailable")?;
    if !matches!(
        reply.outcome,
        v3::Outcome::InvocationResult {
            result: v3::MutationResult::Applied {},
            ..
        }
    ) {
        return Err("Chronos control-task admission refused".into());
    }
    let snapshot = WireClientV1::request(
        &mut client,
        Request {
            version: 1,
            command: Command::Snapshot { task: task.clone() },
        },
    )
    .map_err(|_| "Chronos control-task observation unavailable")?;
    let Outcome::Snapshot {
        value: Some(entry),
        revision: Some(revision),
    } = snapshot.outcome
    else {
        return Err("Chronos control-task observation invalid".into());
    };
    println!(
        "{}",
        serde_json::to_string(&json!({
            "version": 1,
            "task": entry.reference.task,
            "incarnation": entry.reference.incarnation,
            "revision": match revision {
                Revision::Finite(value) => value,
                Revision::Exhausted => "exhausted".into(),
            }
        }))?
    );
    Ok(())
}
