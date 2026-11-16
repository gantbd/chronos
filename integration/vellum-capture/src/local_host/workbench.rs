//! Workbench launch adapter for a Vellum-owned handoff deployment.
use super::{LocalAction, LocalFailure, LocalHandoff, LocalOutcome, LocalSource};
use crate::{
    CaptureJournalV1, SourceAuthorityV1,
    text_handoff::{OfflineTextReviewV1, local_publication::PublicationAuthorityV1},
};
use chronos_client::{WireClientV1, WireClientV4};
use chronos_tui::{HandoffAction, HandoffRequest, HandoffResult, Hub};
use vellum_service::Service;

pub struct ResolvedHandoff<'a> {
    pub source: LocalSource<'a>,
    pub action: LocalAction<'a>,
}

pub trait WorkbenchHandoffPolicy {
    /// Establish source, action and destination correspondence for this request.
    fn resolve(
        &mut self,
        service: &mut Service,
        request: &HandoffRequest,
    ) -> Result<ResolvedHandoff<'_>, LocalFailure>;

    /// Present or retain a review proposal under the host's disclosure policy.
    fn deliver_review(&mut self, review: OfflineTextReviewV1<'_>) -> Result<(), LocalFailure>;
}

/// Start Workbench with a configured Vellum service, receipt-bound handler and
/// deployment policy. The factory runs on the Vellum owner thread.
pub fn run<J, C, A, P, D>(
    hub: Hub,
    start: impl FnOnce() -> Result<(Service, LocalHandoff<J, C, A, P>, D), String> + Send + 'static,
) -> Result<(), Box<dyn std::error::Error>>
where
    J: CaptureJournalV1,
    C: WireClientV1 + WireClientV4,
    A: SourceAuthorityV1,
    P: PublicationAuthorityV1,
    D: WorkbenchHandoffPolicy,
{
    workbench_tui::run_hosted_terminal(hub, move || {
        let (service, mut handoff, mut policy) = start()?;
        let handler = move |service: &mut Service, request: HandoffRequest| {
            dispatch(&mut handoff, &mut policy, service, request).unwrap_or_else(|failure| {
                match failure {
                    LocalFailure::Denied => HandoffResult::Denied,
                    LocalFailure::Stale => HandoffResult::Stale,
                    LocalFailure::Unavailable => HandoffResult::Unavailable,
                    LocalFailure::PublicationUnknown => HandoffResult::PublicationUnknown,
                }
            })
        };
        Ok((service, handler))
    })
}

fn dispatch<J, C, A, P>(
    handoff: &mut LocalHandoff<J, C, A, P>,
    policy: &mut impl WorkbenchHandoffPolicy,
    service: &mut Service,
    request: HandoffRequest,
) -> Result<HandoffResult, LocalFailure>
where
    J: CaptureJournalV1,
    C: WireClientV1 + WireClientV4,
    A: SourceAuthorityV1,
    P: PublicationAuthorityV1,
{
    let resolved = policy.resolve(service, &request)?;
    if !matches!(
        (&request.action, &resolved.action),
        (HandoffAction::Export, LocalAction::Export { .. })
            | (HandoffAction::Review, LocalAction::Review { .. })
    ) {
        return Err(LocalFailure::Unavailable);
    }
    match handoff.execute(
        service,
        (&request.task, &request.revision),
        resolved.source,
        resolved.action,
    )? {
        LocalOutcome::Exported => Ok(HandoffResult::Exported),
        LocalOutcome::Review(review) => {
            let bytes = review.proposed_bytes.map(<[u8]>::to_vec);
            let review = OfflineTextReviewV1 {
                expected_source: review.expected_source,
                text: review.text,
                coordination: review.coordination,
                proposed_bytes: bytes.as_deref(),
            };
            policy.deliver_review(review)?;
            Ok(HandoffResult::ReviewReady)
        }
    }
}
