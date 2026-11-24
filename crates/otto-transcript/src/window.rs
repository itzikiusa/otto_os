//! Windowed reads of long transcripts. A daily session reaches 60–150 MB and
//! tens of thousands of records; folding all of it to show the last screen
//! is linear in the file (and refused past the whole-file read limits), so a
//! long conversation opens on its RECENT part instead: [`fold_window`] folds
//! only the records of the last few MB before a cursor, and older turns page
//! in by cursor. Record indices stay ABSOLUTE — a window starting at record
//! 18 000 numbers its records from 18 000 — so the paging cursor, turn ids and
//! the live tail's delta cursors mean exactly what they mean for a whole-file
//! fold. [`LineIndex`] maps record index → byte offset without parsing: one
//! streamed newline scan, extended incrementally as the append-only file
//! grows.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::{FoldOpts, Folder, Provider};

/// Files at or below this fold whole (exact cumulative stats); longer ones
/// open on a window.
pub const WHOLE_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Source bytes one window folds. It grows ×4 (up to [`MAX_WINDOW_BYTES`])
/// when the span holds no turn boundary — one giant streamed turn.
pub const WINDOW_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_WINDOW_BYTES: u64 = 64 * 1024 * 1024;
const SCAN_CHUNK: usize = 1 << 20;
const CACHED_INDEXES: usize = 32;

/// True when `path` is long enough to open on a window.
pub fn is_long(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() > WHOLE_FILE_BYTES)
}

/// Byte offset of every record (complete, non-blank line) of one file:
/// record `i` starts at `starts[i]`, matching [`crate::for_each_record`]'s
/// numbering (a line of ASCII whitespace is no record).
#[derive(Clone, Default, Debug)]
pub struct LineIndex {
    pub starts: Vec<u64>,
    /// Offset just past the last complete line scanned.
    pub end: u64,
    file: Option<(u64, u64)>,
}

impl LineIndex {
    /// Scan what was appended since the last call (everything the first
    /// time). A shorter or replaced file is scanned again from 0.
    pub fn update(&mut self, path: &Path) -> std::io::Result<()> {
        let mut f = std::fs::File::open(path)?;
        let meta = f.metadata()?;
        let file = file_id(&meta);
        if meta.len() < self.end || file != self.file {
            *self = Self {
                file,
                ..Self::default()
            };
        }
        f.seek(SeekFrom::Start(self.end))?;
        let mut reader = f.take(meta.len() - self.end);
        let mut buf = vec![0u8; SCAN_CHUNK];
        let (mut at, mut line, mut blank) = (self.end, self.end, true);
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            let mut rest = &buf[..n];
            while let Some(nl) = rest.iter().position(|b| *b == b'\n') {
                blank = blank && rest[..nl].iter().all(u8::is_ascii_whitespace);
                if !blank {
                    self.starts.push(line);
                }
                at += nl as u64 + 1;
                (line, blank, self.end) = (at, true, at);
                rest = &rest[nl + 1..];
            }
            blank = blank && rest.iter().all(u8::is_ascii_whitespace);
            at += rest.len() as u64;
        }
    }
}

#[cfg(unix)]
fn file_id(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}
#[cfg(not(unix))]
fn file_id(_: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// The line index of `path`, brought up to date (cached per path, so a
/// re-open scans only what was appended since). Readers of one path take
/// turns on its slot: a concurrent read waits for the tail scan in flight
/// instead of finding the entry gone and rescanning the file from byte 0.
pub fn line_index(path: &Path) -> std::io::Result<Arc<LineIndex>> {
    type Slot = Arc<Mutex<Arc<LineIndex>>>;
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Slot>>> = OnceLock::new();
    let slot = {
        let mut map = CACHE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if map.len() >= CACHED_INDEXES && !map.contains_key(path) {
            if let Some(old) = map.keys().next().cloned() {
                map.remove(&old);
            }
        }
        map.entry(path.to_path_buf()).or_default().clone()
    };
    let mut index = slot.lock().unwrap_or_else(|p| p.into_inner());
    Arc::make_mut(&mut index).update(path)?;
    Ok(index.clone())
}

/// The newest window of `path` for a live tail: the whole file when it is
/// short, else the last [`WINDOW_BYTES`].
pub fn fold_recent<'a>(
    provider: Provider,
    path: &Path,
    opts: FoldOpts<'a>,
) -> std::io::Result<Window<'a>> {
    let span = if is_long(path) {
        WINDOW_BYTES
    } else {
        WHOLE_FILE_BYTES
    };
    fold_window(provider, path, opts, None, span)
}

/// The first value `found` yields walking `path`'s windows from the newest
/// back to the file's start, one window in memory at a time: a lookup of
/// something an older page showed (an artifact opened after "Load earlier").
pub fn find_back<T>(
    provider: Provider,
    path: &Path,
    opts: FoldOpts<'_>,
    mut found: impl FnMut(crate::Folded) -> Option<T>,
) -> std::io::Result<Option<T>> {
    let mut before = None;
    loop {
        let w = fold_window(provider, path, opts.clone(), before, WINDOW_BYTES)?;
        let (first, base) = (w.first, w.base);
        if let Some(t) = found(w.folder.into_folded()) {
            return Ok(Some(t));
        }
        if first == 0 {
            return Ok(None);
        }
        before = Some(base);
    }
}

/// A fold of the newest records before a cursor.
pub struct Window<'a> {
    pub folder: Folder<'a>,
    /// First record folded and its byte offset; the span ends at `end` (the
    /// offset of the cursor's record, or past the last complete line).
    pub first: usize,
    pub start: u64,
    pub end: u64,
    /// First record of the first WHOLE turn: a span that starts mid-file may
    /// cut its first turn, so pages start here and the cut turn opens whole
    /// on the page before (0 when the window reaches the file's start).
    pub base: usize,
}

/// Fold the records before `before` (the file's end when `None`) that fit
/// in `budget` source bytes, numbering them absolutely. Only that span is
/// read and parsed (Codex prescans the same span for its era flags). A span
/// with no turn boundary grows ×4 up to [`MAX_WINDOW_BYTES`]; a single record
/// above that is the one case still refused (`OutOfMemory`).
pub fn fold_window<'a>(
    provider: Provider,
    path: &Path,
    opts: FoldOpts<'a>,
    before: Option<usize>,
    mut budget: u64,
) -> std::io::Result<Window<'a>> {
    let index = line_index(path)?;
    let stop = before.unwrap_or(usize::MAX).min(index.starts.len());
    let end = index.starts.get(stop).copied().unwrap_or(index.end);
    loop {
        let first = index.starts[..stop]
            .partition_point(|s| s + budget < end)
            .min(stop.saturating_sub(1));
        let start = index.starts.get(first).copied().unwrap_or(end);
        if end - start > budget.max(MAX_WINDOW_BYTES) {
            return Err(crate::limit_exceeded());
        }
        let mut bytes = Vec::with_capacity((end - start) as usize);
        let mut f = std::fs::File::open(path)?;
        f.seek(SeekFrom::Start(start))?;
        f.take(end - start).read_to_end(&mut bytes)?;
        let mut folder = Folder::new(provider, opts.clone());
        folder.start_at(first);
        folder.seed_bytes(&bytes);
        let second = folder.turn_first(1);
        let base = match second {
            _ if first == 0 => 0,
            Some(b) => b,
            None if budget < MAX_WINDOW_BYTES => {
                budget = (budget * 4).min(MAX_WINDOW_BYTES);
                continue;
            }
            None => first,
        };
        folder.set_base(base);
        return Ok(Window {
            folder,
            first,
            start,
            end,
            base,
        });
    }
}
