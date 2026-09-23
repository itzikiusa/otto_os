//! Bounded preparation of a single note. Exact hashing streams the source;
//! body retention and parsing have a separate, explicit ceiling.
use crate::{
    parse,
    scan::MAX_FTS_BYTES,
    store::NoteRow,
    types::{ContentIndexStatus, OutgoingLink},
};
use otto_core::{Error, Result};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Semaphore;

pub(crate) struct PreparedNote {
    pub row: NoteRow,
    pub links: Vec<OutgoingLink>,
    pub body: String,
}

impl PreparedNote {
    pub fn meta(&self) -> crate::types::NoteMeta {
        let row = &self.row;
        crate::types::NoteMeta {
            path: row.path.clone(),
            title: row.title.clone(),
            okf_type: row.okf_type.clone(),
            description: row.description.clone(),
            frontmatter: serde_json::from_str(&row.frontmatter_json).unwrap_or_default(),
            tags: serde_json::from_str(&row.tags_json).unwrap_or_default(),
            aliases: serde_json::from_str(&row.aliases_json).unwrap_or_default(),
            headings: serde_json::from_str(&row.headings_json).unwrap_or_default(),
            word_count: row.word_count,
            size: row.size,
            hash: row.hash.clone(),
            reserved: row.reserved,
            has_frontmatter: row.has_frontmatter,
            parse_error: row.parse_error,
            content_index_status: row.content_index_status,
        }
    }
}

pub(crate) struct Preparation {
    admission: Arc<Semaphore>,
    running: Arc<Semaphore>,
}
impl Default for Preparation {
    fn default() -> Self {
        Self {
            admission: Arc::new(Semaphore::new(4)),
            running: Arc::new(Semaphore::new(2)),
        }
    }
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Preparation {
    pub async fn bytes(&self, rel: String, bytes: &[u8], mtime_ns: i64) -> Result<PreparedNote> {
        let admission =
            self.admission.clone().try_acquire_owned().map_err(|_| {
                Error::Conflict("Vault indexing is busy; retry the operation".into())
            })?;
        let running = self
            .running
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::Internal("Vault preparation closed".into()))?;
        // Do not retain an additional submitted buffer while waiting for admission.
        let bytes = bytes.to_vec();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(cancelled.clone());
        tokio::task::spawn_blocking(move || {
            let (_admission, _running) = (admission, running);
            prepare_reader(Cursor::new(bytes), &rel, mtime_ns, &cancelled)
        })
        .await
        .map_err(|e| Error::Internal(format!("note preparation: {e}")))?
    }

    /// Scan-side preparation. Unlike `bytes`, this WAITS for admission: the
    /// scan is sequential and holds no buffer while queued, and failing fast
    /// here marked the whole scan incomplete whenever interactive reads were
    /// in flight (every mutation then reported failure after succeeding).
    pub async fn file(&self, path: PathBuf, rel: String) -> Result<PreparedNote> {
        let admission = self
            .admission
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::Internal("Vault preparation closed".into()))?;
        let running = self
            .running
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::Internal("Vault preparation closed".into()))?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(cancelled.clone());
        tokio::task::spawn_blocking(move || {
            let (_admission, _running) = (admission, running);
            let fd = rustix::fs::open(
                &path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::NONBLOCK
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|e| Error::Internal(format!("open note: {e}")))?;
            let mut file = std::fs::File::from(fd);
            let before = file.metadata().map_err(io_error)?;
            if !before.is_file() {
                return Err(Error::Invalid("note is not a regular file".into()));
            }
            let prepared = prepare_reader(&mut file, &rel, mtime(&before), &cancelled)?;
            let after = file.metadata().map_err(io_error)?;
            let current = std::fs::symlink_metadata(&path).map_err(io_error)?;
            use std::os::unix::fs::MetadataExt;
            if before.len() != after.len()
                || mtime(&before) != mtime(&after)
                || before.ino() != current.ino()
                || before.dev() != current.dev()
                || after.len() != current.len()
                || mtime(&after) != mtime(&current)
                || prepared.row.size as u64 != after.len()
            {
                return Err(Error::Conflict(
                    "note changed during indexing; retry scan".into(),
                ));
            }
            Ok(prepared)
        })
        .await
        .map_err(|e| Error::Internal(format!("note preparation: {e}")))?
    }
}
fn io_error(e: std::io::Error) -> Error {
    Error::Internal(format!("read note: {e}"))
}
pub(crate) fn mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}
fn prepare_reader(
    mut reader: impl Read,
    rel: &str,
    mtime_ns: i64,
    cancelled: &AtomicBool,
) -> Result<PreparedNote> {
    let mut hash = Sha256::new();
    let mut retained = Vec::new();
    let mut size = 0usize;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::Conflict("note indexing canceled".into()));
        }
        let len = reader.read(&mut buffer).map_err(io_error)?;
        if len == 0 {
            break;
        }
        hash.update(&buffer[..len]);
        size += len;
        if size <= MAX_FTS_BYTES as usize {
            retained.extend_from_slice(&buffer[..len]);
        } else if !retained.is_empty() {
            retained = Vec::new();
        }
    }
    let limited = size > MAX_FTS_BYTES as usize;
    let body = String::from_utf8_lossy(&retained).into_owned();
    let parsed = if limited {
        parse::ParsedNote::default()
    } else {
        parse::parse_note(&body)
    };
    let base = rel.rsplit('/').next().unwrap_or(rel).to_ascii_lowercase();
    let row = NoteRow {
        path: rel.into(),
        title: parse::derive_title(&parsed, rel),
        okf_type: parsed.okf_type,
        description: parsed.description,
        frontmatter_json: serde_json::to_string(&parsed.frontmatter)
            .unwrap_or_else(|_| "null".into()),
        tags_json: serde_json::to_string(&parsed.tags).unwrap_or_else(|_| "[]".into()),
        aliases_json: serde_json::to_string(&parsed.aliases).unwrap_or_else(|_| "[]".into()),
        headings_json: serde_json::to_string(&parsed.headings).unwrap_or_else(|_| "[]".into()),
        word_count: parsed.word_count as i64,
        size: size as i64,
        mtime_ns,
        hash: format!("{:x}", hash.finalize()),
        reserved: matches!(base.as_str(), "index.md" | "log.md"),
        has_frontmatter: parsed.has_frontmatter,
        parse_error: parsed.parse_error,
        content_index_status: if limited {
            ContentIndexStatus::SizeLimited
        } else {
            ContentIndexStatus::Full
        },
    };
    Ok(PreparedNote {
        row,
        links: parsed.links,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn exact_threshold_hash_and_metadata_policy() {
        let prep = Preparation::default();
        let mut body = b"# Heading\n[[target]] #tag\n".to_vec();
        body.resize(MAX_FTS_BYTES as usize, b'x');
        let small = prep.bytes("note.md".into(), &body, 1).await.unwrap();
        assert_eq!(small.row.content_index_status, ContentIndexStatus::Full);
        assert_eq!(small.links.len(), 1);
        body.push(b'x');
        let large = prep.bytes("note.md".into(), &body, 2).await.unwrap();
        assert_eq!(
            large.row.content_index_status,
            ContentIndexStatus::SizeLimited
        );
        assert!(large.body.is_empty() && large.links.is_empty());
        assert_eq!(large.row.hash, format!("{:x}", Sha256::digest(&body)));
        assert_eq!(large.row.tags_json, "[]");
    }

    #[tokio::test]
    async fn preparation_admission_bounds_waiting_work_and_cancellation() {
        let prep = Arc::new(Preparation::default());
        let occupied = prep.running.clone().acquire_many_owned(2).await.unwrap();
        let mut tasks = vec![];
        for _ in 0..4 {
            let prep = prep.clone();
            tasks.push(tokio::spawn(async move {
                prep.bytes("note.md".into(), b"# Note", 0).await
            }));
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while prep.admission.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(prep
            .bytes("extra.md".into(), b"extra", 0)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("busy"));
        tasks[0].abort();
        let canceled = tasks.remove(0);
        let _ = canceled.await;
        assert_eq!(prep.admission.available_permits(), 1);
        drop(occupied);
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(prep.admission.available_permits(), 4);
        assert_eq!(prep.running.available_permits(), 2);
    }
}
