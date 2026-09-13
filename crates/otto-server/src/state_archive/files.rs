//! File assets are addressed by trusted root labels, never archive-supplied
//! absolute destination paths. All descendant opens refuse symlinks.
use super::{
    digest, invalid, ArchiveFile, ArchiveRoot, StateArchive, MAX_ARCHIVE_BYTES, MAX_FILE_BYTES,
};
use crate::error::ApiResult;
use base64::{engine::general_purpose::STANDARD, Engine};
use rustix::{
    fd::OwnedFd,
    fs::{self, AtFlags, FileType, Mode, OFlags},
    io::Errno,
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
};
pub const DATA_ROOTS: &[&str] = &[
    "library",
    "canvas",
    "product",
    "snips",
    "transcripts",
    "workflow-context",
    "scheduled",
    "personal",
    "insights",
];
fn err(e: impl std::fmt::Display) -> crate::error::ApiError {
    invalid(&format!("Archive asset: {e}"))
}
fn excluded_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        ".otto-sync"
            | ".git"
            | ".env"
            | "id_rsa"
            | "id_ed25519"
            | "id_ecdsa"
            | "id_dsa"
            | "credentials"
            | "credentials.json"
            | "secrets.json"
            | "secrets.yaml"
            | "secrets.yml"
    ) || name.starts_with(".env.")
        || matches!(
            Path::new(&name).extension().and_then(|s| s.to_str()),
            Some("pem" | "key" | "p12" | "pfx")
        )
}

fn flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}
pub fn relative(path: &str) -> ApiResult<Vec<&str>> {
    if path.is_empty() || path.starts_with('/') || path.contains(['\\', '\0']) {
        return Err(invalid("Invalid archive file path"));
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return Err(invalid(
            "Archive paths cannot contain empty, dot or parent components",
        ));
    }
    Ok(parts)
}
pub fn source_files(
    root: &ArchiveRoot,
    path: &Path,
    output: &mut Vec<ArchiveFile>,
    excluded: &mut Vec<String>,
    bytes: &mut usize,
) -> ApiResult<()> {
    let fd = fs::open(path, flags(), Mode::empty()).map_err(err)?;
    walk(root, &fd, "", output, excluded, bytes)
}
fn walk(
    root: &ArchiveRoot,
    fd: &OwnedFd,
    prefix: &str,
    output: &mut Vec<ArchiveFile>,
    excluded: &mut Vec<String>,
    bytes: &mut usize,
) -> ApiResult<()> {
    let dir = fs::Dir::read_from(fd).map_err(err)?;
    let mut names = Vec::new();
    for item in dir {
        let item = item.map_err(err)?;
        let name = item.file_name().to_str().map_err(err)?;
        if !matches!(name, "." | "..") {
            names.push(name.to_string());
        }
    }
    names.sort();
    for name in names {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if name == ".otto-sync" {
            continue;
        }
        if excluded_name(&name) {
            excluded.push(format!(
                "file {}/{}: credentials or repository metadata",
                root.id, path
            ));
            continue;
        }
        let stat = fs::statat(fd, name.as_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(err)?;
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => {
                let child = fs::openat(fd, name.as_str(), flags(), Mode::empty()).map_err(err)?;
                walk(root, &child, &path, output, excluded, bytes)?;
            }
            FileType::RegularFile => {
                if stat.st_size < 0 || stat.st_size as usize > MAX_FILE_BYTES {
                    return Err(invalid(&format!(
                        "Asset {}/{} exceeds 64 MiB; archive not exported",
                        root.id, path
                    )));
                }
                let file = fs::openat(
                    fd,
                    name.as_str(),
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(err)?;
                let before = fs::fstat(&file).map_err(err)?;
                if FileType::from_raw_mode(before.st_mode) != FileType::RegularFile {
                    return Err(invalid("Asset changed to a non-regular file"));
                }
                let mut file = std::fs::File::from(file);
                let mut data = Vec::new();
                (&mut file)
                    .take((MAX_FILE_BYTES + 1) as u64)
                    .read_to_end(&mut data)
                    .map_err(err)?;
                let after = fs::fstat(&file).map_err(err)?;
                if before.st_size != after.st_size
                    || before.st_mtime != after.st_mtime
                    || before.st_mtime_nsec != after.st_mtime_nsec
                {
                    return Err(invalid("Asset changed during export; retry after saving"));
                }
                if data.len() > MAX_FILE_BYTES {
                    return Err(invalid("Asset grew beyond the 64 MiB limit"));
                }
                *bytes += data.len().div_ceil(3) * 4;
                if *bytes > MAX_ARCHIVE_BYTES {
                    return Err(invalid(
                        "Archive exceeds 256 MiB; no partial archive is returned",
                    ));
                }
                output.push(ArchiveFile {
                    root: root.id.clone(),
                    path,
                    sha256: digest(&data),
                    content_base64: STANDARD.encode(data),
                });
            }
            _ => {
                return Err(invalid(&format!(
                "Asset {}/{} is a symlink or special file; complete export requires regular files",
                root.id, path
            )))
            }
        }
    }
    Ok(())
}
pub fn root_relative(root: &ArchiveRoot, restore_id: &str) -> ApiResult<String> {
    match root.kind.as_str() {
        "data" => {
            let directory = root
                .owner_id
                .as_deref()
                .ok_or_else(|| invalid("Missing data root name"))?;
            if !DATA_ROOTS.contains(&directory) || root.id != format!("data-{directory}") {
                return Err(invalid("Unknown managed asset root"));
            }
            Ok(directory.into())
        }
        "vault" => {
            let id = root
                .owner_id
                .as_deref()
                .ok_or_else(|| invalid("Missing vault root id"))?;
            if id.parse::<i64>().is_err() || root.id != format!("vault-{id}") {
                return Err(invalid("Invalid vault root"));
            }
            Ok(format!("restored/{restore_id}/vaults/{id}"))
        }
        _ => Err(invalid("Unknown archive asset root kind")),
    }
}
pub fn validate_files(archive: &StateArchive) -> ApiResult<()> {
    let roots: BTreeMap<_, _> = archive.roots.iter().map(|r| (r.id.as_str(), r)).collect();
    if roots.len() != archive.roots.len() {
        return Err(invalid("Duplicate archive roots"));
    }
    for root in &archive.roots {
        root_relative(root, "preview")?;
    }
    let mut paths = std::collections::BTreeSet::new();
    let mut total = 0usize;
    for file in &archive.files {
        if !roots.contains_key(file.root.as_str()) {
            return Err(invalid("File refers to an unknown archive root"));
        }
        if relative(&file.path)?.iter().any(|part| excluded_name(part)) {
            return Err(invalid(
                "Credential files and Git metadata cannot be imported",
            ));
        }
        if !paths.insert((&file.root, &file.path)) {
            return Err(invalid("Duplicate archive file"));
        }
        if file.content_base64.len() > MAX_FILE_BYTES.div_ceil(3) * 4 {
            return Err(invalid("Archive file exceeds 64 MiB"));
        }
        total += file.content_base64.len();
        if total > MAX_ARCHIVE_BYTES {
            return Err(invalid("Archive exceeds 256 MiB"));
        }
        let bytes = STANDARD
            .decode(&file.content_base64)
            .map_err(|_| invalid("Invalid archive file encoding"))?;
        if bytes.len() > MAX_FILE_BYTES || digest(&bytes) != file.sha256 {
            return Err(invalid("Archive asset hash/size validation failed"));
        }
    }
    Ok(())
}
/// Inspect every existing parent, including the final path, without following links.
pub fn existing(data_dir: &Path, relative_path: &str) -> ApiResult<bool> {
    let parts = relative(relative_path)?;
    let mut fd = fs::open(data_dir, flags(), Mode::empty()).map_err(err)?;
    for part in &parts[..parts.len() - 1] {
        match fs::openat(&fd, *part, flags(), Mode::empty()) {
            Ok(child) => fd = child,
            Err(Errno::NOENT) => return Ok(false),
            Err(e) => return Err(err(e)),
        }
    }
    match fs::statat(&fd, parts[parts.len() - 1], AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            if FileType::from_raw_mode(stat.st_mode) == FileType::Symlink {
                return Err(invalid("Restore destination contains a symlink"));
            }
            Ok(true)
        }
        Err(Errno::NOENT) => Ok(false),
        Err(e) => Err(err(e)),
    }
}
/// Publish a staged file with exclusive creation; neither leaf nor parent
/// symlinks can redirect the write. Returns false for an existing leaf.
pub fn publish(data_dir: &Path, relative_path: &str, bytes: &[u8]) -> ApiResult<bool> {
    publish_using(data_dir, relative_path, |file| {
        file.write_all(bytes).and_then(|_| file.sync_all())
    })
}
pub(super) fn publish_using(
    data_dir: &Path,
    relative_path: &str,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> ApiResult<bool> {
    let parts = relative(relative_path)?;
    let mut fd = fs::open(data_dir, flags(), Mode::empty()).map_err(err)?;
    for part in &parts[..parts.len() - 1] {
        let child = match fs::openat(&fd, *part, flags(), Mode::empty()) {
            Ok(child) => child,
            Err(Errno::NOENT) => {
                match fs::mkdirat(&fd, *part, Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(Errno::EXIST) => {}
                    Err(e) => return Err(err(e)),
                };
                fs::openat(&fd, *part, flags(), Mode::empty()).map_err(err)?
            }
            Err(e) => return Err(err(e)),
        };
        fd = child;
    }
    let leaf = parts[parts.len() - 1];
    let file = match fs::openat(
        &fd,
        leaf,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    ) {
        Ok(f) => f,
        Err(Errno::EXIST) => return Ok(false),
        Err(e) => return Err(err(e)),
    };
    let mut file = std::fs::File::from(file);
    if let Err(error) = write(&mut file) {
        // The leaf belongs to this exclusive create. Check its inode before cleanup.
        if let (Ok(owned), Ok(current)) = (
            fs::fstat(&file),
            fs::statat(&fd, leaf, AtFlags::SYMLINK_NOFOLLOW),
        ) {
            if owned.st_ino == current.st_ino && owned.st_dev == current.st_dev {
                let _ = fs::unlinkat(&fd, leaf, AtFlags::empty());
            }
        }
        return Err(err(error));
    }
    Ok(true)
}
/// Roll back only bytes still matching our exclusive publication, using the
/// held parent descriptor for the final unlink as well as every lookup.
pub fn remove_matching(data_dir: &Path, relative_path: &str, hash: &str) -> ApiResult<()> {
    let parts = relative(relative_path)?;
    let mut fd = fs::open(data_dir, flags(), Mode::empty()).map_err(err)?;
    for part in &parts[..parts.len() - 1] {
        fd = fs::openat(&fd, *part, flags(), Mode::empty()).map_err(err)?;
    }
    let leaf = parts[parts.len() - 1];
    let file = fs::openat(
        &fd,
        leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(err)?;
    let owned = fs::fstat(&file).map_err(err)?;
    let mut bytes = Vec::new();
    std::fs::File::from(file)
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    let current = fs::statat(&fd, leaf, AtFlags::SYMLINK_NOFOLLOW).map_err(err)?;
    if digest(&bytes) == hash && owned.st_ino == current.st_ino && owned.st_dev == current.st_dev {
        fs::unlinkat(&fd, leaf, AtFlags::empty()).map_err(err)?;
    }
    Ok(())
}
