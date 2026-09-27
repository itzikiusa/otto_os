//! Append-only owner archives. Caller-controlled identifiers never become paths.
use otto_core::{api::*, Error, Result};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub const QUOTA: u64 = 512 * 1024 * 1024;
pub struct Store {
    pub creation: tokio::sync::Mutex<()>,
    root: PathBuf,
    archives: Mutex<HashMap<String, Arc<Archive>>>,
    pub summary: Arc<tokio::sync::Semaphore>,
    pub queued: Arc<tokio::sync::Semaphore>,
    pub inference: Arc<tokio::sync::Semaphore>,
}
pub struct Archive {
    pub control: std::sync::atomic::AtomicU64,
    pub path: PathBuf,
    pub state: Mutex<ArchiveState>,
}
pub struct ArchiveState {
    indexed: bool,
    recovery_gap: Option<RecapEvent>,
    offsets: Vec<(u64, u64)>,
    journal_bytes: u64,
    persisted_at: std::time::Instant,
    pub metadata: RecapMetadata,
    pub summary_task: Option<tokio::task::AbortHandle>,
    pub summary_generation: u64,
}
fn disk(e: impl std::fmt::Display) -> Error {
    Error::Internal(format!("Recap archive: {e}"))
}
pub fn valid_id(id: &str) -> Result<()> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(Error::Invalid("Invalid archive identifier".into()));
    }
    Ok(())
}
fn regular(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path)
        .map_err(disk)?
        .file_type()
        .is_symlink()
    {
        return Err(Error::Forbidden(
            "Archive symlinks are not supported".into(),
        ));
    }
    Ok(())
}
impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        let existed = root.exists();
        fs::create_dir_all(&root).map_err(disk)?;
        if !existed {
            private_dir(&root)?;
        }
        regular(&root)?;
        let store = Self {
            creation: tokio::sync::Mutex::new(()),
            root,
            archives: Mutex::new(HashMap::new()),
            summary: Arc::new(tokio::sync::Semaphore::new(1)),
            queued: Arc::new(tokio::sync::Semaphore::new(8)),
            inference: Arc::new(tokio::sync::Semaphore::new(1)),
        };
        for entry in fs::read_dir(&store.root).map_err(disk)?.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            if valid_id(&id).is_err() {
                continue;
            }
            if let Ok(a) = store.load(&id) {
                let mut s = a.state.lock().unwrap();
                if s.metadata.status != RecapState::Stopped {
                    s.metadata.status = RecapState::Stopped;
                    s.metadata.capture_epoch += 1;
                }
                if matches!(
                    s.metadata.summary_status,
                    RecapSummaryStatus::Running | RecapSummaryStatus::Queued
                ) {
                    s.metadata.summary_status = RecapSummaryStatus::Error;
                    s.metadata.summary_error =
                        Some("Daemon restarted; generate the draft again".into());
                }
                a.set_capture(s.metadata.capture_epoch, false);
                a.save(&s)?;
            }
        }
        Ok(store)
    }
    fn load(&self, id: &str) -> Result<Arc<Archive>> {
        valid_id(id)?;
        let mut all = self.archives.lock().unwrap();
        if let Some(a) = all.get(id) {
            regular(&self.root)?;
            regular(&a.path)?;
            return Ok(a.clone());
        }
        let path = self.root.join(id);
        regular(&path)?;
        regular(&path.join("metadata.json"))?;
        let metadata: RecapMetadata =
            serde_json::from_slice(&fs::read(path.join("metadata.json")).map_err(disk)?)
                .map_err(disk)?;
        if metadata.id != id {
            return Err(Error::Unauthorized);
        }
        let a = Arc::new(Archive {
            control: std::sync::atomic::AtomicU64::new(
                (metadata.capture_epoch << 1) | u64::from(metadata.status == RecapState::Capturing),
            ),
            path,
            state: Mutex::new(ArchiveState {
                indexed: false,
                recovery_gap: None,
                offsets: vec![],
                journal_bytes: 0,
                persisted_at: std::time::Instant::now(),
                metadata,
                summary_task: None,
                summary_generation: 0,
            }),
        });
        all.insert(id.into(), a.clone());
        Ok(a)
    }
    pub fn get(&self, id: &str, owner: &str) -> Result<Arc<Archive>> {
        let a = self.load(id)?;
        if a.state.lock().unwrap().metadata.owner_id != owner {
            return Err(Error::Forbidden(
                "This recap belongs to another owner".into(),
            ));
        }
        a.ensure_index()?;
        Ok(a)
    }
    pub fn get_summary(&self, id: &str, owner: &str) -> Result<Arc<Archive>> {
        let a = self.load(id)?;
        {
            let s = a.state.lock().unwrap();
            if s.metadata.owner_id != owner {
                return Err(Error::Forbidden(
                    "This recap belongs to another owner".into(),
                ));
            }
            if !s.indexed && fs::metadata(a.events_path()?).map_err(disk)?.len() > 3 * 1024 * 1024 {
                return Err(Error::Invalid("This archive exceeds the 3 MiB summary input limit. Export the full timeline and summarize selected sections.".into()));
            }
        }
        a.ensure_index()?;
        Ok(a)
    }
    pub fn list(&self, owner: &str) -> Vec<RecapMetadata> {
        let mut result: Vec<_> = self
            .archives
            .lock()
            .unwrap()
            .values()
            .filter_map(|a| {
                let m = a.state.lock().unwrap().metadata.clone();
                (m.owner_id == owner).then_some(m)
            })
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        result
    }
    pub fn create(&self, metadata: RecapMetadata) -> Result<Arc<Archive>> {
        let path = self.root.join(&metadata.id);
        fs::create_dir(&path).map_err(disk)?;
        private_dir(&path)?;
        fs::create_dir(path.join("images")).map_err(disk)?;
        private_dir(&path.join("images"))?;
        let a = Arc::new(Archive {
            control: std::sync::atomic::AtomicU64::new(
                (metadata.capture_epoch << 1) | u64::from(metadata.status == RecapState::Capturing),
            ),
            path,
            state: Mutex::new(ArchiveState {
                indexed: true,
                recovery_gap: None,
                offsets: vec![],
                journal_bytes: 0,
                persisted_at: std::time::Instant::now(),
                metadata,
                summary_task: None,
                summary_generation: 0,
            }),
        });
        a.save(&a.state.lock().unwrap())?;
        private_options()
            .create_new(true)
            .write(true)
            .open(a.path.join("events.jsonl"))
            .map_err(disk)?;
        self.archives
            .lock()
            .unwrap()
            .insert(a.state.lock().unwrap().metadata.id.clone(), a.clone());
        Ok(a)
    }
}
impl Archive {
    pub fn ensure_index(&self) -> Result<()> {
        let mut s = self.state.lock().unwrap();
        if s.indexed {
            return Ok(());
        }
        regular(&self.path.join("events.jsonl"))?;
        let file = fs::File::open(self.path.join("events.jsonl")).map_err(disk)?;
        let total = file.metadata().map_err(disk)?.len();
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let mut last = 0;
        let mut offset = 0;
        let mut offsets = vec![];
        loop {
            line.clear();
            let count = reader.read_until(b'\n', &mut line).map_err(disk)?;
            if count == 0 {
                break;
            }
            let event = serde_json::from_slice::<RecapEvent>(&line);
            let Ok(event) = event else {
                break;
            };
            if event.seq != last + 1 || line.last() != Some(&b'\n') {
                break;
            }
            if (event.seq - 1).is_multiple_of(128) {
                offsets.push((event.seq, offset));
            }
            last = event.seq;
            offset += count as u64;
        }
        s.offsets = offsets;
        s.journal_bytes = offset;
        s.metadata.last_seq = last;
        s.metadata.bytes_used = total;
        if offset < total {
            s.recovery_gap=Some(RecapEvent{seq:last+1,created_at:s.metadata.updated_at.clone(),capture_epoch:s.metadata.capture_epoch,payload:RecapEventData::Gap{kind:"archive_recovery".into(),member_id:None,source_id:None,reason:format!("Interrupted journal suffix ({} bytes) retained on disk; complete prior events recovered",total-offset)}});
            s.metadata.last_seq += 1;
        }
        regular(&self.path.join("images"))?;
        for entry in fs::read_dir(self.path.join("images"))
            .map_err(disk)?
            .flatten()
        {
            regular(&entry.path())?;
            s.metadata.bytes_used += entry.metadata().map_err(disk)?.len();
        }
        if self.path.join("draft.json").exists() {
            regular(&self.path.join("draft.json"))?;
            s.metadata.bytes_used += fs::metadata(self.path.join("draft.json"))
                .map_err(disk)?
                .len();
        }
        s.indexed = true;
        self.save(&s)
    }
    pub fn export_extent(&self) -> Result<(u64, Vec<u8>)> {
        let s = self.state.lock().unwrap();
        let mut tail = if let Some(gap) = &s.recovery_gap {
            serde_json::to_vec(gap).map_err(disk)?
        } else {
            vec![]
        };
        if !tail.is_empty() {
            tail.push(b'\n');
        }
        Ok((s.journal_bytes, tail))
    }
    pub fn accepts(&self, epoch: u64) -> bool {
        self.control.load(std::sync::atomic::Ordering::Acquire) == (epoch << 1) | 1
    }
    pub fn set_capture(&self, epoch: u64, active: bool) {
        self.control.store(
            (epoch << 1) | u64::from(active),
            std::sync::atomic::Ordering::Release,
        );
    }
    pub fn events_path(&self) -> Result<PathBuf> {
        let path = self.path.join("events.jsonl");
        regular(&path)?;
        Ok(path)
    }
    pub fn save(&self, s: &ArchiveState) -> Result<()> {
        atomic(
            &self.path.join("metadata.json"),
            &serde_json::to_vec(&s.metadata).map_err(disk)?,
        )
    }
    pub fn metadata(&self) -> RecapMetadata {
        self.state.lock().unwrap().metadata.clone()
    }
    pub fn append(&self, epoch: u64, payload: RecapEventData) -> Result<()> {
        self.append_if(epoch, payload, || true).map(|_| ())
    }
    pub fn append_if(
        &self,
        epoch: u64,
        payload: RecapEventData,
        valid: impl Fn() -> bool,
    ) -> Result<bool> {
        let mut s = self.state.lock().unwrap();
        if !valid() || (!matches!(payload, RecapEventData::Capture { .. }) && !self.accepts(epoch))
        {
            return Ok(false);
        }
        if let RecapEventData::Capture { state, .. } = &payload {
            s.metadata.status = *state;
            s.metadata.capture_epoch = epoch;
        }
        let event = RecapEvent {
            seq: s.metadata.last_seq + 1,
            created_at: chrono::Utc::now().to_rfc3339(),
            capture_epoch: epoch,
            payload,
        };
        let mut bytes = serde_json::to_vec(&event).map_err(disk)?;
        bytes.push(b'\n');
        let reserve = if matches!(event.payload, RecapEventData::Capture { .. }) {
            0
        } else {
            8192
        };
        if s.metadata.bytes_used + bytes.len() as u64 + reserve > s.metadata.quota_bytes {
            return Err(Error::Conflict(
                "Recap reached its 512 MiB limit; capture stopped".into(),
            ));
        }
        regular(&self.path.join("events.jsonl"))?;
        OpenOptions::new()
            .append(true)
            .open(self.path.join("events.jsonl"))
            .map_err(disk)?
            .write_all(&bytes)
            .map_err(disk)?;
        if (event.seq - 1).is_multiple_of(128) {
            let offset = s.journal_bytes;
            s.offsets.push((event.seq, offset));
        }
        s.journal_bytes += bytes.len() as u64;
        s.metadata.last_seq = event.seq;
        s.metadata.bytes_used += bytes.len() as u64;
        s.metadata.updated_at = event.created_at;
        if s.metadata.last_seq.is_multiple_of(64)
            || s.persisted_at.elapsed() >= std::time::Duration::from_secs(1)
            || matches!(event.payload, RecapEventData::Capture { .. })
        {
            self.save(&s)?;
            s.persisted_at = std::time::Instant::now();
        }
        Ok(true)
    }
    pub fn transition(&self, view: &RecapCapture) -> Result<()> {
        {
            let mut s = self.state.lock().unwrap();
            s.metadata.status = view.state;
            s.metadata.capture_epoch = view.epoch;
            s.metadata.updated_at = chrono::Utc::now().to_rfc3339();
            self.save(&s)?;
        }
        let _ = self.append(
            view.epoch,
            RecapEventData::Capture {
                state: view.state,
                reason: view.reason.clone(),
                started_at: view.started_at.clone(),
            },
        );
        Ok(())
    }
    pub fn detail(&self, after: u64, limit: usize) -> Result<RecapDetail> {
        let (metadata, offset, extent, recovery_gap) = {
            let s = self.state.lock().unwrap();
            let ix = s
                .offsets
                .partition_point(|(seq, _)| *seq <= after.saturating_add(1));
            (
                s.metadata.clone(),
                ix.checked_sub(1).map(|ix| s.offsets[ix].1).unwrap_or(0),
                s.journal_bytes,
                s.recovery_gap.clone(),
            )
        };
        regular(&self.path.join("events.jsonl"))?;
        let mut file = fs::File::open(self.path.join("events.jsonl")).map_err(disk)?;
        file.seek(SeekFrom::Start(offset)).map_err(disk)?;
        let mut events = vec![];
        for line in BufReader::new(file.take(extent.saturating_sub(offset))).lines() {
            let line = line.map_err(disk)?;
            let event: RecapEvent = serde_json::from_str(&line).map_err(disk)?;
            if event.seq > after {
                events.push(event);
                if events.len() >= limit.clamp(1, 1000) {
                    break;
                }
            }
        }
        if events.len() < limit.clamp(1, 1000) {
            if let Some(gap) = recovery_gap.filter(|g| g.seq > after) {
                events.push(gap);
            }
        }
        let next_cursor = events
            .last()
            .filter(|e| e.seq < metadata.last_seq)
            .map(|e| e.seq);
        let draft = if self.path.join("draft.json").exists() {
            regular(&self.path.join("draft.json"))?;
            Some(
                serde_json::from_slice(&fs::read(self.path.join("draft.json")).map_err(disk)?)
                    .map_err(disk)?,
            )
        } else {
            None
        };
        Ok(RecapDetail {
            metadata,
            events,
            next_cursor,
            draft,
        })
    }
    pub fn summary_events(&self) -> Result<Vec<serde_json::Value>> {
        let path = self.path.join("events.jsonl");
        regular(&path)?;
        let (extent, tail) = self.export_extent()?;
        if extent + tail.len() as u64 > 3 * 1024 * 1024 {
            return Err(Error::Invalid("This archive exceeds the 3 MiB summary input limit. Export the full timeline and summarize selected sections.".into()));
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(disk)?
            .take(extent.min(3 * 1024 * 1024 + 1))
            .read_to_end(&mut bytes)
            .map_err(disk)?;
        bytes.extend(tail);
        if bytes.len() > 3 * 1024 * 1024 {
            return Err(Error::Invalid(
                "Archive exceeds summary limit; export selected sections".into(),
            ));
        }
        bytes
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_slice(l).map_err(disk))
            .collect()
    }
    pub fn representative_images(&self, events: &[serde_json::Value]) -> Result<Vec<PathBuf>> {
        regular(&self.path.join("images"))?;
        let mut paths = vec![];
        for event in events {
            let p = &event["payload"];
            if p["type"] == "screen" {
                if let Some(id) = p["image_id"].as_str() {
                    valid_id(id)?;
                    paths.push(self.path.join("images").join(format!("{id}.jpg")));
                }
            }
        }
        for p in &paths {
            regular(p)?;
        }
        Ok(paths)
    }
    pub fn image(&self, id: &str) -> Result<Vec<u8>> {
        valid_id(id)?;
        regular(&self.path.join("images"))?;
        let path = self.path.join("images").join(format!("{id}.jpg"));
        regular(&path)?;
        fs::read(path).map_err(disk)
    }
    pub fn put_image(&self, bytes: &[u8], valid: impl Fn() -> bool) -> Result<Option<String>> {
        let mut s = self.state.lock().unwrap();
        if !valid() {
            return Ok(None);
        }
        if s.metadata.bytes_used + bytes.len() as u64 + 8192 > s.metadata.quota_bytes {
            return Err(Error::Conflict("Recap storage limit reached".into()));
        }
        regular(&self.path.join("images"))?;
        let id = uuid::Uuid::new_v4().to_string();
        atomic(&self.path.join("images").join(format!("{id}.jpg")), bytes)?;
        s.metadata.bytes_used += bytes.len() as u64;
        self.save(&s)?;
        Ok(Some(id))
    }
    pub fn discard_image(&self, id: &str, size: u64) -> Result<()> {
        valid_id(id)?;
        let mut s = self.state.lock().unwrap();
        fs::remove_file(self.path.join("images").join(format!("{id}.jpg"))).map_err(disk)?;
        s.metadata.bytes_used = s.metadata.bytes_used.saturating_sub(size);
        self.save(&s)
    }
    pub fn draft(&self, draft: &RecapDraft, s: &mut ArchiveState) -> Result<()> {
        let bytes = serde_json::to_vec(draft).map_err(disk)?;
        let path = self.path.join("draft.json");
        let previous = if path.exists() {
            regular(&path)?;
            fs::metadata(&path).map_err(disk)?.len()
        } else {
            0
        };
        let total = s.metadata.bytes_used.saturating_sub(previous) + bytes.len() as u64;
        if total + 8192 > s.metadata.quota_bytes {
            return Err(Error::Conflict(
                "Recap quota cannot fit the draft; export the transcript".into(),
            ));
        }
        atomic(&path, &bytes)?;
        s.metadata.bytes_used = total;
        Ok(())
    }
}
fn private_options() -> OpenOptions {
    let mut opts = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts
}
fn private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(disk)?;
    }
    Ok(())
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut f = private_options()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(disk)?;
    f.write_all(bytes).map_err(disk)?;
    f.sync_data().map_err(disk)?;
    fs::rename(temp, path).map_err(disk)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn metadata(owner: &str) -> RecapMetadata {
        let now = chrono::Utc::now().to_rfc3339();
        RecapMetadata {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: owner.into(),
            room_id: uuid::Uuid::new_v4().to_string(),
            session_id: "session".into(),
            session_title: "Session".into(),
            created_at: now.clone(),
            updated_at: now,
            status: RecapState::Capturing,
            capture_epoch: 2,
            bytes_used: 0,
            quota_bytes: QUOTA,
            last_seq: 0,
            speech_available: false,
            speech_error: Some("Unavailable".into()),
            summary_status: RecapSummaryStatus::Idle,
            summary_error: None,
            summary_through_seq: None,
        }
    }
    fn event() -> RecapEventData {
        RecapEventData::Gap {
            kind: "speech".into(),
            member_id: None,
            source_id: None,
            reason: "Recognizer unavailable".into(),
        }
    }
    #[test]
    fn owner_isolation_restart_and_pagination() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("archives")).unwrap();
        let a = store.create(metadata("alice")).unwrap();
        let id = a.metadata().id;
        a.append(2, event()).unwrap();
        a.append(2, event()).unwrap();
        assert!(store.get(&id, "bob").is_err());
        assert!(store.get("../../etc/passwd", "alice").is_err());
        let page = a.detail(0, 1).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.next_cursor, Some(1));
        drop(store);
        let reopened = Store::open(dir.path().join("archives")).unwrap();
        let a = reopened.get(&id, "alice").unwrap();
        assert_eq!(a.metadata().status, RecapState::Stopped);
        assert_eq!(a.metadata().capture_epoch, 3);
        assert_eq!(a.detail(1, 1).unwrap().events[0].seq, 2);
    }
    #[test]
    fn quota_rejects_before_writing_and_summary_checks_size() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("archives")).unwrap();
        let mut m = metadata("alice");
        m.quota_bytes = 1;
        let a = store.create(m).unwrap();
        assert!(a.append(2, event()).is_err());
        assert_eq!(fs::metadata(a.path.join("events.jsonl")).unwrap().len(), 0);
        let f = OpenOptions::new()
            .write(true)
            .open(a.path.join("events.jsonl"))
            .unwrap();
        f.set_len(3 * 1024 * 1024 + 1).unwrap();
        a.state.lock().unwrap().journal_bytes = 3 * 1024 * 1024 + 1;
        assert!(a
            .summary_events()
            .unwrap_err()
            .to_string()
            .contains("Export"));
    }
    #[test]
    fn interrupted_suffix_preserves_original_bytes_and_recovers_prefix_lazily() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("archives");
        let store = Store::open(root.clone()).unwrap();
        let a = store.create(metadata("alice")).unwrap();
        a.append(2, event()).unwrap();
        let id = a.metadata().id;
        let path = a.path.join("events.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"seq\":2")
            .unwrap();
        let original = fs::read(&path).unwrap();
        drop(store);
        let reopened = Store::open(root).unwrap();
        assert_eq!(reopened.list("alice").len(), 1);
        assert!(
            !reopened.archives.lock().unwrap()[&id]
                .state
                .lock()
                .unwrap()
                .indexed
        );
        let a = reopened.get(&id, "alice").unwrap();
        let detail = a.detail(0, 10).unwrap();
        assert_eq!(detail.events.len(), 2);
        assert!(
            matches!(&detail.events[1].payload,RecapEventData::Gap{kind,..} if kind=="archive_recovery")
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(a.summary_events().unwrap().len(), 2);
        let (extent, tail) = a.export_extent().unwrap();
        assert!(extent < original.len() as u64);
        assert!(String::from_utf8(tail)
            .unwrap()
            .contains("archive_recovery"));
    }
    #[test]
    fn sparse_index_and_journal_recovery_preserve_tail() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("archives");
        let store = Store::open(root.clone()).unwrap();
        let a = store.create(metadata("alice")).unwrap();
        for _ in 0..300 {
            a.append(2, event()).unwrap();
        }
        let id = a.metadata().id;
        assert_eq!(
            a.detail(257, 2)
                .unwrap()
                .events
                .iter()
                .map(|e| e.seq)
                .collect::<Vec<_>>(),
            vec![258, 259]
        );
        assert_eq!(a.state.lock().unwrap().offsets.len(), 3);
        drop(store);
        let reopened = Store::open(root).unwrap();
        let a = reopened.get(&id, "alice").unwrap();
        assert_eq!(a.metadata().last_seq, 300);
        assert_eq!(a.detail(299, 2).unwrap().events[0].seq, 300);
        assert!(!a.accepts(2));
    }
    #[test]
    fn revoked_source_cannot_write_image_after_waiting_for_archive_lock() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("archives")).unwrap();
        let a = store.create(metadata("alice")).unwrap();
        let source = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let guard = a.state.lock().unwrap();
        let worker = a.clone();
        let token = source.clone();
        let thread = std::thread::spawn(move || {
            worker.put_image(b"image", || {
                worker.accepts(2) && token.load(std::sync::atomic::Ordering::Acquire) == 1
            })
        });
        source.store(2, std::sync::atomic::Ordering::Release);
        drop(guard);
        assert!(thread.join().unwrap().unwrap().is_none());
        assert_eq!(fs::read_dir(a.path.join("images")).unwrap().count(), 0);
    }
    #[test]
    fn revocation_fences_queued_events_without_waiting_for_writer() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("archives")).unwrap();
        let a = store.create(metadata("alice")).unwrap();
        let guard = a.state.lock().unwrap();
        a.set_capture(3, false);
        drop(guard);
        a.append(2, event()).unwrap();
        assert_eq!(a.metadata().last_seq, 0);
        a.set_capture(4, true);
        a.append(2, event()).unwrap();
        assert_eq!(a.metadata().last_seq, 0);
        a.append(4, event()).unwrap();
        assert_eq!(a.metadata().last_seq, 1);
    }
    #[cfg(unix)]
    #[test]
    fn rejects_archive_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("archives");
        let store = Store::open(root.clone()).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        std::os::unix::fs::symlink(dir.path(), root.join(&id)).unwrap();
        assert!(store.get(&id, "alice").is_err());
    }
}
