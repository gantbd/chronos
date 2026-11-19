//! Agora-attributed worker positions with owner-returned Vellum context.
//!
//! One dedicated room topic carries only worker-position publications. Agora
//! owns author, message identity and sequence; Vellum artifact targets supply
//! context evidence. Self-asserted contribution or artifact identities are not
//! accepted from the message body.

use super::{FrontierKindV1, WorkerFrontierV1, frontier::WorkerFrontierSourceV1};
use crate::alignment::{AlignmentLimitsV1, OwnerEvidenceRefV1, PrincipalRefV1};
use agora_client::v2::{ClientErrorV2, ClientV2, ServicePortV2};
use agora_protocol::v2::{
    CommandV2, OutcomeV2, PageCursorV2, ReplyV2, TopicIdV1, VellumArtifactMessageV1,
};
use chronos_protocol::{Phase, Revision, alignment_v1 as alignment};
use chronos_sdk_embedded::engine::{
    RevisionedTaskSnapshotV1, StateRevision, TaskId, TaskPhase, TaskRefV1, TaskSnapshotV1,
};
use serde::{Deserialize, Serialize};

pub const WORKER_POSITION_PROFILE_V1: &str = "chronos-worker-position-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerPositionKindV1 {
    ExternalSupervisor,
    PantheonConstrainedWorker,
}

impl WorkerPositionKindV1 {
    fn domain(self) -> FrontierKindV1 {
        match self {
            Self::ExternalSupervisor => FrontierKindV1::ExternalSupervisor,
            Self::PantheonConstrainedWorker => FrontierKindV1::PantheonConstrainedWorker,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerPositionPublicationV1 {
    pub profile: String,
    pub version: u16,
    pub chronos_scope: String,
    pub kind: WorkerPositionKindV1,
    pub principal: String,
    pub task: Option<alignment::ControlTaskObservation>,
    pub attempt: Option<alignment::OwnerEvidenceReference>,
    pub lease: Option<alignment::OwnerEvidenceReference>,
    pub evidence: Vec<alignment::OwnerEvidenceReference>,
}

pub fn encode_worker_position_v1(
    publication: &WorkerPositionPublicationV1,
    limits: AlignmentLimitsV1,
) -> Result<String, AgoraWorkerFrontierErrorV1<()>> {
    validate_publication(publication, limits)?;
    let bytes = serde_json::to_vec(publication).map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)?;
    if bytes.len() > limits.max_record_bytes {
        return Err(AgoraWorkerFrontierErrorV1::Limit);
    }
    String::from_utf8(bytes).map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)
}

fn validate_publication<E>(
    publication: &WorkerPositionPublicationV1,
    limits: AlignmentLimitsV1,
) -> Result<(), AgoraWorkerFrontierErrorV1<E>> {
    if publication.profile != WORKER_POSITION_PROFILE_V1
        || publication.version != 1
        || publication.chronos_scope.is_empty()
        || publication.chronos_scope.len() > limits.max_field_bytes
        || publication.chronos_scope.chars().any(char::is_control)
    {
        return Err(AgoraWorkerFrontierErrorV1::Invalid);
    }
    let principal = PrincipalRefV1::new(publication.principal.clone(), limits)
        .map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)?;
    let evidence = publication
        .evidence
        .iter()
        .map(|value| evidence_from_public(value, limits))
        .collect::<Result<Vec<_>, _>>()?;
    WorkerFrontierV1::new(
        publication.kind.domain(),
        principal,
        publication
            .task
            .as_ref()
            .map(task_from_public)
            .transpose()?,
        publication
            .attempt
            .as_ref()
            .map(|value| evidence_from_public(value, limits))
            .transpose()?,
        publication
            .lease
            .as_ref()
            .map(|value| evidence_from_public(value, limits))
            .transpose()?,
        None,
        evidence,
        limits,
    )
    .map(|_| ())
    .map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)
}

#[derive(Debug)]
pub enum AgoraWorkerFrontierErrorV1<E> {
    Source(ClientErrorV2<E>),
    Refused,
    Invalid,
    Limit,
    Incomplete,
    Unavailable,
}

pub struct AgoraWorkerFrontierSourceV1<P> {
    client: ClientV2<P>,
    room_id: String,
    topic_id: TopicIdV1,
    page_size: u32,
    max_pages: usize,
    limits: AlignmentLimitsV1,
}

impl<P: ServicePortV2> AgoraWorkerFrontierSourceV1<P> {
    pub fn new(
        client: ClientV2<P>,
        room_id: String,
        topic_id: TopicIdV1,
        page_size: u32,
        max_pages: usize,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AgoraWorkerFrontierErrorV1<P::Error>> {
        if room_id.is_empty()
            || room_id.len() > limits.max_field_bytes
            || room_id.chars().any(char::is_control)
            || page_size == 0
            || max_pages == 0
            || limits.max_record_bytes == 0
        {
            return Err(AgoraWorkerFrontierErrorV1::Invalid);
        }
        Ok(Self {
            client,
            room_id,
            topic_id,
            page_size,
            max_pages,
            limits,
        })
    }

    pub fn into_client(self) -> ClientV2<P> {
        self.client
    }

    fn latest(
        &mut self,
        principal: &PrincipalRefV1,
    ) -> Result<VellumArtifactMessageV1, AgoraWorkerFrontierErrorV1<P::Error>> {
        let mut cursor: Option<PageCursorV2> = None;
        let mut revision = None;
        let mut latest: Option<VellumArtifactMessageV1> = None;
        for page_index in 0..self.max_pages {
            let reply = self
                .client
                .execute(CommandV2::VellumArtifactMessages {
                    room_id: self.room_id.clone(),
                    cursor,
                    limit: self.page_size,
                })
                .map_err(AgoraWorkerFrontierErrorV1::Source)?;
            let page = match reply {
                ReplyV2::Accepted(OutcomeV2::VellumArtifactMessages(page)) => page,
                ReplyV2::Accepted(_) => return Err(AgoraWorkerFrontierErrorV1::Invalid),
                ReplyV2::Refused(_) => return Err(AgoraWorkerFrontierErrorV1::Refused),
            };
            if page.room_id != self.room_id || revision.is_some_and(|value| value != page.revision)
            {
                return Err(AgoraWorkerFrontierErrorV1::Invalid);
            }
            revision = Some(page.revision);
            for message in page.messages {
                if message.author == principal.as_str()
                    && message.topic_id == Some(self.topic_id)
                    && latest
                        .as_ref()
                        .is_none_or(|retained| retained.sequence < message.sequence)
                {
                    latest = Some(message);
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                return latest.ok_or(AgoraWorkerFrontierErrorV1::Unavailable);
            }
            if page_index + 1 == self.max_pages {
                return Err(AgoraWorkerFrontierErrorV1::Incomplete);
            }
        }
        Err(AgoraWorkerFrontierErrorV1::Incomplete)
    }
}

impl<P: ServicePortV2> WorkerFrontierSourceV1 for AgoraWorkerFrontierSourceV1<P> {
    type Error = AgoraWorkerFrontierErrorV1<P::Error>;

    fn current(
        &mut self,
        chronos_scope: &str,
        kind: FrontierKindV1,
        principal: &PrincipalRefV1,
    ) -> Result<WorkerFrontierV1, Self::Error> {
        let message = self.latest(principal)?;
        if message.body.len() > self.limits.max_record_bytes
            || message.reply_to_root.is_some()
            || message.references.is_empty()
            || message.references.len() > self.limits.max_evidence
        {
            return Err(AgoraWorkerFrontierErrorV1::Invalid);
        }
        let publication: WorkerPositionPublicationV1 =
            serde_json::from_str(&message.body).map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)?;
        if publication.profile != WORKER_POSITION_PROFILE_V1
            || publication.version != 1
            || publication.chronos_scope != chronos_scope
            || publication.kind.domain() != kind
            || publication.principal != principal.as_str()
        {
            return Err(AgoraWorkerFrontierErrorV1::Invalid);
        }

        let contribution = OwnerEvidenceRefV1::new(
            "agora".into(),
            format!(
                "room/{}/message/{}",
                self.room_id,
                hex(&message.message_id.into_bytes())
            ),
            message.sequence.to_string(),
            self.limits,
        )
        .map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)?;
        let mut evidence = publication
            .evidence
            .iter()
            .map(|value| evidence_from_public(value, self.limits))
            .collect::<Result<Vec<_>, _>>()?;
        for reference in message.references {
            let target = reference
                .target
                .ok_or(AgoraWorkerFrontierErrorV1::Unavailable)?;
            evidence.push(
                OwnerEvidenceRefV1::new(
                    "vellum".into(),
                    format!(
                        "workspace/{}/document/{}/reference/{}",
                        target.workspace_id,
                        target.document_id,
                        hex(&reference.reference_id.into_bytes())
                    ),
                    target.source_revision.to_string(),
                    self.limits,
                )
                .map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)?,
            );
        }
        WorkerFrontierV1::new(
            kind,
            principal.clone(),
            publication
                .task
                .as_ref()
                .map(task_from_public)
                .transpose()?,
            publication
                .attempt
                .as_ref()
                .map(|value| evidence_from_public(value, self.limits))
                .transpose()?,
            publication
                .lease
                .as_ref()
                .map(|value| evidence_from_public(value, self.limits))
                .transpose()?,
            Some(contribution),
            evidence,
            self.limits,
        )
        .map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)
    }
}

fn evidence_from_public<E>(
    value: &alignment::OwnerEvidenceReference,
    limits: AlignmentLimitsV1,
) -> Result<OwnerEvidenceRefV1, AgoraWorkerFrontierErrorV1<E>> {
    OwnerEvidenceRefV1::new(
        value.owner.clone(),
        value.resource.clone(),
        value.revision.clone(),
        limits,
    )
    .map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)
}

fn task_from_public<E>(
    value: &alignment::ControlTaskObservation,
) -> Result<RevisionedTaskSnapshotV1, AgoraWorkerFrontierErrorV1<E>> {
    let incarnation = decimal(&value.reference.incarnation)?;
    let revision = match &value.revision {
        Revision::Finite(value) => StateRevision::finite(decimal(value)?),
        Revision::Exhausted => StateRevision::Exhausted,
    };
    Ok(RevisionedTaskSnapshotV1 {
        reference: TaskRefV1::from_observation(
            TaskId::new(value.reference.task.clone()),
            incarnation,
        ),
        revision,
        task: TaskSnapshotV1 {
            phase: match value.snapshot.phase {
                Phase::Blocked => TaskPhase::Blocked,
                Phase::Ready => TaskPhase::Ready,
                Phase::Running => TaskPhase::Running,
                Phase::Uncertain => TaskPhase::Uncertain,
                Phase::Succeeded => TaskPhase::Succeeded,
                Phase::Failed => TaskPhase::Failed,
                Phase::Cancelled => TaskPhase::Cancelled,
            },
            attempt_number: value.snapshot.attempt_number,
            dependency_count: value.snapshot.dependency_count,
            required_capability_count: value.snapshot.required_capability_count,
            priority: value.snapshot.priority,
            max_attempts: value.snapshot.max_attempts,
        },
    })
}

fn decimal<E>(value: &str) -> Result<u64, AgoraWorkerFrontierErrorV1<E>> {
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(AgoraWorkerFrontierErrorV1::Invalid);
    }
    value
        .parse()
        .map_err(|_| AgoraWorkerFrontierErrorV1::Invalid)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_client::v2::PortFailureV2;
    use agora_protocol::v2::{
        ClientIdV2, MessageIdV1, MessageVellumArtifactReferenceV1, RequestV2,
        VellumArtifactMessagePageV1, VellumArtifactReferenceIdV1, VellumDocumentTargetV1,
    };
    use std::collections::VecDeque;

    struct Port {
        replies: VecDeque<ReplyV2>,
    }

    impl ServicePortV2 for Port {
        type Error = ();

        fn execute(
            &mut self,
            _: ClientIdV2,
            _: RequestV2,
        ) -> Result<ReplyV2, PortFailureV2<Self::Error>> {
            Ok(self.replies.pop_front().expect("bounded test reply"))
        }
    }

    fn limits() -> AlignmentLimitsV1 {
        AlignmentLimitsV1 {
            max_field_bytes: 4096,
            max_evidence: 8,
            max_next_tasks: 4,
            max_record_bytes: 64 * 1024,
        }
    }

    fn principal(value: &str) -> PrincipalRefV1 {
        PrincipalRefV1::new(value.into(), limits()).unwrap()
    }

    fn publication(principal: &str) -> WorkerPositionPublicationV1 {
        WorkerPositionPublicationV1 {
            profile: WORKER_POSITION_PROFILE_V1.into(),
            version: 1,
            chronos_scope: "delivery".into(),
            kind: WorkerPositionKindV1::ExternalSupervisor,
            principal: principal.into(),
            task: None,
            attempt: None,
            lease: None,
            evidence: vec![alignment::OwnerEvidenceReference {
                owner: "tellus".into(),
                resource: "grounding/delivery".into(),
                revision: "3".into(),
            }],
        }
    }

    fn message(sequence: u64, body: String) -> VellumArtifactMessageV1 {
        VellumArtifactMessageV1 {
            message_id: MessageIdV1::new([u8::try_from(sequence).unwrap(); 16]),
            sequence,
            author: "frontier-supervisor".into(),
            logical_time: sequence,
            body,
            addressed_to: vec!["product-manager".into()],
            reply_to_root: None,
            topic_id: Some(TopicIdV1::new([4; 16])),
            references: vec![MessageVellumArtifactReferenceV1 {
                reference_id: VellumArtifactReferenceIdV1::new([8; 16]),
                target: Some(VellumDocumentTargetV1 {
                    workspace_id: 5,
                    document_id: 6,
                    source_revision: 7,
                }),
            }],
        }
    }

    fn page(messages: Vec<VellumArtifactMessageV1>, next: Option<PageCursorV2>) -> ReplyV2 {
        ReplyV2::Accepted(OutcomeV2::VellumArtifactMessages(
            VellumArtifactMessagePageV1 {
                revision: 12,
                room_id: "alignment-room".into(),
                messages,
                oldest_retained_sequence: 0,
                next_message_sequence: 20,
                next,
            },
        ))
    }

    fn source(replies: Vec<ReplyV2>, max_pages: usize) -> AgoraWorkerFrontierSourceV1<Port> {
        AgoraWorkerFrontierSourceV1::new(
            ClientV2::new(
                ClientIdV2(1),
                Port {
                    replies: replies.into(),
                },
            ),
            "alignment-room".into(),
            TopicIdV1::new([4; 16]),
            8,
            max_pages,
            limits(),
        )
        .unwrap()
    }

    #[test]
    fn derives_contribution_and_vellum_evidence_from_agora_owner_facts() {
        let body =
            encode_worker_position_v1(&publication("frontier-supervisor"), limits()).unwrap();
        let mut source = source(vec![page(vec![message(9, body)], None)], 1);
        let observed = source
            .current(
                "delivery",
                FrontierKindV1::ExternalSupervisor,
                &principal("frontier-supervisor"),
            )
            .unwrap();

        let contribution = observed.contribution().unwrap();
        assert_eq!(contribution.owner(), "agora");
        assert_eq!(contribution.revision(), "9");
        assert!(
            contribution
                .resource()
                .starts_with("room/alignment-room/message/")
        );
        assert_eq!(observed.evidence().len(), 2);
        assert!(
            observed
                .evidence()
                .iter()
                .any(|value| value.owner() == "tellus")
        );
        assert!(observed.evidence().iter().any(|value| {
            value.owner() == "vellum"
                && value
                    .resource()
                    .starts_with("workspace/5/document/6/reference/")
                && value.revision() == "7"
        }));
    }

    #[test]
    fn latest_topic_publication_cannot_fall_back_to_an_older_valid_value() {
        let valid =
            encode_worker_position_v1(&publication("frontier-supervisor"), limits()).unwrap();
        let mut source = source(
            vec![page(
                vec![message(8, valid), message(9, "not-json".into())],
                None,
            )],
            1,
        );
        assert!(matches!(
            source.current(
                "delivery",
                FrontierKindV1::ExternalSupervisor,
                &principal("frontier-supervisor"),
            ),
            Err(AgoraWorkerFrontierErrorV1::Invalid)
        ));
    }

    #[test]
    fn incomplete_history_and_tombstoned_vellum_context_refuse() {
        let valid =
            encode_worker_position_v1(&publication("frontier-supervisor"), limits()).unwrap();
        let mut incomplete = source(
            vec![page(vec![message(8, valid.clone())], Some(PageCursorV2(4)))],
            1,
        );
        assert!(matches!(
            incomplete.current(
                "delivery",
                FrontierKindV1::ExternalSupervisor,
                &principal("frontier-supervisor"),
            ),
            Err(AgoraWorkerFrontierErrorV1::Incomplete)
        ));

        let mut tombstone = message(9, valid);
        tombstone.references[0].target = None;
        let mut unavailable = source(vec![page(vec![tombstone], None)], 1);
        assert!(matches!(
            unavailable.current(
                "delivery",
                FrontierKindV1::ExternalSupervisor,
                &principal("frontier-supervisor"),
            ),
            Err(AgoraWorkerFrontierErrorV1::Unavailable)
        ));
    }

    #[test]
    fn message_body_cannot_substitute_scope_role_or_principal() {
        for publication in [
            WorkerPositionPublicationV1 {
                chronos_scope: "other".into(),
                ..publication("frontier-supervisor")
            },
            WorkerPositionPublicationV1 {
                kind: WorkerPositionKindV1::PantheonConstrainedWorker,
                ..publication("frontier-supervisor")
            },
            publication("someone-else"),
        ] {
            let body = encode_worker_position_v1(&publication, limits()).unwrap();
            let mut source = source(vec![page(vec![message(9, body)], None)], 1);
            assert!(matches!(
                source.current(
                    "delivery",
                    FrontierKindV1::ExternalSupervisor,
                    &principal("frontier-supervisor"),
                ),
                Err(AgoraWorkerFrontierErrorV1::Invalid)
            ));
        }
    }

    #[test]
    fn encoder_refuses_invalid_publication_before_agora_delivery() {
        let invalid_attempt = WorkerPositionPublicationV1 {
            attempt: Some(alignment::OwnerEvidenceReference {
                owner: "chronos".into(),
                resource: "attempt/1".into(),
                revision: "2".into(),
            }),
            ..publication("frontier-supervisor")
        };
        assert!(matches!(
            encode_worker_position_v1(&invalid_attempt, limits()),
            Err(AgoraWorkerFrontierErrorV1::Invalid)
        ));

        let invalid_scope = WorkerPositionPublicationV1 {
            chronos_scope: "\n".into(),
            ..publication("frontier-supervisor")
        };
        assert!(matches!(
            encode_worker_position_v1(&invalid_scope, limits()),
            Err(AgoraWorkerFrontierErrorV1::Invalid)
        ));
    }
}
