# Engine 0.3 boundary and workspace migration

This is a source/API boundary change, not a checkpoint-format or scheduling
policy change. The engine remains one package with no runtime dependencies.

| Previous `chronos_engine` import | New owner |
| --- | --- |
| `CheckpointStore`, `LoadedCheckpoint`, `StoreError`, `StoredCheckpointError`, `IoStage` | `chronos_host` |
| `DirectoryCheckpointStore`, `VolatileCheckpointStore` | `chronos_adapter_checkpoint` |
| `Checkpoint`, `CheckpointError`, `CommitReceipt`, `AcknowledgementStrength`, `RecoveryLimits`, `SnapshotRevision`, prepared transitions | Remain `chronos_engine` |

The workspace pins engine `=0.3.0`. Add explicit host/adapter
dependencies only to consumers that need those contracts. Pure scheduling
consumers continue to use only `chronos-engine`. Existing inspected Moira and
Keystone qualification consumers use path dependencies and do not import the
removed storage APIs; their lockfiles must resolve the new engine version.

`LoadedCheckpoint::from_storage` adapts owned bytes and their claimed revision.
It grants no task/lease authority. `recover` still checks canonical decoding,
limits and stored-versus-payload revision agreement. Checkpoint V1/V2 byte formats,
receipt validation, optimistic predecessor checks, file/directory synchronization,
retention and indeterminate-error behavior are preserved.

The existing `checkpoint_store_behavior`, `directory_checkpoint_store_behavior`,
`prepared_transition_behavior` and `command_v2_behavior` integration tests moved
unchanged except imports into the checkpoint-adapter crate. Store fault-injection
unit tests moved with the implementation. No storage test was replaced with
an engine-only simulation. The stored-revision safety mutant now runs against
the host/adapter boundary; the other four mutants remain engine tests.

The v0.1/v0.2 release-readiness ledgers are historical evidence, not claims that
v0.3 has inherited a registry release or all optional external verification tools.
No registry publication, Git commit, remote update or compatibility shim is
performed by this migration.

## Categorical source layout

`chronos/Cargo.toml` is now the non-UI workspace. Engine source and fixtures
moved intact into `engine/chronos-engine`; core consumers must point their
Cargo path dependency there, not at the virtual workspace root. The Moira core,
Moira SQLite adapter and Keystone Chronos/Moira qualification paths were updated.
The fuzz target points to the same engine package. No engine Rust API,
checkpoint format or scheduling policy changed as part of this relocation.

The terminal and macOS presentation packages moved into sibling `chronos-tui`
and `chronos-gui-macos` directories with independent manifests, lockfiles,
checks and launch guidance. This source extraction did not initialize Git or
create/rewrite history; these new directories are not yet independent Git
repositories. Preserve them with the surrounding workspace until repository
initialization is explicitly requested.

`make check` builds the core independently of those UI checkouts;
`make check-all` also checks both sibling presentations. `make integration-check`
remains an alias for core checks. `make qualify-gui` delegates to the explicit
own-window probe in the GUI project. The retired integration manifest/lockfile
is superseded by the root and presentation manifests/locks; its old build cache
is retained, not removed. Historical v0.1/v0.2 evidence keeps its original paths.

## Unpublished integration boundary cleanup

The non-engine crates remain source-distributed, unpublished 0.1 packages.
Their native Rust imports changed while retaining the existing JSON V1 profile:

| Previous integration API | Current owner |
| --- | --- |
| `chronos_protocol::{RequestV1, ResponseV1, ReplyV1, RefusalV1, OperationV1}` | `chronos_host::native` |
| `chronos_client::ClientV1` | `chronos_host::ClientV1` |
| `chronos_client::{EmbeddedClient, EmbeddedError}` | `chronos_adapter_native` |
| DTOs such as `Request`, `Response`, `Revision`, `Reference`, `Limits` | `chronos_protocol`; also reexported by `chronos_wire` |
| Wire-to-native `decode_request` / `Request::native` | `chronos_adapter_native::decode_request` / `NativeRequest` |
| Engine-to-wire `From` implementations | `chronos_adapter_native::IntoWire` and `response` |
| `chronos_local::{WriterOwnership, create_private_directory, restrict_socket}` | `chronos_standalone::ownership` (concrete process composition) |

`chronos_wire::decode_request` now returns the validated public DTO rather than
an engine-native command. Remote clients, protocol, codec and per-client model
have no normal dependency on engine, host, service or concrete adapters. Embedded
hosts select the native adapter explicitly. No engine 0.3 API, checkpoint bytes,
wire version, task authority, authorization or unknown-outcome policy changes.
