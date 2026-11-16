//! Disposable native composition. All positive policy answers are explicit
//! fixture premises, not a production authorization evaluator or persisted grant.
use chronos_adapter_checkpoint::DirectoryServiceEnvelopeStore;
use chronos_sdk_embedded::{
    engine::{EngineLimits, RecoveryLimits, StateRevision, TaskSpecV2},
    host::{
        native::RequestV1, native_allocation::AllocationReceiptV1, native_v2::RequestV2,
        native_v3::*,
    },
    service::{
        Authorizer, Limits,
        atomic::{AllocationAuthorizer, AtomicLimits, AtomicService, InvocationAuthorizer},
        coordination_v2::AuthorizerV2,
    },
};
use chronos_vellum_capture::{
    journal::DirectoryCaptureJournalV1, native::ScopedAtomicServiceV1, *,
};
use vellum_engine::{
    coordinator::{CoordinatorLimits, SourceCoordinator},
    instrumentation::Recorder,
};
use vellum_host::annotation::*;
struct FixturePolicy;
impl Authorizer for FixturePolicy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl AuthorizerV2 for FixturePolicy {
    fn allows_v2(&mut self, _: &RequestV2) -> bool {
        true
    }
}
impl InvocationAuthorizer for FixturePolicy {
    fn allows_invocation(&mut self, _: &InvocationRequestV1) -> bool {
        true
    }
    fn allows_retained_result(&mut self, _: &InvocationV1, _: &MutationV1) -> bool {
        true
    }
}
impl AllocationAuthorizer for FixturePolicy {
    fn allows_allocation(&mut self, _: &InvocationV1) -> bool {
        true
    }
    fn allows_allocation_result(&mut self, _: &InvocationV1, _: &AllocationReceiptV1) -> bool {
        true
    }
}
struct SourcePolicy(SourceReferenceV1);
impl SourceAuthorityV1 for SourcePolicy {
    fn allows(&mut self, source: &SourceReferenceV1) -> bool {
        *source == self.0
    }
}
struct Temporary(std::path::PathBuf);
impl Temporary {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "chronos-native-capture-example-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
type Backend = ScopedAtomicServiceV1<FixturePolicy, DirectoryServiceEnvelopeStore>;
fn backend(directory: &std::path::Path, reopen: bool) -> Backend {
    let store = DirectoryServiceEnvelopeStore::open(directory, 1_000_000, 16).unwrap();
    let limits = Limits {
        max_graph_tasks: 4,
        max_page_entries: 4,
    };
    let recovery = RecoveryLimits::new(900_000, EngineLimits::default());
    let atomic = AtomicLimits {
        max_envelope_bytes: 1_000_000,
        max_retained_bytes: 8192,
        max_command_bytes: 2048,
        max_key_bytes: 64,
        max_invocations: 4,
    };
    let mut service = if reopen {
        AtomicService::open_existing(store, FixturePolicy, limits, recovery, atomic).unwrap()
    } else {
        AtomicService::create_new(store, FixturePolicy, limits, recovery, atomic).unwrap()
    };
    service.enable_allocation_receipts().unwrap();
    ScopedAtomicServiceV1::new("fixture-register".into(), service, 64).unwrap()
}
fn run() {
    let temporary = Temporary::new();
    let journal_directory = temporary.0.join("capture");
    let service_directory = temporary.0.join("service");
    std::fs::create_dir(&journal_directory).unwrap();
    std::fs::create_dir(&service_directory).unwrap();
    let limits = LimitsV1 {
        max_field_bytes: 64,
        max_record_bytes: 4096,
        max_dependencies: 4,
        max_capabilities: 4,
    };
    let mut documents = SourceCoordinator::new(CoordinatorLimits {
        max_document_incarnations: 2,
        max_participants: 2,
        max_staged_bytes: 128,
        source: vellum_engine::source::Limits {
            max_source_bytes: 128,
            max_edits: 4,
            max_edit_payload_bytes: 128,
        },
    });
    let mut recorder = Recorder::disabled();
    let text = "Fixture prose and code remain source text.";
    let version = documents.create(text.as_bytes(), &mut recorder).unwrap();
    let document = DocumentAssociation {
        native: version.document.clone(),
        identity: DocumentIdentity {
            namespace: "fixture-workspace".into(),
            value: "fixture-document".into(),
        },
    };
    let association = SourceAssociation {
        native: version,
        evidence: SourceEvidence {
            domain: "fixture-version".into(),
            token: "original".into(),
            byte_length: text.len() as u64,
        },
        text: text.into(),
    };
    let target = Target::Range {
        source: association.evidence.clone(),
        start: 0,
        end: 7,
    };
    let mut authority = SourcePolicy(SourceReferenceV1 {
        document: document.identity.clone(),
        target: target.clone(),
    });
    let source = capture_source(&document, &association, target, limits, &mut authority).unwrap();
    let intent = IntentV1 {
        source: source.clone(),
        service_scope: "fixture-register".into(),
        invocation: InvocationV1 {
            serial: 1,
            key: "fixture-intent".into(),
        },
        expected: StateRevision::ZERO,
        task: TaskSpecV2::new("fixture-task"),
    };
    let journal = DirectoryCaptureJournalV1::open(&journal_directory, limits, 16).unwrap();
    let mut capture = CaptureV1::begin(journal, intent, limits, &mut authority).unwrap();
    let mut service = backend(&service_directory, false);
    assert_eq!(
        capture.step(&mut service, &mut authority),
        Ok(StepV1::Attempted)
    );
    // Lose all live integration/service state before association publication.
    drop(capture);
    drop(service);
    let journal = DirectoryCaptureJournalV1::open(&journal_directory, limits, 16).unwrap();
    let mut capture = CaptureV1::recover(journal, &source, limits, &mut authority).unwrap();
    let mut service = backend(&service_directory, true);
    let StepV1::Associated(original) = capture.step(&mut service, &mut authority).unwrap() else {
        panic!("receipt")
    };
    assert_eq!(capture.signals().offers, 0);
    assert_eq!(service.service_mut().signals().executions, 0);
    drop(capture);
    drop(service);
    // The mapping itself also survives another complete restart.
    let journal = DirectoryCaptureJournalV1::open(&journal_directory, limits, 16).unwrap();
    let mut capture = CaptureV1::recover(journal, &source, limits, &mut authority).unwrap();
    let mut service = backend(&service_directory, true);
    assert_eq!(
        capture.step(&mut service, &mut authority),
        Ok(StepV1::Associated(original))
    );
    assert_eq!(capture.signals().commits, 0);
    assert_eq!(
        documents
            .read(&document.native, &mut recorder)
            .unwrap()
            .source
            .text(),
        text
    );
}
fn main() {
    run();
    println!("Native capture: original mapping recovered from local storage without resubmission.");
}
#[cfg(test)]
mod tests {
    #[test]
    fn source_and_both_local_registers_compose_through_native_sdk() {
        super::run();
    }
}
