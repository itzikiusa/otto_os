//! Wire framing, checkpoint schema, and bounded sequenced output ring shared
//! between `ottod` and detached `runtime-holder` processes.

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use otto_core::agent_runtime::{RuntimeProvider, TurnItem, TurnOrigin};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Wire protocol version between `ottod` and `runtime-holder`.
pub const PROTO_VERSION: u32 = 1;

/// Required environment guard so `runtime-holder` refuses accidental direct invocation.
pub const HOLDER_ENV: &str = "OTTO_RUNTIME_HOLDER";

/// Stdout prefix emitted once the holder bound its socket and spawned the child CLI.
pub const READY_PREFIX: &str = "OTTO-RUNTIME-HOLDER-READY ";

/// Stdout prefix emitted if startup fails before detachment.
pub const ERROR_PREFIX: &str = "OTTO-RUNTIME-HOLDER-ERROR ";

/// Maximum buffered child output retained across daemon restarts (16 MiB).
pub const MAX_RING_BYTES: usize = 16 * 1024 * 1024;

/// Maximum single frame size over the holder Unix domain socket (32 MiB).
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// How long an unattached holder whose child has already exited retains the
/// final exit status and buffered output before cleaning itself up.
pub const DEFAULT_EXIT_LINGER: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolderChildCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderTurnSnapshot {
    pub id: String,
    pub origin: TurnOrigin,
    #[serde(default)]
    pub interrupting: bool,
    #[serde(default)]
    pub stalled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolderCheckpoint {
    pub session_id: String,
    pub provider: RuntimeProvider,
    #[serde(default)]
    pub acked_seq: u64,
    #[serde(default)]
    pub driver_state: serde_json::Value,
    #[serde(default)]
    pub active_turn_id: Option<String>,
    #[serde(default)]
    pub active_turn: Option<HolderTurnSnapshot>,
    #[serde(default)]
    pub streaming_items: Vec<TurnItem>,
    #[serde(default)]
    pub request_ids_prefix: String,
    #[serde(default)]
    pub claude_login: Option<Option<PathBuf>>,
    #[serde(default)]
    pub fresh_starting: bool,
}

impl HolderCheckpoint {
    /// Serialize checkpoint as an opaque JSON value so the `runtime-holder`
    /// process never fails frame deserialization across `TurnItem` schema skew.
    pub fn to_wire_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Hydrate a [`HolderCheckpoint`] from wire JSON, falling back to per-field
    /// and per-item recovery if a binary upgrade/rollback skewed nested types.
    pub fn from_wire_value(v: serde_json::Value) -> io::Result<Self> {
        if let Ok(cp) = serde_json::from_value::<Self>(v.clone()) {
            return Ok(cp);
        }
        let session_id = v
            .get("session_id")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing session_id in runtime holder checkpoint",
                )
            })?
            .to_string();
        let provider = v
            .get("provider")
            .cloned()
            .and_then(|x| serde_json::from_value::<RuntimeProvider>(x).ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing or unsupported provider in runtime holder checkpoint",
                )
            })?;
        let acked_seq = checkpoint_acked_seq(&v);
        let driver_state = v
            .get("driver_state")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let active_turn_id = v
            .get("active_turn_id")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        let active_turn = v
            .get("active_turn")
            .cloned()
            .and_then(|x| serde_json::from_value::<HolderTurnSnapshot>(x).ok());
        let streaming_items = v
            .get("streaming_items")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| serde_json::from_value::<TurnItem>(item.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        let request_ids_prefix = v
            .get("request_ids_prefix")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        let claude_login = v
            .get("claude_login")
            .cloned()
            .and_then(|x| serde_json::from_value::<Option<Option<PathBuf>>>(x).ok())
            .unwrap_or(None);
        let fresh_starting = v
            .get("fresh_starting")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        Ok(Self {
            session_id,
            provider,
            acked_seq,
            driver_state,
            active_turn_id,
            active_turn,
            streaming_items,
            request_ids_prefix,
            claude_login,
            fresh_starting,
        })
    }
}

pub fn checkpoint_acked_seq(v: &serde_json::Value) -> u64 {
    v.get("acked_seq").and_then(|x| x.as_u64()).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolderSpawnSpec {
    pub command: HolderChildCommand,
    pub initial_checkpoint: serde_json::Value,
    #[serde(default)]
    pub exit_linger_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolderReady {
    pub holder_pid: u32,
    pub child_pid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineStream {
    Out,
    Err,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequencedLine {
    pub seq: u64,
    pub stream: LineStream,
    pub line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlong_bytes: Option<usize>,
}

impl SequencedLine {
    pub fn approx_bytes(&self) -> usize {
        self.line.len().saturating_add(24)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HolderSignal {
    Interrupt,
    Terminate,
    Kill,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DaemonFrame {
    Hello { proto_version: u32 },
    Stdin { line: String },
    CloseStdin,
    Signal { signal: HolderSignal },
    Checkpoint { checkpoint: serde_json::Value },
    Detach,
    Shutdown { grace_ms: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HolderFrame {
    Hello {
        proto_version: u32,
        holder_pid: u32,
        child_pid: u32,
        checkpoint: serde_json::Value,
        exited: Option<HolderExitStatus>,
    },
    ReplayLine {
        entry: SequencedLine,
    },
    ReplayDone,
    Line {
        entry: SequencedLine,
    },
    Exit {
        status: HolderExitStatus,
    },
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderExitStatus {
    pub code: Option<i32>,
    pub stderr_tail: String,
}

/// Bounded FIFO ring of sequenced child lines capped at `max_bytes`.
pub struct LineRing {
    max_bytes: usize,
    used_bytes: usize,
    next_seq: u64,
    entries: VecDeque<SequencedLine>,
}

impl LineRing {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes: max_bytes.max(1024),
            used_bytes: 0,
            next_seq: 1,
            entries: VecDeque::new(),
        }
    }

    pub fn push_line(&mut self, stream: LineStream, line: String) -> SequencedLine {
        self.push_inner(stream, line, None)
    }

    pub fn push_overlong(&mut self, stream: LineStream, dropped_bytes: usize) -> SequencedLine {
        self.push_inner(stream, String::new(), Some(dropped_bytes))
    }

    fn push_inner(
        &mut self,
        stream: LineStream,
        line: String,
        overlong_bytes: Option<usize>,
    ) -> SequencedLine {
        let seq = self.next_seq;
        self.next_seq += 1;
        let entry = SequencedLine {
            seq,
            stream,
            line,
            overlong_bytes,
        };
        let cost = entry.approx_bytes();
        if cost <= self.max_bytes {
            while self.used_bytes + cost > self.max_bytes {
                let Some(evicted) = self.entries.pop_front() else {
                    self.used_bytes = 0;
                    break;
                };
                self.used_bytes = self.used_bytes.saturating_sub(evicted.approx_bytes());
            }
            self.used_bytes += cost;
            self.entries.push_back(entry.clone());
        }
        entry
    }

    /// Evict acknowledged entries strictly up to `acked_seq` so steady-state
    /// memory stays minimal while retaining everything after `acked_seq`.
    pub fn prune_acked(&mut self, acked_seq: u64) {
        while self.entries.front().is_some_and(|e| e.seq <= acked_seq) {
            if let Some(evicted) = self.entries.pop_front() {
                self.used_bytes = self.used_bytes.saturating_sub(evicted.approx_bytes());
            }
        }
    }

    pub fn replay_after(&self, acked_seq: u64) -> Vec<SequencedLine> {
        self.entries
            .iter()
            .filter(|e| e.seq > acked_seq)
            .cloned()
            .collect()
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    frame: &T,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(frame)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime holder frame exceeds limit",
        ));
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await
}

pub async fn read_frame<R: AsyncRead + Unpin, T: DeserializeOwned>(
    reader: &mut BufReader<R>,
    buf: &mut Vec<u8>,
) -> io::Result<Option<T>> {
    buf.clear();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(pos) = chunk.iter().position(|&b| b == b'\n') {
            if buf.len() + pos > MAX_FRAME_BYTES {
                reader.consume(pos + 1);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "runtime holder frame exceeds limit",
                ));
            }
            buf.extend_from_slice(&chunk[..pos]);
            reader.consume(pos + 1);
            break;
        }
        if buf.len() + chunk.len() > MAX_FRAME_BYTES {
            let n = chunk.len();
            reader.consume(n);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime holder frame exceeds limit",
            ));
        }
        let n = chunk.len();
        buf.extend_from_slice(chunk);
        reader.consume(n);
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    serde_json::from_slice(buf)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_ring_enforces_byte_budget_and_replays_strictly_after_acked_seq() {
        let mut ring = LineRing::new(1024);
        let a = ring.push_line(LineStream::Out, "first".into());
        let b = ring.push_line(LineStream::Err, "second".into());
        let c = ring.push_line(LineStream::Out, "third".into());
        assert_eq!((a.seq, b.seq, c.seq), (1, 2, 3));

        let replay = ring.replay_after(1);
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].line, "second");
        assert_eq!(replay[1].line, "third");

        ring.prune_acked(2);
        assert_eq!(ring.replay_after(0).len(), 1);
        assert_eq!(ring.replay_after(0)[0].seq, 3);

        for i in 0..50 {
            ring.push_line(LineStream::Out, format!("line-{i}-{}", "x".repeat(40)));
        }
        assert!(ring.used_bytes <= 1024);
        let tail = ring.replay_after(0);
        assert!(!tail.is_empty());
        assert_eq!(tail.last().unwrap().seq, 53);
    }
}
