//! Vellum-owned local service composition for Chronos text handoff.
#[cfg(feature = "workbench")]
pub mod workbench;
use crate::{
    CaptureJournalV1, CaptureV1, ErrorV1, SourceAuthorityV1,
    retained::{ExactTask, ReceiptError, RetainedReceiptClient},
    text_handoff::{
        BoundHandoffErrorV1, CurrentTaskV1, HandoffBindingV1, HandoffExportV1, HandoffReviewInput,
        OfflineTextReviewV1, ScopedTaskViewV1, TextErrorV1, TextHandoffAdapterV1, WorkContextV1,
        local_publication::{
            DestinationV1, ExportErrorV1, PublicationAuthorityV1, PublicationErrorV1,
        },
    },
};
use chronos_client::{WireClientV1, WireClientV4};
use chronos_protocol::{Reference, Revision};
use vellum_engine::coordinator::DocumentVersion;
use vellum_host::annotation::DocumentAssociation;
use vellum_protocol::{ClientId, RefusalV1, WorkspaceDocumentIdV1};
use vellum_service::Service;

/// Resolved and authorized by the Vellum host. A selected task supplies
/// neither a Vellum client nor a document capability.
pub struct LocalSource<'a> {
    pub client: ClientId,
    pub document: WorkspaceDocumentIdV1,
    pub association: &'a DocumentAssociation,
    pub version: &'a DocumentVersion,
}

pub enum LocalAction<'a> {
    Export {
        context: WorkContextV1<'a>,
        destination: DestinationV1<'a>,
    },
    Review {
        record: &'a [u8],
        edited: &'a [u8],
    },
}

pub enum LocalOutcome<'a> {
    Exported,
    Review(OfflineTextReviewV1<'a>),
}

#[derive(Debug, Eq, PartialEq)]
pub enum LocalFailure {
    Denied,
    Stale,
    Unavailable,
    PublicationUnknown,
}

pub struct LocalHandoff<J, C, A, P> {
    pub journal: CaptureV1<J>,
    pub receipts: RetainedReceiptClient<C>,
    pub source_authority: A,
    pub publication_authority: P,
    pub adapter: TextHandoffAdapterV1,
}

impl<J, C, A, P> LocalHandoff<J, C, A, P>
where
    J: CaptureJournalV1,
    C: WireClientV1 + WireClientV4,
    A: SourceAuthorityV1,
    P: PublicationAuthorityV1,
{
    /// Execute a single host-resolved request against the same service that
    /// processes editor input. Review returns a proposal for the host to show.
    pub fn execute<'a>(
        &mut self,
        service: &mut Service,
        selected: (&Reference, &Revision),
        source: LocalSource<'_>,
        action: LocalAction<'a>,
    ) -> Result<LocalOutcome<'a>, LocalFailure> {
        let snapshot = match self.receipts.exact_task(selected.0, selected.1) {
            Ok(ExactTask::Current(snapshot)) => snapshot,
            Ok(ExactTask::Stale) => return Err(LocalFailure::Stale),
            Err(
                ReceiptError::Transport(_) | ReceiptError::Protocol | ReceiptError::Unavailable,
            ) => {
                return Err(LocalFailure::Unavailable);
            }
        };
        let scope = crate::RetainedAllocationV1::scope(&self.receipts).to_owned();
        let task = ScopedTaskViewV1 {
            service_scope: &scope,
            snapshot: &snapshot,
        };
        let mut binding = HandoffBindingV1 {
            capture: &self.journal,
            backend: &mut self.receipts,
            source_authority: &mut self.source_authority,
        };
        let capture = binding
            .capture_live_source(source.association, source.version, &task, |expected| {
                service
                    .capture_document_for_handoff(source.client, source.document, expected)
                    .map(Some)
            })
            .map_err(|error| {
                classify(error, |refusal| match refusal {
                    RefusalV1::Stale | RefusalV1::UnknownDocument => LocalFailure::Stale,
                    RefusalV1::AuthorityDenied | RefusalV1::UnknownClient => LocalFailure::Denied,
                    _ => LocalFailure::Unavailable,
                })
            })?
            .ok_or(LocalFailure::Denied)?;
        match action {
            LocalAction::Export {
                context,
                destination,
            } => {
                self.adapter
                    .export_to_new_directory(
                        HandoffExportV1 {
                            document: source.association,
                            capture,
                            task,
                            context,
                        },
                        destination,
                        &mut self.publication_authority,
                    )
                    .map_err(|error| match error {
                        ExportErrorV1::DestinationDenied
                        | ExportErrorV1::Source(TextErrorV1::Unauthorized) => LocalFailure::Denied,
                        ExportErrorV1::Publication(PublicationErrorV1::OutcomeUnknown) => {
                            LocalFailure::PublicationUnknown
                        }
                        _ => LocalFailure::Unavailable,
                    })?;
                Ok(LocalOutcome::Exported)
            }
            LocalAction::Review { record, edited } => self
                .adapter
                .review_associated_record(
                    HandoffReviewInput {
                        record,
                        document: source.association,
                        current: &capture,
                        edited,
                        current_task: CurrentTaskV1::Observed(task),
                    },
                    &mut self.publication_authority,
                )
                .map(LocalOutcome::Review)
                .map_err(|error| {
                    classify(error, |error| match error {
                        TextErrorV1::Unauthorized => LocalFailure::Denied,
                        TextErrorV1::WrongDocument => LocalFailure::Stale,
                        _ => LocalFailure::Unavailable,
                    })
                }),
        }
    }
}

fn classify<E>(
    error: BoundHandoffErrorV1<E>,
    operation: impl FnOnce(E) -> LocalFailure,
) -> LocalFailure {
    match error {
        BoundHandoffErrorV1::AssociationUnavailable => LocalFailure::Stale,
        BoundHandoffErrorV1::Association(ErrorV1::Unauthorized) => LocalFailure::Denied,
        BoundHandoffErrorV1::Association(_) => LocalFailure::Unavailable,
        BoundHandoffErrorV1::Handoff(error) => operation(error),
    }
}
