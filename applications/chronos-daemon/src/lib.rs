//! Local durable composition root. No TCP listener or ambient agent identity.
#![forbid(unsafe_code)]
#![cfg(unix)]

mod backend;
mod driver;
pub mod lifecycle;
pub mod ownership;
pub mod signals;
use ownership::WriterOwnership;

use chronos_host::native::{OperationV1, RequestV1};
use chronos_local::{IO_TIMEOUT, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, socket_path};
use chronos_service::Authorizer;
use std::{
    io::BufReader,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
};

/// The process owner chooses this policy at launch. Wire callers cannot elevate.
pub struct OperatorPolicy {
    pub allow_writes: bool,
}
impl Authorizer for OperatorPolicy {
    fn allows(&mut self, request: &RequestV1) -> bool {
        self.allow_writes
            || matches!(
                request.operation(),
                OperationV1::Revision | OperationV1::Snapshot | OperationV1::Page
            )
    }
}
impl chronos_service::dependencies::DependencyAuthorizer for OperatorPolicy {
    fn allows_dependency_read(
        &mut self,
        request: &chronos_host::dependencies::DependencyRequest,
    ) -> bool {
        self.allows(&RequestV1::Snapshot {
            task: request.task.task_id().clone(),
        })
    }

    fn allows_dependency_edge(
        &mut self,
        _: &chronos_host::dependencies::DependencyRequest,
        neighbor: &chronos_engine::TaskRefV1,
    ) -> bool {
        self.allows(&RequestV1::Snapshot {
            task: neighbor.task_id().clone(),
        })
    }
}
impl chronos_service::coordination_v2::AuthorizerV2 for OperatorPolicy {
    fn allows_v2(&mut self, request: &chronos_host::native_v2::RequestV2) -> bool {
        self.allow_writes || matches!(request, chronos_host::native_v2::RequestV2::Explain { .. })
    }
}
impl chronos_service::atomic::InvocationAuthorizer for OperatorPolicy {
    fn allows_invocation(
        &mut self,
        request: &chronos_host::native_v3::InvocationRequestV1,
    ) -> bool {
        self.allow_writes
            || !matches!(
                request,
                chronos_host::native_v3::InvocationRequestV1::Execute { .. }
            )
    }
    fn allows_retained_result(
        &mut self,
        _: &chronos_host::native_v3::InvocationV1,
        _: &chronos_host::native_v3::MutationV1,
    ) -> bool {
        true
    }
}
impl chronos_service::atomic::AllocationAuthorizer for OperatorPolicy {
    fn allows_allocation(&mut self, _: &chronos_host::native_v3::InvocationV1) -> bool {
        true
    }
    fn allows_allocation_result(
        &mut self,
        _: &chronos_host::native_v3::InvocationV1,
        _: &chronos_host::native_allocation::AllocationReceiptV1,
    ) -> bool {
        true
    }
}
impl chronos_service::scheduling::SchedulingAuthorizerV1 for OperatorPolicy {
    fn allows_scheduling(
        &mut self,
        request: &chronos_host::scheduling::SchedulingRequestV1,
    ) -> bool {
        use chronos_host::scheduling::SchedulingRequestV1;
        match request {
            SchedulingRequestV1::Control {
                command: chronos_engine::SchedulingCommandV1::ConfirmQuiescent { .. },
                ..
            } => false,
            SchedulingRequestV1::Control { .. } => self.allow_writes,
            _ => true,
        }
    }
    fn allows_scheduling_result(
        &mut self,
        _: &chronos_host::scheduling::SchedulingRequestV1,
        _: &chronos_host::scheduling::SchedulingResponseV1,
    ) -> bool {
        true
    }
}
impl chronos_service::observer::ObserverAuthorizer for OperatorPolicy {
    fn allows_observer(&mut self, _: &chronos_host::native_observer::ObserverRequestV1) -> bool {
        true
    }
    fn allows_observer_changes(
        &mut self,
        _: &[chronos_host::native_observer::ChangeSetV1],
    ) -> bool {
        true
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ServerOptions {
    pub allow_writes: bool,
    pub atomic_state: bool,
    pub observer_history: bool,
    pub allocation_receipts: bool,
}
/// Passive host-export surface. Options distinguish unavailable/fenced service
/// observations from actual zero occupancy. No task, principal or path labels.
#[derive(Clone, Copy, Debug, Default)]
pub struct RuntimeSignals {
    pub scheduling: Option<chronos_service::scheduling::SchedulingSignalsV1>,
    pub legacy_persistence: Option<chronos_service::durable::PersistenceSignals>,
    pub atomic: Option<chronos_service::atomic::AtomicSignals>,
    pub native: Option<chronos_service::Signals>,
    pub coordination_v2: Option<chronos_service::coordination_v2::SignalsV2>,
    pub observer: Option<chronos_service::observer::FeedSignals>,
    pub observer_usage: Option<chronos_service::observer::FeedUsage>,
    pub invocation_usage: Option<chronos_service::atomic::AtomicUsage>,
}
pub fn migrate_to_envelope(directory: &Path) -> Result<(), &'static str> {
    backend::migrate(directory)
}

pub struct Server {
    listener: UnixListener,
    path: PathBuf,
    service: backend::Backend,
    ownership: WriterOwnership,
    socket_identity: (u64, u64),
    control: UnixListener,
    control_path: PathBuf,
    control_identity: (u64, u64),
    lifecycle: lifecycle::Lifecycle,
    signals: TransportSignals,
}

/// Content-free interface measurements. No principal, task or path labels.
/// Kernel backlog occupancy is unknown, not reported as a zero-length queue.
#[derive(Clone, Copy, Debug, Default)]
pub struct TransportSignals {
    pub accepted: u64,
    pub frame_refused: u64,
    pub invalid_requests: u64,
    pub delivery_failed: u64,
    pub service_time: std::time::Duration,
    pub saturated: bool,
    pub ownership_failures: u64,
    pub fenced: bool,
    pub active_obligations: u8,
    pub control_requests: u64,
    pub control_refused: u64,
    pub control_delivery_failed: u64,
    pub control_time: std::time::Duration,
    pub drain_requests: u64,
    pub signal_requests: u64,
}
fn increment(value: &mut u64, saturated: &mut bool) {
    if let Some(next) = value.checked_add(1) {
        *value = next;
    } else {
        *saturated = true;
    }
}

fn add_time(value: &mut std::time::Duration, delta: std::time::Duration, saturated: &mut bool) {
    if let Some(total) = value.checked_add(delta) {
        *value = total;
    } else {
        *value = std::time::Duration::MAX;
        *saturated = true;
    }
}

impl Server {
    pub fn bind(directory: &Path, allow_writes: bool) -> Result<Self, &'static str> {
        Self::bind_with_options(
            directory,
            ServerOptions {
                allow_writes,
                ..ServerOptions::default()
            },
        )
    }
    pub fn bind_with_options(
        directory: &Path,
        options: ServerOptions,
    ) -> Result<Self, &'static str> {
        if options.allocation_receipts && !options.atomic_state {
            return Err("allocation receipts require explicit atomic-state selection");
        }
        ownership::create_private_directory(directory).map_err(|_| "private directory refused")?;
        let ownership =
            WriterOwnership::acquire(directory).map_err(|_| "writer ownership refused")?;
        let directory = ownership.directory();
        let path = socket_path(directory);
        let control_path = chronos_local::control_path(directory);
        let predecessor_socket = ownership::observe_endpoint(&path, ownership.created())
            .map_err(|_| "data endpoint refused")?;
        let predecessor_control = ownership::observe_endpoint(&control_path, ownership.created())
            .map_err(|_| "control endpoint refused")?;
        let service = backend::Backend::open(&ownership, options)?;
        ownership::remove_observed_endpoint(&path, predecessor_socket, &ownership)
            .map_err(|_| "data endpoint changed")?;
        ownership::remove_observed_endpoint(&control_path, predecessor_control, &ownership)
            .map_err(|_| "control endpoint changed")?;
        let listener = UnixListener::bind(&path).map_err(|_| "socket bind failed")?;
        ownership::restrict_socket(&path).map_err(|_| "socket permissions failed")?;
        let socket_identity = ownership::identity(
            &std::fs::symlink_metadata(&path).map_err(|_| "socket identity unavailable")?,
        );
        let control = UnixListener::bind(&control_path).map_err(|_| "control bind failed")?;
        ownership::restrict_socket(&control_path).map_err(|_| "control permissions failed")?;
        let control_identity = ownership::identity(
            &std::fs::symlink_metadata(&control_path)
                .map_err(|_| "control identity unavailable")?,
        );
        Ok(Self {
            listener,
            path,
            service,
            ownership,
            socket_identity,
            control,
            control_path,
            control_identity,
            lifecycle: lifecycle::Lifecycle::default(),
            signals: TransportSignals::default(),
        })
    }

    pub fn signals(&self) -> TransportSignals {
        let mut signals = self.signals;
        signals.fenced = self.lifecycle.state() == chronos_wire::control::LifecycleV1::Fenced;
        signals.active_obligations = self.lifecycle.active();
        signals
    }
    pub fn runtime_signals(&self) -> RuntimeSignals {
        self.service.signals()
    }

    /// Exactly one bounded frame per connection. The single writer has
    /// inactivity timeouts, not a total request or
    /// storage deadline. No hidden work queue.
    pub fn serve_one(&mut self) -> Result<(), &'static str> {
        self.verify_ownership()?;
        if !self.lifecycle.ready() {
            return Err("service not ready");
        }
        let (stream, _) = self.listener.accept().map_err(|_| "accept failed")?;
        self.verify_ownership()?;
        self.lifecycle
            .admit()
            .map_err(|_| "request admission refused")?;
        increment(&mut self.signals.accepted, &mut self.signals.saturated);
        let started = std::time::Instant::now();
        let mut result = self.handle(stream);
        if self.lifecycle.finish().is_err() {
            result = Err("request obligation discharge failed");
        }
        if result.is_err() {
            self.lifecycle.fence();
        }
        add_time(
            &mut self.signals.service_time,
            started.elapsed(),
            &mut self.signals.saturated,
        );
        result
    }

    fn verify_ownership(&mut self) -> Result<(), &'static str> {
        if self.lifecycle.state() == chronos_wire::control::LifecycleV1::Fenced {
            return Err("server fenced");
        }
        let valid = self.ownership.verify().is_ok()
            && self.service.verify(self.ownership.directory()).is_ok()
            && chronos_local::verify_socket(self.ownership.directory()).is_ok()
            && std::fs::symlink_metadata(&self.path)
                .is_ok_and(|meta| ownership::identity(&meta) == self.socket_identity)
            && chronos_local::verify_control_socket(self.ownership.directory()).is_ok()
            && std::fs::symlink_metadata(&self.control_path)
                .is_ok_and(|meta| ownership::identity(&meta) == self.control_identity);
        if !valid {
            increment(
                &mut self.signals.ownership_failures,
                &mut self.signals.saturated,
            );
            self.lifecycle.fence();
            return Err("writer or socket ownership lost; server fenced");
        }
        Ok(())
    }

    fn handle(&mut self, mut stream: UnixStream) -> Result<(), &'static str> {
        if stream.set_read_timeout(Some(IO_TIMEOUT)).is_err()
            || stream.set_write_timeout(Some(IO_TIMEOUT)).is_err()
        {
            increment(&mut self.signals.frame_refused, &mut self.signals.saturated);
            return Ok(());
        }
        let frame = match chronos_wire::read_frame(&mut BufReader::new(&stream), MAX_REQUEST_BYTES)
        {
            Ok(Some(frame)) => frame,
            // A malformed/slow caller does not take down the service.
            _ => {
                increment(&mut self.signals.frame_refused, &mut self.signals.saturated);
                return Ok(());
            }
        };
        self.verify_ownership()?;
        let response = match chronos_wire::decode_profile(&frame, chronos_local::wire_limits()) {
            Ok(chronos_wire::ProfileRequest::ScheduledCancellationV8(request)) => {
                chronos_wire::ProfileResponse::ScheduledCancellationV8(self.service.v8(request)?)
            }
            Ok(chronos_wire::ProfileRequest::ScheduledAdmissionV7(request)) => {
                chronos_wire::ProfileResponse::ScheduledAdmissionV7(self.service.v7(request)?)
            }
            Ok(chronos_wire::ProfileRequest::V1(request)) => {
                chronos_wire::ProfileResponse::V1(chronos_adapter_native::response(
                    self.service
                        .dispatch(
                            chronos_adapter_native::NativeRequest::native(
                                request,
                                chronos_local::wire_limits(),
                            )
                            .map_err(|_| "validated native request refused")?,
                        )
                        .map_err(|_| "persistence fenced; outcome unknown")?,
                ))
            }
            Ok(chronos_wire::ProfileRequest::V2(request)) => chronos_wire::ProfileResponse::V2(
                chronos_adapter_native::coordination_v2::response(
                    self.service
                        .dispatch_v2(
                            chronos_adapter_native::coordination_v2::native(
                                request,
                                chronos_local::wire_limits(),
                            )
                            .map_err(|_| "validated native request refused")?,
                        )
                        .map_err(|_| "persistence fenced; outcome unknown")?,
                ),
            ),
            Ok(chronos_wire::ProfileRequest::V3(request)) => {
                chronos_wire::ProfileResponse::V3(self.service.v3(request)?)
            }
            Ok(chronos_wire::ProfileRequest::V4(request)) => {
                chronos_wire::ProfileResponse::V4(self.service.v4(request)?)
            }
            Ok(chronos_wire::ProfileRequest::DependenciesV6(request)) => {
                chronos_wire::ProfileResponse::DependenciesV6(
                    self.service.dependencies_v6(request)?,
                )
            }
            Ok(chronos_wire::ProfileRequest::SchedulingV5(request)) => {
                chronos_wire::ProfileResponse::SchedulingV5(self.service.scheduling_v5(request)?)
            }
            Err(error) => {
                increment(
                    &mut self.signals.invalid_requests,
                    &mut self.signals.saturated,
                );
                chronos_wire::profile_refusal(&frame, chronos_local::wire_limits(), error)
            }
        };
        self.verify_ownership()?;
        // Delivery failure does not undo a commit and must not replay it.
        if chronos_wire::write_frame(&mut stream, &response, MAX_RESPONSE_BYTES).is_err() {
            increment(
                &mut self.signals.delivery_failed,
                &mut self.signals.saturated,
            );
        }
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Writer lock is still held. Never remove another kind of resource.
        for (path, expected) in [
            (&self.path, self.socket_identity),
            (&self.control_path, self.control_identity),
        ] {
            let _ = ownership::remove_observed_endpoint(path, Some(expected), &self.ownership);
        }
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;
    #[test]
    fn passive_counts_and_durations_clamp_and_report_fidelity_loss() {
        let mut saturated = false;
        let mut counter = u64::MAX;
        increment(&mut counter, &mut saturated);
        assert_eq!(counter, u64::MAX);
        assert!(saturated);
        saturated = false;
        let mut time = std::time::Duration::MAX;
        add_time(
            &mut time,
            std::time::Duration::from_nanos(1),
            &mut saturated,
        );
        assert_eq!(time, std::time::Duration::MAX);
        assert!(saturated);
    }
}
