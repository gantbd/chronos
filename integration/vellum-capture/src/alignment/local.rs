//! Private local listener lifecycle and one bounded request per connection.
//!
//! The composition root still owns signal installation and supplies a wake
//! channel. This module owns only the independent alignment endpoint.

use super::{
    AlignmentJournalV1,
    host::{AlignmentFrontierV1, AlignmentHostV1},
};
use rustix::event::{PollFd, PollFlags, poll};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, Read},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const IO_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AlignmentListenerSignalsV1 {
    pub accepted: u64,
    pub delivery_failed: u64,
    pub service_time: Duration,
    pub ownership_failures: u64,
    pub saturated: bool,
    pub fenced: bool,
}

/// One process-owned alignment endpoint. The persistent advisory lock permits
/// safe stale-socket recovery after process death without deleting a live or
/// replaced endpoint.
pub struct AlignmentListenerV1 {
    listener: UnixListener,
    directory: PathBuf,
    root_identity: (u64, u64),
    lock: File,
    lock_identity: (u64, u64),
    path: PathBuf,
    socket_identity: (u64, u64),
    signals: AlignmentListenerSignalsV1,
}

impl AlignmentListenerV1 {
    pub fn bind(directory: impl AsRef<Path>) -> Result<Self, &'static str> {
        let directory = directory.as_ref();
        verify_private_directory(directory).map_err(|_| "private alignment directory refused")?;
        let root = std::fs::symlink_metadata(directory)
            .map_err(|_| "alignment directory identity unavailable")?;
        let directory = std::fs::canonicalize(directory)
            .map_err(|_| "alignment directory canonicalization failed")?;
        let flags = i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())
            .map_err(|_| "alignment lock flags unsupported")?;
        let lock_path = directory.join("alignment-owner.lock");
        let open = |create: bool| {
            OpenOptions::new()
                .create_new(create)
                .read(true)
                .write(true)
                .mode(0o600)
                .custom_flags(flags)
                .open(&lock_path)
        };
        let (lock, created) = match open(true) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (
                open(false).map_err(|_| "alignment owner lock refused")?,
                false,
            ),
            Err(_) => return Err("alignment owner lock refused"),
        };
        let lock_metadata = lock
            .metadata()
            .map_err(|_| "alignment owner lock identity unavailable")?;
        if !lock_metadata.is_file()
            || lock_metadata.uid() != rustix::process::geteuid().as_raw()
            || lock_metadata.mode() & 0o077 != 0
            || lock_metadata.nlink() != 1
        {
            return Err("private alignment owner lock required");
        }
        lock.try_lock()
            .map_err(|_| "alignment endpoint already owned")?;

        let path = directory.join("chronos.alignment.sock");
        let predecessor = observe_socket(&path)?;
        if created && predecessor.is_some() {
            return Err("unattributed alignment endpoint refused");
        }
        if predecessor.is_some() {
            std::fs::remove_file(&path).map_err(|_| "stale alignment endpoint removal failed")?;
        }
        let listener = UnixListener::bind(&path).map_err(|_| "alignment endpoint bind failed")?;
        if std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).is_err() {
            let _ = std::fs::remove_file(&path);
            return Err("alignment endpoint permissions failed");
        }
        let socket = std::fs::symlink_metadata(&path)
            .map_err(|_| "alignment endpoint identity unavailable")?;
        let owner = Self {
            listener,
            directory,
            root_identity: identity(&root),
            lock,
            lock_identity: identity(&lock_metadata),
            path,
            socket_identity: identity(&socket),
            signals: AlignmentListenerSignalsV1::default(),
        };
        owner
            .verify_ownership()
            .map_err(|_| "alignment endpoint ownership refused")?;
        Ok(owner)
    }

    pub fn signals(&self) -> AlignmentListenerSignalsV1 {
        self.signals
    }

    pub fn serve_one<J, F, D>(
        &mut self,
        host: &mut AlignmentHostV1<J, F, D>,
        max_request_bytes: usize,
        max_response_bytes: usize,
        limits: chronos_protocol::alignment_v1::Limits,
    ) -> Result<(), &'static str>
    where
        J: AlignmentJournalV1,
        F: AlignmentFrontierV1,
        D: super::AlignmentDisclosureV1,
    {
        self.guard_ownership()?;
        let (stream, _) = self
            .listener
            .accept()
            .map_err(|_| "alignment accept failed")?;
        self.guard_ownership()?;
        increment(&mut self.signals.accepted, &mut self.signals.saturated);
        let started = Instant::now();
        let result = serve_connection(host, stream, max_request_bytes, max_response_bytes, limits);
        add_time(
            &mut self.signals.service_time,
            started.elapsed(),
            &mut self.signals.saturated,
        );
        if result.is_err() {
            increment(
                &mut self.signals.delivery_failed,
                &mut self.signals.saturated,
            );
        }
        result
    }

    /// OS-readiness-driven loop. A single byte `1` drains the read-only owner;
    /// any other wake token refuses rather than silently stopping.
    pub fn run_with_shutdown<J, F, D>(
        &mut self,
        host: &mut AlignmentHostV1<J, F, D>,
        wake: &UnixStream,
        max_request_bytes: usize,
        max_response_bytes: usize,
        limits: chronos_protocol::alignment_v1::Limits,
    ) -> Result<(), &'static str>
    where
        J: AlignmentJournalV1,
        F: AlignmentFrontierV1,
        D: super::AlignmentDisclosureV1,
    {
        loop {
            self.guard_ownership()?;
            let mut fds = [
                PollFd::new(&self.listener, PollFlags::IN),
                PollFd::new(wake, PollFlags::IN),
            ];
            loop {
                match poll(&mut fds, None) {
                    Ok(_) => break,
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(_) => return self.fence("alignment readiness wait failed"),
                }
            }
            if fds
                .iter()
                .any(|fd| fd.revents().intersects(PollFlags::ERR | PollFlags::NVAL))
            {
                return self.fence("alignment descriptor failed");
            }
            if fds[1].revents().contains(PollFlags::IN) {
                let mut token = [0; 1];
                let mut source = wake;
                source
                    .read_exact(&mut token)
                    .map_err(|_| "alignment shutdown wake failed")?;
                if token != [1] {
                    return self.fence("invalid alignment shutdown wake");
                }
                self.guard_ownership()?;
                return Ok(());
            }
            if fds[1].revents().contains(PollFlags::HUP) {
                return self.fence("alignment shutdown channel lost");
            }
            if fds[0].revents().contains(PollFlags::IN) {
                // A client delivery failure is observable but does not corrupt
                // the immutable register or justify fencing later reads.
                let _ = self.serve_one(host, max_request_bytes, max_response_bytes, limits);
                continue;
            }
            return self.fence("unexpected alignment readiness event");
        }
    }

    fn verify_ownership(&self) -> Result<(), ()> {
        verify_private_directory(&self.directory).map_err(|_| ())?;
        let root = std::fs::symlink_metadata(&self.directory).map_err(|_| ())?;
        let lock = std::fs::symlink_metadata(self.directory.join("alignment-owner.lock"))
            .map_err(|_| ())?;
        let socket = std::fs::symlink_metadata(&self.path).map_err(|_| ())?;
        if identity(&root) != self.root_identity
            || !lock.is_file()
            || lock.uid() != rustix::process::geteuid().as_raw()
            || lock.mode() & 0o077 != 0
            || lock.nlink() != 1
            || identity(&lock) != self.lock_identity
            || identity(&self.lock.metadata().map_err(|_| ())?) != self.lock_identity
            || !socket.file_type().is_socket()
            || socket.uid() != rustix::process::geteuid().as_raw()
            || socket.mode() & 0o077 != 0
            || socket.nlink() != 1
            || identity(&socket) != self.socket_identity
        {
            return Err(());
        }
        Ok(())
    }

    fn guard_ownership(&mut self) -> Result<(), &'static str> {
        if self.verify_ownership().is_ok() {
            Ok(())
        } else {
            increment(
                &mut self.signals.ownership_failures,
                &mut self.signals.saturated,
            );
            self.fence("alignment endpoint ownership changed")
        }
    }

    fn fence<T>(&mut self, message: &'static str) -> Result<T, &'static str> {
        self.signals.fenced = true;
        Err(message)
    }
}

impl Drop for AlignmentListenerV1 {
    fn drop(&mut self) {
        if self.verify_ownership().is_ok() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn observe_socket(path: &Path) -> Result<Option<(u64, u64)>, &'static str> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("alignment endpoint observation failed"),
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err("private alignment endpoint required");
    }
    Ok(Some(identity(&metadata)))
}

fn verify_private_directory(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "private owner-only alignment directory required",
        ));
    }
    Ok(())
}

fn identity(metadata: &std::fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

fn increment(value: &mut u64, saturated: &mut bool) {
    if let Some(next) = value.checked_add(1) {
        *value = next;
    } else {
        *saturated = true;
    }
}

fn add_time(value: &mut Duration, delta: Duration, saturated: &mut bool) {
    if let Some(total) = value.checked_add(delta) {
        *value = total;
    } else {
        *value = Duration::MAX;
        *saturated = true;
    }
}

pub fn serve_connection<J, F, D>(
    host: &mut AlignmentHostV1<J, F, D>,
    mut stream: UnixStream,
    max_request_bytes: usize,
    max_response_bytes: usize,
    limits: chronos_protocol::alignment_v1::Limits,
) -> Result<(), &'static str>
where
    J: AlignmentJournalV1,
    F: AlignmentFrontierV1,
    D: super::AlignmentDisclosureV1,
{
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|_| "alignment connection timeout setup failed")?;
    let frame = match chronos_wire::read_frame(&mut BufReader::new(&stream), max_request_bytes) {
        Ok(Some(frame)) => frame,
        Ok(None) => return Ok(()),
        Err(error) => {
            let response = chronos_wire::alignment_refusal_v1(match error {
                chronos_wire::FrameError::Limit => chronos_protocol::CodecError::Limit,
                chronos_wire::FrameError::Capacity => chronos_protocol::CodecError::Capacity,
                chronos_wire::FrameError::Io(_) | chronos_wire::FrameError::Truncated => {
                    chronos_protocol::CodecError::Malformed
                }
            });
            return chronos_wire::write_frame(&mut stream, &response, max_response_bytes)
                .map_err(|_| "alignment refusal delivery failed");
        }
    };
    let response =
        match chronos_wire::decode_alignment_request_v1(&frame, max_request_bytes, limits) {
            Ok(request) => host.dispatch(request),
            Err(error) => chronos_wire::alignment_refusal_v1(error),
        };
    chronos_wire::write_frame(&mut stream, &response, max_response_bytes)
        .map_err(|_| "alignment response delivery failed")
}
