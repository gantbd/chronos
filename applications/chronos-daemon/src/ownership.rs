//! Concrete standalone ownership policy, never linked by a remote client.
#![forbid(unsafe_code)]
use chronos_local::verify_private_directory;
use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

/// Never recursively creates ancestors or changes an existing directory's mode.
pub fn create_private_directory(path: &Path) -> io::Result<()> {
    let leaf: PathBuf = path.components().collect();
    let path = leaf.as_path();
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    verify_private_directory(path)
}

/// Advisory process lock, released by the OS on drop/process death. The lock
/// inode stays in place; deleting it would permit two independent lock owners.
pub struct WriterOwnership {
    file: File,
    directory: PathBuf,
    root_identity: (u64, u64),
    lock_identity: (u64, u64),
    created: bool,
}
impl WriterOwnership {
    pub fn acquire(directory: &Path) -> io::Result<Self> {
        let leaf: PathBuf = directory.components().collect();
        let directory = leaf.as_path();
        verify_private_directory(directory)?;
        let root = std::fs::symlink_metadata(directory)?;
        let directory = std::fs::canonicalize(directory)?;
        let flags = i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())
            .map_err(|_| io::Error::other("unsupported open flags"))?;
        let open = |create: bool| {
            OpenOptions::new()
                .create_new(create)
                .read(true)
                .write(true)
                .mode(0o600)
                .custom_flags(flags)
                .open(directory.join("writer.lock"))
        };
        let (file, created) = match open(true) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (open(false)?, false),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private regular lock file required",
            ));
        }
        file.try_lock().map_err(io::Error::from)?;
        if created {
            file.sync_all()?;
            File::open(&directory)?.sync_all()?;
        }
        let ownership = Self {
            file,
            directory,
            root_identity: identity(&root),
            lock_identity: identity(&metadata),
            created,
        };
        ownership.verify()?;
        Ok(ownership)
    }

    /// Only atomic creation of this persistent lock establishes first-start
    /// provenance. Missing checkpoints under an existing lock are not initial.
    pub fn created(&self) -> bool {
        self.created
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn verify(&self) -> io::Result<()> {
        verify_private_directory(&self.directory)?;
        let root = std::fs::symlink_metadata(&self.directory)?;
        let lock = std::fs::symlink_metadata(self.directory.join("writer.lock"))?;
        if identity(&root) != self.root_identity
            || !lock.is_file()
            || lock.uid() != rustix::process::geteuid().as_raw()
            || lock.mode() & 0o077 != 0
            || lock.nlink() != 1
            || identity(&lock) != self.lock_identity
            || identity(&self.file.metadata()?) != self.lock_identity
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "writer resource identity changed",
            ));
        }
        Ok(())
    }
}

pub(crate) fn identity(metadata: &std::fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

pub fn restrict_socket(path: &Path) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

pub(crate) fn observe_endpoint(path: &Path, created: bool) -> io::Result<Option<(u64, u64)>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if created
        || !metadata.file_type().is_socket()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "endpoint refused",
        ));
    }
    Ok(Some(identity(&metadata)))
}

pub(crate) fn remove_observed_endpoint(
    path: &Path,
    expected: Option<(u64, u64)>,
    owner: &WriterOwnership,
) -> io::Result<()> {
    owner.verify()?;
    if observe_endpoint(path, false)? != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "endpoint changed",
        ));
    }
    if expected.is_some() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
