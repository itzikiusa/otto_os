//! Bounded reuse of immutable file folds. Authorization remains at callers.
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use otto_core::{Error, Result};
use otto_transcript::{Folded, Provider, SubagentMeta};
use tokio::sync::{watch, Semaphore};

const MAX_ENTRIES: usize = 32;
const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 32 * 1024 * 1024;
const IDLE: Duration = Duration::from_secs(120);
const MAX_WAITERS: usize = 8;

#[derive(Clone)]
pub struct CacheKey {
    pub root: PathBuf,
    pub path: PathBuf,
    pub provider: Provider,
    pub sub: Option<String>,
}

type Identity = (PathBuf, PathBuf, &'static str, Option<String>);
impl CacheKey {
    fn identity(&self) -> Identity {
        (
            self.root.clone(),
            self.path.clone(),
            self.provider.as_str(),
            self.sub.clone(),
        )
    }

    fn stamp(&self) -> Result<Vec<(PathBuf, FileStamp)>> {
        let target = match self.sub.as_deref() {
            Some(sub) => otto_transcript::subagent_path(&self.path, sub)
                .ok_or_else(|| Error::Invalid("bad subagent id".into()))?,
            None => self.path.clone(),
        };
        let mut stamps = vec![(target.clone(), FileStamp::read(&target)?)];
        if self.sub.is_none() && self.provider == Provider::Claude {
            if let Some(dir) = otto_transcript::subagents_dir(&self.path) {
                match std::fs::read_dir(dir) {
                    Ok(entries) => {
                        for entry in entries {
                            let path = entry.map_err(io_error)?.path();
                            if path
                                .file_name()
                                .is_some_and(|n| n.to_string_lossy().ends_with(".meta.json"))
                            {
                                stamps.push((path.clone(), FileStamp::read(&path)?));
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_error(e)),
                }
            }
        }
        stamps.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(stamps)
    }
}

#[derive(Clone, PartialEq, Eq)]
struct FileStamp {
    bytes: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
impl FileStamp {
    fn read(path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path).map_err(io_error)?;
        if !meta.is_file() {
            return Err(Error::Invalid("transcript is not a regular file".into()));
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            bytes: meta.len(),
            modified: meta.modified().map_err(io_error)?,
            #[cfg(unix)]
            identity: (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec()),
        })
    }
}
fn io_error(e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        Error::NotFound("transcript file".into())
    } else {
        Error::Internal(format!("transcript metadata: {e}"))
    }
}
fn busy() -> Error {
    Error::Conflict("transcript busy; retry shortly".into())
}

pub struct Snapshot {
    pub folded: Folded,
    pub subagents: Vec<SubagentMeta>,
}
impl Snapshot {
    /// Conservative retained-payload accounting, not a claim of exact RSS.
    fn charge(&self) -> usize {
        struct Counter(usize);
        impl Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self.0.saturating_add(bytes.len());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut counter = Counter(0);
        for turn in &self.folded.turns {
            let _ = serde_json::to_writer(&mut counter, &turn.turn);
        }
        let _ = serde_json::to_writer(&mut counter, &self.folded.artifacts);
        let _ = serde_json::to_writer(&mut counter, &self.subagents);
        for value in [
            &self.folded.session_id,
            &self.folded.title,
            &self.folded.cwd,
            &self.folded.model,
            &self.folded.first_prompt,
            &self.folded.first_ts,
            &self.folded.last_ts,
        ]
        .into_iter()
        .flatten()
        {
            counter.0 = counter.0.saturating_add(value.capacity());
        }
        counter
            .0
            .saturating_mul(2)
            .saturating_add(
                self.folded
                    .turns
                    .len()
                    .saturating_mul(std::mem::size_of::<otto_transcript::FoldedTurn>()),
            )
            .saturating_add(4096)
    }
}

type Outcome = std::result::Result<Arc<Snapshot>, Arc<str>>;
enum Entry {
    Ready {
        stamp: Vec<(PathBuf, FileStamp)>,
        value: Arc<Snapshot>,
        bytes: usize,
        touched: Instant,
    },
    Folding {
        stamp: Vec<(PathBuf, FileStamp)>,
        sender: watch::Sender<Option<Outcome>>,
    },
}
struct Inner {
    entries: Mutex<HashMap<Identity, Entry>>,
    permits: Arc<Semaphore>,
}
#[derive(Clone)]
pub struct TranscriptCache {
    inner: Arc<Inner>,
}
impl Default for TranscriptCache {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                entries: Mutex::new(HashMap::new()),
                permits: Arc::new(Semaphore::new(2)),
            }),
        }
    }
}
impl TranscriptCache {
    pub async fn get(
        &self,
        key: CacheKey,
        build: impl FnOnce() -> Result<Snapshot> + Send + 'static,
    ) -> Result<Arc<Snapshot>> {
        let key2 = key.clone();
        let stamp = tokio::task::spawn_blocking(move || key2.stamp())
            .await
            .map_err(|e| Error::Internal(format!("transcript metadata task: {e}")))??;
        let id = key.identity();
        let mut receiver = {
            let mut entries = self.inner.entries.lock().unwrap_or_else(|e| e.into_inner());
            entries.retain(
                |_, entry| !matches!(entry, Entry::Ready{touched,..} if touched.elapsed() >= IDLE),
            );
            match entries.get_mut(&id) {
                Some(Entry::Ready {
                    stamp: old,
                    value,
                    touched,
                    ..
                }) if *old == stamp => {
                    *touched = Instant::now();
                    return Ok(value.clone());
                }
                Some(Entry::Folding { stamp: old, sender }) => {
                    if *old != stamp || sender.receiver_count() >= MAX_WAITERS {
                        return Err(busy());
                    }
                    sender.subscribe()
                }
                _ => {
                    let permit = self
                        .inner
                        .permits
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| busy())?;
                    let (sender, receiver) = watch::channel(None);
                    entries.insert(
                        id.clone(),
                        Entry::Folding {
                            stamp: stamp.clone(),
                            sender: sender.clone(),
                        },
                    );
                    let inner = self.inner.clone();
                    // This detached task owns the actual blocking job. Dropping an
                    // HTTP waiter never frees the permit or abandons publication.
                    tokio::spawn(async move {
                        let result = tokio::task::spawn_blocking(move || {
                            let _permit = permit;
                            if key.stamp()? != stamp {
                                return Err(busy());
                            }
                            let value = Arc::new(build()?);
                            if key.stamp()? != stamp {
                                return Err(busy());
                            }
                            let bytes = value.charge();
                            Ok((value, bytes, stamp))
                        })
                        .await;
                        let result = match result {
                            Ok(result) => result,
                            Err(e) => Err(Error::Internal(format!("transcript fold task: {e}"))),
                        };
                        let outcome = {
                            let mut entries =
                                inner.entries.lock().unwrap_or_else(|e| e.into_inner());
                            entries.remove(&id);
                            match result {
                                Ok((value, bytes, stamp)) => {
                                    if bytes <= MAX_ENTRY_BYTES {
                                        entries.insert(
                                            id,
                                            Entry::Ready {
                                                stamp,
                                                value: value.clone(),
                                                bytes,
                                                touched: Instant::now(),
                                            },
                                        );
                                        Self::evict(&mut entries);
                                    }
                                    Ok(value)
                                }
                                Err(e) => Err(Arc::<str>::from(e.to_string())),
                            }
                        };
                        let _ = sender.send(Some(outcome));
                    });
                    receiver
                }
            }
        };
        loop {
            if let Some(outcome) = receiver.borrow().clone() {
                return outcome.map_err(|e| {
                    if e.contains("transcript busy") {
                        busy()
                    } else {
                        Error::Internal(e.to_string())
                    }
                });
            }
            receiver
                .changed()
                .await
                .map_err(|_| Error::Internal("transcript fold ended without a result".into()))?;
        }
    }

    fn evict(entries: &mut HashMap<Identity, Entry>) {
        loop {
            let ready: Vec<_> = entries
                .iter()
                .filter_map(|(key, entry)| match entry {
                    Entry::Ready { bytes, touched, .. } => Some((key.clone(), *bytes, *touched)),
                    _ => None,
                })
                .collect();
            if ready.len() <= MAX_ENTRIES
                && ready.iter().map(|(_, bytes, _)| bytes).sum::<usize>() <= MAX_BYTES
            {
                break;
            }
            if let Some((key, _, _)) = ready.into_iter().min_by_key(|(_, _, t)| *t) {
                entries.remove(&key);
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key(dir: &tempfile::TempDir, name: &str) -> CacheKey {
        let path = dir.path().join(name);
        std::fs::write(
            &path,
            "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
        )
        .unwrap();
        CacheKey {
            root: dir.path().into(),
            path,
            provider: Provider::Claude,
            sub: None,
        }
    }
    fn build(
        key: &CacheKey,
        count: Arc<AtomicUsize>,
    ) -> impl FnOnce() -> Result<Snapshot> + Send + 'static {
        let path = key.path.clone();
        move || {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(Snapshot {
                folded: otto_transcript::fold_file(Provider::Claude, &path, Default::default())
                    .unwrap(),
                subagents: vec![],
            })
        }
    }
    #[tokio::test]
    async fn unchanged_pages_reuse_one_real_file_fold() {
        let dir = tempfile::tempdir().unwrap();
        let key = key(&dir, "a.jsonl");
        let count = Arc::new(AtomicUsize::new(0));
        let cache = TranscriptCache::default();
        let a = cache
            .get(key.clone(), build(&key, count.clone()))
            .await
            .unwrap();
        let b = cache
            .get(key.clone(), build(&key, count.clone()))
            .await
            .unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "unchanged pages must not refold the whole file"
        );
        assert!(Arc::ptr_eq(&a, &b));
    }
    #[tokio::test]
    async fn replacement_append_and_sidecar_changes_invalidate() {
        let dir = tempfile::tempdir().unwrap();
        let key = key(&dir, "a.jsonl");
        let count = Arc::new(AtomicUsize::new(0));
        let cache = TranscriptCache::default();
        cache
            .get(key.clone(), build(&key, count.clone()))
            .await
            .unwrap();
        let original = std::fs::read(&key.path).unwrap();
        std::fs::write(dir.path().join("replacement"), &original).unwrap();
        std::fs::rename(dir.path().join("replacement"), &key.path).unwrap();
        cache
            .get(key.clone(), build(&key, count.clone()))
            .await
            .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&key.path)
            .unwrap()
            .write_all(b"{}\n")
            .unwrap();
        cache
            .get(key.clone(), build(&key, count.clone()))
            .await
            .unwrap();
        let side = otto_transcript::subagents_dir(&key.path).unwrap();
        std::fs::create_dir_all(&side).unwrap();
        std::fs::write(side.join("agent-a.meta.json"), "{}").unwrap();
        cache
            .get(key.clone(), build(&key, count.clone()))
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn canceled_waiter_keeps_worker_permit_and_same_key_coalesces() {
        let dir = tempfile::tempdir().unwrap();
        let a = key(&dir, "a.jsonl");
        let b = key(&dir, "b.jsonl");
        let c = key(&dir, "c.jsonl");
        let cache = TranscriptCache::default();
        let count = Arc::new(AtomicUsize::new(0));
        let (started_a, rx_a) = tokio::sync::oneshot::channel();
        let (release_a, wait_a) = std::sync::mpsc::channel();
        let ca = cache.clone();
        let ka = a.clone();
        let fa = build(&a, count.clone());
        let task_a = tokio::spawn(async move {
            ca.get(ka, move || {
                let _ = started_a.send(());
                let _ = wait_a.recv_timeout(Duration::from_secs(5));
                fa()
            })
            .await
        });
        rx_a.await.unwrap();
        let same_cache = cache.clone();
        let same_key = a.clone();
        let same_count = count.clone();
        let same = tokio::spawn(async move {
            same_cache
                .get(same_key.clone(), build(&same_key, same_count))
                .await
        });
        let (started_b, rx_b) = tokio::sync::oneshot::channel();
        let (release_b, wait_b) = std::sync::mpsc::channel();
        let cb = cache.clone();
        let kb = b.clone();
        let fb = build(&b, count.clone());
        let task_b = tokio::spawn(async move {
            cb.get(kb, move || {
                let _ = started_b.send(());
                let _ = wait_b.recv_timeout(Duration::from_secs(5));
                fb()
            })
            .await
        });
        rx_b.await.unwrap();
        task_a.abort();
        let saturated = cache.get(c.clone(), build(&c, count.clone())).await;
        release_a.send(()).unwrap();
        release_b.send(()).unwrap();
        assert!(matches!(saturated, Err(Error::Conflict(_))));
        same.await.unwrap().unwrap();
        task_b.await.unwrap().unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "same-key follower must not start another fold"
        );
        cache
            .get(c.clone(), build(&c, count.clone()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn changes_during_fold_are_never_published_as_current() {
        let dir = tempfile::tempdir().unwrap();
        let key = key(&dir, "a.jsonl");
        let count = Arc::new(AtomicUsize::new(0));
        let cache = TranscriptCache::default();
        let path = key.path.clone();
        let fold = build(&key, count.clone());
        let result = cache
            .get(key.clone(), move || {
                let snapshot = fold()?;
                std::fs::write(path, "{}\n").unwrap();
                Ok(snapshot)
            })
            .await;
        assert!(matches!(result, Err(Error::Conflict(_))));
        cache
            .get(key.clone(), build(&key, count.clone()))
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn least_recent_ready_entries_are_evicted() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranscriptCache::default();
        let count = Arc::new(AtomicUsize::new(0));
        let first = key(&dir, "first.jsonl");
        cache
            .get(first.clone(), build(&first, count.clone()))
            .await
            .unwrap();
        for n in 0..MAX_ENTRIES {
            let key = key(&dir, &format!("{n}.jsonl"));
            cache
                .get(key.clone(), build(&key, count.clone()))
                .await
                .unwrap();
        }
        cache
            .get(first.clone(), build(&first, count.clone()))
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), MAX_ENTRIES + 2);
    }

    #[test]
    fn cache_charge_includes_owned_fold_metadata() {
        let mut folded = otto_transcript::fold(Provider::Claude, &[], Default::default());
        folded.first_prompt = Some("x".repeat(64 * 1024));
        let snapshot = Snapshot {
            folded,
            subagents: vec![],
        };
        assert!(
            snapshot.charge() >= 128 * 1024,
            "owned metadata must count toward the byte budget"
        );
    }
}
