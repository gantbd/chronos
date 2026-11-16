//! Owner-only local operational profile, separate from task coordination V1.
//! Fixed-size frames contain no task, principal, capability or locator fields.
use crate::CodecError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CommandV1 {
    Status = 0,
    Drain = 1,
    Maintain = 2,
}
impl CommandV1 {
    pub const fn encode(self) -> [u8; 4] {
        [b'C', b'C', 1, self as u8]
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        header(bytes, 4)?;
        match bytes[3] {
            0 => Ok(Self::Status),
            1 => Ok(Self::Drain),
            2 => Ok(Self::Maintain),
            _ => Err(CodecError::Malformed),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum LifecycleV1 {
    Running = 0,
    Draining = 1,
    Stopped = 2,
    Fenced = 3,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MaintenanceV1 {
    NotRequested = 0,
    Inspected = 1,
}

/// Readiness is a current admission observation, not an SLO or effect receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusV1 {
    pub lifecycle: LifecycleV1,
    pub ready: bool,
    pub obligations: u8,
    pub maintenance: MaintenanceV1,
    pub entries: u64,
    pub committed: u64,
    pub temporary: u64,
    pub other: u64,
}
impl StatusV1 {
    fn validate(self) -> Result<(), CodecError> {
        if self.obligations > 1
            || (self.lifecycle == LifecycleV1::Stopped && self.obligations != 0)
            || self.ready != (self.lifecycle == LifecycleV1::Running && self.obligations == 0)
            || self.entries > 4096
            || self
                .committed
                .checked_add(self.temporary)
                .and_then(|v| v.checked_add(self.other))
                != Some(self.entries)
            || (self.maintenance == MaintenanceV1::NotRequested && self.entries != 0)
        {
            return Err(CodecError::Malformed);
        }
        Ok(())
    }
    pub fn encode(self) -> Result<[u8; 40], CodecError> {
        self.validate()?;
        let mut bytes = [0; 40];
        bytes[..3].copy_from_slice(b"CC\x01");
        bytes[3] = self.lifecycle as u8;
        bytes[4] = u8::from(self.ready);
        bytes[5] = self.obligations;
        bytes[6] = self.maintenance as u8;
        for (chunk, value) in bytes[8..].chunks_exact_mut(8).zip([
            self.entries,
            self.committed,
            self.temporary,
            self.other,
        ]) {
            chunk.copy_from_slice(&value.to_be_bytes());
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        header(bytes, 40)?;
        let lifecycle = match bytes[3] {
            0 => LifecycleV1::Running,
            1 => LifecycleV1::Draining,
            2 => LifecycleV1::Stopped,
            3 => LifecycleV1::Fenced,
            _ => return Err(CodecError::Malformed),
        };
        let ready = match bytes[4] {
            0 => false,
            1 => true,
            _ => return Err(CodecError::Malformed),
        };
        let maintenance = match bytes[6] {
            0 => MaintenanceV1::NotRequested,
            1 => MaintenanceV1::Inspected,
            _ => return Err(CodecError::Malformed),
        };
        if bytes[7] != 0 {
            return Err(CodecError::Malformed);
        }
        let mut values = [0; 4];
        for (value, chunk) in values.iter_mut().zip(bytes[8..].chunks_exact(8)) {
            *value = u64::from_be_bytes(chunk.try_into().map_err(|_| CodecError::Malformed)?);
        }
        let status = Self {
            lifecycle,
            ready,
            obligations: bytes[5],
            maintenance,
            entries: values[0],
            committed: values[1],
            temporary: values[2],
            other: values[3],
        };
        status.validate()?;
        Ok(status)
    }
}
fn header(bytes: &[u8], length: usize) -> Result<(), CodecError> {
    if bytes.len() != length || &bytes[..2] != b"CC" {
        return Err(CodecError::Malformed);
    }
    if bytes[2] != 1 {
        return Err(CodecError::Version);
    }
    Ok(())
}
