//! Fixed owner-only operator exchange. No automatic retry or task authority.
pub use chronos_wire::control::{CommandV1, LifecycleV1, MaintenanceV1, StatusV1};
use std::{
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::Path,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    NotSent,
    OutcomeUnknown,
    InvalidResponse,
}

pub fn request(directory: &Path, command: CommandV1) -> Result<StatusV1, ControlError> {
    let path = crate::verify_control_socket(directory).map_err(|_| ControlError::NotSent)?;
    let mut stream = UnixStream::connect(path).map_err(|_| ControlError::NotSent)?;
    stream
        .set_read_timeout(Some(crate::IO_TIMEOUT))
        .map_err(|_| ControlError::NotSent)?;
    stream
        .set_write_timeout(Some(crate::IO_TIMEOUT))
        .map_err(|_| ControlError::NotSent)?;
    stream
        .write_all(&command.encode())
        .and_then(|()| stream.shutdown(Shutdown::Write))
        .map_err(|_| ControlError::OutcomeUnknown)?;
    let mut bytes = [0; 40];
    stream
        .read_exact(&mut bytes)
        .map_err(|_| ControlError::OutcomeUnknown)?;
    let mut trailing = [0; 1];
    if stream
        .read(&mut trailing)
        .map_err(|_| ControlError::OutcomeUnknown)?
        != 0
    {
        return Err(ControlError::InvalidResponse);
    }
    StatusV1::decode(&bytes).map_err(|_| ControlError::InvalidResponse)
}
