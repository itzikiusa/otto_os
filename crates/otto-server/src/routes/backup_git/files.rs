//! Descriptor-relative snapshot I/O. Parent and leaf links are never followed;
//! publication replaces directory entries, never opens an existing file for write.
use super::{conflict, digest, internal, invalid, relative, ApiResult};
use rustix::{
    fd::OwnedFd,
    fs::{self, AtFlags, FileType, Mode, OFlags},
    io::Errno,
};
use std::{
    io::{Read, Write},
    path::Path,
};

pub(super) const MAX_FILE: usize = 64 * 1024 * 1024;
pub(super) const MAX_TOTAL: usize = 256 * 1024 * 1024;
pub(super) const MAX_ENTRIES: usize = 100_000;
fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}
fn parent(root: &Path, path: &str, create: bool) -> ApiResult<Option<(OwnedFd, String)>> {
    relative(path)?;
    let parts: Vec<_> = path.split('/').collect();
    let mut fd = fs::open(root, dir_flags(), Mode::empty()).map_err(internal)?;
    for part in &parts[..parts.len() - 1] {
        fd = match fs::openat(&fd, *part, dir_flags(), Mode::empty()) {
            Ok(child) => child,
            Err(Errno::NOENT) if create => {
                match fs::mkdirat(&fd, *part, Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(Errno::EXIST) => (),
                    Err(e) => return Err(internal(e)),
                }
                fs::openat(&fd, *part, dir_flags(), Mode::empty())
                    .map_err(|_| conflict("Snapshot parent changed or is a symlink"))?
            }
            Err(Errno::NOENT) => return Ok(None),
            Err(_) => {
                return Err(conflict(
                    "Snapshot parent must be a directory without symlinks",
                ))
            }
        };
    }
    Ok(Some((fd, parts.last().unwrap().to_string())))
}
fn read_at(fd: &OwnedFd, name: &str) -> ApiResult<Option<Vec<u8>>> {
    let file = match fs::openat(
        fd,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(Errno::NOENT) => return Ok(None),
        Err(_) => return Err(conflict("Snapshot file is inaccessible or a symlink")),
    };
    let stat = fs::fstat(&file).map_err(internal)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(invalid("Snapshot entries must be regular files"));
    }
    if stat.st_size < 0 || stat.st_size as usize > MAX_FILE {
        return Err(invalid("Snapshot file exceeds 64 MiB"));
    }
    let mut bytes = Vec::new();
    std::fs::File::from(file)
        .take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(internal)?;
    if bytes.len() > MAX_FILE {
        return Err(invalid("Snapshot file exceeds 64 MiB"));
    }
    Ok(Some(bytes))
}
pub(super) fn read(root: &Path, path: &str) -> ApiResult<Option<Vec<u8>>> {
    match parent(root, path, false)? {
        Some((fd, leaf)) => read_at(&fd, &leaf),
        None => Ok(None),
    }
}
fn verify(fd: &OwnedFd, leaf: &str, expected: Option<&str>) -> ApiResult<()> {
    if read_at(fd, leaf)?.as_ref().map(|b| digest(b)).as_deref() != expected {
        return Err(conflict(
            "Snapshot file changed since preview; export stopped",
        ));
    }
    Ok(())
}
pub(super) fn publish(
    root: &Path,
    path: &str,
    bytes: &[u8],
    expected: Option<&str>,
) -> ApiResult<()> {
    if bytes.len() > MAX_FILE {
        return Err(invalid("Snapshot file exceeds 64 MiB"));
    }
    let (fd, leaf) = parent(root, path, true)?.ok_or_else(|| invalid("Missing snapshot parent"))?;
    let temporary = format!(".otto-export-{}", uuid::Uuid::new_v4());
    let opened = fs::openat(
        &fd,
        temporary.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(internal)?;
    let result = (|| {
        let mut file = std::fs::File::from(opened);
        file.write_all(bytes).map_err(internal)?;
        file.sync_all().map_err(internal)?;
        // Hash after staging, immediately before publication. No existing leaf is
        // opened for writing, so even a concurrent symlink cannot redirect bytes.
        verify(&fd, &leaf, expected)?;
        if expected.is_none() {
            fs::linkat(
                &fd,
                temporary.as_str(),
                &fd,
                leaf.as_str(),
                AtFlags::empty(),
            )
            .map_err(|_| conflict("Snapshot destination appeared during export"))?;
        } else {
            fs::renameat(&fd, temporary.as_str(), &fd, leaf.as_str()).map_err(internal)?;
        }
        Ok(())
    })();
    let _ = fs::unlinkat(&fd, temporary.as_str(), AtFlags::empty());
    result
}
pub(super) fn remove(root: &Path, path: &str, expected: &str) -> ApiResult<()> {
    let (fd, leaf) = parent(root, path, false)?
        .ok_or_else(|| conflict("Snapshot file disappeared during export"))?;
    verify(&fd, &leaf, Some(expected))?;
    fs::unlinkat(&fd, leaf.as_str(), AtFlags::empty()).map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anchored_parent_does_not_follow_a_replacement_symlink() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("child")).unwrap();
        let (fd, _) = parent(root.path(), "child/note", false).unwrap().unwrap();
        std::fs::rename(root.path().join("child"), root.path().join("original")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), root.path().join("child")).unwrap();
        assert!(read_at(&fd, "note").unwrap().is_none());
        assert!(publish(root.path(), "child/note", b"secret", None).is_err());
        assert!(!outside.path().join("note").exists());
    }
    #[test]
    fn stale_leaf_and_oversized_files_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        publish(root.path(), "note", b"first", None).unwrap();
        assert!(publish(root.path(), "note", b"replace", Some(&digest(b"old"))).is_err());
        assert!(remove(root.path(), "note", &digest(b"old")).is_err());
        assert_eq!(read(root.path(), "note").unwrap().unwrap(), b"first");
        let file = std::fs::File::create(root.path().join("large")).unwrap();
        file.set_len(MAX_FILE as u64 + 1).unwrap();
        assert!(read(root.path(), "large").is_err());
    }
}
