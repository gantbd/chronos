//! Bounded command-journal outbox with explicit prefix acknowledgement.
//!
//! Records are content-free and remain pending until a host confirms durable
//! append. The journal itself is in-memory and makes no durability claim.

use std::collections::VecDeque;

use crate::{
    CommandKindV1, CommandKindV2, CommandOutcomeV1, CommandOutcomeV2, CommandV1, CommandV2,
    Disposition, Engine,
};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct AuditCorrelation([u8; 16]);

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct AuditScope([u8; 16]);

impl AuditCorrelation {
    pub const fn new(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl std::fmt::Debug for AuditCorrelation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AuditCorrelation(<redacted>)")
    }
}

impl AuditScope {
    pub const fn new(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl std::fmt::Debug for AuditScope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AuditScope(<redacted>)")
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AuditRecordV1 {
    scope: AuditScope,
    sequence: u64,
    correlation: AuditCorrelation,
    kind: CommandKindV1,
    disposition: Disposition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditDecodeError {
    InvalidLength,
    UnsupportedVersion(u8),
    InvalidCommandKind,
    InvalidDisposition,
}

impl AuditRecordV1 {
    pub const ENCODED_LEN: usize = 43;

    pub const fn scope(&self) -> AuditScope {
        self.scope
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn correlation(&self) -> AuditCorrelation {
        self.correlation
    }

    pub const fn kind(&self) -> CommandKindV1 {
        self.kind
    }

    pub const fn disposition(&self) -> Disposition {
        self.disposition
    }

    pub fn encode(&self) -> [u8; Self::ENCODED_LEN] {
        let mut encoded = [0_u8; Self::ENCODED_LEN];
        encoded[0] = 1;
        encoded[1..17].copy_from_slice(&self.scope.0);
        encoded[17..25].copy_from_slice(&self.sequence.to_le_bytes());
        encoded[25..41].copy_from_slice(&self.correlation.0);
        encoded[41] = command_kind_tag(self.kind);
        encoded[42] = disposition_tag(self.disposition);
        encoded
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, AuditDecodeError> {
        if encoded.len() != Self::ENCODED_LEN {
            return Err(AuditDecodeError::InvalidLength);
        }
        if encoded[0] != 1 {
            return Err(AuditDecodeError::UnsupportedVersion(encoded[0]));
        }
        let mut sequence = [0_u8; 8];
        sequence.copy_from_slice(&encoded[17..25]);
        let mut scope = [0_u8; 16];
        scope.copy_from_slice(&encoded[1..17]);
        let mut correlation = [0_u8; 16];
        correlation.copy_from_slice(&encoded[25..41]);
        Ok(Self {
            scope: AuditScope(scope),
            sequence: u64::from_le_bytes(sequence),
            correlation: AuditCorrelation(correlation),
            kind: command_kind_from_tag(encoded[41])?,
            disposition: disposition_from_tag(encoded[42])?,
        })
    }
}

const fn command_kind_tag(kind: CommandKindV1) -> u8 {
    match kind {
        CommandKindV1::Submit => 0,
        CommandKindV1::IssueGrant => 1,
        CommandKindV1::RevokeAllGrants => 2,
        CommandKindV1::Claim => 3,
        CommandKindV1::Cancel => 4,
        CommandKindV1::Heartbeat => 5,
        CommandKindV1::Complete => 6,
        CommandKindV1::Fail => 7,
        CommandKindV1::ReleaseExpired => 8,
        CommandKindV1::ResolveUncertain => 9,
        CommandKindV1::PrepareCheckpoint => 10,
    }
}

fn command_kind_from_tag(tag: u8) -> Result<CommandKindV1, AuditDecodeError> {
    match tag {
        0 => Ok(CommandKindV1::Submit),
        1 => Ok(CommandKindV1::IssueGrant),
        2 => Ok(CommandKindV1::RevokeAllGrants),
        3 => Ok(CommandKindV1::Claim),
        4 => Ok(CommandKindV1::Cancel),
        5 => Ok(CommandKindV1::Heartbeat),
        6 => Ok(CommandKindV1::Complete),
        7 => Ok(CommandKindV1::Fail),
        8 => Ok(CommandKindV1::ReleaseExpired),
        9 => Ok(CommandKindV1::ResolveUncertain),
        10 => Ok(CommandKindV1::PrepareCheckpoint),
        _ => Err(AuditDecodeError::InvalidCommandKind),
    }
}

const fn disposition_tag(disposition: Disposition) -> u8 {
    match disposition {
        Disposition::Completed => 0,
        Disposition::Empty => 1,
        Disposition::Refused => 2,
    }
}

fn disposition_from_tag(tag: u8) -> Result<Disposition, AuditDecodeError> {
    match tag {
        0 => Ok(Disposition::Completed),
        1 => Ok(Disposition::Empty),
        2 => Ok(Disposition::Refused),
        _ => Err(AuditDecodeError::InvalidDisposition),
    }
}

impl std::fmt::Debug for AuditRecordV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuditRecordV1")
            .field("scope", &"<redacted>")
            .field("sequence", &self.sequence)
            .field("correlation", &"<redacted>")
            .field("kind", &self.kind)
            .field("disposition", &self.disposition)
            .finish()
    }
}

/// Stable content-free audit record for the closed v0.2 command vocabulary.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AuditRecordV2 {
    scope: AuditScope,
    sequence: u64,
    correlation: AuditCorrelation,
    kind: CommandKindV2,
    disposition: Disposition,
}

impl AuditRecordV2 {
    pub const ENCODED_LEN: usize = 43;

    pub const fn scope(&self) -> AuditScope {
        self.scope
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn correlation(&self) -> AuditCorrelation {
        self.correlation
    }

    pub const fn kind(&self) -> CommandKindV2 {
        self.kind
    }

    pub const fn disposition(&self) -> Disposition {
        self.disposition
    }

    pub fn encode(&self) -> [u8; Self::ENCODED_LEN] {
        let mut encoded = [0_u8; Self::ENCODED_LEN];
        encoded[0] = 2;
        encoded[1..17].copy_from_slice(&self.scope.0);
        encoded[17..25].copy_from_slice(&self.sequence.to_le_bytes());
        encoded[25..41].copy_from_slice(&self.correlation.0);
        encoded[41] = command_kind_v2_tag(self.kind);
        encoded[42] = disposition_tag(self.disposition);
        encoded
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, AuditDecodeError> {
        if encoded.len() != Self::ENCODED_LEN {
            return Err(AuditDecodeError::InvalidLength);
        }
        if encoded[0] != 2 {
            return Err(AuditDecodeError::UnsupportedVersion(encoded[0]));
        }
        let mut sequence = [0_u8; 8];
        sequence.copy_from_slice(&encoded[17..25]);
        let mut scope = [0_u8; 16];
        scope.copy_from_slice(&encoded[1..17]);
        let mut correlation = [0_u8; 16];
        correlation.copy_from_slice(&encoded[25..41]);
        Ok(Self {
            scope: AuditScope(scope),
            sequence: u64::from_le_bytes(sequence),
            correlation: AuditCorrelation(correlation),
            kind: command_kind_v2_from_tag(encoded[41])?,
            disposition: disposition_from_tag(encoded[42])?,
        })
    }
}

impl std::fmt::Debug for AuditRecordV2 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuditRecordV2")
            .field("scope", &"<redacted>")
            .field("sequence", &self.sequence)
            .field("correlation", &"<redacted>")
            .field("kind", &self.kind)
            .field("disposition", &self.disposition)
            .finish()
    }
}

const fn command_kind_v2_tag(kind: CommandKindV2) -> u8 {
    match kind {
        CommandKindV2::AdvanceTime => 0,
        CommandKindV2::SubmitGraph => 1,
        CommandKindV2::IssueGrant => 2,
        CommandKindV2::RevokeGrant => 3,
        CommandKindV2::RevokeAllGrants => 4,
        CommandKindV2::ClaimNext => 5,
        CommandKindV2::ClaimTask => 6,
        CommandKindV2::Cancel => 7,
        CommandKindV2::Heartbeat => 8,
        CommandKindV2::Complete => 9,
        CommandKindV2::FenceUncertain => 10,
        CommandKindV2::Fail => 11,
        CommandKindV2::ReleaseExpired => 12,
        CommandKindV2::ResolveUncertain => 13,
        CommandKindV2::Retire => 14,
        CommandKindV2::ReleaseExpiredGrants => 15,
    }
}

fn command_kind_v2_from_tag(tag: u8) -> Result<CommandKindV2, AuditDecodeError> {
    match tag {
        0 => Ok(CommandKindV2::AdvanceTime),
        1 => Ok(CommandKindV2::SubmitGraph),
        2 => Ok(CommandKindV2::IssueGrant),
        3 => Ok(CommandKindV2::RevokeGrant),
        4 => Ok(CommandKindV2::RevokeAllGrants),
        5 => Ok(CommandKindV2::ClaimNext),
        6 => Ok(CommandKindV2::ClaimTask),
        7 => Ok(CommandKindV2::Cancel),
        8 => Ok(CommandKindV2::Heartbeat),
        9 => Ok(CommandKindV2::Complete),
        10 => Ok(CommandKindV2::FenceUncertain),
        11 => Ok(CommandKindV2::Fail),
        12 => Ok(CommandKindV2::ReleaseExpired),
        13 => Ok(CommandKindV2::ResolveUncertain),
        14 => Ok(CommandKindV2::Retire),
        15 => Ok(CommandKindV2::ReleaseExpiredGrants),
        _ => Err(AuditDecodeError::InvalidCommandKind),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalError {
    Capacity,
    Full,
    SequenceExhausted,
    StaleAcknowledgement,
    FutureAcknowledgement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalUsage {
    pub pending: usize,
    pub capacity: usize,
    pub next_sequence: u64,
}

/// Owns an engine and prevents command execution when its pending audit outbox
/// cannot accept exactly one more record.
pub struct CommandJournal<const N: usize> {
    engine: Engine,
    scope: AuditScope,
    pending: VecDeque<AuditRecordV1>,
    next_sequence: u64,
    acknowledged_through: Option<u64>,
}

impl<const N: usize> CommandJournal<N> {
    pub fn try_new(
        engine: Engine,
        scope: AuditScope,
        next_sequence: u64,
    ) -> Result<Self, JournalError> {
        let mut pending = VecDeque::new();
        pending
            .try_reserve_exact(N)
            .map_err(|_| JournalError::Capacity)?;
        Ok(Self {
            engine,
            scope,
            pending,
            next_sequence,
            acknowledged_through: next_sequence.checked_sub(1),
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn pending(&self) -> impl Iterator<Item = &AuditRecordV1> {
        self.pending.iter()
    }

    pub fn usage(&self) -> JournalUsage {
        JournalUsage {
            pending: self.pending.len(),
            capacity: N,
            next_sequence: self.next_sequence,
        }
    }

    pub fn execute(
        &mut self,
        correlation: AuditCorrelation,
        command: CommandV1,
    ) -> Result<CommandOutcomeV1, JournalError> {
        if self.pending.len() == N {
            return Err(JournalError::Full);
        }
        if self.next_sequence == u64::MAX {
            return Err(JournalError::SequenceExhausted);
        }
        let kind = command.kind();
        let outcome = self.engine.execute(command);
        self.pending.push_back(AuditRecordV1 {
            scope: self.scope,
            sequence: self.next_sequence,
            correlation,
            kind,
            disposition: outcome.disposition(),
        });
        self.next_sequence += 1;
        Ok(outcome)
    }

    /// Discharges a prefix only after the host has durably appended every
    /// record through `sequence` to its independently tested audit sink.
    pub fn acknowledge_through(&mut self, sequence: u64) -> Result<(), JournalError> {
        if self
            .acknowledged_through
            .is_some_and(|acknowledged| sequence <= acknowledged)
        {
            return Err(JournalError::StaleAcknowledgement);
        }
        if sequence >= self.next_sequence {
            return Err(JournalError::FutureAcknowledgement);
        }
        while self
            .pending
            .front()
            .is_some_and(|record| record.sequence <= sequence)
        {
            self.pending.pop_front();
        }
        self.acknowledged_through = Some(sequence);
        Ok(())
    }
}

impl<const N: usize> std::fmt::Debug for CommandJournal<N> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommandJournal")
            .field("capacity", &N)
            .field("pending", &self.pending.len())
            .field("next_sequence", &self.next_sequence)
            .field("acknowledged_through", &self.acknowledged_through)
            .finish_non_exhaustive()
    }
}

/// V2 journal owner; capacity proof precedes every v0.2 command transition.
pub struct CommandJournalV2<const N: usize> {
    engine: Engine,
    scope: AuditScope,
    pending: VecDeque<AuditRecordV2>,
    next_sequence: u64,
    acknowledged_through: Option<u64>,
}

impl<const N: usize> CommandJournalV2<N> {
    pub fn try_new(
        engine: Engine,
        scope: AuditScope,
        next_sequence: u64,
    ) -> Result<Self, JournalError> {
        let mut pending = VecDeque::new();
        pending
            .try_reserve_exact(N)
            .map_err(|_| JournalError::Capacity)?;
        Ok(Self {
            engine,
            scope,
            pending,
            next_sequence,
            acknowledged_through: next_sequence.checked_sub(1),
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn pending(&self) -> impl Iterator<Item = &AuditRecordV2> {
        self.pending.iter()
    }

    pub fn usage(&self) -> JournalUsage {
        JournalUsage {
            pending: self.pending.len(),
            capacity: N,
            next_sequence: self.next_sequence,
        }
    }

    pub fn execute(
        &mut self,
        correlation: AuditCorrelation,
        command: CommandV2,
    ) -> Result<CommandOutcomeV2, JournalError> {
        if self.pending.len() == N {
            return Err(JournalError::Full);
        }
        if self.next_sequence == u64::MAX {
            return Err(JournalError::SequenceExhausted);
        }
        let kind = command.kind();
        let outcome = self.engine.execute_v2(command);
        self.pending.push_back(AuditRecordV2 {
            scope: self.scope,
            sequence: self.next_sequence,
            correlation,
            kind,
            disposition: outcome.disposition(),
        });
        self.next_sequence += 1;
        Ok(outcome)
    }

    pub fn acknowledge_through(&mut self, sequence: u64) -> Result<(), JournalError> {
        if self
            .acknowledged_through
            .is_some_and(|acknowledged| sequence <= acknowledged)
        {
            return Err(JournalError::StaleAcknowledgement);
        }
        if sequence >= self.next_sequence {
            return Err(JournalError::FutureAcknowledgement);
        }
        while self
            .pending
            .front()
            .is_some_and(|record| record.sequence <= sequence)
        {
            self.pending.pop_front();
        }
        self.acknowledged_through = Some(sequence);
        Ok(())
    }
}

impl<const N: usize> std::fmt::Debug for CommandJournalV2<N> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommandJournalV2")
            .field("capacity", &N)
            .field("pending", &self.pending.len())
            .field("next_sequence", &self.next_sequence)
            .field("acknowledged_through", &self.acknowledged_through)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_exhaustion_refuses_before_execution() {
        let mut journal =
            CommandJournal::<1>::try_new(Engine::new(), AuditScope::new([0; 16]), 0).unwrap();
        journal.next_sequence = u64::MAX;
        assert_eq!(
            journal.execute(AuditCorrelation::new([0; 16]), CommandV1::RevokeAllGrants),
            Err(JournalError::SequenceExhausted)
        );
        assert_eq!(journal.pending.len(), 0);
    }
}
