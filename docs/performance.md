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

No numeric product SLO is selected. The active host-scoped smoke gate is:

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
