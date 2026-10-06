//! A session's CURRENT terminal screen as plain text — the body of
//! `GET /sessions/{id}/screen` ([`crate::http`]), for the Home "Otto School"
//! 3D view that paints each agent's live screen onto a PC monitor (it polls
//! ~every 2 s for ≤ 12 visible sessions). Rows come from the emulator mirror
//! ([`otto_pty::PtyHandle::screen_rows`]: plain text, trailing blanks
//! trimmed, no scrollback), with trailing empty rows dropped, each row
//! clipped to [`MAX_COLS`] chars (char-boundary safe) and only the LAST
//! [`MAX_ROWS`] kept. A session without a live PTY reads as
//! [`SessionScreen::offline`] — the read never spawns or resumes anything.

use otto_pty::PtyHandle;
use serde::Serialize;

/// Per-row char cap — a monitor texture never needs more.
pub const MAX_COLS: usize = 240;
/// Rows kept, counted from the bottom (where the prompt / latest output is).
pub const MAX_ROWS: usize = 60;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionScreen {
    /// The session has a live PTY right now.
    pub live: bool,
    /// Emulator size; `0` when not live.
    pub cols: u16,
    pub rows: u16,
    /// Plain screen rows, top to bottom (see the module doc for the clipping).
    pub lines: Vec<String>,
}

impl SessionScreen {
    pub fn offline() -> Self {
        Self {
            live: false,
            cols: 0,
            rows: 0,
            lines: Vec::new(),
        }
    }

    /// Read `handle`'s screen. Takes the emulator mutex the PTY reader also
    /// holds, so async callers run it on the blocking pool.
    pub fn capture(handle: &PtyHandle) -> Self {
        let (rows, cols) = handle.screen_size();
        Self {
            live: true,
            cols,
            rows,
            lines: clip_lines(handle.screen_rows()),
        }
    }
}

/// Drop trailing empty rows, keep the last [`MAX_ROWS`], clip each row to
/// [`MAX_COLS`] chars (never slicing inside a multibyte char).
fn clip_lines(mut rows: Vec<String>) -> Vec<String> {
    while rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }
    let skip = rows.len().saturating_sub(MAX_ROWS);
    rows.into_iter()
        .skip(skip)
        .map(|r| match r.char_indices().nth(MAX_COLS) {
            Some((cut, _)) => r[..cut].to_string(),
            None => r,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_trailing_blank_rows_and_keeps_the_last_rows() {
        let mut rows: Vec<String> = (0..80).map(|i| format!("row {i}")).collect();
        rows.extend([String::new(), String::new()]);
        let out = clip_lines(rows);
        assert_eq!(out.len(), MAX_ROWS);
        assert_eq!(out.first().unwrap(), "row 20");
        assert_eq!(out.last().unwrap(), "row 79");
        assert!(clip_lines(vec![String::new(); 5]).is_empty());
    }

    #[test]
    fn clips_rows_on_char_boundaries() {
        // 3-byte chars: a byte slice at 240 would land mid-char and panic.
        let wide = "界".repeat(MAX_COLS + 10);
        let out = clip_lines(vec![wide, "short".into()]);
        assert_eq!(out[0].chars().count(), MAX_COLS);
        assert_eq!(out[1], "short");
    }
}
