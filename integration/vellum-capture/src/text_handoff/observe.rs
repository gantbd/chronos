//! Integration-owned timing and labels; Keystone owns only bounded arithmetic.
use super::*;
use crate::{CaptureJournalV1, CaptureV1, ErrorV1, RetainedAllocationV1, SourceAuthorityV1};
use keystone_observe::{FixedDurationHistogram, FixedDurationHistogramSnapshot, SaturatingCounter};
use std::time::{Duration, Instant};
pub struct TextHandoffAdapterV1 {
    limits: TextLimitsV1,
    exports: SaturatingCounter,
    reviews: SaturatingCounter,
    encodes: SaturatingCounter,
    restores: SaturatingCounter,
    record_high_water: usize,
    refused: SaturatingCounter,
    unauthorized: SaturatingCounter,
    source_conflicts: SaturatingCounter,
    binding_conflicts: SaturatingCounter,
    source_high_water: usize,
    handoff_high_water: usize,
    residence: FixedDurationHistogram<5>,
}
#[derive(Clone, Debug)]
pub struct TextHandoffSignalsV1 {
    pub exports: u64,
    pub reviews: u64,
    pub encodes: u64,
    pub restores: u64,
    pub record_high_water_bytes: usize,
    pub refused: u64,
    pub unauthorized: u64,
    pub source_conflicts: u64,
    pub binding_conflicts: u64,
    pub source_high_water_bytes: usize,
    pub handoff_high_water_bytes: usize,
    pub max_source_bytes: usize,
    pub max_handoff_bytes: usize,
    /// Whole-call residence includes current host policy and rendering/comparison;
    /// excludes filesystem delivery, engine scheduling and executor work.
    pub residence: FixedDurationHistogramSnapshot<5>,
    pub saturated: bool,
}
impl TextHandoffAdapterV1 {
    pub fn new(limits: TextLimitsV1) -> Self {
        Self {
            limits,
            exports: SaturatingCounter::new(),
            reviews: SaturatingCounter::new(),
            encodes: SaturatingCounter::new(),
            restores: SaturatingCounter::new(),
            record_high_water: 0,
            refused: SaturatingCounter::new(),
            unauthorized: SaturatingCounter::new(),
            source_conflicts: SaturatingCounter::new(),
            binding_conflicts: SaturatingCounter::new(),
            source_high_water: 0,
            handoff_high_water: 0,
            residence: FixedDurationHistogram::try_new([
                Duration::from_micros(50),
                Duration::from_micros(250),
                Duration::from_millis(1),
                Duration::from_millis(5),
                Duration::from_millis(20),
            ])
            .expect("fixed strictly increasing measurement buckets"),
        }
    }
    fn refusal(&mut self, error: TextErrorV1) {
        let _ = self.refused.increment();
        if error == TextErrorV1::Unauthorized {
            let _ = self.unauthorized.increment();
        }
    }
    pub fn encode(&mut self, handoff: &TextHandoffV1) -> Result<Vec<u8>, TextErrorV1> {
        let _ = self.encodes.increment();
        let started = Instant::now();
        let result = encode_text_handoff_v1(handoff, self.limits);
        let _ = self.residence.observe(started.elapsed());
        match &result {
            Ok(bytes) => self.record_high_water = self.record_high_water.max(bytes.len()),
            Err(error) => self.refusal(*error),
        }
        result
    }
    pub fn restore(&mut self, bytes: &[u8]) -> Result<TextHandoffV1, TextErrorV1> {
        let _ = self.restores.increment();
        let started = Instant::now();
        let result = decode_text_handoff_v1(bytes, self.limits);
        let _ = self.residence.observe(started.elapsed());
        match &result {
            Ok(_) => self.record_high_water = self.record_high_water.max(bytes.len()),
            Err(error) => self.refusal(*error),
        }
        result
    }
    pub fn export(
        &mut self,
        document: &DocumentAssociation,
        capture: CapturedSourceSerializationV1,
        task: ScopedTaskViewV1<'_>,
        context: WorkContextV1<'_>,
        authority: &mut impl TextHandoffAuthorityV1,
    ) -> Result<TextHandoffV1, TextErrorV1> {
        let _ = self.exports.increment();
        let started = Instant::now();
        let result =
            export_text_handoff_v1(document, capture, task, context, self.limits, authority);
        let _ = self.residence.observe(started.elapsed());
        match &result {
            Ok(handoff) => {
                self.source_high_water = self.source_high_water.max(handoff.source_bytes().len());
                self.handoff_high_water = self.handoff_high_water.max(handoff.instructions().len());
            }
            Err(error) => self.refusal(*error),
        }
        result
    }
    pub fn review<'a>(
        &mut self,
        handoff: &TextHandoffV1,
        document: &DocumentAssociation,
        current: &CapturedSourceSerializationV1,
        edited: &'a [u8],
        current_task: CurrentTaskV1<'_>,
        authority: &mut impl TextHandoffAuthorityV1,
    ) -> Result<OfflineTextReviewV1<'a>, TextErrorV1> {
        let _ = self.reviews.increment();
        let started = Instant::now();
        let result = review_offline_text_v1(
            handoff,
            document,
            current,
            edited,
            current_task,
            self.limits,
            authority,
        );
        let _ = self.residence.observe(started.elapsed());
        match &result {
            Ok(review) => {
                if review.text == TextMergeV1::Conflict {
                    let _ = self.source_conflicts.increment();
                }
                if matches!(
                    review.coordination,
                    CoordinationReviewV1::WrongService
                        | CoordinationReviewV1::ChangedIncarnation
                        | CoordinationReviewV1::RevisionRegressed
                ) {
                    let _ = self.binding_conflicts.increment();
                }
            }
            Err(error) => self.refusal(*error),
        }
        result
    }
    pub fn signals(&self) -> TextHandoffSignalsV1 {
        let residence = self.residence.snapshot();
        let saturated = residence.fidelity_lost
            || [
                self.exports,
                self.reviews,
                self.encodes,
                self.restores,
                self.refused,
                self.unauthorized,
                self.source_conflicts,
                self.binding_conflicts,
            ]
            .iter()
            .any(|counter| counter.is_saturated());
        TextHandoffSignalsV1 {
            exports: self.exports.value(),
            reviews: self.reviews.value(),
            encodes: self.encodes.value(),
            restores: self.restores.value(),
            record_high_water_bytes: self.record_high_water,
            refused: self.refused.value(),
            unauthorized: self.unauthorized.value(),
            source_conflicts: self.source_conflicts.value(),
            binding_conflicts: self.binding_conflicts.value(),
            source_high_water_bytes: self.source_high_water,
            handoff_high_water_bytes: self.handoff_high_water,
            max_source_bytes: self.limits.max_source_bytes,
            max_handoff_bytes: self.limits.max_handoff_bytes,
            residence,
            saturated,
        }
    }
}

/// Host-supplied evidence and context for one export. Constructing this value
/// does not authorize source disclosure or publication.
pub struct HandoffExportV1<'a> {
    pub document: &'a DocumentAssociation,
    pub capture: CapturedSourceSerializationV1,
    pub task: ScopedTaskViewV1<'a>,
    pub context: WorkContextV1<'a>,
}

/// Inputs from a current source capture and a previously exported offline copy.
pub struct HandoffReviewV1<'context, 'edit> {
    pub record: &'context [u8],
    pub document: &'context DocumentAssociation,
    pub current: &'context CapturedSourceSerializationV1,
    pub edited: &'edit [u8],
    pub current_task: CurrentTaskV1<'context>,
}

pub(crate) struct HandoffReviewInput<'context, 'edit> {
    pub(crate) record: &'context [u8],
    pub(crate) document: &'context DocumentAssociation,
    pub(crate) current: &'context CapturedSourceSerializationV1,
    pub(crate) edited: &'edit [u8],
    pub(crate) current_task: CurrentTaskV1<'context>,
}

/// Exact live source selected by the Vellum host.
pub struct WorkspaceHandoffSourceV1<'a> {
    pub workspace: &'a mut vellum_workspace::Workspace,
    pub member: &'a vellum_workspace::WorkspaceDocumentId,
    pub document: &'a DocumentAssociation,
    pub version: &'a DocumentVersion,
}

#[derive(Debug)]
pub enum WorkspaceHandoffErrorV1<E> {
    Capture(BoundHandoffErrorV1<vellum_workspace::WorkspaceError>),
    SourceDenied,
    Operation(BoundHandoffErrorV1<E>),
}

/// Host-supplied association and current authorization for receipt-backed handoff.
pub struct HandoffBindingV1<'a, J, B, A> {
    pub capture: &'a CaptureV1<J>,
    pub backend: &'a mut B,
    pub source_authority: &'a mut A,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BoundHandoffErrorV1<E> {
    AssociationUnavailable,
    Association(ErrorV1),
    Handoff(E),
}

impl<J: CaptureJournalV1, B: RetainedAllocationV1, A: SourceAuthorityV1>
    HandoffBindingV1<'_, J, B, A>
{
    /// Capture live bytes only after the receipt binds the task to this
    /// document. The Vellum workspace resolves membership and rechecks source
    /// read authority. The host supplies the stable/native correspondence.
    pub fn capture_workspace_source(
        &mut self,
        workspace: &mut vellum_workspace::Workspace,
        member: &vellum_workspace::WorkspaceDocumentId,
        document: &DocumentAssociation,
        expected: &DocumentVersion,
        task: &ScopedTaskViewV1<'_>,
        authorize: impl FnOnce(&vellum_workspace::WorkspaceDocumentId, &DocumentVersion) -> bool,
    ) -> Result<
        Option<CapturedSourceSerializationV1>,
        BoundHandoffErrorV1<vellum_workspace::WorkspaceError>,
    > {
        self.capture_live_source(document, expected, task, |expected| {
            workspace.capture_document_for_handoff(member, expected, authorize)
        })
    }

    /// Verify the retained allocation before requesting live bytes from a
    /// Vellum owner, including an existing in-process Vellum service.
    pub fn capture_live_source<E>(
        &mut self,
        document: &DocumentAssociation,
        expected: &DocumentVersion,
        task: &ScopedTaskViewV1<'_>,
        capture: impl FnOnce(&DocumentVersion) -> Result<Option<CapturedSourceSerializationV1>, E>,
    ) -> Result<Option<CapturedSourceSerializationV1>, BoundHandoffErrorV1<E>> {
        self.require(document, task)?;
        if expected.document != document.native {
            return Err(BoundHandoffErrorV1::AssociationUnavailable);
        }
        let captured = capture(expected).map_err(BoundHandoffErrorV1::Handoff)?;
        if captured
            .as_ref()
            .is_some_and(|source| source.source() != expected)
        {
            return Err(BoundHandoffErrorV1::AssociationUnavailable);
        }
        Ok(captured)
    }

    fn require<E>(
        &mut self,
        document: &DocumentAssociation,
        task: &ScopedTaskViewV1<'_>,
    ) -> Result<(), BoundHandoffErrorV1<E>> {
        if task.service_scope != self.backend.scope() {
            return Err(BoundHandoffErrorV1::Association(ErrorV1::WrongScope));
        }
        if !self
            .capture
            .confirms_handoff_association(
                &document.identity,
                &task.snapshot.reference,
                self.backend,
                self.source_authority,
            )
            .map_err(BoundHandoffErrorV1::Association)?
        {
            return Err(BoundHandoffErrorV1::AssociationUnavailable);
        }
        Ok(())
    }
}

impl TextHandoffAdapterV1 {
    /// Read the current source and return a review proposal. The proposal
    /// borrows only the offline edit and carries the exact Vellum version.
    pub fn review_workspace_record<'a>(
        &mut self,
        source: WorkspaceHandoffSourceV1<'_>,
        task: ScopedTaskViewV1<'_>,
        offline: (&[u8], &'a [u8]),
        binding: &mut HandoffBindingV1<
            '_,
            impl CaptureJournalV1,
            impl RetainedAllocationV1,
            impl SourceAuthorityV1,
        >,
        authorize: impl FnOnce(&vellum_workspace::WorkspaceDocumentId, &DocumentVersion) -> bool,
        authority: &mut impl TextHandoffAuthorityV1,
    ) -> Result<OfflineTextReviewV1<'a>, WorkspaceHandoffErrorV1<TextErrorV1>> {
        let current = binding
            .capture_workspace_source(
                source.workspace,
                source.member,
                source.document,
                source.version,
                &task,
                authorize,
            )
            .map_err(WorkspaceHandoffErrorV1::Capture)?
            .ok_or(WorkspaceHandoffErrorV1::SourceDenied)?;
        self.review_associated_record(
            HandoffReviewInput {
                record: offline.0,
                document: source.document,
                current: &current,
                edited: offline.1,
                current_task: CurrentTaskV1::Observed(task),
            },
            authority,
        )
        .map_err(WorkspaceHandoffErrorV1::Operation)
    }

    /// Publish a new handoff directory using a receipt-bound live capture.
    #[cfg(target_os = "macos")]
    pub fn export_workspace_to_new_directory(
        &mut self,
        source: WorkspaceHandoffSourceV1<'_>,
        task: ScopedTaskViewV1<'_>,
        publication: (WorkContextV1<'_>, local_publication::DestinationV1<'_>),
        binding: &mut HandoffBindingV1<
            '_,
            impl CaptureJournalV1,
            impl RetainedAllocationV1,
            impl SourceAuthorityV1,
        >,
        authorize: impl FnOnce(&vellum_workspace::WorkspaceDocumentId, &DocumentVersion) -> bool,
        authority: &mut impl local_publication::PublicationAuthorityV1,
    ) -> Result<(), WorkspaceHandoffErrorV1<local_publication::ExportErrorV1>> {
        let capture = binding
            .capture_workspace_source(
                source.workspace,
                source.member,
                source.document,
                source.version,
                &task,
                authorize,
            )
            .map_err(WorkspaceHandoffErrorV1::Capture)?
            .ok_or(WorkspaceHandoffErrorV1::SourceDenied)?;
        self.export_to_new_directory(
            HandoffExportV1 {
                document: source.document,
                capture,
                task,
                context: publication.0,
            },
            publication.1,
            authority,
        )
        .map_err(BoundHandoffErrorV1::Handoff)
        .map_err(WorkspaceHandoffErrorV1::Operation)
    }

    /// Restore and review the offline copy only for the journaled task.
    /// The caller supplies the current observation and owns any later edit.
    pub fn review_bound_record<'a>(
        &mut self,
        request: HandoffReviewV1<'_, 'a>,
        binding: &mut HandoffBindingV1<
            '_,
            impl CaptureJournalV1,
            impl RetainedAllocationV1,
            impl SourceAuthorityV1,
        >,
        authority: &mut impl TextHandoffAuthorityV1,
    ) -> Result<OfflineTextReviewV1<'a>, BoundHandoffErrorV1<TextErrorV1>> {
        let CurrentTaskV1::Observed(task) = &request.current_task else {
            return Err(BoundHandoffErrorV1::AssociationUnavailable);
        };
        binding.require(request.document, task)?;
        self.review_associated_record(
            HandoffReviewInput {
                record: request.record,
                document: request.document,
                current: request.current,
                edited: request.edited,
                current_task: request.current_task,
            },
            authority,
        )
    }

    pub(crate) fn review_associated_record<'a>(
        &mut self,
        request: HandoffReviewInput<'_, 'a>,
        authority: &mut impl TextHandoffAuthorityV1,
    ) -> Result<OfflineTextReviewV1<'a>, BoundHandoffErrorV1<TextErrorV1>> {
        let CurrentTaskV1::Observed(task) = &request.current_task else {
            return Err(BoundHandoffErrorV1::AssociationUnavailable);
        };
        let handoff = self
            .restore(request.record)
            .map_err(BoundHandoffErrorV1::Handoff)?;
        if handoff.service_scope != task.service_scope
            || handoff.task != task.snapshot.reference
            || task.snapshot.revision < handoff.revision
        {
            return Err(BoundHandoffErrorV1::AssociationUnavailable);
        }
        self.review(
            &handoff,
            request.document,
            request.current,
            request.edited,
            request.current_task,
            authority,
        )
        .map_err(BoundHandoffErrorV1::Handoff)
    }

    #[cfg(target_os = "macos")]
    pub fn export_bound_to_new_directory(
        &mut self,
        request: HandoffExportV1<'_>,
        destination: local_publication::DestinationV1<'_>,
        binding: &mut HandoffBindingV1<
            '_,
            impl CaptureJournalV1,
            impl RetainedAllocationV1,
            impl SourceAuthorityV1,
        >,
        authority: &mut impl local_publication::PublicationAuthorityV1,
    ) -> Result<(), BoundHandoffErrorV1<local_publication::ExportErrorV1>> {
        binding.require(request.document, &request.task)?;
        self.export_to_new_directory(request, destination, authority)
            .map_err(BoundHandoffErrorV1::Handoff)
    }

    /// Restore and compare. Even a matching task remains an unverified restored
    /// association. The result is a proposal guarded by the live source version.
    pub fn review_record<'a>(
        &mut self,
        request: HandoffReviewV1<'_, 'a>,
        authority: &mut impl TextHandoffAuthorityV1,
    ) -> Result<OfflineTextReviewV1<'a>, TextErrorV1> {
        let handoff = self.restore(request.record)?;
        self.review(
            &handoff,
            request.document,
            request.current,
            request.edited,
            request.current_task,
            authority,
        )
    }

    #[cfg(target_os = "macos")]
    pub fn export_to_new_directory(
        &mut self,
        request: HandoffExportV1<'_>,
        destination: local_publication::DestinationV1<'_>,
        authority: &mut impl local_publication::PublicationAuthorityV1,
    ) -> Result<(), local_publication::ExportErrorV1> {
        use local_publication::ExportErrorV1;
        let handoff = self
            .export(
                request.document,
                request.capture,
                request.task,
                request.context,
                authority,
            )
            .map_err(ExportErrorV1::Source)?;
        if !authority.allows_destination(request.document, &destination) {
            return Err(ExportErrorV1::DestinationDenied);
        }
        local_publication::publish_new_directory(
            destination.parent,
            destination.leaf,
            destination.work_name,
            destination.baseline_name,
            &handoff,
            self.limits,
        )
        .map_err(ExportErrorV1::Publication)
    }
}
