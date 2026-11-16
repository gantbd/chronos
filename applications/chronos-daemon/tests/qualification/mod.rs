//! Explicit source-candidate gates, not portable production latency promises.
use super::*;
use chronos_client::{
    WireClientV2,
    observer::{Observer, ObserverLimits},
};
use chronos_wire::coordination_v2 as v2;
use std::{
    sync::{Arc, Barrier},
    time::Instant,
};

fn current(client: &mut LocalClient) -> Revision {
    let Outcome::Revision { revision } = client.request(revision()).unwrap().outcome else {
        panic!("revision expected")
    };
    revision
}

fn task(index: usize, structural: bool) -> TaskSpec {
    let base = index / 128 * 128;
    let dependencies = if !structural || index == base {
        vec![]
    } else if index - base < 64 {
        vec![format!("t{}", index - 1)]
    } else {
        vec![format!("t{base}")]
    };
    TaskSpec {
        id: format!("t{index}"),
        priority: 1,
        dependencies,
        capabilities: vec![],
        max_attempts: 2,
        retry_safety: RetrySafety::FenceUnknown,
    }
}

fn populate(client: &mut LocalClient, count: usize, structural: bool) -> Vec<Duration> {
    let mut samples = Vec::new();
    for first in (0..count).step_by(128) {
        let expected = current(client);
        let started = Instant::now();
        assert!(matches!(
            client
                .request(Request {
                    version: 1,
                    command: Command::SubmitGraph {
                        expected,
                        tasks: (first..count.min(first + 128))
                            .map(|i| task(i, structural))
                            .collect()
                    }
                })
                .unwrap()
                .outcome,
            Outcome::Applied {}
        ));
        samples.push(started.elapsed());
    }
    samples
}

fn percentile(samples: &mut [Duration], percent: usize) -> Duration {
    assert!(!samples.is_empty());
    assert!((1..=100).contains(&percent));
    samples.sort_unstable();
    samples[(samples.len() * percent).div_ceil(100) - 1]
}

fn report(operation: &str, samples: &mut [Duration], budget: Option<(u64, u64)>) {
    let p50 = percentile(samples, 50);
    let p95 = percentile(samples, 95);
    let p99 = percentile(samples, 99);
    println!(
        "operation={operation} count={} p50_ns={} p95_ns={} p99_ns={} max_ns={}",
        samples.len(),
        p50.as_nanos(),
        p95.as_nanos(),
        p99.as_nanos(),
        samples.last().unwrap().as_nanos()
    );
    if let Some((p95_ms, p99_ms)) = budget {
        assert!(
            p95 <= Duration::from_millis(p95_ms),
            "{operation}: p95 budget exceeded"
        );
        assert!(
            p99 <= Duration::from_millis(p99_ms),
            "{operation}: p99 budget exceeded"
        );
    }
}

#[test]
fn nearest_rank_includes_the_tail_and_budget_rejects_slow_samples() {
    let mut samples: Vec<_> = (1..=100).rev().map(Duration::from_millis).collect();
    assert_eq!(percentile(&mut samples, 95), Duration::from_millis(95));
    assert_eq!(percentile(&mut samples, 99), Duration::from_millis(99));
    assert!(
        std::panic::catch_unwind(|| report(
            "synthetic",
            &mut [Duration::from_millis(6); 100],
            Some((5, 20))
        ))
        .is_err()
    );
    assert!(
        std::panic::catch_unwind(|| {
            let mut tail = [Duration::from_millis(1); 100];
            tail[98..].fill(Duration::from_millis(21));
            report("synthetic", &mut tail, Some((5, 20)));
        })
        .is_err()
    );
}

#[test]
#[ignore = "explicit optimized reference-host performance acceptance gate"]
fn local_candidate_performance_gate() {
    if cfg!(debug_assertions) {
        panic!("qualification requires --release");
    }
    println!(
        "profile=private_local_source tasks=1000 repeats=2 os={} arch={} optimized=true",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    for clients in [1, 4] {
        let mut reads = Vec::new();
        let mut explains = Vec::new();
        let mut admissions = Vec::new();
        let mut writes = Vec::new();
        let mut conflicts = Vec::new();
        let mut retires = Vec::new();
        for repetition in 0..2 {
            let directory = Directory::new();
            let server = Running::start_flags(
                &directory.0,
                &["--atomic-state", "--observer-history", "--allow-writes"],
            );
            let mut client = LocalClient::new(&directory.0);
            admissions.extend(populate(&mut client, 1000, true));
            // Warm connections/decoders before the read-only concurrent phase.
            for _ in 0..16 {
                let _ = current(&mut client);
            }
            let barrier = Arc::new(Barrier::new(clients));
            std::thread::scope(|scope| {
                let handles: Vec<_> = (0..clients)
                    .map(|_| {
                        let path = &directory.0;
                        let barrier = &barrier;
                        scope.spawn(move || {
                            let mut client = LocalClient::new(path);
                            barrier.wait();
                            (0..(1024 / clients))
                                .map(|_| {
                                    let started = Instant::now();
                                    let _ = current(&mut client);
                                    let read = started.elapsed();
                                    let started = Instant::now();
                                    assert!(matches!(
                                        client
                                            .request_v2(v2::Request {
                                                version: 2,
                                                command: v2::Command::Explain {
                                                    task: "t999".into()
                                                }
                                            })
                                            .unwrap()
                                            .outcome,
                                        v2::Outcome::Explanation { value: Some(_) }
                                    ));
                                    (read, started.elapsed())
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                for handle in handles {
                    for (read, explain) in handle.join().unwrap() {
                        reads.push(read);
                        explains.push(explain);
                    }
                }
            });
            // 256 distinct fan-out leaves: cancellation cannot invalidate another
            // candidate by dependency propagation. CAS races are not retried.
            let leaves: Vec<_> = (0..1000).filter(|i| i % 128 >= 64).take(256).collect();
            let barrier = Arc::new(Barrier::new(clients));
            let mut committed = Vec::new();
            std::thread::scope(|scope| {
                let handles: Vec<_> = (0..clients)
                    .map(|worker| {
                        let path = &directory.0;
                        let leaves = &leaves;
                        let barrier = &barrier;
                        scope.spawn(move || {
                            let mut client = LocalClient::new(path);
                            barrier.wait();
                            leaves
                                .iter()
                                .skip(worker)
                                .step_by(clients)
                                .map(|index| {
                                    let v2::Outcome::Explanation { value: Some(value) } = client
                                        .request_v2(v2::Request {
                                            version: 2,
                                            command: v2::Command::Explain {
                                                task: format!("t{index}"),
                                            },
                                        })
                                        .unwrap()
                                        .outcome
                                    else {
                                        panic!("explanation expected")
                                    };
                                    let reference = value.entry.reference;
                                    let started = Instant::now();
                                    let response = client
                                        .request(Request {
                                            version: 1,
                                            command: Command::Cancel {
                                                expected: value.revision,
                                                task: reference.clone(),
                                            },
                                        })
                                        .unwrap();
                                    let elapsed = started.elapsed();
                                    match response.outcome {
                                        Outcome::Applied {} => (elapsed, Some(reference)),
                                        Outcome::Refused {
                                            failure: Failure::RevisionMismatch { .. },
                                        } => (elapsed, None),
                                        _ => panic!("unexpected cancellation outcome"),
                                    }
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                for handle in handles {
                    for (elapsed, reference) in handle.join().unwrap() {
                        if let Some(reference) = reference {
                            writes.push(elapsed);
                            committed.push(reference);
                        } else {
                            conflicts.push(elapsed);
                        }
                    }
                }
            });
            assert!(!committed.is_empty());
            // Retirement is a separate sequential class, never pooled with CAS refusals.
            for task in committed {
                let expected = current(&mut client);
                let started = Instant::now();
                let result = client
                    .request_v2(v2::Request {
                        version: 2,
                        command: v2::Command::Retire { expected, task },
                    })
                    .unwrap();
                retires.push(started.elapsed());
                assert!(matches!(result.outcome, v2::Outcome::Applied {}));
            }
            let mut observer = Observer::new(ObserverLimits {
                page_size: 64,
                max_pages: 17,
                max_entries: 1000,
                max_id_bytes: 256,
            })
            .unwrap();
            let started = Instant::now();
            assert!(!observer.resync_feed(&mut client).unwrap());
            println!(
                "operation=observer_resync clients={clients} repetition={repetition} elapsed_ns={} entries={}",
                started.elapsed().as_nanos(),
                observer.view().unwrap().entries.len()
            );
            assert_eq!(observer.feed_signals().resyncs, 1);
            drop(server);
            let started = Instant::now();
            let _recovered = Running::start(&directory.0, false);
            let _ = current(&mut client);
            println!(
                "operation=recovery_to_first_reply clients={clients} repetition={repetition} elapsed_ns={}",
                started.elapsed().as_nanos()
            );
        }
        println!("phase_clients={clients} repeats=2 attempted_cancels=512");
        assert_eq!(writes.len() + conflicts.len(), 512);
        assert!(
            writes.len() >= 128,
            "insufficient successful durable samples"
        );
        // Successful durable results alone must pass; refusals cannot improve them.
        report("revision", &mut reads, Some((5, 20)));
        report("explain", &mut explains, Some((5, 20)));
        report("submit_graph_sequential", &mut admissions, Some((100, 250)));
        report("cancel_applied", &mut writes, Some((100, 250)));
        report("retire_applied_sequential", &mut retires, Some((100, 250)));
        if !conflicts.is_empty() {
            report("cancel_revision_conflict", &mut conflicts, None);
        }
    }
}

#[test]
#[ignore = "explicit optimized configured-capacity qualification"]
fn local_candidate_capacity_gate() {
    if cfg!(debug_assertions) {
        panic!("qualification requires --release");
    }
    let directory = Directory::new();
    let server = Running::start_flags(
        &directory.0,
        &["--atomic-state", "--observer-history", "--allow-writes"],
    );
    let mut client = LocalClient::new(&directory.0);
    let _ = populate(&mut client, 10_000, true);
    let before = current(&mut client);
    let result = client
        .request(Request {
            version: 1,
            command: Command::SubmitGraph {
                expected: before.clone(),
                tasks: vec![task(10_000, false)],
            },
        })
        .unwrap();
    assert!(matches!(
        result.outcome,
        Outcome::Refused {
            failure: Failure::Refused {
                code: FailureCode::TaskLimit
            }
        }
    ));
    assert!(
        matches!((before, current(&mut client)), (Revision::Finite(a), Revision::Finite(b)) if a == b)
    );
    let mut observer = Observer::new(ObserverLimits {
        page_size: 64,
        max_pages: 158,
        max_entries: 10_000,
        max_id_bytes: 256,
    })
    .unwrap();
    let started = Instant::now();
    assert!(!observer.resync_feed(&mut client).unwrap());
    assert_eq!(observer.view().unwrap().entries.len(), 10_000);
    println!(
        "operation=capacity_resync tasks=10000 elapsed_ns={}",
        started.elapsed().as_nanos()
    );
    drop(server);
    let started = Instant::now();
    let _server = Running::start_flags(&directory.0, &["--observer-history"]);
    assert!(!observer.resync_feed(&mut client).unwrap());
    assert_eq!(observer.view().unwrap().entries.len(), 10_000);
    println!(
        "operation=capacity_recovery_and_resync tasks=10000 elapsed_ns={}",
        started.elapsed().as_nanos()
    );
}
