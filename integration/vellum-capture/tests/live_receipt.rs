#![cfg(unix)]

use chronos_client::WireClientV3;
use chronos_local::LocalClient;
use chronos_protocol::{RetrySafety, Revision, TaskSpec, coordination_v3 as wire};
use chronos_sdk_embedded::{
    engine::StateRevision,
    host::{
        native_allocation::{AllocationClientV1, AllocationReceiptV1, AllocationResultV1},
        native_v3::InvocationV1,
    },
};
use chronos_vellum_capture::retained::{ExactTask, RetainedReceiptClient};
use std::{
    ffi::OsStr,
    io::{BufRead, BufReader},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Directory(PathBuf);

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Daemon(Child);

impl Daemon {
    fn start(binary: &OsStr, directory: &Path, options: &[&str]) -> Self {
        let mut daemon = Self(
            Command::new(binary)
                .arg(directory)
                .args(options)
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let stderr = daemon.0.stderr.take().unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let _ = send.send(line);
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
        });
        let startup = receive.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(startup.contains("ready"), "{startup}");
        daemon
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires Unix socket access and CHRONOS_TEST_DAEMON pointing to chronos-service"]
fn read_only_handoff_reader_recovers_original_receipt_after_daemon_restart() {
    let binary = std::env::var_os("CHRONOS_TEST_DAEMON").expect("set CHRONOS_TEST_DAEMON");
    let path = PathBuf::from(format!("/tmp/ch-receipt-{}", std::process::id()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let directory = Directory(path);
    let invocation = || InvocationV1 {
        serial: 1,
        key: "handoff-test".into(),
    };
    let mut reader = RetainedReceiptClient::new(
        "local-test-scope".into(),
        LocalClient::new(&directory.0),
        chronos_local::wire_limits(),
    );

    let daemon = Daemon::start(
        &binary,
        &directory.0,
        &["--atomic-state", "--allocation-receipts", "--allow-writes"],
    );
    submit_draft(&directory.0);
    let original = reader.allocation(invocation()).unwrap().unwrap();
    let AllocationResultV1::Created(AllocationReceiptV1 { revision, tasks }) = &original else {
        panic!("expected original allocation");
    };
    assert_eq!(*revision, StateRevision::finite(1));
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_id(), &"draft".into());
    drop(daemon);

    let _daemon = Daemon::start(&binary, &directory.0, &["--atomic-state"]);
    assert_eq!(reader.allocation(invocation()).unwrap().unwrap(), original);
    let reference = chronos_protocol::Reference {
        task: "draft".into(),
        incarnation: tasks[0].incarnation().get().to_string(),
    };
    assert!(matches!(
        reader.exact_task(&reference, &Revision::Finite("1".into())).unwrap(),
        ExactTask::Current(snapshot) if snapshot.reference == tasks[0]
    ));
    assert!(matches!(
        reader
            .exact_task(&reference, &Revision::Finite("0".into()))
            .unwrap(),
        ExactTask::Stale
    ));
}

fn submit_draft(directory: &Path) {
    let mut client = LocalClient::new(directory);
    let reply = client
        .request_v3(wire::Request {
            version: 3,
            command: wire::Command::ExecuteInvocation {
                invocation: wire::Invocation {
                    serial: "1".into(),
                    key: "handoff-test".into(),
                },
                command: wire::Mutation::SubmitGraph {
                    expected: Revision::Finite("0".into()),
                    tasks: vec![TaskSpec {
                        id: "draft".into(),
                        priority: 0,
                        dependencies: vec![],
                        capabilities: vec![],
                        max_attempts: 1,
                        retry_safety: RetrySafety::FenceUnknown,
                    }],
                },
            },
        })
        .unwrap();
    assert!(matches!(
        reply.outcome,
        wire::Outcome::InvocationResult {
            result: wire::MutationResult::Applied {},
            ..
        }
    ));
}

#[cfg(all(target_os = "macos", feature = "workbench"))]
#[test]
#[ignore = "requires tmux, Unix socket access and CHRONOS_TEST_DAEMON"]
fn hosted_workbench_exports_and_reviews_from_tmux() {
    use chronos_sdk_embedded::engine::TaskSpecV2;
    use chronos_vellum_capture::{
        CaptureJournalV1, IntentV1, LimitsV1, PhaseV1, RecordV1, SourceReferenceV1,
        journal::DirectoryCaptureJournalV1,
    };
    use keystone_storage::Revision as JournalRevision;
    use vellum_host::annotation::{DocumentIdentity, SourceEvidence, Target};

    let binary = std::env::var_os("CHRONOS_TEST_DAEMON").expect("set CHRONOS_TEST_DAEMON");
    let root = PathBuf::from(format!("/tmp/ch-workbench-{}", std::process::id()));
    let private = |path: &Path| {
        std::fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    };
    private(&root);
    let root = Directory(root);
    let chronos = root.0.join("chronos");
    let journal = root.0.join("journal");
    let output = root.0.join("output");
    for path in [&chronos, &journal, &output] {
        private(path);
    }
    let source = root.0.join("draft.txt");
    let original = b"\xef\xbb\xbfalpha\r\n";
    std::fs::write(&source, original).unwrap();
    let _daemon = Daemon::start(
        &binary,
        &chronos,
        &["--atomic-state", "--allocation-receipts", "--allow-writes"],
    );
    submit_draft(&chronos);
    let invocation = InvocationV1 {
        serial: 1,
        key: "handoff-test".into(),
    };
    let scope = "tmux-chronos";
    let mut reader = RetainedReceiptClient::new(
        scope.into(),
        LocalClient::new(&chronos),
        chronos_local::wire_limits(),
    );
    let AllocationResultV1::Created(receipt) =
        reader.allocation(invocation.clone()).unwrap().unwrap()
    else {
        panic!("expected retained allocation");
    };
    let limits = LimitsV1 {
        max_field_bytes: 4096,
        max_record_bytes: 256 * 1024,
        max_dependencies: 128,
        max_capabilities: 64,
    };
    let intent = IntentV1 {
        source: SourceReferenceV1 {
            document: DocumentIdentity {
                namespace: "fixture".into(),
                value: "draft".into(),
            },
            target: Target::Range {
                source: SourceEvidence {
                    domain: "fixture".into(),
                    token: "original".into(),
                    byte_length: 6,
                },
                start: 0,
                end: 5,
            },
        },
        service_scope: scope.into(),
        invocation,
        expected: StateRevision::ZERO,
        task: TaskSpecV2::new("draft"),
    };
    let mut store = DirectoryCaptureJournalV1::open(&journal, limits, 16).unwrap();
    for (revision, phase) in [
        PhaseV1::Prepared,
        PhaseV1::Dispatched,
        PhaseV1::Associated(receipt),
    ]
    .into_iter()
    .enumerate()
    {
        let revision = revision as u64;
        store
            .commit(
                revision.checked_sub(1).map(JournalRevision),
                &RecordV1 {
                    revision,
                    intent: intent.clone(),
                    phase,
                },
            )
            .unwrap();
    }
    drop(store);
    let socket = root.0.join("tmux.sock");
    struct Terminal(PathBuf);
    impl Terminal {
        fn call(&self, args: &[&str]) -> std::process::Output {
            Command::new("tmux")
                .arg("-S")
                .arg(&self.0)
                .args(["-f", "/dev/null"])
                .args(args)
                .output()
                .unwrap()
        }
        fn expect(&self, marker: &str) {
            let stop = std::time::Instant::now() + Duration::from_secs(8);
            loop {
                let captured = self.call(&["capture-pane", "-p", "-t", "handoff"]);
                let screen = String::from_utf8_lossy(&captured.stdout);
                if screen.contains(marker) {
                    return;
                }
                assert!(
                    std::time::Instant::now() < stop,
                    "missing {marker:?}: {screen}\n{}",
                    String::from_utf8_lossy(&captured.stderr)
                );
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    impl Drop for Terminal {
        fn drop(&mut self) {
            let _ = self.call(&["kill-server"]);
        }
    }
    let terminal = Terminal(socket);
    let launch = terminal.call(&[
        "new-session",
        "-d",
        "-x",
        "190",
        "-y",
        "36",
        "-s",
        "handoff",
        "/bin/sh",
        "-c",
        "exec \"$@\"",
        "sh",
        env!("CARGO_BIN_EXE_vellum-chronos-workbench"),
        chronos.to_str().unwrap(),
        journal.to_str().unwrap(),
        source.to_str().unwrap(),
        output.to_str().unwrap(),
        scope,
        "fixture",
        "draft",
    ]);
    assert!(
        launch.status.success(),
        "{}",
        String::from_utf8_lossy(&launch.stderr)
    );
    terminal.expect("Observed 1 tasks");
    terminal.call(&["send-keys", "-t", "handoff", "Tab", "X", "Tab"]);
    terminal.expect("Xalpha");
    terminal.call(&["send-keys", "-t", "handoff", "2", "h", "Enter"]);
    terminal.expect("Handoff exported.");
    let work = output.join("handoff/work.txt");
    assert_eq!(std::fs::read(&work).unwrap(), b"\xef\xbb\xbfXalpha\r\n");
    std::fs::write(work, b"\xef\xbb\xbfXbeta\r\n").unwrap();
    terminal.call(&["send-keys", "-t", "handoff", "v"]);
    terminal.expect("Review ready in the handoff host.");
    let report = std::fs::read_to_string(output.join("handoff/review.txt")).unwrap();
    assert!(report.contains("Text Proposed"), "{report}");
    assert_eq!(std::fs::read(source).unwrap(), original);
    terminal.call(&["send-keys", "-t", "handoff", "C-q", "C-q"]);
}
