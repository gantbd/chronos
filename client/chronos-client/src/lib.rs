//! Transport-neutral remote client port; no engine, host or concrete effects.
#![forbid(unsafe_code)]
pub mod observer;
pub mod workbench;
/// Atomic graph-plus-eligibility admission with command-scoped reconciliation.
pub trait ScheduledAdmissionWireClientV7 {
    type Error;
    fn request_scheduled_admission_v7(
        &mut self,
        request: chronos_protocol::coordination_v7::Request,
    ) -> Result<chronos_protocol::coordination_v7::Response, Self::Error>;
}
/// Explicit opt-in. Execution-stop attestations are deliberately absent.
pub trait SchedulingWireClientV5 {
    type Error;
    fn request_scheduling_v5(
        &mut self,
        request: chronos_protocol::scheduling_v5::Request,
    ) -> Result<chronos_protocol::scheduling_v5::Response, Self::Error>;
}
pub trait WireClientV4 {
    type Error;
    fn request_v4(
        &mut self,
        request: chronos_protocol::coordination_v4::Request,
    ) -> Result<chronos_protocol::coordination_v4::Response, Self::Error>;
}

pub trait WireClientV3 {
    type Error;
    fn request_v3(
        &mut self,
        request: chronos_protocol::coordination_v3::Request,
    ) -> Result<chronos_protocol::coordination_v3::Response, Self::Error>;
}

/// Explicit opt-in; implementing V1 never grants V2 mutation support.
pub trait WireClientV2 {
    type Error;
    fn request_v2(
        &mut self,
        request: chronos_protocol::coordination_v2::Request,
    ) -> Result<chronos_protocol::coordination_v2::Response, Self::Error>;
}

/// Common serializable client contract for MCP, human interfaces and local or
/// remote hosts. Transport failure is not a delivered domain refusal and must
/// never trigger an automatic mutation retry.
pub trait WireClientV1 {
    type Error;
    fn request(
        &mut self,
        request: chronos_protocol::Request,
    ) -> Result<chronos_protocol::Response, Self::Error>;
}

pub trait DependencyWireClientV6 {
    type Error;
    fn request_dependencies_v6(
        &mut self,
        request: chronos_protocol::dependencies_v6::Request,
    ) -> Result<chronos_protocol::dependencies_v6::Response, Self::Error>;
}
