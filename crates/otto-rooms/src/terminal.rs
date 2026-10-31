//! Room terminal flow state: each viewer's resync queue, pause gate and
//! wire frames. The socket that applies them (and the credit gate shared with
//! `/ws/term`) lives in otto-server's `rooms/terminal.rs`.
use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::Bytes;
use otto_core::{Error, Result};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::broadcast;

pub const FLOW_AUTO_RESUME: Duration = Duration::from_secs(2);
pub const RESYNC_INTERVAL: Duration = Duration::from_millis(500);
/// Credit `ack` budget. A viewer acks every 64 KB it parses — ~100/s while
/// it drains a flood at WebKit's 5–8 MB/s — which would trip the 60/s
/// general frame limit and drop the socket. An ack only moves this viewer's
/// own window, so it gets its own budget and skips the room lock.
pub const ACK_RATE: f64 = 400.0;
pub const ACK_BURST: f64 = 800.0;

/// Recovery cannot be rejected after the client discards its renderer backlog.
/// Keep one pending request, coalescing bursts without delaying its deadline.
#[derive(Default)]
pub struct ResyncQueue {
    pub pending: Option<(usize, tokio::time::Instant)>,
    next_allowed: Option<tokio::time::Instant>,
}
impl ResyncQueue {
    pub fn request(&mut self, lines: usize, now: tokio::time::Instant) {
        let due = self.next_allowed.unwrap_or(now).max(now);
        self.pending = Some((lines.min(10_000), self.pending.map_or(due, |(_, at)| at)));
    }
    pub fn deadline(&self) -> Option<tokio::time::Instant> {
        self.pending.map(|(_, at)| at)
    }
    pub fn take_ready(&mut self, now: tokio::time::Instant) -> Option<usize> {
        let (lines, at) = self.pending?;
        if now < at {
            return None;
        }
        self.pending = None;
        self.next_allowed = Some(now + RESYNC_INTERVAL);
        Some(lines)
    }
}

/// Output flow belongs to this viewer, independently of the room driver.
#[derive(Default)]
pub struct FlowGate {
    pub paused_until: Option<tokio::time::Instant>,
}
impl FlowGate {
    pub fn pause(&mut self) {
        self.paused_until = Some(tokio::time::Instant::now() + FLOW_AUTO_RESUME);
    }
    pub fn resume(&mut self, output: &mut broadcast::Receiver<Bytes>) -> bool {
        // Inspect a fixed backlog count, never chase a continuously producing
        // PTY with a drain loop. The snapshot replaces this receiver atomically.
        self.paused_until.take().is_some() && !output.is_empty()
    }
}
/// Sleep until `deadline`, or forever when there is none (a `select!` arm).
pub async fn flow_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    Input {
        data: String,
        grant_epoch: u64,
    },
    Resize {
        cols: u16,
        rows: u16,
        grant_epoch: u64,
    },
    Scrollback {
        #[serde(default)]
        lines: Option<usize>,
    },
    Snapshot,
    Resync {
        #[serde(default)]
        lines: Option<usize>,
    },
    Pause,
    Resume,
    /// Credit flow control, same contract as `/ws/term` (docs/contracts/
    /// ws.md): at most `window` unacknowledged binary bytes to this viewer.
    /// Like pause/resume it needs no driver grant.
    Credit {
        #[serde(default)]
        window: u64,
    },
    /// Cumulative credited bytes this viewer parsed or dropped.
    Ack {
        bytes: u64,
    },
    /// Find-bar search over the session's ring-buffer scrollback, same reply
    /// as `/ws/term` (`search_result`). Read-only: any admitted member.
    Search {
        query: String,
    },
}
/// Longest find query the relay will scan for.
pub const MAX_SEARCH_QUERY: usize = 256;
/// Matches returned per search (same cap as `/ws/term`).
pub const MAX_SEARCH_RESULTS: usize = 200;
/// The `search_result` frame the terminal find bar consumes.
pub fn search_result_frame(query: &str, matches: Vec<(usize, String)>) -> String {
    let matches: Vec<serde_json::Value> = matches
        .into_iter()
        .map(|(line, text)| serde_json::json!({ "line": line, "text": text }))
        .collect();
    serde_json::json!({ "type": "search_result", "query": query, "matches": matches }).to_string()
}
/// A driver's `input` frame: base64, at most 32 KiB decoded.
pub fn decode_input(data: &str) -> Result<Vec<u8>> {
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| Error::Invalid("Input must use base64 encoding".into()))?;
    if bytes.len() > 32768 {
        return Err(Error::Invalid("Input exceeds 32 KiB".into()));
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_decodes_the_existing_terminal_wire_format() {
        assert_eq!(decode_input("aGVsbG8NCg==").unwrap(), b"hello\r\n");
        assert!(decode_input("%invalid").is_err());
    }

    #[test]
    fn search_frame_parses_and_replies_in_the_find_bar_shape() {
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"search","query":"err"}"#),
            Ok(Frame::Search { query }) if query == "err"
        ));
        let reply: serde_json::Value =
            serde_json::from_str(&search_result_frame("a\tb", vec![(3, "x\u{1b}y".into())]))
                .unwrap();
        assert_eq!(reply["type"], "search_result");
        assert_eq!(reply["query"], "a\tb");
        assert_eq!(reply["matches"][0]["line"], 3);
    }

    #[test]
    fn terminal_flow_frames_need_no_driver_epoch() {
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"pause"}"#),
            Ok(Frame::Pause)
        ));
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"resume"}"#),
            Ok(Frame::Resume)
        ));
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"resync","lines":1000}"#),
            Ok(Frame::Resync { lines: Some(1000) })
        ));
    }

    /// r3-10-06: room viewers use the same credit window as `/ws/term`.
    #[test]
    fn terminal_credit_frames_parse_without_a_driver_epoch() {
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"credit","window":1048576}"#),
            Ok(Frame::Credit { window: 1_048_576 })
        ));
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"credit"}"#),
            Ok(Frame::Credit { window: 0 })
        ));
        assert!(matches!(
            serde_json::from_str::<Frame>(r#"{"type":"ack","bytes":65536}"#),
            Ok(Frame::Ack { bytes: 65_536 })
        ));
        assert!(
            serde_json::from_str::<Frame>(r#"{"type":"ack","bytes":1,"grant_epoch":3}"#).is_err(),
            "still deny_unknown_fields"
        );
    }

    #[test]
    fn resync_bursts_coalesce_without_starving_recovery() {
        let now = tokio::time::Instant::now();
        let mut recovery = ResyncQueue::default();
        recovery.request(100, now);
        assert_eq!(
            recovery.take_ready(now),
            Some(100),
            "first recovery is immediate"
        );
        recovery.request(200, now + Duration::from_millis(10));
        for ms in 11..500 {
            recovery.request(usize::MAX, now + Duration::from_millis(ms));
            assert_eq!(recovery.take_ready(now + Duration::from_millis(ms)), None);
            assert_eq!(recovery.deadline(), Some(now + RESYNC_INTERVAL));
        }
        assert_eq!(recovery.take_ready(now + RESYNC_INTERVAL), Some(10_000));
        assert_eq!(
            recovery.take_ready(now + RESYNC_INTERVAL),
            None,
            "one response per coalesced burst"
        );
        recovery.request(0, now + RESYNC_INTERVAL);
        assert_eq!(recovery.deadline(), Some(now + 2 * RESYNC_INTERVAL));
        assert_eq!(recovery.take_ready(now + 2 * RESYNC_INTERVAL), Some(0));
    }

    #[test]
    fn resume_requests_one_snapshot_only_when_output_was_skipped() {
        let (tx, mut output) = broadcast::channel(2);
        let mut flow = FlowGate::default();
        flow.pause();
        assert!(
            !flow.resume(&mut output),
            "quiet pause must not rebuild the screen"
        );
        flow.pause();
        for _ in 0..8 {
            tx.send(Bytes::from_static(b"output")).unwrap();
        }
        assert!(
            flow.resume(&mut output),
            "lagged receiver must resynchronize"
        );
        assert!(
            !flow.resume(&mut output),
            "duplicate resume must not send another snapshot"
        );
        // The caller replaces the old receiver with the snapshot subscription.
        // Flow detection itself never drains a live producer.
        assert!(!output.is_empty());
        output = tx.subscribe();
        tx.send(Bytes::from_static(b"live")).unwrap();
        assert_eq!(output.try_recv().unwrap(), b"live"[..]);
    }

    #[tokio::test]
    async fn repeated_pause_rearms_auto_resume() {
        let (tx, mut output) = broadcast::channel(2);
        let mut flow = FlowGate::default();
        flow.pause();
        let first = flow.paused_until.unwrap();
        tokio::time::sleep(Duration::from_millis(1)).await;
        flow.pause();
        assert!(flow.paused_until.unwrap() > first);
        assert!(flow.paused_until.unwrap() > tokio::time::Instant::now() + Duration::from_secs(1));
        // Expire the deadline without spending two wall-clock seconds in a unit test.
        flow.paused_until = Some(tokio::time::Instant::now());
        tx.send(Bytes::from_static(b"held")).unwrap();
        tokio::time::timeout(Duration::from_millis(100), flow_deadline(flow.paused_until))
            .await
            .unwrap();
        assert!(flow.resume(&mut output));
        assert!(flow.paused_until.is_none());
    }
}
