//! Reusable process-owned lifecycle for independent local read-only endpoints.
//! Protocol decoding, disclosure and owner semantics remain with each caller.

use rustix::event::{PollFd, PollFlags, poll};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(crate) struct LocalReadEndpointProfileV1 {
    pub(crate) socket_name: &'static str,
    pub(crate) lock_name: &'static str,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalReadEndpointSignalsV1 {
    pub accepted: u64,
    pub delivery_failed: u64,
    pub service_time: Duration,
    pub ownership_failures: u64,
    pub saturated: bool,
    pub fenced: bool,
}

pub(crate) struct LocalReadEndpointV1 {
    listener: UnixListener,
    directory: PathBuf,
    root_identity: (u64, u64),
    lock: File,
    lock_identity: (u64, u64),
    lock_name: &'static str,
    path: PathBuf,
    socket_identity: (u64, u64),
    signals: LocalReadEndpointSignalsV1,
}

impl LocalReadEndpointV1 {
    pub(crate) fn bind(
        directory: impl AsRef<Path>,
        profile: LocalReadEndpointProfileV1,
    ) -> Result<Self, &'static str> {
        let directory = directory.as_ref();
        verify_private_directory(directory)
            .map_err(|_| "private read endpoint directory refused")?;
        let root = std::fs::symlink_metadata(directory)
            .map_err(|_| "read endpoint directory identity unavailable")?;
        let directory = std::fs::canonicalize(directory)
            .map_err(|_| "read endpoint directory canonicalization failed")?;
        let flags = i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())
            .map_err(|_| "read endpoint lock flags unsupported")?;
        let lock_path = directory.join(profile.lock_name);
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
                open(false).map_err(|_| "read endpoint owner lock refused")?,
                false,
            ),
            Err(_) => return Err("read endpoint owner lock refused"),
        };
        let lock_metadata = lock
            .metadata()
            .map_err(|_| "read endpoint owner lock identity unavailable")?;
        if !lock_metadata.is_file()
            || lock_metadata.uid() != rustix::process::geteuid().as_raw()
            || lock_metadata.mode() & 0o077 != 0
            || lock_metadata.nlink() != 1
        {
            return Err("private read endpoint owner lock required");
        }
        lock.try_lock().map_err(|_| "read endpoint already owned")?;

        let path = directory.join(profile.socket_name);
        let predecessor = observe_socket(&path)?;
        if created && predecessor.is_some() {
            return Err("unattributed read endpoint refused");
        }
        if predecessor.is_some() {
            std::fs::remove_file(&path).map_err(|_| "stale read endpoint removal failed")?;
        }
        let listener = UnixListener::bind(&path).map_err(|_| "read endpoint bind failed")?;
        if std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).is_err() {
            let _ = std::fs::remove_file(&path);
            return Err("read endpoint permissions failed");
        }
        let socket =
            std::fs::symlink_metadata(&path).map_err(|_| "read endpoint identity unavailable")?;
        let owner = Self {
            listener,
            directory,
            root_identity: identity(&root),
            lock,
            lock_identity: identity(&lock_metadata),
            lock_name: profile.lock_name,
            path,
            socket_identity: identity(&socket),
            signals: LocalReadEndpointSignalsV1::default(),
        };
        owner
            .verify_ownership()
            .map_err(|_| "read endpoint ownership refused")?;
        Ok(owner)
    }

    pub(crate) fn signals(&self) -> LocalReadEndpointSignalsV1 {
        self.signals
    }

    pub(crate) fn serve_one(
        &mut self,
        service: &mut impl FnMut(UnixStream) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        self.guard_ownership()?;
        let (stream, _) = self
            .listener
            .accept()
            .map_err(|_| "read endpoint accept failed")?;
        self.guard_ownership()?;
        increment(&mut self.signals.accepted, &mut self.signals.saturated);
        let started = Instant::now();
        let result = service(stream);
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

    pub(crate) fn run_with_shutdown(
        &mut self,
        wake: &UnixStream,
        service: &mut impl FnMut(UnixStream) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
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
                    Err(_) => return self.fence("read endpoint readiness wait failed"),
                }
            }
            if fds
                .iter()
                .any(|fd| fd.revents().intersects(PollFlags::ERR | PollFlags::NVAL))
            {
                return self.fence("read endpoint descriptor failed");
            }
            if fds[1].revents().contains(PollFlags::IN) {
                let mut token = [0; 1];
                let mut source = wake;
                source
                    .read_exact(&mut token)
                    .map_err(|_| "read endpoint shutdown wake failed")?;
                if token != [1] {
                    return self.fence("invalid read endpoint shutdown wake");
                }
                self.guard_ownership()?;
                return Ok(());
            }
            if fds[1].revents().contains(PollFlags::HUP) {
                return self.fence("read endpoint shutdown channel lost");
            }
            if fds[0].revents().contains(PollFlags::IN) {
                let _ = self.serve_one(service);
                continue;
            }
            return self.fence("unexpected read endpoint readiness event");
        }
    }

    fn verify_ownership(&self) -> Result<(), ()> {
        verify_private_directory(&self.directory).map_err(|_| ())?;
        let root = std::fs::symlink_metadata(&self.directory).map_err(|_| ())?;
        let lock =
            std::fs::symlink_metadata(self.directory.join(self.lock_name)).map_err(|_| ())?;
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
            self.fence("read endpoint ownership changed")
        }
    }

    fn fence<T>(&mut self, message: &'static str) -> Result<T, &'static str> {
        self.signals.fenced = true;
        Err(message)
    }
}

impl Drop for LocalReadEndpointV1 {
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
        Err(_) => return Err("read endpoint observation failed"),
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err("private read endpoint required");
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
            "private owner-only read endpoint directory required",
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
