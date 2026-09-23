//! File assets are addressed by trusted root labels, never archive-supplied
//! absolute destination paths. All descendant opens refuse symlinks.
use super::{
    digest, invalid, ArchiveFile, ArchiveRoot, ArchiveRow, StateArchive, MAX_ARCHIVE_BYTES,
    MAX_FILE_BYTES,
};
use crate::error::ApiResult;
use base64::{engine::general_purpose::STANDARD, Engine};
use rustix::{
    fd::OwnedFd,
    fs::{self, AtFlags, FileType, Mode, OFlags},
    io::Errno,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
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
/// The Design Hall blob store (`<data>/design/blobs/<sha256>`), relative to
/// the data dir. NOT a [`DATA_ROOTS`] entry — it is never walked wholesale:
/// only the blobs the saved `design_versions` / `design_artifacts` rows
/// reference are archived, after every other root and best-effort within the
/// remaining size budget (see [`design_blob_files`]).
pub const DESIGN_BLOBS_DIR: &str = "design/blobs";
pub const DESIGN_BLOBS_ROOT_ID: &str = "data-design/blobs";
/// Room kept free under [`MAX_ARCHIVE_BYTES`] for the JSON envelope when
/// design blobs fill the budget (records, manifests, per-file keys).
const DESIGN_BLOB_HEADROOM: usize = 8 * 1024 * 1024;
/// JSON bytes one archived file costs besides its base64 content.
const ARCHIVE_FILE_OVERHEAD: usize = 256;

pub fn design_blobs_root() -> ArchiveRoot {
    ArchiveRoot {
        id: DESIGN_BLOBS_ROOT_ID.into(),
        kind: "data".into(),
        owner_id: Some(DESIGN_BLOBS_DIR.into()),
    }
}

/// A design blob name: exactly 64 lowercase hex chars (its sha256).
pub fn is_blob_name(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Every blob the saved design rows point at: version bytes
/// (`design_versions.blob_sha256`) and thumbnails (`design_artifacts.thumb_blob`).
pub fn design_blob_refs(records: &BTreeMap<String, Vec<ArchiveRow>>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (table, column) in [
        ("design_versions", "blob_sha256"),
        ("design_artifacts", "thumb_blob"),
    ] {
        for row in records.get(table).into_iter().flatten() {
            if let Some(sha) = row
                .get(column)
                .and_then(Value::as_str)
                .filter(|s| is_blob_name(s))
            {
                out.insert(sha.to_string());
            }
        }
    }
    out
}

/// Archive the referenced design blobs, BEST-EFFORT: a blob that would push
/// the archive past `MAX_ARCHIVE_BYTES` (minus headroom) — or is over the
/// per-file cap — is skipped, and so is a missing / unreadable / corrupt one
/// (its bytes must still hash to its name); each kind of skip is summarized
/// in ONE `excluded` note (+ a warning log) instead of failing the export —
/// the design rows stay restorable, their skipped content degrades to a clear
/// "design blob … not found". Returns how many blobs were archived.
pub fn design_blob_files(
    data_dir: &Path,
    shas: &BTreeSet<String>,
    output: &mut Vec<ArchiveFile>,
    excluded: &mut Vec<String>,
    bytes: &mut usize,
) -> ApiResult<usize> {
    if shas.is_empty() {
        return Ok(0);
    }
    let dir = data_dir.join(DESIGN_BLOBS_DIR);
    let fd = match fs::open(dir.as_path(), flags(), Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => {
            excluded.push(format!(
                "design blobs: {} referenced blob(s) not archived — <data>/{DESIGN_BLOBS_DIR} is missing",
                shas.len()
            ));
            return Ok(0);
        }
        Err(e) => return Err(err(e)),
    };
    let budget = MAX_ARCHIVE_BYTES.saturating_sub(DESIGN_BLOB_HEADROOM);
    let (mut archived, mut missing, mut over, mut over_bytes) = (0usize, 0usize, 0usize, 0usize);
    for sha in shas.iter().filter(|s| is_blob_name(s)) {
        let stat = match fs::statat(&fd, sha.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(_) => {
                missing += 1;
                continue;
            }
        };
        let size = usize::try_from(stat.st_size).unwrap_or(usize::MAX);
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            missing += 1;
            continue;
        }
        if size > MAX_FILE_BYTES
            || bytes.saturating_add(size.div_ceil(3) * 4 + ARCHIVE_FILE_OVERHEAD) > budget
        {
            over += 1;
            over_bytes = over_bytes.saturating_add(size);
            continue;
        }
        let Ok(file) = fs::openat(
            &fd,
            sha.as_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        ) else {
            missing += 1;
            continue;
        };
        let mut data = Vec::new();
        let read = std::fs::File::from(file)
            .take((MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut data);
        // Immutable, content-addressed: the bytes must hash to the name.
        if read.is_err() || data.len() != size || digest(&data) != *sha {
            missing += 1;
            continue;
        }
        *bytes += data.len().div_ceil(3) * 4 + ARCHIVE_FILE_OVERHEAD;
        output.push(ArchiveFile {
            root: DESIGN_BLOBS_ROOT_ID.into(),
            path: sha.clone(),
            sha256: sha.clone(),
            content_base64: STANDARD.encode(&data),
        });
        archived += 1;
    }
    if over > 0 {
        tracing::warn!(
            skipped = over,
            bytes = over_bytes,
            "saved-state archive: design blobs skipped to stay under the archive cap"
        );
        excluded.push(format!(
            "design blobs: {over} of {} skipped (~{} MiB) to stay under the 256 MiB archive cap — back up <data>/{DESIGN_BLOBS_DIR} with the data dir",
            shas.len(),
            over_bytes.div_ceil(1024 * 1024)
        ));
    }
    if missing > 0 {
        excluded.push(format!(
            "design blobs: {missing} referenced blob(s) missing, unreadable or corrupt in <data>/{DESIGN_BLOBS_DIR}"
        ));
    }
    Ok(archived)
}

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
            let known = DATA_ROOTS.contains(&directory) || directory == DESIGN_BLOBS_DIR;
            if !known || root.id != format!("data-{directory}") {
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
        // Design blobs restore under their content hash only.
        if file.root == DESIGN_BLOBS_ROOT_ID
            && !(is_blob_name(&file.path) && file.path == file.sha256)
        {
            return Err(invalid("A design blob must be named by its sha256"));
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
