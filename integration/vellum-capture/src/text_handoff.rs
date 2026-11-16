//! Lossless source plus a human-readable handoff. No Org/Markdown parser,
//! evaluation, file I/O, engine mutation or authoritative status import.
use chronos_sdk_embedded::engine::{RevisionedTaskSnapshotV1, StateRevision, TaskRefV1};
use vellum_engine::{coordinator::DocumentVersion, resource::CapturedSourceSerializationV1};
use vellum_host::annotation::{DocumentAssociation, DocumentIdentity};
mod codec;
#[cfg(target_os = "macos")]
pub mod local_publication;
mod observe;
pub use codec::{decode_text_handoff_v1, encode_text_handoff_v1, max_encoded_handoff_bytes_v1};
#[cfg(target_os = "macos")]
pub(crate) use observe::HandoffReviewInput;
pub use observe::{
    BoundHandoffErrorV1, HandoffBindingV1, HandoffExportV1, HandoffReviewV1, TextHandoffAdapterV1,
    TextHandoffSignalsV1, WorkspaceHandoffErrorV1, WorkspaceHandoffSourceV1,
};

#[derive(Clone, Copy, Debug)]
pub struct TextLimitsV1 {
    pub max_source_bytes: usize,
    pub max_field_bytes: usize,
    pub max_handoff_bytes: usize,
}
pub struct WorkContextV1<'a> {
    pub title: &'a str,
    pub objective: &'a str,
    pub next_action: &'a str,
    pub dependencies: &'a str,
    pub notes: &'a str,
}
pub struct ScopedTaskViewV1<'a> {
    pub service_scope: &'a str,
    pub snapshot: &'a RevisionedTaskSnapshotV1,
}
/// Explicit full-document/context/status disclosure, independent of permission
/// to read an annotation anchor. The host establishes current rights and scope.
pub trait TextHandoffAuthorityV1 {
    fn allows_export(
        &mut self,
        document: &DocumentAssociation,
        capture: &CapturedSourceSerializationV1,
        task: &ScopedTaskViewV1<'_>,
        context: &WorkContextV1<'_>,
    ) -> bool;
    fn allows_review(
        &mut self,
        document: &DocumentAssociation,
        capture: &CapturedSourceSerializationV1,
    ) -> bool;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextErrorV1 {
    Unauthorized,
    Limit,
    Capacity,
    Invalid,
    WrongDocument,
}
pub struct TextHandoffV1 {
    origin: HandoffOriginV1,
    document: DocumentIdentity,
    service_scope: String,
    task: TaskRefV1,
    revision: StateRevision,
    original: Vec<u8>,
    instructions: String,
}
/// Provenance of this in-memory carrier, never a grant or durable authenticity proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffOriginV1 {
    CapturedHere,
    RestoredUnverified,
}
impl std::fmt::Debug for TextHandoffV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TextHandoffV1(<redacted>)")
    }
}
impl TextHandoffV1 {
    pub fn origin(&self) -> HandoffOriginV1 {
        self.origin
    }
    pub fn source_bytes(&self) -> &[u8] {
        &self.original
    }
    pub fn instructions(&self) -> &str {
        &self.instructions
    }
}
struct Text {
    value: String,
    limit: usize,
}
impl Text {
    fn put(&mut self, value: &str) -> Result<(), TextErrorV1> {
        if value.len() > self.limit.saturating_sub(self.value.len()) {
            return Err(TextErrorV1::Limit);
        }
        self.value
            .try_reserve(value.len())
            .map_err(|_| TextErrorV1::Capacity)?;
        self.value.push_str(value);
        Ok(())
    }
    fn section(&mut self, label: &str, value: &str) -> Result<(), TextErrorV1> {
        self.put(label)?;
        self.put(":\n")?;
        for line in value.split('\n') {
            self.put("    ")?;
            self.put(line)?;
            self.put("\n")?;
        }
        self.put("\n")
    }
}
pub fn export_text_handoff_v1(
    document: &DocumentAssociation,
    capture: CapturedSourceSerializationV1,
    task: ScopedTaskViewV1<'_>,
    context: WorkContextV1<'_>,
    limits: TextLimitsV1,
    authority: &mut impl TextHandoffAuthorityV1,
) -> Result<TextHandoffV1, TextErrorV1> {
    if !authority.allows_export(document, &capture, &task, &context) {
        return Err(TextErrorV1::Unauthorized);
    }
    if capture.source().document != document.native {
        return Err(TextErrorV1::WrongDocument);
    }
    if capture.bytes().len() > limits.max_source_bytes {
        return Err(TextErrorV1::Limit);
    }
    let fields = [
        context.title,
        context.objective,
        context.next_action,
        context.dependencies,
        context.notes,
        task.service_scope,
        task.snapshot.reference.task_id().as_str(),
        &document.identity.namespace,
        &document.identity.value,
    ];
    if fields.iter().any(|s| s.len() > limits.max_field_bytes) {
        return Err(TextErrorV1::Limit);
    }
    if [
        context.title,
        context.objective,
        context.next_action,
        task.service_scope,
        document.identity.namespace.as_str(),
        document.identity.value.as_str(),
    ]
    .iter()
    .any(|s| s.trim().is_empty())
    {
        return Err(TextErrorV1::Invalid);
    }
    if matches!(task.snapshot.revision, StateRevision::Finite(u64::MAX))
        || task.snapshot.reference.incarnation().get() == u64::MAX
    {
        return Err(TextErrorV1::Invalid);
    }
    let mut text = Text {
        value: String::new(),
        limit: limits.max_handoff_bytes,
    };
    text.put("WORK HANDOFF — LAST OBSERVED SNAPSHOT\n\n")?;
    text.put(
        "The source copy is ordinary editable text. It retains its original bytes and syntax.\n",
    )?;
    text.put("This handoff may be stale. It grants no lease or execution permission.\n")?;
    text.put("Offline progress and checkmarks require explicit reconciliation.\n")?;
    text.put("Verify existing execution stopped before repeating running or uncertain work.\n\n")?;
    text.put(&format!("Last observed phase: {:?}\nLast observed revision: {:?}\nService scope: {:?}\nTask: {:?}\nTask incarnation: {}\nDocument: {:?} / {:?}\n\n",
        task.snapshot.task.phase, task.snapshot.revision, task.service_scope, task.snapshot.reference.task_id().as_str(),
        task.snapshot.reference.incarnation().get(), document.identity.namespace, document.identity.value))?;
    for (label, value) in [
        ("Title", context.title),
        ("Intended outcome", context.objective),
        ("Next action", context.next_action),
        ("Dependencies and context", context.dependencies),
        ("Notes", context.notes),
    ] {
        text.section(label, value)?;
    }
    let (_, original) = capture.into_parts();
    Ok(TextHandoffV1 {
        origin: HandoffOriginV1::CapturedHere,
        document: document.identity.clone(),
        service_scope: task.service_scope.into(),
        task: task.snapshot.reference.clone(),
        revision: task.snapshot.revision,
        original,
        instructions: text.value,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextMergeV1 {
    Unchanged,
    AlreadyPresent,
    Proposed,
    Conflict,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoordinationReviewV1 {
    RestoredUnverified,
    Required,
    Unavailable,
    TaskMissing,
    WrongService,
    ChangedIncarnation,
    RevisionRegressed,
}
pub enum CurrentTaskV1<'a> {
    Unavailable,
    Missing,
    Observed(ScopedTaskViewV1<'a>),
}
/// A proposal only. Applying it requires the document owner's normal current
/// authority/version guard; it can never execute or release a Chronos attempt.
pub struct OfflineTextReviewV1<'a> {
    pub expected_source: DocumentVersion,
    pub text: TextMergeV1,
    pub coordination: CoordinationReviewV1,
    pub proposed_bytes: Option<&'a [u8]>,
}
pub fn review_offline_text_v1<'a>(
    handoff: &TextHandoffV1,
    document: &DocumentAssociation,
    current: &CapturedSourceSerializationV1,
    edited: &'a [u8],
    current_task: CurrentTaskV1<'_>,
    limits: TextLimitsV1,
    authority: &mut impl TextHandoffAuthorityV1,
) -> Result<OfflineTextReviewV1<'a>, TextErrorV1> {
    if !authority.allows_review(document, current) {
        return Err(TextErrorV1::Unauthorized);
    }
    if document.identity != handoff.document || current.source().document != document.native {
        return Err(TextErrorV1::WrongDocument);
    }
    if edited.len() > limits.max_source_bytes
        || current.bytes().len() > limits.max_source_bytes
        || handoff.original.len() > limits.max_source_bytes
    {
        return Err(TextErrorV1::Limit);
    }
    std::str::from_utf8(edited).map_err(|_| TextErrorV1::Invalid)?;
    let text = if edited == handoff.original {
        TextMergeV1::Unchanged
    } else if edited == current.bytes() {
        TextMergeV1::AlreadyPresent
    } else if current.bytes() == handoff.original {
        TextMergeV1::Proposed
    } else {
        TextMergeV1::Conflict
    };
    let coordination = match current_task {
        CurrentTaskV1::Unavailable => CoordinationReviewV1::Unavailable,
        CurrentTaskV1::Missing => CoordinationReviewV1::TaskMissing,
        CurrentTaskV1::Observed(view) if view.service_scope != handoff.service_scope => {
            CoordinationReviewV1::WrongService
        }
        CurrentTaskV1::Observed(view) if view.snapshot.reference != handoff.task => {
            CoordinationReviewV1::ChangedIncarnation
        }
        CurrentTaskV1::Observed(view) if view.snapshot.revision < handoff.revision => {
            CoordinationReviewV1::RevisionRegressed
        }
        CurrentTaskV1::Observed(_) if handoff.origin == HandoffOriginV1::RestoredUnverified => {
            CoordinationReviewV1::RestoredUnverified
        }
        CurrentTaskV1::Observed(_) => CoordinationReviewV1::Required,
    };
    Ok(OfflineTextReviewV1 {
        expected_source: current.source().clone(),
        text,
        coordination,
        proposed_bytes: if text == TextMergeV1::Proposed {
            Some(edited)
        } else {
            None
        },
    })
}
