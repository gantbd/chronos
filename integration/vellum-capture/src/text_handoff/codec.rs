//! Bounded V1 carrier for a source baseline, readable card and observed identity.
//! No filesystem, authentication, runtime handle, execution authority or import.
use super::*;
use chronos_sdk_embedded::engine::TaskId;
const MAGIC: &[u8] = b"CHTH\0\x01";

pub fn max_encoded_handoff_bytes_v1(limits: TextLimitsV1) -> Result<usize, TextErrorV1> {
    // Four identity strings, baseline and card: six u32 lengths, incarnation
    // and revision u64s. The revision's MAX representation means Exhausted.
    limits
        .max_field_bytes
        .checked_mul(4)
        .and_then(|n| n.checked_add(limits.max_source_bytes))
        .and_then(|n| n.checked_add(limits.max_handoff_bytes))
        .and_then(|n| n.checked_add(MAGIC.len() + 6 * 4 + 2 * 8))
        .ok_or(TextErrorV1::Limit)
}
struct Writer {
    bytes: Vec<u8>,
    max: usize,
}
impl Writer {
    fn put(&mut self, bytes: &[u8]) -> Result<(), TextErrorV1> {
        if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            return Err(TextErrorV1::Limit);
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| TextErrorV1::Capacity)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn field(&mut self, bytes: &[u8]) -> Result<(), TextErrorV1> {
        self.put(
            &u32::try_from(bytes.len())
                .map_err(|_| TextErrorV1::Limit)?
                .to_be_bytes(),
        )?;
        self.put(bytes)
    }
}
fn validate_fields(
    fields: [&str; 4],
    source: &[u8],
    card: &str,
    incarnation: u64,
    limits: TextLimitsV1,
) -> Result<(), TextErrorV1> {
    if fields.iter().any(|s| s.len() > limits.max_field_bytes)
        || source.len() > limits.max_source_bytes
        || card.len() > limits.max_handoff_bytes
    {
        return Err(TextErrorV1::Limit);
    }
    if fields.iter().any(|s| s.trim().is_empty())
        || incarnation == u64::MAX
        || !card.starts_with("WORK HANDOFF — LAST OBSERVED SNAPSHOT\n")
    {
        return Err(TextErrorV1::Invalid);
    }
    std::str::from_utf8(source).map_err(|_| TextErrorV1::Invalid)?;
    Ok(())
}
/// Serializes observations only. The host still establishes current disclosure
/// and destination-write rights; already captured bytes do not supply either.
pub fn encode_text_handoff_v1(
    handoff: &TextHandoffV1,
    limits: TextLimitsV1,
) -> Result<Vec<u8>, TextErrorV1> {
    let fields = [
        handoff.document.namespace.as_str(),
        handoff.document.value.as_str(),
        handoff.service_scope.as_str(),
        handoff.task.task_id().as_str(),
    ];
    validate_fields(
        fields,
        &handoff.original,
        &handoff.instructions,
        handoff.task.incarnation().get(),
        limits,
    )?;
    let mut writer = Writer {
        bytes: Vec::new(),
        max: max_encoded_handoff_bytes_v1(limits)?,
    };
    writer.put(MAGIC)?;
    for value in fields {
        writer.field(value.as_bytes())?;
    }
    writer.put(&handoff.task.incarnation().get().to_be_bytes())?;
    let revision = match handoff.revision {
        StateRevision::Finite(n) if n != u64::MAX => n,
        StateRevision::Exhausted => u64::MAX,
        _ => return Err(TextErrorV1::Invalid),
    };
    writer.put(&revision.to_be_bytes())?;
    writer.field(&handoff.original)?;
    writer.field(handoff.instructions.as_bytes())?;
    Ok(writer.bytes)
}
struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8], TextErrorV1> {
        let (value, rest) = self
            .bytes
            .split_at_checked(size)
            .ok_or(TextErrorV1::Invalid)?;
        self.bytes = rest;
        Ok(value)
    }
    fn field(&mut self, max: usize) -> Result<&'a [u8], TextErrorV1> {
        let size = u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| TextErrorV1::Invalid)?);
        let size = usize::try_from(size).map_err(|_| TextErrorV1::Limit)?;
        if size > max {
            return Err(TextErrorV1::Limit);
        }
        self.take(size)
    }
    fn text(&mut self, max: usize) -> Result<&'a str, TextErrorV1> {
        std::str::from_utf8(self.field(max)?).map_err(|_| TextErrorV1::Invalid)
    }
    fn u64(&mut self) -> Result<u64, TextErrorV1> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().map_err(|_| TextErrorV1::Invalid)?,
        ))
    }
}
fn own(value: &str) -> Result<String, TextErrorV1> {
    let mut result = String::new();
    result
        .try_reserve_exact(value.len())
        .map_err(|_| TextErrorV1::Capacity)?;
    result.push_str(value);
    Ok(result)
}
/// Decodes an explicitly unverified carrier after the host has read it under
/// its normal resource rights. Strict parsing is not integrity/authentication.
/// Unknown versions, truncations, excess lengths and tails refuse before effects.
pub fn decode_text_handoff_v1(
    bytes: &[u8],
    limits: TextLimitsV1,
) -> Result<TextHandoffV1, TextErrorV1> {
    if bytes.len() > max_encoded_handoff_bytes_v1(limits)? {
        return Err(TextErrorV1::Limit);
    }
    let mut reader = Reader { bytes };
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(TextErrorV1::Invalid);
    }
    let namespace = reader.text(limits.max_field_bytes)?;
    let document = reader.text(limits.max_field_bytes)?;
    let scope = reader.text(limits.max_field_bytes)?;
    let task = reader.text(limits.max_field_bytes)?;
    let incarnation = reader.u64()?;
    let revision = StateRevision::finite(reader.u64()?);
    let source = reader.field(limits.max_source_bytes)?;
    let card = reader.text(limits.max_handoff_bytes)?;
    if !reader.bytes.is_empty() {
        return Err(TextErrorV1::Invalid);
    }
    validate_fields(
        [namespace, document, scope, task],
        source,
        card,
        incarnation,
        limits,
    )?;
    // Allocate only after the whole bounded record has validated.
    let mut original = Vec::new();
    original
        .try_reserve_exact(source.len())
        .map_err(|_| TextErrorV1::Capacity)?;
    original.extend_from_slice(source);
    Ok(TextHandoffV1 {
        origin: HandoffOriginV1::RestoredUnverified,
        document: DocumentIdentity {
            namespace: own(namespace)?,
            value: own(document)?,
        },
        service_scope: own(scope)?,
        task: TaskRefV1::from_observation(TaskId::new(own(task)?), incarnation),
        revision,
        original,
        instructions: own(card)?,
    })
}
