//! Native composition, outside both engines and their distribution facades.
//! No UI, parsing, execution, clock advance, automatic retry or source mutation.
#![forbid(unsafe_code)]
pub mod journal;
#[cfg(target_os = "macos")]
pub mod local_host;
pub mod native;
pub mod retained;
pub mod text_handoff;
use chronos_sdk_embedded::{
    engine::{StateRevision, TaskRefV1, TaskSpecV2},
    host::{
        native_allocation::{AllocationClientV1, AllocationReceiptV1, AllocationResultV1},
        native_v3::{
            InvocationRefusalV1, InvocationReplyV1, InvocationRequestV1, InvocationResponseV1,
            InvocationV1, MutationOutcomeV1, MutationV1,
        },
    },
};
use keystone_storage::{CommitGate, CommitGateRefusal, CommitOutcome, Revision};
use vellum_host::annotation::{DocumentAssociation, DocumentIdentity, SourceAssociation, Target};

/// Vellum's stable source-version evidence and original anchor, never a path,
/// parser-generated task identity, process-local ID or refreshed selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceReferenceV1 {
    pub document: DocumentIdentity,
    pub target: Target,
}

/// The host must establish current source disclosure/capture rights. A source
/// association, persisted identity or this predicate's earlier answer is not a grant.
pub trait SourceAuthorityV1 {
    fn allows(&mut self, source: &SourceReferenceV1) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitsV1 {
    pub max_field_bytes: usize,
    pub max_record_bytes: usize,
    pub max_dependencies: usize,
    pub max_capabilities: usize,
}

/// One intent creates exactly one task. No task summary or source text is
/// synthesized; the caller supplies the existing finite task vocabulary.
#[derive(Clone, Eq, PartialEq)]
pub struct IntentV1 {
    pub source: SourceReferenceV1,
    pub service_scope: String,
    pub invocation: InvocationV1,
    pub expected: StateRevision,
    pub task: TaskSpecV2,
}
impl std::fmt::Debug for IntentV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IntentV1(<redacted>)")
    }
}

/// Capture from host-established Vellum associations. The source host owns
/// their correspondence and evidence domain, including restart validation.
/// Both point profiles remain intact; offsets are UTF-8 byte boundaries.
pub fn capture_source(
    document: &DocumentAssociation,
    source: &SourceAssociation,
    target: Target,
    limits: LimitsV1,
    authority: &mut impl SourceAuthorityV1,
) -> Result<SourceReferenceV1, ErrorV1> {
    for field in [
        &document.identity.namespace,
        &document.identity.value,
        &target.source().domain,
        &target.source().token,
    ] {
        if field.len() > limits.max_field_bytes {
            return Err(ErrorV1::Limit);
        }
    }
    let reference = SourceReferenceV1 {
        document: document.identity.clone(),
        target,
    };
    if !authority.allows(&reference) {
        return Err(ErrorV1::Unauthorized);
    }
    if source.native.document != document.native
        || reference.target.source() != &source.evidence
        || u64::try_from(source.text.len()).ok() != Some(source.evidence.byte_length)
    {
        return Err(ErrorV1::Invalid);
    }
    let boundary = |offset| {
        usize::try_from(offset)
            .ok()
            .is_some_and(|offset| source.text.is_char_boundary(offset))
    };
    let valid = match &reference.target {
        Target::Document { .. } => true,
        Target::Point { offset, .. } => boundary(*offset),
        Target::Range { start, end, .. } => start <= end && boundary(*start) && boundary(*end),
    };
    if !valid {
        return Err(ErrorV1::Invalid);
    }
    Ok(reference)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PhaseV1 {
    Prepared,
    /// Persisted BEFORE the only execution offer. Recovery may query, never send.
    Dispatched,
    Associated(AllocationReceiptV1),
    Rejected(MutationOutcomeV1),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordV1 {
    pub revision: u64,
    pub intent: IntentV1,
    pub phase: PhaseV1,
}

/// One exclusively owned, bounded register per capture intent. Implementations
/// must enforce CAS, validate a closed persisted schema and honor limits BEFORE
/// allocating on load. Load returns only committed, durable state. No in-memory
/// implementation qualifies physical durability. Storage selection/encoding is
/// the integration host's job, not Vellum's or Chronos's engine.
pub trait CaptureJournalV1 {
    type Error;
    fn load(&mut self, limits: LimitsV1) -> Result<Option<RecordV1>, Self::Error>;
    fn commit(
        &mut self,
        expected: Option<Revision>,
        record: &RecordV1,
    ) -> Result<CommitOutcome, Self::Error>;
}

/// A trusted binding to ONE service register. The host owns scope uniqueness
/// and correspondence; the scope is correlation, never authority or a locator.
pub trait RetainedAllocationV1: AllocationClientV1 {
    fn scope(&self) -> &str;
}

impl<B: CaptureBackendV1> RetainedAllocationV1 for B {
    fn scope(&self) -> &str {
        CaptureBackendV1::scope(self)
    }
}

pub trait CaptureBackendV1: AllocationClientV1 {
    fn scope(&self) -> &str;
    fn invoke(
        &mut self,
        request: InvocationRequestV1,
    ) -> Result<InvocationReplyV1, Self::TransportError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorV1 {
    Unauthorized,
    Invalid,
    Limit,
    Conflict,
    WrongScope,
    Storage,
    Fenced,
    Exhausted,
    Transport,
    Protocol,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StepV1 {
    Attempted,
    Associated(AllocationReceiptV1),
    Rejected(MutationOutcomeV1),
    /// Includes legacy/unavailable receipts. No retry is implied.
    Unavailable,
    Refused(chronos_sdk_embedded::host::native_v3::InvocationRefusalV1),
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SignalsV1 {
    pub steps: u64,
    pub offers: u64,
    pub queries: u64,
    pub commits: u64,
    pub commit_failures: u64,
    pub refusals: u64,
    pub failures: u64,
    pub residence: std::time::Duration,
    pub backend_time: std::time::Duration,
    pub storage_time: std::time::Duration,
    pub saturated: bool,
}
impl SignalsV1 {
    fn duration(
        value: &mut std::time::Duration,
        elapsed: std::time::Duration,
        saturated: &mut bool,
    ) {
        match value.checked_add(elapsed) {
            Some(next) => *value = next,
            None => {
                *value = std::time::Duration::MAX;
                *saturated = true;
            }
        }
    }
    fn count(value: &mut u64, saturated: &mut bool) {
        match value.checked_add(1) {
            Some(next) => *value = next,
            None => *saturated = true,
        }
    }
}

pub struct CaptureV1<J> {
    journal: J,
    record: RecordV1,
    gate: CommitGate,
    signals: SignalsV1,
}
impl<J: CaptureJournalV1> CaptureV1<J> {
    pub fn begin(
        mut journal: J,
        intent: IntentV1,
        limits: LimitsV1,
        authority: &mut impl SourceAuthorityV1,
    ) -> Result<Self, ErrorV1> {
        if !authority.allows(&intent.source) {
            return Err(ErrorV1::Unauthorized);
        }
        validate(&intent, limits)?;
        if journal
            .load(limits)
            .map_err(|_| ErrorV1::Storage)?
            .is_some()
        {
            return Err(ErrorV1::Conflict);
        }
        let record = RecordV1 {
            revision: 0,
            intent,
            phase: PhaseV1::Prepared,
        };
        let mut gate = CommitGate::new(None);
        let outcome = journal.commit(None, &record);
        gate.resolve(Revision(0), outcome)
            .map_err(|_| ErrorV1::Storage)?;
        Ok(Self {
            journal,
            record,
            gate,
            signals: SignalsV1 {
                commits: 1,
                ..Default::default()
            },
        })
    }

    /// Expected source is supplied before presence lookup for authorization and
    /// exact association checking. Recovery does not refresh/rebase the anchor.
    pub fn recover(
        mut journal: J,
        source: &SourceReferenceV1,
        limits: LimitsV1,
        authority: &mut impl SourceAuthorityV1,
    ) -> Result<Self, ErrorV1> {
        if !authority.allows(source) {
            return Err(ErrorV1::Unauthorized);
        }
        let record = journal
            .load(limits)
            .map_err(|_| ErrorV1::Storage)?
            .ok_or(ErrorV1::Invalid)?;
        if record.intent.source != *source {
            return Err(ErrorV1::Conflict);
        }
        validate(&record.intent, limits)?;
        match &record.phase {
            PhaseV1::Associated(receipt) => validate_receipt(&record.intent, receipt)?,
            PhaseV1::Rejected(MutationOutcomeV1::Applied) => return Err(ErrorV1::Invalid),
            _ => {}
        }
        let minimum = match record.phase {
            PhaseV1::Prepared => 0,
            PhaseV1::Dispatched => 1,
            _ => 2,
        };
        if record.revision != minimum {
            return Err(ErrorV1::Invalid);
        }
        let gate = CommitGate::new(Some(Revision(record.revision)));
        Ok(Self {
            journal,
            record,
            gate,
            signals: SignalsV1::default(),
        })
    }

    /// Check the exact document/task association through a fresh authorized
    /// receipt read. This never offers an invocation or changes the journal.
    pub fn confirms_handoff_association(
        &self,
        document: &DocumentIdentity,
        task: &TaskRefV1,
        backend: &mut impl RetainedAllocationV1,
        authority: &mut impl SourceAuthorityV1,
    ) -> Result<bool, ErrorV1> {
        if !authority.allows(&self.record.intent.source) {
            return Err(ErrorV1::Unauthorized);
        }
        if self.gate.is_fenced() {
            return Err(ErrorV1::Fenced);
        }
        if backend.scope() != self.record.intent.service_scope {
            return Err(ErrorV1::WrongScope);
        }
        let PhaseV1::Associated(original) = &self.record.phase else {
            return Ok(false);
        };
        if *document != self.record.intent.source.document
            || original.tasks.as_slice() != std::slice::from_ref(task)
        {
            return Ok(false);
        }
        let result = backend
            .allocation(self.record.intent.invocation.clone())
            .map_err(|_| ErrorV1::Transport)?;
        match result {
            Ok(AllocationResultV1::Created(current)) if current == *original => Ok(true),
            Ok(AllocationResultV1::Created(_)) => Err(ErrorV1::Protocol),
            Ok(AllocationResultV1::Unavailable)
            | Err(InvocationRefusalV1::Expired | InvocationRefusalV1::OutOfSequence) => Ok(false),
            Err(InvocationRefusalV1::Unauthorized) => Err(ErrorV1::Unauthorized),
            Err(_) => Err(ErrorV1::Protocol),
        }
    }

    pub fn signals(&self) -> SignalsV1 {
        self.signals
    }

    /// Bounded single step: at most one native call, no polling/sleep/retry loop.
    /// Residence, backend and storage timing have no content-bearing labels.
    pub fn step(
        &mut self,
        backend: &mut impl CaptureBackendV1,
        authority: &mut impl SourceAuthorityV1,
    ) -> Result<StepV1, ErrorV1> {
        SignalsV1::count(&mut self.signals.steps, &mut self.signals.saturated);
        let start = std::time::Instant::now();
        let result = self.step_inner(backend, authority);
        SignalsV1::duration(
            &mut self.signals.residence,
            start.elapsed(),
            &mut self.signals.saturated,
        );
        if result.is_err() {
            SignalsV1::count(&mut self.signals.failures, &mut self.signals.saturated);
        }
        if matches!(result, Err(ErrorV1::Unauthorized) | Ok(StepV1::Refused(_))) {
            SignalsV1::count(&mut self.signals.refusals, &mut self.signals.saturated);
        }
        result
    }
    fn step_inner(
        &mut self,
        backend: &mut impl CaptureBackendV1,
        authority: &mut impl SourceAuthorityV1,
    ) -> Result<StepV1, ErrorV1> {
        if !authority.allows(&self.record.intent.source) {
            return Err(ErrorV1::Unauthorized);
        }
        if self.gate.is_fenced() {
            return Err(ErrorV1::Fenced);
        }
        if backend.scope() != self.record.intent.service_scope {
            return Err(ErrorV1::WrongScope);
        }
        if let PhaseV1::Rejected(outcome) = self.record.phase {
            SignalsV1::count(&mut self.signals.queries, &mut self.signals.saturated);
            let start = std::time::Instant::now();
            let reply = backend.invoke(InvocationRequestV1::Lookup {
                invocation: self.record.intent.invocation.clone(),
            });
            SignalsV1::duration(
                &mut self.signals.backend_time,
                start.elapsed(),
                &mut self.signals.saturated,
            );
            return match reply.map_err(|_| ErrorV1::Transport)? {
                Ok(InvocationResponseV1::Outcome { serial, result })
                    if serial == self.record.intent.invocation.serial && result == outcome =>
                {
                    Ok(StepV1::Rejected(result))
                }
                Err(refusal) => Ok(StepV1::Refused(refusal)),
                _ => Err(ErrorV1::Protocol),
            };
        }
        if self.record.phase == PhaseV1::Prepared {
            self.persist(PhaseV1::Dispatched)?;
            SignalsV1::count(&mut self.signals.offers, &mut self.signals.saturated);
            let intent = &self.record.intent;
            let start = std::time::Instant::now();
            let reply = backend.invoke(InvocationRequestV1::Execute {
                invocation: intent.invocation.clone(),
                command: MutationV1::SubmitGraph {
                    expected: intent.expected,
                    tasks: vec![intent.task.clone()],
                },
            });
            SignalsV1::duration(
                &mut self.signals.backend_time,
                start.elapsed(),
                &mut self.signals.saturated,
            );
            let reply = reply.map_err(|_| ErrorV1::Transport)?;
            return match reply {
                Ok(InvocationResponseV1::Outcome { serial, result })
                    if serial == intent.invocation.serial =>
                {
                    if result == MutationOutcomeV1::Applied {
                        Ok(StepV1::Attempted)
                    } else {
                        self.persist(PhaseV1::Rejected(result))?;
                        Ok(StepV1::Rejected(result))
                    }
                }
                Ok(_) => Err(ErrorV1::Protocol),
                Err(refusal) => Ok(StepV1::Refused(refusal)),
            };
        }
        SignalsV1::count(&mut self.signals.queries, &mut self.signals.saturated);
        let start = std::time::Instant::now();
        let reply = backend.allocation(self.record.intent.invocation.clone());
        SignalsV1::duration(
            &mut self.signals.backend_time,
            start.elapsed(),
            &mut self.signals.saturated,
        );
        match reply.map_err(|_| ErrorV1::Transport)? {
            Err(refusal) => Ok(StepV1::Refused(refusal)),
            Ok(AllocationResultV1::Unavailable) => Ok(StepV1::Unavailable),
            Ok(AllocationResultV1::Created(receipt)) => {
                validate_receipt(&self.record.intent, &receipt)?;
                if let PhaseV1::Associated(original) = &self.record.phase {
                    if *original != receipt {
                        return Err(ErrorV1::Protocol);
                    }
                } else {
                    self.persist(PhaseV1::Associated(receipt.clone()))?;
                }
                Ok(StepV1::Associated(receipt))
            }
        }
    }
    fn persist(&mut self, phase: PhaseV1) -> Result<(), ErrorV1> {
        let proposed = self.gate.next_revision().map_err(|error| match error {
            CommitGateRefusal::Fenced => ErrorV1::Fenced,
            CommitGateRefusal::RevisionExhausted => ErrorV1::Exhausted,
        })?;
        let candidate = RecordV1 {
            revision: proposed.0,
            intent: self.record.intent.clone(),
            phase,
        };
        let start = std::time::Instant::now();
        let outcome = self
            .journal
            .commit(self.gate.confirmed_revision(), &candidate);
        SignalsV1::duration(
            &mut self.signals.storage_time,
            start.elapsed(),
            &mut self.signals.saturated,
        );
        if self.gate.resolve(proposed, outcome).is_err() {
            SignalsV1::count(
                &mut self.signals.commit_failures,
                &mut self.signals.saturated,
            );
            return Err(ErrorV1::Storage);
        }
        SignalsV1::count(&mut self.signals.commits, &mut self.signals.saturated);
        self.record = candidate;
        Ok(())
    }
}

fn validate_receipt(intent: &IntentV1, receipt: &AllocationReceiptV1) -> Result<(), ErrorV1> {
    if receipt.tasks.len() != 1
        || receipt.tasks[0].task_id() != &intent.task.id
        || !matches!(receipt.revision, StateRevision::Finite(n) if n != u64::MAX)
        || receipt.tasks[0].incarnation().get() == u64::MAX
    {
        return Err(ErrorV1::Protocol);
    }
    Ok(())
}
fn validate(intent: &IntentV1, limits: LimitsV1) -> Result<(), ErrorV1> {
    let source = intent.source.target.source();
    let fields = [
        &intent.source.document.namespace,
        &intent.source.document.value,
        &source.domain,
        &source.token,
        &intent.service_scope,
        &intent.invocation.key,
    ];
    if intent.invocation.serial == 0 || fields.iter().any(|field| field.is_empty()) {
        return Err(ErrorV1::Invalid);
    }
    let mut bytes = 256_usize; // logical scalar, variant and single-receipt reservation, not RSS/encoding
    let mut charge = |field: &str| -> Result<(), ErrorV1> {
        if field.len() > limits.max_field_bytes {
            return Err(ErrorV1::Limit);
        }
        bytes = bytes
            .checked_add(
                field
                    .len()
                    .checked_mul(2)
                    .and_then(|value| value.checked_add(8))
                    .ok_or(ErrorV1::Limit)?,
            )
            .ok_or(ErrorV1::Limit)?;
        if bytes > limits.max_record_bytes {
            return Err(ErrorV1::Limit);
        }
        Ok(())
    };
    for field in fields {
        charge(field)?;
    }
    charge(intent.task.id.as_str())?;
    if intent.task.dependencies.len() > limits.max_dependencies
        || intent.task.required_capabilities.len() > limits.max_capabilities
    {
        return Err(ErrorV1::Limit);
    }
    for id in &intent.task.dependencies {
        charge(id.as_str())?;
    }
    for capability in &intent.task.required_capabilities {
        charge(capability)?;
    }
    let valid = match &intent.source.target {
        Target::Document { .. } => true,
        Target::Point { offset, .. } => *offset <= source.byte_length,
        Target::Range { start, end, .. } => start <= end && *end <= source.byte_length,
    };
    if !valid {
        return Err(ErrorV1::Invalid);
    }
    Ok(())
}
