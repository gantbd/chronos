//! Authenticated local owner clients for the read-only product-alignment host.
//!
//! This module packages observation only. It cannot create a proposal, mutate
//! Chronos, apply a recommendation, answer a human gate, or grant authority.

use super::{
    AlignmentBasisV1, AlignmentErrorV1, AlignmentLimitsV1, OwnerEvidenceRefV1,
    frontier::{
        ChronosControlTaskReadErrorV1, ChronosControlTaskSourceV1, LiveAlignmentFrontierV1,
        MoiraCohortBindingV1, MoiraCohortFrontierSourceV1, MoiraDeliberationFrontierSourceV1,
        OwnerComposedEvidenceFrontierSourceV1, TellusGroundingFrontierSourceV1,
        VellumLocatorBudgetPolicyFrontierSourceV1, VellumLocatorGoalFrontierSourceV1,
    },
};
use moira_client::{
    DeliberationServicePort,
    unix::{DeliberationClientErrorV1, DeliberationClientLimitsV1, DeliberationUnixPort},
};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use vellum_adapter_local_authentication::{LocalCredentialClaimProviderV1, LocalCredentialStoreV1};
use vellum_transport::{AuthenticatedUnixConnector, RemoteTransport, TransportLimits};

type AuthenticatedVellumTransportV1 = RemoteTransport<AuthenticatedUnixConnector>;
type LocalGoalSourceV1 = VellumLocatorGoalFrontierSourceV1<AuthenticatedVellumTransportV1>;
type LocalTaskSourceV1 = ChronosControlTaskSourceV1<chronos_local::LocalClient>;
type LocalCohortSourceV1 = MoiraCohortFrontierSourceV1<ReconnectingDeliberationPortV1>;
type LocalDeliberationSourceV1 = MoiraDeliberationFrontierSourceV1<ReconnectingDeliberationPortV1>;
type LocalGroundingSourceV1 = TellusGroundingFrontierSourceV1<tellus_local_client::LocalPort>;
type LocalBudgetSourceV1 =
    VellumLocatorBudgetPolicyFrontierSourceV1<AuthenticatedVellumTransportV1>;
type LocalEvidenceSourceV1 = OwnerComposedEvidenceFrontierSourceV1<
    LocalDeliberationSourceV1,
    LocalGroundingSourceV1,
    LocalBudgetSourceV1,
>;

pub type LocalProductAlignmentFrontierV1 = LiveAlignmentFrontierV1<
    LocalGoalSourceV1,
    LocalTaskSourceV1,
    LocalCohortSourceV1,
    LocalEvidenceSourceV1,
>;

/// Private local endpoints and authentication material. Debug output is
/// deliberately content-free so credential paths and bearer values cannot
/// enter logs or operator surfaces through ordinary formatting.
pub struct LocalAlignmentConnectionsV1 {
    chronos_directory: PathBuf,
    moira_socket: PathBuf,
    vellum_socket: PathBuf,
    vellum_credential: PathBuf,
    tellus_endpoint: SocketAddr,
    tellus_bearer_token: String,
    timeout: Duration,
    max_moira_frame_bytes: usize,
}

impl std::fmt::Debug for LocalAlignmentConnectionsV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LocalAlignmentConnectionsV1(<redacted>)")
    }
}

impl LocalAlignmentConnectionsV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chronos_directory: PathBuf,
        moira_socket: PathBuf,
        vellum_socket: PathBuf,
        vellum_credential: PathBuf,
        tellus_endpoint: SocketAddr,
        tellus_bearer_token: String,
        timeout: Duration,
        max_moira_frame_bytes: usize,
    ) -> Result<Self, LocalAlignmentBuildErrorV1> {
        if [
            chronos_directory.as_path(),
            moira_socket.as_path(),
            vellum_socket.as_path(),
            vellum_credential.as_path(),
        ]
        .iter()
        .any(|path| !path.is_absolute())
            || !tellus_endpoint.ip().is_loopback()
            || timeout.is_zero()
            || max_moira_frame_bytes == 0
            || tellus_bearer_token.is_empty()
            || tellus_bearer_token.len() > 8192
            || tellus_bearer_token.contains(['\r', '\n'])
        {
            return Err(LocalAlignmentBuildErrorV1::InvalidConfiguration);
        }
        Ok(Self {
            chronos_directory,
            moira_socket,
            vellum_socket,
            vellum_credential,
            tellus_endpoint,
            tellus_bearer_token,
            timeout,
            max_moira_frame_bytes,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalAlignmentBindingV1 {
    pub chronos_scope: String,
    pub deliberation: moira_protocol::deliberation::DeliberationIdV1,
    pub external_advocate: [u8; 16],
    pub product_manager: [u8; 16],
    pub frontier_supervisor: [u8; 16],
    pub goal_locator: String,
    pub budget_policy_locator: String,
    pub budget_policy_resource: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalAlignmentBuildErrorV1 {
    InvalidConfiguration,
    VellumCredential,
    VellumTransport,
    VellumConnection,
    MoiraConnection,
    TellusConnection,
}

/// Connect every owner through its public local client and build the exact
/// twice-sampled read-only frontier retained by the proposal basis.
pub fn connect_local_product_alignment_v1(
    retained: AlignmentBasisV1,
    binding: LocalAlignmentBindingV1,
    connections: LocalAlignmentConnectionsV1,
    limits: AlignmentLimitsV1,
) -> Result<LocalProductAlignmentFrontierV1, LocalAlignmentBuildErrorV1> {
    if binding.chronos_scope != retained.chronos_scope()
        || binding.goal_locator.is_empty()
        || binding.budget_policy_locator.is_empty()
        || binding.budget_policy_resource.is_empty()
        || binding.goal_locator.chars().any(char::is_control)
        || binding.budget_policy_locator.chars().any(char::is_control)
        || binding.budget_policy_resource.chars().any(char::is_control)
        || retained.budget_policy().owner() != "vellum"
        || retained.budget_policy().resource() != binding.budget_policy_resource
    {
        return Err(LocalAlignmentBuildErrorV1::InvalidConfiguration);
    }

    let goal_client = connect_vellum(
        &connections.vellum_socket,
        &connections.vellum_credential,
        connections.timeout,
    )?;
    let budget_client = connect_vellum(
        &connections.vellum_socket,
        &connections.vellum_credential,
        connections.timeout,
    )?;
    let goal = VellumLocatorGoalFrontierSourceV1::new(
        goal_client,
        binding.goal_locator,
        retained.goal().document.namespace.clone(),
        retained.goal().document.value.clone(),
    )
    .map_err(|_| LocalAlignmentBuildErrorV1::InvalidConfiguration)?;
    let task = ChronosControlTaskSourceV1::new(
        chronos_local::LocalClient::new(connections.chronos_directory),
        limits.max_field_bytes,
    )
    .map_err(|_: ChronosControlTaskReadErrorV1<()>| {
        LocalAlignmentBuildErrorV1::InvalidConfiguration
    })?;

    let cohort = retained.cohort().clone();
    let cohort_binding = MoiraCohortBindingV1::new(
        binding.chronos_scope.clone(),
        binding.deliberation,
        (
            binding.external_advocate,
            cohort.external_advocate().clone(),
        ),
        (binding.product_manager, cohort.product_manager().clone()),
        (
            binding.frontier_supervisor,
            cohort.frontier_supervisor().clone(),
        ),
        limits,
    )
    .map_err(map_alignment_error)?;
    let moira_limits = DeliberationClientLimitsV1 {
        max_frame_bytes: connections.max_moira_frame_bytes,
        max_requests: 1,
        io_timeout: connections.timeout,
    };
    let cohort_source = MoiraCohortFrontierSourceV1::new(
        ReconnectingDeliberationPortV1::new(connections.moira_socket.clone(), moira_limits)?,
        cohort_binding,
    );
    let deliberation_source = MoiraDeliberationFrontierSourceV1::new(
        ReconnectingDeliberationPortV1::new(connections.moira_socket, moira_limits)?,
        binding.chronos_scope.clone(),
        binding.deliberation,
        cohort.clone(),
        limits,
    )
    .map_err(map_alignment_error)?;
    let grounding_source = TellusGroundingFrontierSourceV1::new(
        tellus_local_client::LocalPort::new(
            connections.tellus_endpoint,
            Some(connections.tellus_bearer_token),
            connections.timeout,
        )
        .map_err(|_| LocalAlignmentBuildErrorV1::TellusConnection)?,
        binding.chronos_scope.clone(),
        cohort.clone(),
        limits,
    )
    .map_err(map_alignment_error)?;
    let budget_source = VellumLocatorBudgetPolicyFrontierSourceV1::new(
        budget_client,
        binding.budget_policy_locator,
        binding.budget_policy_resource,
        binding.chronos_scope,
        cohort,
        limits,
    )
    .map_err(map_alignment_error)?;
    let retained_budget_policy: OwnerEvidenceRefV1 = retained.budget_policy().clone();
    let evidence = OwnerComposedEvidenceFrontierSourceV1::new(
        deliberation_source,
        grounding_source,
        budget_source,
        retained_budget_policy,
        limits,
    )
    .map_err(map_alignment_error)?;

    Ok(LiveAlignmentFrontierV1::new(
        retained,
        goal,
        task,
        cohort_source,
        evidence,
        limits,
    ))
}

fn connect_vellum(
    socket: &Path,
    credential: &Path,
    timeout: Duration,
) -> Result<vellum_client::Client<AuthenticatedVellumTransportV1>, LocalAlignmentBuildErrorV1> {
    let mut store = LocalCredentialStoreV1::default();
    let record = store
        .load(credential)
        .map_err(|_| LocalAlignmentBuildErrorV1::VellumCredential)?;
    let connector = AuthenticatedUnixConnector::with_provider_and_timeout(
        socket.to_path_buf(),
        LocalCredentialClaimProviderV1::new(record),
        timeout,
    )
    .map_err(|_| LocalAlignmentBuildErrorV1::VellumTransport)?;
    let transport = RemoteTransport::new(connector, TransportLimits::reference())
        .map_err(|_| LocalAlignmentBuildErrorV1::VellumTransport)?;
    vellum_client::Client::connect(transport)
        .map_err(|_| LocalAlignmentBuildErrorV1::VellumConnection)
}

fn map_alignment_error(_: AlignmentErrorV1) -> LocalAlignmentBuildErrorV1 {
    LocalAlignmentBuildErrorV1::InvalidConfiguration
}

/// Reconnect for every Moira read. The registry revision brackets in the
/// higher-level readers preserve cross-connection stability, while a daemon
/// restart does not permanently exhaust or poison a long-running PM worker.
pub struct ReconnectingDeliberationPortV1 {
    endpoint: PathBuf,
    limits: DeliberationClientLimitsV1,
}

impl ReconnectingDeliberationPortV1 {
    fn new(
        endpoint: PathBuf,
        limits: DeliberationClientLimitsV1,
    ) -> Result<Self, LocalAlignmentBuildErrorV1> {
        if !endpoint.is_absolute()
            || limits.max_frame_bytes == 0
            || limits.max_requests != 1
            || limits.io_timeout.is_zero()
        {
            return Err(LocalAlignmentBuildErrorV1::InvalidConfiguration);
        }
        let _ = DeliberationUnixPort::connect(&endpoint, limits)
            .map_err(|_| LocalAlignmentBuildErrorV1::MoiraConnection)?;
        Ok(Self { endpoint, limits })
    }
}

impl DeliberationServicePort for ReconnectingDeliberationPortV1 {
    type Error = DeliberationClientErrorV1;

    fn execute(
        &mut self,
        command: moira_protocol::deliberation::CommandV1,
    ) -> Result<moira_protocol::deliberation::ReplyV1, Self::Error> {
        let mut port = DeliberationUnixPort::connect(&self.endpoint, self.limits)?;
        port.execute(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_configuration_is_absolute_bounded_and_redacted() {
        let valid = LocalAlignmentConnectionsV1::new(
            "/private/chronos".into(),
            "/private/moira.sock".into(),
            "/private/vellum.sock".into(),
            "/private/vellum.credential".into(),
            "127.0.0.1:4500".parse().unwrap(),
            "secret".into(),
            Duration::from_secs(2),
            64 * 1024,
        )
        .unwrap();
        assert_eq!(
            format!("{valid:?}"),
            "LocalAlignmentConnectionsV1(<redacted>)"
        );
        assert_eq!(
            LocalAlignmentConnectionsV1::new(
                "relative".into(),
                "/private/moira.sock".into(),
                "/private/vellum.sock".into(),
                "/private/vellum.credential".into(),
                "127.0.0.1:4500".parse().unwrap(),
                "secret".into(),
                Duration::from_secs(2),
                64 * 1024,
            )
            .unwrap_err(),
            LocalAlignmentBuildErrorV1::InvalidConfiguration
        );
        assert_eq!(
            LocalAlignmentConnectionsV1::new(
                "/private/chronos".into(),
                "/private/moira.sock".into(),
                "/private/vellum.sock".into(),
                "/private/vellum.credential".into(),
                "192.0.2.1:4500".parse().unwrap(),
                "secret\nheader".into(),
                Duration::from_secs(2),
                64 * 1024,
            )
            .unwrap_err(),
            LocalAlignmentBuildErrorV1::InvalidConfiguration
        );
        assert_eq!(
            LocalAlignmentConnectionsV1::new(
                "/private/chronos".into(),
                "/private/moira.sock".into(),
                "/private/vellum.sock".into(),
                "/private/vellum.credential".into(),
                "127.0.0.1:4500".parse().unwrap(),
                String::new(),
                Duration::from_secs(2),
                64 * 1024,
            )
            .unwrap_err(),
            LocalAlignmentBuildErrorV1::InvalidConfiguration
        );
    }
}
