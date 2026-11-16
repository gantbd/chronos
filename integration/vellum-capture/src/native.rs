//! Trusted service-register binding. Scope is correlation and is never inferred
//! from a task name or filesystem locator. The supplied policy stays authoritative.
use crate::{CaptureBackendV1, ErrorV1};
use chronos_sdk_embedded::{
    host::{
        native_allocation::{AllocationClientV1, AllocationReplyV1},
        native_v3::{InvocationReplyV1, InvocationRequestV1, InvocationV1},
        service_envelope::ServiceEnvelopeStore,
    },
    service::atomic::{AllocationAuthorizer, AtomicError, AtomicService},
};

pub struct ScopedAtomicServiceV1<A, S> {
    scope: String,
    service: AtomicService<A, S>,
}
impl<A, S> ScopedAtomicServiceV1<A, S> {
    /// Host-established correspondence with one exclusively owned register.
    /// Changing its data store requires a different scope or explicit migration.
    pub fn new(
        scope: String,
        service: AtomicService<A, S>,
        max_scope_bytes: usize,
    ) -> Result<Self, ErrorV1> {
        if scope.is_empty() {
            return Err(ErrorV1::Invalid);
        }
        if scope.len() > max_scope_bytes {
            return Err(ErrorV1::Limit);
        }
        Ok(Self { scope, service })
    }
    /// Trusted composition-root access. No additional permission is conferred.
    pub fn service_mut(&mut self) -> &mut AtomicService<A, S> {
        &mut self.service
    }
}
impl<A: AllocationAuthorizer, S: ServiceEnvelopeStore> AllocationClientV1
    for ScopedAtomicServiceV1<A, S>
{
    type TransportError = AtomicError;
    fn allocation(&mut self, invocation: InvocationV1) -> Result<AllocationReplyV1, AtomicError> {
        self.service.allocation(invocation)
    }
}
impl<A: AllocationAuthorizer, S: ServiceEnvelopeStore> CaptureBackendV1
    for ScopedAtomicServiceV1<A, S>
{
    fn scope(&self) -> &str {
        &self.scope
    }
    fn invoke(&mut self, request: InvocationRequestV1) -> Result<InvocationReplyV1, AtomicError> {
        self.service.invoke(request)
    }
}
