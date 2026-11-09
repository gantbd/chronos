use chronos_engine::{
    AuditCorrelation, AuditDecodeError, AuditRecordV1, AuditScope, CommandJournal, CommandV1,
    Disposition, JournalError, Task, TaskId,
};

#[test]
fn journal_orders_completed_and_refused_commands_without_exposing_content() {
    let mut journal = CommandJournal::<4>::try_new(
        chronos_engine::Engine::new(),
        AuditScope::new([0x22; 16]),
        0,
    )
    .unwrap();
    let correlation = AuditCorrelation::new([0x5a; 16]);

    journal
        .execute(
            correlation,
            CommandV1::Submit(Task::new("sensitive-id", "sensitive summary")),
        )
        .unwrap();
    journal
        .execute(
            correlation,
            CommandV1::Submit(Task::new("sensitive-id", "sensitive summary")),
        )
        .unwrap();

    let records = journal.pending().collect::<Vec<_>>();
    assert_eq!(records[0].sequence(), 0);
    assert_eq!(records[0].disposition(), Disposition::Completed);
    assert_eq!(records[1].sequence(), 1);
    assert_eq!(records[1].disposition(), Disposition::Refused);
    assert_eq!(journal.usage().pending, 2);
    assert_eq!(journal.usage().capacity, 4);
    let debug = format!("{records:?}");
    assert!(!debug.contains("sensitive-id"));
    assert!(!debug.contains("sensitive summary"));
    assert!(!debug.contains("5a5a"));
}

#[test]
fn audit_record_v1_has_a_canonical_fixed_width_encoding() {
    let mut journal = CommandJournal::<1>::try_new(
        chronos_engine::Engine::new(),
        AuditScope::new([0x22; 16]),
        0,
    )
    .unwrap();
    journal
        .execute(
            AuditCorrelation::new([0x11; 16]),
            CommandV1::Submit(Task::new("task", "work")),
        )
        .unwrap();
    let record = *journal.pending().next().unwrap();

    let encoded = record.encode();
    assert_eq!(encoded.len(), AuditRecordV1::ENCODED_LEN);
    assert_eq!(encoded[0], 1);
    assert_eq!(AuditRecordV1::decode(&encoded).unwrap(), record);
    assert_eq!(
        AuditRecordV1::decode(&encoded[..encoded.len() - 1]),
        Err(AuditDecodeError::InvalidLength)
    );
    let mut invalid_kind = encoded;
    invalid_kind[41] = u8::MAX;
    assert_eq!(
        AuditRecordV1::decode(&invalid_kind),
        Err(AuditDecodeError::InvalidCommandKind)
    );
}

#[test]
fn full_journal_backpressures_before_the_command_can_mutate_state() {
    let mut journal = CommandJournal::<1>::try_new(
        chronos_engine::Engine::new(),
        AuditScope::new([0x22; 16]),
        0,
    )
    .unwrap();
    let correlation = AuditCorrelation::new([0; 16]);
    journal
        .execute(correlation, CommandV1::Submit(Task::new("first", "work")))
        .unwrap();

    assert_eq!(
        journal.execute(correlation, CommandV1::Submit(Task::new("second", "work")),),
        Err(JournalError::Full)
    );
    assert!(journal.engine().task(&TaskId::from("second")).is_none());
}

#[test]
fn durable_prefix_acknowledgement_is_monotone_and_releases_capacity() {
    let mut journal = CommandJournal::<2>::try_new(
        chronos_engine::Engine::new(),
        AuditScope::new([0x22; 16]),
        0,
    )
    .unwrap();
    let correlation = AuditCorrelation::new([1; 16]);
    for id in ["a", "b"] {
        journal
            .execute(correlation, CommandV1::Submit(Task::new(id, "work")))
            .unwrap();
    }

    journal.acknowledge_through(0).unwrap();
    assert_eq!(journal.pending().count(), 1);
    assert_eq!(
        journal.acknowledge_through(0),
        Err(JournalError::StaleAcknowledgement)
    );
    assert_eq!(
        journal.acknowledge_through(9),
        Err(JournalError::FutureAcknowledgement)
    );
    journal
        .execute(correlation, CommandV1::Submit(Task::new("c", "work")))
        .unwrap();
    assert_eq!(journal.pending().count(), 2);
}

#[test]
fn restored_scope_and_next_sequence_prevent_restart_identity_reuse() {
    let scope = AuditScope::new([0x33; 16]);
    let mut journal =
        CommandJournal::<1>::try_new(chronos_engine::Engine::new(), scope, 41).unwrap();

    journal
        .execute(
            AuditCorrelation::new([0x44; 16]),
            CommandV1::Submit(Task::new("task", "work")),
        )
        .unwrap();
    let record = journal.pending().next().unwrap();
    assert_eq!(record.scope(), scope);
    assert_eq!(record.sequence(), 41);
    assert_eq!(
        journal.acknowledge_through(40),
        Err(JournalError::StaleAcknowledgement)
    );
}
