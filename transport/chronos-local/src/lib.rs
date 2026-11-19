//! Local Unix client transport and read-only endpoint verification.
//! Same-OS-user operator boundary; not remote or per-agent authentication.
#![forbid(unsafe_code)]
#![cfg(unix)]

use chronos_client::WireClientV1;
use chronos_wire::{Request, Response};
use std::{
    io::{self, BufReader, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    time::Duration,
};

pub mod alignment;
pub mod control;
mod dependencies;
pub mod dual_frontier;
mod scheduling;

pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const IO_TIMEOUT: Duration = Duration::from_secs(2);

pub fn wire_limits() -> chronos_wire::Limits {
    chronos_wire::Limits {
        max_frame_bytes: MAX_REQUEST_BYTES,
        max_id_bytes: 256,
        max_graph_tasks: 128,
        max_dependencies_per_task: 128,
        max_capabilities_per_task: 64,
        max_capability_bytes: 256,
        max_page_entries: 64,
    }
}

/// Requires an existing non-symlink, owner-only directory. Ancestor integrity
/// and the same-user trust boundary are deployment obligations.
pub fn verify_private_directory(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private owner-only directory required",
        ));
    }
    Ok(())
}

pub fn socket_path(directory: &Path) -> PathBuf {
    directory.join("chronos.sock")
}

pub fn verify_socket(directory: &Path) -> io::Result<PathBuf> {
    verify_named_socket(directory, "chronos.sock")
}

pub fn control_path(directory: &Path) -> PathBuf {
    directory.join("chronos.control.sock")
}
pub fn verify_control_socket(directory: &Path) -> io::Result<PathBuf> {
    verify_named_socket(directory, "chronos.control.sock")
}

pub fn alignment_socket_path(directory: &Path) -> PathBuf {
    directory.join("chronos.alignment.sock")
}

pub fn verify_alignment_socket(directory: &Path) -> io::Result<PathBuf> {
    verify_named_socket(directory, "chronos.alignment.sock")
}

pub fn dual_frontier_socket_path(directory: &Path) -> PathBuf {
    directory.join("chronos.dual-frontier.sock")
}

pub fn verify_dual_frontier_socket(directory: &Path) -> io::Result<PathBuf> {
    verify_named_socket(directory, "chronos.dual-frontier.sock")
}

fn verify_named_socket(directory: &Path, name: &str) -> io::Result<PathBuf> {
    verify_private_directory(directory)?;
    let path = directory.join(name);
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private owner socket required",
        ));
    }
    Ok(path)
}

#[derive(Debug)]
pub enum CallError {
    /// No request bytes were sent; safe to inspect/reconnect, not an implicit retry.
    NotSent,
    /// A command may have committed. Inspect current state before another mutation.
    OutcomeUnknown,
    /// A received reply cannot be trusted; a sent mutation may still have committed.
    InvalidResponse,
}

pub struct LocalClient {
    directory: PathBuf,
}
impl chronos_client::WireClientV4 for LocalClient {
    type Error = CallError;
    fn request_v4(
        &mut self,
        request: chronos_wire::coordination_v4::Request,
    ) -> Result<chronos_wire::coordination_v4::Response, CallError> {
        request
            .validate(wire_limits())
            .map_err(|_| CallError::NotSent)?;
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_socket(&self.directory).map_err(|_| CallError::NotSent)?;
        let mut stream = UnixStream::connect(path).map_err(|_| CallError::NotSent)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .write_all(&bytes)
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|_| CallError::OutcomeUnknown)?;
        let frame = chronos_wire::read_frame(&mut BufReader::new(stream), MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::OutcomeUnknown)?
            .ok_or(CallError::OutcomeUnknown)?;
        let response = chronos_wire::decode_response_v4(&frame, MAX_RESPONSE_BYTES, wire_limits())
            .map_err(|_| CallError::InvalidResponse)?;
        response
            .validate_for(&request, wire_limits())
            .map_err(|_| CallError::InvalidResponse)?;
        Ok(response)
    }
}
impl chronos_client::ScheduledAdmissionWireClientV7 for LocalClient {
    type Error = CallError;
    fn request_scheduled_admission_v7(
        &mut self,
        request: chronos_wire::coordination_v7::Request,
    ) -> Result<chronos_wire::coordination_v7::Response, CallError> {
        request
            .validate(wire_limits())
            .map_err(|_| CallError::NotSent)?;
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_socket(&self.directory).map_err(|_| CallError::NotSent)?;
        let mut stream = UnixStream::connect(path).map_err(|_| CallError::NotSent)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .write_all(&bytes)
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|_| CallError::OutcomeUnknown)?;
        let frame = chronos_wire::read_frame(&mut BufReader::new(stream), MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::OutcomeUnknown)?
            .ok_or(CallError::OutcomeUnknown)?;
        let response = chronos_wire::decode_response_v7(&frame, MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::InvalidResponse)?;
        response
            .validate_for(&request, wire_limits())
            .map_err(|_| CallError::InvalidResponse)?;
        Ok(response)
    }
}
impl chronos_client::ScheduledCancellationWireClientV8 for LocalClient {
    type Error = CallError;
    fn request_scheduled_cancellation_v8(
        &mut self,
        request: chronos_wire::coordination_v8::Request,
    ) -> Result<chronos_wire::coordination_v8::Response, CallError> {
        request
            .validate(wire_limits())
            .map_err(|_| CallError::NotSent)?;
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_socket(&self.directory).map_err(|_| CallError::NotSent)?;
        let mut stream = UnixStream::connect(path).map_err(|_| CallError::NotSent)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .write_all(&bytes)
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|_| CallError::OutcomeUnknown)?;
        let frame = chronos_wire::read_frame(&mut BufReader::new(stream), MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::OutcomeUnknown)?
            .ok_or(CallError::OutcomeUnknown)?;
        let response = chronos_wire::decode_response_v8(&frame, MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::InvalidResponse)?;
        response
            .validate_for(&request, wire_limits())
            .map_err(|_| CallError::InvalidResponse)?;
        Ok(response)
    }
}
impl chronos_client::WorkflowWireClientV9 for LocalClient {
    type Error = CallError;
    fn request_workflow_v9(
        &mut self,
        request: chronos_wire::workflow_v9::Request,
    ) -> Result<chronos_wire::workflow_v9::Response, CallError> {
        request
            .validate(wire_limits())
            .map_err(|_| CallError::NotSent)?;
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_socket(&self.directory).map_err(|_| CallError::NotSent)?;
        let mut stream = UnixStream::connect(path).map_err(|_| CallError::NotSent)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .write_all(&bytes)
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|_| CallError::OutcomeUnknown)?;
        let frame = chronos_wire::read_frame(&mut BufReader::new(stream), MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::OutcomeUnknown)?
            .ok_or(CallError::OutcomeUnknown)?;
        let response = chronos_wire::decode_response_v9(&frame, MAX_RESPONSE_BYTES, wire_limits())
            .map_err(|_| CallError::InvalidResponse)?;
        response
            .validate_for(&request, wire_limits())
            .map_err(|_| CallError::InvalidResponse)?;
        Ok(response)
    }
}
impl LocalClient {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
}
impl WireClientV1 for LocalClient {
    type Error = CallError;
    fn request(&mut self, request: Request) -> Result<Response, CallError> {
        // Encode before connecting; serialization/limit refusal sends nothing.
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_socket(&self.directory).map_err(|_| CallError::NotSent)?;
        let mut stream = UnixStream::connect(path).map_err(|_| CallError::NotSent)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .write_all(&bytes)
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|_| CallError::OutcomeUnknown)?;
        let frame = chronos_wire::read_frame(&mut BufReader::new(stream), MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::OutcomeUnknown)?
            .ok_or(CallError::OutcomeUnknown)?;
        chronos_wire::decode_response(&frame, MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::InvalidResponse)
    }
}

impl chronos_client::WireClientV2 for LocalClient {
    type Error = CallError;
    fn request_v2(
        &mut self,
        request: chronos_wire::coordination_v2::Request,
    ) -> Result<chronos_wire::coordination_v2::Response, CallError> {
        request
            .validate(wire_limits())
            .map_err(|_| CallError::NotSent)?;
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_socket(&self.directory).map_err(|_| CallError::NotSent)?;
        let mut stream = UnixStream::connect(path).map_err(|_| CallError::NotSent)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .write_all(&bytes)
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|_| CallError::OutcomeUnknown)?;
        let frame = chronos_wire::read_frame(&mut BufReader::new(stream), MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::OutcomeUnknown)?
            .ok_or(CallError::OutcomeUnknown)?;
        chronos_wire::decode_response_v2(&frame, MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::InvalidResponse)
    }
}

impl chronos_client::WireClientV3 for LocalClient {
    type Error = CallError;
    fn request_v3(
        &mut self,
        request: chronos_wire::coordination_v3::Request,
    ) -> Result<chronos_wire::coordination_v3::Response, CallError> {
        request
            .validate(wire_limits())
            .map_err(|_| CallError::NotSent)?;
        let bytes =
            chronos_wire::encode(&request, MAX_REQUEST_BYTES).map_err(|_| CallError::NotSent)?;
        let path = verify_socket(&self.directory).map_err(|_| CallError::NotSent)?;
        let mut stream = UnixStream::connect(path).map_err(|_| CallError::NotSent)?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|_| CallError::NotSent)?;
        stream
            .write_all(&bytes)
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|_| CallError::OutcomeUnknown)?;
        let frame = chronos_wire::read_frame(&mut BufReader::new(stream), MAX_RESPONSE_BYTES)
            .map_err(|_| CallError::OutcomeUnknown)?
            .ok_or(CallError::OutcomeUnknown)?;
        let response = chronos_wire::decode_response_v3(&frame, MAX_RESPONSE_BYTES, wire_limits())
            .map_err(|_| CallError::InvalidResponse)?;
        response
            .validate_for(&request, wire_limits())
            .map_err(|_| CallError::InvalidResponse)?;
        Ok(response)
    }
}
