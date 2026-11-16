use std::time::SystemTime;

use chronos_engine::{
    AuditCorrelation, AuditDecodeError, AuditRecordV2, AuditScope, CommandJournalV2, CommandKindV2,
    CommandOutcomeV2, CommandV2, Disposition, Engine, JournalError, StateRevision, TaskId,
    TaskSpecV2,
};

#[test]
fn v2_audit_records_have_distinct_stable_tags_and_redacted_fixed_width_encoding() {
    let mut journal =
        CommandJournalV2::<2>::try_new(Engine::new(), AuditScope::new([0x22; 16]), 7).unwrap();
    journal
        .execute(
            AuditCorrelation::new([0x44; 16]),
            CommandV2::SubmitGraph {
                expected: StateRevision::ZERO,
                tasks: vec![TaskSpecV2::new("sensitive")],
            },
        )
        .unwrap();
    let record = *journal.pending().next().unwrap();
    let encoded = record.encode();

    assert_eq!(encoded.len(), AuditRecordV2::ENCODED_LEN);
    assert_eq!(encoded[0], 2);
    assert_eq!(record.kind(), CommandKindV2::SubmitGraph);
    assert_eq!(record.disposition(), Disposition::Completed);
    assert_eq!(AuditRecordV2::decode(&encoded).unwrap(), record);
    assert!(!format!("{record:?}").contains("sensitive"));
    let kinds = [
        CommandKindV2::AdvanceTime,
        CommandKindV2::SubmitGraph,
        CommandKindV2::IssueGrant,
        CommandKindV2::RevokeGrant,
        CommandKindV2::RevokeAllGrants,
        CommandKindV2::ClaimNext,
        CommandKindV2::ClaimTask,
        CommandKindV2::Cancel,
        CommandKindV2::Heartbeat,
        CommandKindV2::Complete,
        CommandKindV2::FenceUncertain,
        CommandKindV2::Fail,
        CommandKindV2::ReleaseExpired,
        CommandKindV2::ResolveUncertain,
        CommandKindV2::Retire,
        CommandKindV2::ReleaseExpiredGrants,
    ];
    for (tag, expected) in kinds.into_iter().enumerate() {
        let mut tagged = encoded;
        tagged[41] = u8::try_from(tag).unwrap();
        assert_eq!(AuditRecordV2::decode(&tagged).unwrap().kind(), expected);
    }
    let mut invalid = encoded;
    invalid[41] = u8::MAX;
    assert_eq!(
        AuditRecordV2::decode(&invalid),
        Err(AuditDecodeError::InvalidCommandKind)
    );
}

#[test]
fn full_v2_journal_refuses_before_time_or_task_state_can_change() {
    let mut journal =
        CommandJournalV2::<1>::try_new(Engine::new(), AuditScope::new([0; 16]), 0).unwrap();
    let correlation = AuditCorrelation::new([0; 16]);
    assert!(matches!(
        journal
            .execute(
                correlation,
                CommandV2::AdvanceTime {
                    now: SystemTime::UNIX_EPOCH,
                },
            )
            .unwrap(),
        CommandOutcomeV2::AdvanceTime(Ok(_))
    ));

    assert_eq!(
        journal.execute(
            correlation,
            CommandV2::SubmitGraph {
                expected: StateRevision::ZERO,
                tasks: vec![TaskSpecV2::new("never-published")],
            },
        ),
        Err(JournalError::Full)
    );
    assert!(
        journal
            .engine()
            .task(&TaskId::from("never-published"))
            .is_none()
    );
    assert_eq!(journal.engine().task_state_revision(), StateRevision::ZERO);
}
