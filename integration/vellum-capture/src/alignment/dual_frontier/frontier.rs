//! Stable, owner-attributed reads of the two worker positions reconciled by
//! the product manager. Sources publish facts; this sampler cannot infer them.

use super::{FrontierKindV1, WorkerFrontierV1};
use crate::alignment::{AlignmentBasisV1, AlignmentLimitsV1, PrincipalRefV1};

/// One worker-owned observation port. Implementations decide how the named
/// principal authenticates and publishes its position; the caller pre-binds
/// scope, role and identity and accepts no substitution.
pub trait WorkerFrontierSourceV1 {
    type Error;

    fn current(
        &mut self,
        chronos_scope: &str,
        kind: FrontierKindV1,
        principal: &PrincipalRefV1,
    ) -> Result<WorkerFrontierV1, Self::Error>;
}

#[derive(Clone, Eq, PartialEq)]
pub struct StableWorkerFrontiersV1 {
    external: WorkerFrontierV1,
    constrained: WorkerFrontierV1,
}

impl std::fmt::Debug for StableWorkerFrontiersV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StableWorkerFrontiersV1(<redacted>)")
    }
}

impl StableWorkerFrontiersV1 {
    pub fn external(&self) -> &WorkerFrontierV1 {
        &self.external
    }

    pub fn constrained(&self) -> &WorkerFrontierV1 {
        &self.constrained
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum StableWorkerFrontierErrorV1<X, C> {
    External(X),
    Constrained(C),
    Invalid,
    Unstable,
}

/// Double-collects both independently owned positions. Equality across the two
/// complete samples proves only stable observation during this call; it does
/// not claim a distributed transaction or transfer either worker's authority.
pub struct StableWorkerFrontierSamplerV1<X, C> {
    external: X,
    constrained: C,
    chronos_scope: String,
    external_principal: PrincipalRefV1,
    constrained_principal: PrincipalRefV1,
    limits: AlignmentLimitsV1,
}

impl<X, C> StableWorkerFrontierSamplerV1<X, C>
where
    X: WorkerFrontierSourceV1,
    C: WorkerFrontierSourceV1,
{
    pub fn new(
        basis: &AlignmentBasisV1,
        constrained_principal: PrincipalRefV1,
        external: X,
        constrained: C,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, StableWorkerFrontierErrorV1<X::Error, C::Error>> {
        super::super::revalidate_basis(basis, limits)
            .map_err(|_| StableWorkerFrontierErrorV1::Invalid)?;
        let external_principal = basis.cohort().frontier_supervisor().clone();
        PrincipalRefV1::new(constrained_principal.as_str().to_owned(), limits)
            .map_err(|_| StableWorkerFrontierErrorV1::Invalid)?;
        if constrained_principal == external_principal
            || constrained_principal == *basis.cohort().external_advocate()
            || constrained_principal == *basis.cohort().product_manager()
        {
            return Err(StableWorkerFrontierErrorV1::Invalid);
        }
        Ok(Self {
            external,
            constrained,
            chronos_scope: basis.chronos_scope().to_owned(),
            external_principal,
            constrained_principal,
            limits,
        })
    }

    pub fn sample(
        &mut self,
    ) -> Result<StableWorkerFrontiersV1, StableWorkerFrontierErrorV1<X::Error, C::Error>> {
        let external_first = self
            .external
            .current(
                &self.chronos_scope,
                FrontierKindV1::ExternalSupervisor,
                &self.external_principal,
            )
            .map_err(StableWorkerFrontierErrorV1::External)?;
        let constrained_first = self
            .constrained
            .current(
                &self.chronos_scope,
                FrontierKindV1::PantheonConstrainedWorker,
                &self.constrained_principal,
            )
            .map_err(StableWorkerFrontierErrorV1::Constrained)?;
        let external_second = self
            .external
            .current(
                &self.chronos_scope,
                FrontierKindV1::ExternalSupervisor,
                &self.external_principal,
            )
            .map_err(StableWorkerFrontierErrorV1::External)?;
        let constrained_second = self
            .constrained
            .current(
                &self.chronos_scope,
                FrontierKindV1::PantheonConstrainedWorker,
                &self.constrained_principal,
            )
            .map_err(StableWorkerFrontierErrorV1::Constrained)?;

        for (value, kind, principal) in [
            (
                &external_first,
                FrontierKindV1::ExternalSupervisor,
                &self.external_principal,
            ),
            (
                &external_second,
                FrontierKindV1::ExternalSupervisor,
                &self.external_principal,
            ),
            (
                &constrained_first,
                FrontierKindV1::PantheonConstrainedWorker,
                &self.constrained_principal,
            ),
            (
                &constrained_second,
                FrontierKindV1::PantheonConstrainedWorker,
                &self.constrained_principal,
            ),
        ] {
            super::revalidate_worker(value, self.limits)
                .map_err(|_| StableWorkerFrontierErrorV1::Invalid)?;
            if value.kind() != kind || value.principal() != principal {
                return Err(StableWorkerFrontierErrorV1::Invalid);
            }
        }
        if external_first != external_second || constrained_first != constrained_second {
            return Err(StableWorkerFrontierErrorV1::Unstable);
        }
        Ok(StableWorkerFrontiersV1 {
            external: external_first,
            constrained: constrained_first,
        })
    }

    pub fn into_sources(self) -> (X, C) {
        (self.external, self.constrained)
    }
}
