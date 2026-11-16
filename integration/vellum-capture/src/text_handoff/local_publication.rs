//! Create-only publication under a host-authorized, private local directory.
use super::{TextErrorV1, TextHandoffV1, TextLimitsV1, encode_text_handoff_v1};
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags, RenameFlags};
use rustix::io::Errno;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);
const CARD: &str = "handoff.txt";
const RECORD: &str = "handoff-state.bin";
const STAGE_PREFIX: &[u8] = b".chronos-handoff-";
const MNT_LOCAL: u32 = 0x0000_1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationErrorV1 {
    InvalidDestination,
    Encoding(TextErrorV1),
    Conflict { cleanup_failed: bool },
    BeforePublication { cleanup_failed: bool },
    OutcomeUnknown,
}

pub struct DestinationV1<'a> {
    pub parent: &'a File,
    pub leaf: &'a OsStr,
    pub work_name: &'a OsStr,
    pub baseline_name: &'a OsStr,
}

pub trait PublicationAuthorityV1: super::TextHandoffAuthorityV1 {
    fn allows_destination(
        &mut self,
        document: &vellum_host::annotation::DocumentAssociation,
        destination: &DestinationV1<'_>,
    ) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportErrorV1 {
    Source(TextErrorV1),
    DestinationDenied,
    Publication(PublicationErrorV1),
}

fn leaf(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 255
        && bytes != b"."
        && bytes != b".."
        && !bytes.contains(&b'/')
        && !bytes.contains(&0)
}

/// The caller establishes current disclosure and destination authority before
/// invoking this effect and supplies the exact parent descriptor. The parent
/// is private to the host's trusted OS user. Names select direct children.
/// Success follows full synchronization of all files and both directories.
/// An unknown result requires inspection before another publication.
pub fn publish_new_directory(
    parent: &File,
    destination: &OsStr,
    work_name: &OsStr,
    baseline_name: &OsStr,
    handoff: &TextHandoffV1,
    limits: TextLimitsV1,
) -> Result<(), PublicationErrorV1> {
    use PublicationErrorV1 as Error;
    if !leaf(destination)
        || destination.as_bytes().starts_with(STAGE_PREFIX)
        || !leaf(work_name)
        || !leaf(baseline_name)
        || work_name == baseline_name
        || [work_name, baseline_name]
            .iter()
            .any(|name| *name == OsStr::new(CARD) || *name == OsStr::new(RECORD))
    {
        return Err(Error::InvalidDestination);
    }
    let stat = fs::fstat(parent).map_err(|_| Error::InvalidDestination)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_mode & 0o077 != 0
        || fs::fstatfs(parent)
            .map_err(|_| Error::InvalidDestination)?
            .f_flags
            & MNT_LOCAL
            == 0
    {
        return Err(Error::InvalidDestination);
    }
    let record = encode_text_handoff_v1(handoff, limits).map_err(Error::Encoding)?;
    let entries = [
        (work_name, handoff.source_bytes()),
        (baseline_name, handoff.source_bytes()),
        (OsStr::new(CARD), handoff.instructions().as_bytes()),
        (OsStr::new(RECORD), record.as_slice()),
    ];
    let stage_name = format!(
        ".chronos-handoff-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
    );
    fs::mkdirat(parent, &stage_name, Mode::RWXU).map_err(|_| Error::BeforePublication {
        cleanup_failed: false,
    })?;
    let stage = fs::openat(
        parent,
        &stage_name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Error::BeforePublication {
        cleanup_failed: fs::unlinkat(parent, &stage_name, AtFlags::REMOVEDIR).is_err(),
    })?;
    let cleanup = || {
        let mut failed = false;
        for (name, _) in entries {
            if let Err(error) = fs::unlinkat(&stage, name, AtFlags::empty()) {
                failed |= error != Errno::NOENT;
            }
        }
        failed | fs::unlinkat(parent, &stage_name, AtFlags::REMOVEDIR).is_err()
    };
    let stage_result = (|| -> std::io::Result<()> {
        for (name, bytes) in entries {
            let mut file = File::from(fs::openat(
                &stage,
                name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )?);
            file.write_all(bytes)?;
            fs::fcntl_fullfsync(&file)?;
        }
        fs::fcntl_fullfsync(&stage)?;
        Ok(())
    })();
    if stage_result.is_err() {
        return Err(Error::BeforePublication {
            cleanup_failed: cleanup(),
        });
    }
    match fs::renameat_with(
        parent,
        &stage_name,
        parent,
        destination,
        RenameFlags::NOREPLACE,
    ) {
        Ok(()) => fs::fcntl_fullfsync(parent).map_err(|_| Error::OutcomeUnknown),
        Err(Errno::EXIST) => Err(Error::Conflict {
            cleanup_failed: cleanup(),
        }),
        Err(_) => Err(Error::OutcomeUnknown),
    }
}
