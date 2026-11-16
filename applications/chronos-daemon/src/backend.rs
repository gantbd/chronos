//! Local composition policy. A persistent profile marker prevents downgrade to
//! preserved legacy checkpoints after selecting envelope state.
use crate::{
    OperatorPolicy, ServerOptions,
    ownership::{self, WriterOwnership},
};
use chronos_adapter_checkpoint::{DirectoryCheckpointStore, DirectoryServiceEnvelopeStore};
use chronos_engine::{EngineLimits, RecoveryLimits};
use chronos_host::{
    CheckpointStore, MaintenanceSnapshot,
    native::{ReplyV1, RequestV1},
    native_observer::*,
    native_v2::{ReplyV2, RequestV2},
};
use chronos_service::{
    Limits,
    atomic::{AtomicLimits, AtomicService},
    durable::DurableService,
    observer::FeedLimits,
};
use chronos_wire::coordination_v3 as wire;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt},
    path::Path,
};
const MARKER: &str = ".chronos-envelope-v1";
const MARKER_BYTES: &[u8] = b"CHRONOS-SERVICE-ENVELOPE-V1\n";
const MAX_STORE: usize = 10 * 1024 * 1024;
fn limits() -> Limits {
    Limits {
        max_graph_tasks: 128,
        max_page_entries: 64,
    }
}
fn recovery() -> RecoveryLimits {
    RecoveryLimits::new(8 * 1024 * 1024, EngineLimits::default())
}
fn atomic_limits() -> AtomicLimits {
    AtomicLimits {
        max_envelope_bytes: MAX_STORE,
        max_retained_bytes: 1024 * 1024,
        max_command_bytes: chronos_local::MAX_REQUEST_BYTES,
        max_key_bytes: 256,
        max_invocations: 1024,
    }
}
fn feed_limits() -> FeedLimits {
    FeedLimits {
        max_cached_tasks: 10_000,
        max_cached_bytes: 4 * 1024 * 1024,
        max_transitions: 128,
        max_history_bytes: 1024 * 1024,
        max_changes_per_transition: 128,
        max_page_transitions: 64,
        max_response_bytes: chronos_local::MAX_RESPONSE_BYTES / 8,
    }
}
enum Kind {
    Legacy(Box<DurableService<OperatorPolicy, DirectoryCheckpointStore>>),
    Atomic(Box<AtomicService<OperatorPolicy, DirectoryServiceEnvelopeStore>>),
}
pub(crate) struct Backend {
    inner: Kind,
    marker: Option<Marker>,
    scope: Option<(String, ObserverCursorV1)>,
}
struct Marker {
    file: File,
    identity: (u64, u64),
}
impl Marker {
    fn create(directory: &Path) -> Result<Self, &'static str> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .read(true)
            .mode(0o600)
            .open(directory.join(MARKER))
            .map_err(|_| "profile marker creation refused")?;
        file.write_all(MARKER_BYTES)
            .and_then(|()| file.sync_all())
            .map_err(|_| "profile marker durability unknown; inspect storage")?;
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| "profile marker publication unknown; inspect storage")?;
        let identity =
            ownership::identity(&file.metadata().map_err(|_| "profile marker unavailable")?);
        let marker = Self { file, identity };
        marker.verify(directory)?;
        Ok(marker)
    }
    fn open(directory: &Path) -> Result<Self, &'static str> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(
                rustix::fs::OFlags::NOFOLLOW
                    .bits()
                    .try_into()
                    .map_err(|_| "no-follow flag unavailable")?,
            )
            .open(directory.join(MARKER))
            .map_err(|_| "profile marker refused")?;
        let identity =
            ownership::identity(&file.metadata().map_err(|_| "profile marker unavailable")?);
        let marker = Self { file, identity };
        marker.verify(directory)?;
        Ok(marker)
    }
    fn verify(&self, directory: &Path) -> Result<(), &'static str> {
        let metadata = std::fs::symlink_metadata(directory.join(MARKER))
            .map_err(|_| "profile marker missing")?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.len() != MARKER_BYTES.len() as u64
            || ownership::identity(&metadata) != self.identity
        {
            return Err("profile marker ownership lost");
        }
        let mut bytes = [0; MARKER_BYTES.len()];
        // The marker is fixed-size and never interpreted as arbitrary input.
        self.file
            .read_exact_at(&mut bytes, 0)
            .map_err(|_| "profile marker read refused")?;
        if bytes != MARKER_BYTES {
            return Err("profile marker invalid");
        }
        Ok(())
    }
}
fn has_service_files(directory: &Path) -> Result<bool, &'static str> {
    let mut found = false;
    for (index, entry) in std::fs::read_dir(directory)
        .map_err(|_| "profile inspection refused")?
        .enumerate()
    {
        if index >= 4096 {
            return Err("profile directory limit");
        }
        let entry = entry.map_err(|_| "profile inspection refused")?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("service-") && name.ends_with(".bin"))
        {
            found = true;
        }
    }
    Ok(found)
}
impl Backend {
    pub(crate) fn signals(&self) -> crate::RuntimeSignals {
        match &self.inner {
            Kind::Legacy(service) => crate::RuntimeSignals {
                scheduling: None,
                legacy_persistence: Some(service.signals()),
                atomic: None,
                native: service.native_signals(),
                coordination_v2: service.coordination_v2_signals(),
                observer: service.observer_signals(),
                observer_usage: service.observer_usage(),
                invocation_usage: None,
            },
            Kind::Atomic(service) => crate::RuntimeSignals {
                scheduling: service.scheduling_signals(),
                legacy_persistence: None,
                atomic: Some(service.signals()),
                native: service.native_signals(),
                coordination_v2: service.coordination_v2_signals(),
                observer: service.observer_signals(),
                observer_usage: service.observer_usage(),
                invocation_usage: Some(service.usage()),
            },
        }
    }
    pub(crate) fn open(
        owner: &WriterOwnership,
        options: ServerOptions,
    ) -> Result<Self, &'static str> {
        let directory = owner.directory();
        let marker = match std::fs::symlink_metadata(directory.join(MARKER)) {
            Ok(_) => Some(Marker::open(directory)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("profile inspection refused"),
        };
        let records = has_service_files(directory)?;
        let policy = OperatorPolicy {
            allow_writes: options.allow_writes,
        };
        let (inner, marker) = if let Some(marker) = marker {
            if !records {
                return Err("initialized service envelope missing; operator recovery required");
            }
            let store = DirectoryServiceEnvelopeStore::open(directory, MAX_STORE, 4096)
                .map_err(|_| "envelope store refused")?;
            (
                Kind::Atomic(Box::new(
                    AtomicService::open_existing(
                        store,
                        policy,
                        limits(),
                        recovery(),
                        atomic_limits(),
                    )
                    .map_err(|_| "envelope recovery refused")?,
                )),
                Some(marker),
            )
        } else {
            if records {
                return Err(
                    "service envelopes exist without profile marker; operator recovery required",
                );
            }
            let store = DirectoryCheckpointStore::open(directory, 8 * 1024 * 1024, 4096)
                .map_err(|_| "store refused")?;
            let initialized = store
                .load()
                .map_err(|_| "store inspection refused")?
                .is_some();
            if options.atomic_state {
                if initialized {
                    return Err("legacy checkpoint exists; explicit envelope migration required");
                }
                if !owner.created() {
                    return Err("initialized state missing; operator recovery required");
                }
                let marker = Marker::create(directory)?;
                let store = DirectoryServiceEnvelopeStore::open(directory, MAX_STORE, 4096)
                    .map_err(|_| "envelope store refused")?;
                (
                    Kind::Atomic(Box::new(
                        AtomicService::create_new(
                            store,
                            policy,
                            limits(),
                            recovery(),
                            atomic_limits(),
                        )
                        .map_err(|_| "envelope initialization refused")?,
                    )),
                    Some(marker),
                )
            } else {
                let service = if initialized {
                    DurableService::open_existing(store, policy, limits(), recovery())
                } else if owner.created() {
                    DurableService::create_new(store, policy, limits(), recovery())
                } else {
                    return Err("initialized checkpoint missing; operator recovery required");
                }
                .map_err(|_| "creation or recovery refused")?;
                (Kind::Legacy(Box::new(service)), None)
            }
        };
        let mut backend = Self {
            inner,
            marker,
            scope: None,
        };
        if options.allocation_receipts {
            match &mut backend.inner {
                Kind::Atomic(service) => service.enable_allocation_receipts().map_err(
                    |_| "allocation profile upgrade unavailable; inspect committed state",
                )?,
                Kind::Legacy(_) => return Err("allocation receipts require atomic state"),
            }
        }
        if options.observer_history {
            match &mut backend.inner {
                Kind::Legacy(service) => service
                    .enable_observer(feed_limits())
                    .map_err(|_| "observer unavailable")?
                    .map_err(|_| "observer limits refused")?,
                Kind::Atomic(service) => service
                    .enable_observer(feed_limits())
                    .map_err(|_| "observer unavailable")?
                    .map_err(|_| "observer limits refused")?,
            }
            let ObserverResponseV1::ResyncRequired { head } = backend
                .observe(ObserverRequestV1 {
                    after: None,
                    limit: 1,
                })?
                .map_err(|_| "observer initialization refused")?
            else {
                return Err("observer initialization mismatch");
            };
            let mut random = [0_u8; 32];
            File::open("/dev/urandom")
                .and_then(|mut file| file.read_exact(&mut random))
                .map_err(|_| "observer generation entropy unavailable")?;
            let token = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            backend.scope = Some((token, head));
        }
        Ok(backend)
    }
    pub(crate) fn verify(&self, directory: &Path) -> Result<(), &'static str> {
        match &self.marker {
            Some(marker) => marker.verify(directory),
            None => match std::fs::symlink_metadata(directory.join(MARKER)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                _ => Err("service profile changed; restart required"),
            },
        }
    }
    pub(crate) fn dispatch(&mut self, request: RequestV1) -> Result<ReplyV1, &'static str> {
        match &mut self.inner {
            Kind::Legacy(service) => service
                .dispatch(request)
                .map_err(|_| "persistence fenced; outcome unknown"),
            Kind::Atomic(service) => service
                .dispatch(request)
                .map_err(|_| "persistence fenced; outcome unknown"),
        }
    }
    pub(crate) fn dispatch_v2(&mut self, request: RequestV2) -> Result<ReplyV2, &'static str> {
        match &mut self.inner {
            Kind::Legacy(service) => service
                .dispatch_v2(request)
                .map_err(|_| "persistence fenced; outcome unknown"),
            Kind::Atomic(service) => service
                .dispatch_v2(request)
                .map_err(|_| "persistence fenced; outcome unknown"),
        }
    }
    fn observe(&mut self, request: ObserverRequestV1) -> Result<ObserverReplyV1, &'static str> {
        match &mut self.inner {
            Kind::Legacy(service) => service.observe(request).map_err(|_| "service fenced"),
            Kind::Atomic(service) => service.observe(request).map_err(|_| "service fenced"),
        }
    }
    pub(crate) fn v3(&mut self, request: wire::Request) -> Result<wire::Response, &'static str> {
        use chronos_adapter_native::coordination_v3 as adapter;
        let response = match request.command {
            wire::Command::Observe { after, limit } => {
                let scope = self
                    .scope
                    .as_ref()
                    .map(|(token, cursor)| (token.as_str(), cursor));
                let request = adapter::observer_request(after, limit, scope)
                    .map_err(|_| "observer conversion refused")?;
                let reply = self.observe(request)?;
                adapter::observer_response(
                    reply,
                    self.scope.as_ref().map_or("", |(token, _)| token.as_str()),
                )
            }
            command => match &mut self.inner {
                Kind::Atomic(service) => adapter::invocation_response(
                    service
                        .invoke(
                            adapter::invocation(command, chronos_local::wire_limits())
                                .map_err(|_| "invocation conversion refused")?,
                        )
                        .map_err(|_| "persistence fenced; invocation outcome unknown")?,
                ),
                Kind::Legacy(_) => wire::Response {
                    version: 3,
                    outcome: wire::Outcome::Refused {
                        code: wire::RefusalCode::Unavailable,
                    },
                },
            },
        };
        if let Err(error) = chronos_wire::encode(&response, chronos_local::MAX_RESPONSE_BYTES) {
            return Ok(wire::Response {
                version: 3,
                outcome: wire::Outcome::Refused {
                    code: match error {
                        chronos_wire::CodecError::Capacity => wire::RefusalCode::Capacity,
                        _ => wire::RefusalCode::Limit,
                    },
                },
            });
        }
        Ok(response)
    }
    pub(crate) fn v4(
        &mut self,
        request: chronos_wire::coordination_v4::Request,
    ) -> Result<chronos_wire::coordination_v4::Response, &'static str> {
        use chronos_wire::coordination_v4 as v4;
        let invocation = chronos_adapter_native::coordination_v4::invocation(
            request,
            chronos_local::wire_limits(),
        )
        .map_err(|_| "allocation request conversion refused")?;
        let serial = invocation.serial.to_string();
        let response = match &mut self.inner {
            Kind::Atomic(service) => chronos_adapter_native::coordination_v4::response(
                &serial,
                service
                    .allocation(invocation)
                    .map_err(|_| "allocation observation unavailable; persistence fenced")?,
                chronos_local::wire_limits(),
            ),
            Kind::Legacy(_) => v4::Response {
                version: 4,
                outcome: v4::Outcome::Refused {
                    code: v4::RefusalCode::Unavailable,
                },
            },
        };
        Ok(response)
    }
    pub(crate) fn dependencies_v6(
        &mut self,
        request: chronos_wire::dependencies_v6::Request,
    ) -> Result<chronos_wire::dependencies_v6::Response, &'static str> {
        use chronos_host::dependencies::{
            DependencyDirection, DependencyRefusal, DependencyRequest,
        };
        use chronos_wire::dependencies_v6 as wire;
        let wire::Command::Neighbors {
            expected,
            task,
            direction,
            limit,
        } = request.command;
        let chronos_wire::Revision::Finite(expected) = expected else {
            return Err("validated dependency revision refused");
        };
        let native = DependencyRequest {
            expected: chronos_engine::StateRevision::finite(
                expected
                    .parse()
                    .map_err(|_| "validated dependency revision refused")?,
            ),
            task: chronos_engine::TaskRefV1::from_observation(
                chronos_engine::TaskId::from(task.task.as_str()),
                task.incarnation
                    .parse()
                    .map_err(|_| "validated dependency incarnation refused")?,
            ),
            direction: match direction {
                wire::Direction::Prerequisites => DependencyDirection::Prerequisites,
                wire::Direction::Dependents => DependencyDirection::Dependents,
            },
            limit,
        };
        let reply = match &mut self.inner {
            Kind::Legacy(service) => service
                .visible_dependencies(native)
                .map_err(|_| "dependency service fenced")?,
            Kind::Atomic(service) => service
                .visible_dependencies(native)
                .map_err(|_| "dependency service fenced")?,
        };
        let outcome = match reply {
            Ok(visible) => wire::Outcome::VisibleNeighbors {
                revision: chronos_wire::Revision::Finite(expected),
                task,
                direction,
                neighbors: visible
                    .neighbors
                    .into_iter()
                    .map(|reference| chronos_wire::Reference {
                        task: reference.task_id().as_str().into(),
                        incarnation: reference.incarnation().get().to_string(),
                    })
                    .collect(),
                truncated: visible.truncated,
            },
            Err(refusal) => wire::Outcome::Refused {
                code: match refusal {
                    DependencyRefusal::Unauthorized => wire::Refusal::Unauthorized,
                    DependencyRefusal::InvalidLimit => wire::Refusal::Limit,
                    DependencyRefusal::Stale => wire::Refusal::Stale,
                },
            },
        };
        Ok(wire::Response {
            version: 6,
            outcome,
        })
    }

    pub(crate) fn scheduling_v5(
        &mut self,
        request: chronos_wire::scheduling_v5::Request,
    ) -> Result<chronos_wire::scheduling_v5::Response, &'static str> {
        use chronos_wire::scheduling_v5 as wire;
        let native = match chronos_adapter_native::scheduling_v5::native(
            request.clone(),
            chronos_local::wire_limits(),
        ) {
            Ok(native) => native,
            Err(error) => {
                return Ok(wire::Response {
                    version: 5,
                    outcome: wire::Outcome::Refused {
                        code: match error {
                            chronos_wire::CodecError::Limit => wire::RefusalCode::Limit,
                            chronos_wire::CodecError::Capacity => wire::RefusalCode::Capacity,
                            _ => wire::RefusalCode::InvalidRequest,
                        },
                    },
                });
            }
        };
        Ok(match &mut self.inner {
            Kind::Atomic(service) => chronos_adapter_native::scheduling_v5::response(
                &request,
                service
                    .dispatch_scheduling(native)
                    .map_err(|_| "scheduling persistence fenced; outcome unknown")?,
                chronos_local::wire_limits(),
            ),
            Kind::Legacy(_) => wire::Response {
                version: 5,
                outcome: wire::Outcome::Refused {
                    code: wire::RefusalCode::Unavailable,
                },
            },
        })
    }
    pub(crate) fn maintain(&mut self) -> Result<MaintenanceSnapshot, &'static str> {
        match &mut self.inner {
            Kind::Legacy(service) => service.maintain().map_err(|_| "maintenance fenced"),
            Kind::Atomic(service) => service.maintain().map_err(|_| "maintenance fenced"),
        }
    }
}
pub(crate) fn migrate(directory: &Path) -> Result<(), &'static str> {
    chronos_local::verify_private_directory(directory)
        .map_err(|_| "private source directory required")?;
    let owner = WriterOwnership::acquire(directory).map_err(|_| "exclusive writer required")?;
    if std::fs::symlink_metadata(directory.join(MARKER)).is_ok() || has_service_files(directory)? {
        return Err("envelope profile already selected or requires recovery");
    }
    let legacy = DirectoryCheckpointStore::open(directory, 8 * 1024 * 1024, 4096)
        .map_err(|_| "legacy store refused")?;
    legacy
        .load()
        .map_err(|_| "legacy checkpoint unavailable")?
        .ok_or("legacy checkpoint missing")?
        .recover(recovery())
        .map_err(|_| "legacy recovery refused")?;
    owner.verify().map_err(|_| "writer ownership lost")?;
    let _marker = Marker::create(directory)?;
    let destination = DirectoryServiceEnvelopeStore::open(directory, MAX_STORE, 4096)
        .map_err(|_| "envelope store refused")?;
    AtomicService::migrate_from_checkpoint(
        &legacy,
        destination,
        OperatorPolicy {
            allow_writes: false,
        },
        limits(),
        recovery(),
        atomic_limits(),
    )
    .map_err(|_| "migration outcome unknown; inspect storage")?;
    owner.verify().map_err(|_| "writer ownership lost")?;
    Ok(())
}
