//! Deterministic time primitives for simulations and acceptance tests.
//!
//! Production hosts may use their own clock source, but engine commands always
//! receive an explicit instant. This clock performs no sleeping or I/O.

use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockError {
    Overflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualClock {
    now: SystemTime,
}

impl VirtualClock {
    pub fn new(now: SystemTime) -> Self {
        Self { now }
    }

    pub fn at_unix_epoch() -> Self {
        Self::new(SystemTime::UNIX_EPOCH)
    }

    pub fn now(&self) -> SystemTime {
        self.now
    }

    pub fn advance(&mut self, duration: Duration) -> Result<SystemTime, ClockError> {
        let next = self.now.checked_add(duration).ok_or(ClockError::Overflow)?;
        self.now = next;
        Ok(next)
    }
}
