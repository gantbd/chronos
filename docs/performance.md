# Performance discipline

Performance claims require an explicit workload, build profile, machine/runtime
description, sample method, and budget. Initial benchmark populations will cover:

- admission by task count and dependency fan-out;
- byte-bound validation for identifiers, capabilities, keys, and reasons;
- claim selection with mixed priorities and capabilities;
- dependency release fan-out;
- cancellation/dependency-failure propagation fan-out;
- expiry processing by running population;
- uncertain-effect population and reconciliation throughput;
- retained-outcome FIFO saturation and query traversal;
- checkpoint encoding/decoding and recovery at declared byte/state ceilings;
- migration once a second persisted version exists;
- fairness behavior under retry and capability scarcity.

Report throughput, median, p95/p99, maximum, allocation/retention, and saturation
where meaningful. Compilation time is not service time. A microbenchmark does not
establish an end-to-end SLO, and an average cannot satisfy a tail-latency budget.

No numeric production SLO is selected. The host-scoped engine smoke gate is:

```sh
make performance CHRONOS_PERF_BUDGET_MS=<positive-stage-budget>
```

It compiles an optimized build and separately budgets admission of 512 tasks,
strict-priority claims of all 512 tasks, checkpoint encoding of 512 running
tasks, bounded recovery, V2 atomic admission of 512 tasks, and V2 paging of all
512 tasks in pages of 64. The environment value is required and applies to each
stage; zero, absence, parse failure, or a stage overrun fails the gate.
The test is ignored by the ordinary correctness suite so a hidden machine-speed
assumption cannot become a semantic test.

Evidence recorded on 2026-11-08 for the executed `arm64` Darwin host, Rust
1.95.0 optimized build, and a 100 ms per-stage budget: admission 0.187 ms, claim
9.613 ms, checkpoint 0.049 ms, recovery 0.302 ms, V2 atomic admission 0.195 ms,
and V2 paging 0.073 ms. These samples establish only that exact smoke workload
on that host/run. They are not tail distributions, allocation measurements,
connector latency, or a product SLO. Criterion-quality sampling and declared
deployment budgets remain release-profile work.

## Synthetic private-local request calibration

```sh
make measure-local-latency
```

This explicit opt-in runs an optimized service child in a temporary private
directory. Two complete warmup cycles precede 64 sequential measured cycles.
Each cycle reads the revision, durably admits one task, explains it, durably
cancels it, explains it again and durably retires it. The task name is reused
only with a fresh incarnation. Functional assertions reject wrong outcomes;
there is deliberately no numeric speed assertion or inferred product budget.
The benchmark is ignored by ordinary correctness/candidate gates.

Timings cover the synchronous local-client call: endpoint checks, connection,
codec/framing, daemon processing, durable checkpoint publication for mutations,
and reply delivery. They exclude compilation, startup, warmup, external workers,
UI presentation and concurrent load. Fixed-class output contains no task,
principal or path labels. The report gives sample count, minimum, nearest-rank
p50/p95/p99 and maximum. These small samples do not estimate production tails
with statistical confidence; for 64 samples, nearest-rank p99 is the maximum.
The workload has at most one retained task and no dependency fan-out. Allocation,
large graphs, saturation, cold starts, concurrent clients and other filesystems
remain separate workloads; no throughput or bottleneck claim is inferred.

Recorded on 2026-11-10 with Rust 1.95.0, optimized `aarch64-apple-darwin`,
same-host Unix transport and the current directory checkpoint adapter:

| Operation | Samples | p50 ms | p95 ms | Sample p99 ms | Maximum ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Revision | 64 | 0.109 | 0.160 | 0.712 | 0.712 |
| Graph admission | 64 | 15.196 | 21.189 | 33.922 | 33.922 |
| Explain | 128 | 0.123 | 0.169 | 0.191 | 0.579 |
| Cancel | 64 | 14.942 | 18.558 | 60.279 | 60.279 |
| Retire | 64 | 15.070 | 17.753 | 38.885 | 38.885 |

These observations separate read and durable-write behavior; they neither
establish a production SLO nor justify weakening durability to meet one.

## Adopted private-local source-candidate acceptance

On 2026-11-10 the owner selected the private-local source candidate and
provisional engineering performance gates (A/A). Run:

```sh
make qualify-local-performance
```

This required part of `check-local-profile` uses optimized builds and the
recorded reference host, not a portable production SLO. Revision and Explain
requests must meet sample p95 <= 5 ms and p99 <= 20 ms. Successful durable
SubmitGraph, Cancel and Retire requests must separately meet sample p95 <=
100 ms and p99 <= 250 ms. Correctness, authorization and durable publication
are unchanged. No average, domain refusal or revision conflict can satisfy a
successful-write budget. A failing run remains a failed qualification; rerunning
does not erase it or authorize changing a budget.

The executable population starts with 1,000 tasks, eight bounded admission
batches, independent chains and root fan-out. Each one/four-client configuration
runs twice in a fresh private directory with atomic envelopes and observer
history enabled. Per configuration it measures 2,048 Revision and 2,048 Explain
requests, then 512 total cancellation attempts on distinct fan-out leaves.
Cancellation starts concurrently for four clients; optimistic revision conflicts
are reported separately, never retried or counted as commits. Successful
cancellations are retired sequentially as a distinct operation class. Admission
and retirement are explicitly sequential; this is not four-client throughput or
mixed read/write latency certification. The population decreases during retirement.

All results report operation/sample count, p50/p95/p99/max; labels contain no
task identities, invocation keys or paths. Timings include local client transport,
daemon work, durable publication when required, and response delivery. The
precondition query for cancellation/retirement is outside that mutation's timing.
Warmup is 16 revision requests before each concurrent read phase; startup and
population admission are not hidden in read timings. Admissions are measured
from initial creation and have only 16 samples/configuration, so their p99 is
the maximum, not a statistically established production tail. Successful CAS
sample counts are reported rather than assumed. Two repetitions are engineering
regression checks, not confidence intervals or a production workload model.

A separate configured-capacity gate admits 10,000 tasks, rejects the next task
without advancing engine revision, validates a bounded full observer snapshot,
restarts from the same durable store, explicitly re-enables the ephemeral feed,
and validates all 10,000 recovered entries. Observer resync and recovery-to-first-
reply timings are reported separately without invented numerical budgets.
Charged state/history ceilings and fail-closed behavior remain enforced; these
tests do not measure allocation, RSS, CPU utilization, power-loss behavior,
every byte ceiling, other filesystems or sustained saturation. Existing focused
boundary/failure tests remain part of the source-candidate gate.

For a performance investigation run this target alone, not the full correctness
suite. For a candidate milestone run `make check-local-profile`; independent UI
and unavailable optional external-tool gates do not enter this release unit.

The complete `make check-local-profile` run passed on 2026-11-10 with Rust 1.95.0
on Darwin arm64. Worst measured read p95/p99 across the selected classes and
configurations was 0.405/0.466 ms; worst successful durable p95/p99 was
22.508/58.525 ms. A cancel maximum of 294.789 ms remains visible and is not
represented as meeting a maximum-latency target. At configured capacity,
10,000-task resync took 33.080 ms and recovery plus resync 49.872 ms. See the
roadmap's final delivery entry for per-class results, conflict counts and the
separate focused-run observations. This records an engineering gate pass, not
a production tail-latency guarantee.

## R4 startup calibration on the C1 binary

Eight initializations and eight reopens used the optimized C1 service with
`--atomic-state --allocation-receipts` and private temporary state. Timing
started before spawning the process, sampled its readiness announcement, and
ended after a successful V1 Revision reply over its private socket. Each
trial shut down the child and checked for a successful exit.

| Sample | Median to ready | Maximum to ready | Median to first reply | Maximum to first reply |
| --- | ---: | ---: | ---: | ---: |
| New empty state | 34.131 ms | 93.167 ms | 34.350 ms | 94.384 ms |
| Reopen empty state | 4.405 ms | 5.326 ms | 4.751 ms | 5.642 ms |

The first request after readiness took at most 1.218 ms. This small same-host
sample covers empty stores and the already-built binary. The earlier
ten-second readiness outlier did not recur and remains unexplained. Cold
launch, populated stores and any release latency budget require separate evidence.

The follow-up calibration measures all three startup cases and checks B3-to-C1
upgrade with the actual binaries:

```sh
python3 scripts/measure-startup.py \
  target/source-check-c1/target/release/chronos-service \
  --previous-binary target/source-check-b3/target/debug/chronos-service
```

Eight trials per case measured median and maximum time to first reply of
34.032 / 47.303 ms on new state, 4.118 / 4.461 ms on empty reopen, and
4.592 / 4.937 ms on 1,000-task reopen. The JSON output includes both binary
hashes, every timing sample and the upgrade result. Reopen repeats the same
store, so the observations reflect this host and its cache state. The earlier
readiness outlier remains unexplained.

A first-execution probe copied the exact C1 binary to a new temporary path
and verified its SHA-256 before launching it. The first copy exceeded the
90-second readiness deadline. A process sample at 19 seconds showed the
main thread at `_dyld_start`, before any Chronos frame appeared. The
pre-application delay has no established cause. Reproduce the comparison
with `--trials 1 --fresh-copy --readiness-timeout 90` on the startup script.
The deadline ends the synthetic trial and does not set a release budget.

A follow-up held the host, volume, executable bytes and empty-state profile
constant. The original reached first reply in 55.958 ms. A temporary symlink
to the same executable inode reached first reply in 34.698 ms. A copied
executable on the same volume had a distinct inode and exceeded the 30-second
readiness deadline. This comparison associates the delay with first execution
of a new file on this host. It leaves the OS mechanism unresolved. The result
is recorded in `target/source-check-c1/r4-startup-path-comparison.json`.

The startup script preserves an alternate path passed as its binary argument.
On a failed fixture it emits the failed stage, readiness deadline and any
completed profiles in JSON, then exits with status 1.

On macOS 26.6.2, the original and a byte-identical temporary copy both passed
`codesign --verify --strict`. Both had the same arm64 ad hoc signature and
Code Directory hash, and listed `com.apple.provenance` as their only extended
attribute. This check found no difference in embedded signature validation or
visible quarantine attributes. The first-execution delay remains unexplained.
[Apple's code-signature verification reference](https://developer.apple.com/library/archive/technotes/tn2206/)
