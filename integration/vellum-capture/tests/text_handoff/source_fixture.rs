use vellum_engine::{
    coordinator::{CoordinatorConfig, CoordinatorLimits, DocumentVersion, SourceCoordinator},
    instrumentation::Recorder,
    resource::{CapturedSourceSerializationV1, ResourceLimits},
};
use vellum_host::annotation::{DocumentAssociation, DocumentIdentity};

pub(crate) fn document(bytes: &[u8]) -> (SourceCoordinator, DocumentAssociation, DocumentVersion) {
    let mut owner = SourceCoordinator::configured(CoordinatorConfig {
        limits: CoordinatorLimits {
            max_document_incarnations: 4,
            max_participants: 4,
            max_staged_bytes: 8192,
            source: vellum_engine::source::Limits {
                max_source_bytes: 4096,
                max_edits: 4,
                max_edit_payload_bytes: 4096,
            },
        },
        invocation: None,
        history: None,
        search: None,
        annotations: None,
        resources: Some(ResourceLimits {
            max_providers: 1,
            max_live_tasks: 4,
            max_issued_tasks: 8,
            max_queued_per_destination: 4,
            max_outstanding: 4,
            max_blob_bytes: 8192,
            max_key_bytes: 64,
            max_validator_bytes: 64,
            max_encoders: 1,
            max_live_artifacts: 4,
            max_issued_artifacts: 16,
            max_held_artifact_bytes: 16_384,
            max_future_baseline_bytes: 65_536,
            max_live_reads: 4,
            max_issued_reads: 16,
            max_read_chunks: 8,
            max_held_read_bytes: 16_384,
            max_restorers: 1,
            max_restoration_bytes: 8192,
        }),
    });
    let version = owner.create(bytes, &mut Recorder::disabled()).unwrap();
    let document = DocumentAssociation {
        native: version.document.clone(),
        identity: DocumentIdentity {
            namespace: "fixture-workspace".into(),
            value: "fixture-document".into(),
        },
    };
    (owner, document, version)
}
pub(crate) fn capture(
    owner: &mut SourceCoordinator,
    version: &DocumentVersion,
) -> CapturedSourceSerializationV1 {
    let mut recorder = Recorder::disabled();
    let serialization = owner
        .source_serialization(&version.document, &mut recorder)
        .unwrap();
    let prepared = owner
        .prepare_source_artifact(version, serialization, &mut recorder)
        .unwrap();
    owner
        .capture_prepared_source_serialization(&prepared, &mut recorder)
        .unwrap()
}
