# Chronos engine

Pure Rust scheduling and coordination for autonomous agents. Commands operate
on bounded, versioned state under caller-supplied time and authority facts.
The engine performs no filesystem, network, process, UI or system-clock effects
and has no runtime dependencies.

Engine 0.3 preserves canonical checkpoint V1/V2 and scheduling semantics.
Storage ports live in `chronos-host`; concrete persistence, service, protocol,
transport, connectors and SDK distributions are separate workspace packages.
The terminal and macOS presentations are independent sibling projects.

From the source workspace: `cargo test -p chronos-engine --locked --offline`.
See the repository README and `docs/` for contracts and qualification limits.
