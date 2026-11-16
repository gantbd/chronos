# Shared local workbench

The first target is one trusted OS owner, one durable local Chronos service,
MCP-connected coordination agents, and independent terminal/macOS human clients.
This is source-built local use, not a signed installer or remote service.

Build once from the Chronos checkout; rebuilding is explicit after source changes:

```sh
sh scripts/workbench.sh build-core
sh scripts/workbench.sh build-tui
# Optional alternative presentation:
sh scripts/workbench.sh build-gui
```

Choose a new private directory under a trusted parent. Start the service in one
terminal and connect each client to that exact absolute directory:

```sh
sh scripts/workbench.sh service /private/tmp/my-chronos --allow-writes
# In another terminal:
sh scripts/workbench.sh status /private/tmp/my-chronos
sh scripts/workbench.sh tui /private/tmp/my-chronos --allow-writes
# Or use the GUI instead:
sh scripts/workbench.sh gui /private/tmp/my-chronos --allow-writes
```

For an MCP-capable harness, configure a stdio server process with executable
`/bin/sh` and arguments: the absolute path to `scripts/workbench.sh`, `agent`,
the same absolute session directory, and optionally `--allow-writes`. The harness
must perform the MCP initialization handshake and discover `chronos_coordinate`.
Use the host harness's configuration format; these executable/argument values
are not a claim that every harness shares one configuration-file schema.
The service must already be running. Launching the connector does not launch it.

Both the service and client independently require explicit write enablement.
Omit `--allow-writes` for a read-only client; an agent cannot elevate through
tool arguments. Task submission uses the observed revision; stale/unknown
outcomes require explicit inspection, never automatic resubmission. The tested
MCP-agent/UI-model scenario submits from both clients and observes the same
original incarnation and updated revision through the one service.

MCP currently provides coordination, not worker execution. A harness can opt
into scheduling inspection and controls by passing `--scheduling` after
the service directory to the MCP executable or `workbench.sh agent`. Discover
`chronos_schedule`; it uses the existing V5 profile on an explicitly selected
atomic service. Without `--allow-writes` its schema exposes only overview and
eligibility, and runtime checks refuse writes even if a harness ignores that
schema. Service authorization still applies. Controls retain expected-revision
guards; a stale/unknown result requires explicit inspection. No claim, execution,
interrupt or executor-stop attestation is supplied through this tool.

Pass `--dependencies` to the MCP executable to discover
`chronos_dependencies`. It reads one hop of prerequisites or dependents for
the supplied task incarnation and finite revision. Chronos filters disclosure
under the current service policy. An empty visible result, a truncated page,
and a stale refusal remain distinguishable.

Pass `--scheduling` to `workbench.sh tui` or `workbench.sh gui` for the corresponding
human controls. This still requires the service's explicit `--atomic-state`
profile and independent `--allow-writes` flags for mutations. Use the terminal's
`s` key or the GUI's Refresh scheduling button before proposing a control. A
separate confirmation preserves the observed revision/task incarnation. The
terminal can inspect this profile with `--scheduling --snapshot` without taking
over a terminal. UI/connector flags do not upgrade a legacy service's storage.

Trusted worker hosts use the embedded SDK to claim/renew/report/fence/reconcile. Wiring a real agent
executor to that port, with explicitly scoped authority and effect correlation,
is the next adoption step after eligibility/budgets. No bearer credential is
serialized through MCP by this launcher. Agents under one OS user are mutually
trusted in this profile; use a different profile for distrustful principals.

`sh scripts/workbench.sh shutdown /private/tmp/my-chronos` requests graceful
drain. It does not cancel external effects or delete tasks. Restart uses the
same directory and preserves the existing recovery/ownership checks. Do not
delete initialized state to bypass a refusal. Atomic-envelope/observer options
remain explicit service flags, including the established migration procedure
for an existing legacy store; the launcher does not select or migrate them.

Text continuity is a required next adoption capability (R17), not supplied by
the current launcher, task-ID snapshot or binary capture journal. Work documents
must remain independently readable/editable with last-observed status made clear.
Offline progress is reconciled explicitly; it cannot stand in for live execution
authority or confirmation that an uncertain executor stopped.
