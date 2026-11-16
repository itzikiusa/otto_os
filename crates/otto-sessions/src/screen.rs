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

/// True when the bottom of an agent PTY screen holds an interactive
/// permission / approval / question prompt (as opposed to an idle composer).
/// Shared by the status task (an agent blocked on approval is not working,
/// whatever its turn record says) and the needs-you monitor.
pub fn awaits_approval(rows: &[String]) -> bool {
    const NEEDLES: &[&str] = &[
        "do you want to ",
        "would you like to run",
        "approve and run",
        "allow once",
        "allow for this session",
        "enter to confirm",
        "esc to cancel",
    ];
    rows.iter()
        .rev()
        .filter(|r| !r.trim().is_empty())
        .take(16)
        .any(|r| {
            let low = r.to_ascii_lowercase();
            NEEDLES.iter().any(|n| low.contains(n))
        })
}

/// One hash per screen row of its text with what an IDLE TUI animates masked
/// out (see [`mask_volatile`]) — the session status task compares two ticks'
/// prints to tell real output from a repaint. Text only: cursor position and
/// blink, colours and other attributes never reach it.
pub fn row_prints(rows: &[String]) -> Vec<u64> {
    hash_rows(rows, false)
}

/// [`row_prints`] with only wall-clock times masked: spinner glyphs and
/// elapsed-time counters stay in. Where these change and [`row_prints`] does
/// not, a BUSY animation ticked (a spinner, a "thinking (12s)" counter) — as
/// opposed to idle decoration (a clock), which changes neither.
pub fn busy_prints(rows: &[String]) -> Vec<u64> {
    hash_rows(rows, true)
}

fn hash_rows(rows: &[String], keep_busy: bool) -> Vec<u64> {
    use std::hash::{Hash, Hasher};
    rows.iter()
        .map(|r| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            mask_row(r, keep_busy).hash(&mut h);
            h.finish()
        })
        .collect()
}

/// Spinner frames TUIs cycle through while they wait (braille dots, Claude's
/// stars, quarter circles). Masked wherever they appear on a row.
fn is_spinner_glyph(c: char) -> bool {
    matches!(c, '\u{2800}'..='\u{28FF}')
        || matches!(
            c,
            '✻' | '✶'
                | '✳'
                | '✢'
                | '✽'
                | '✺'
                | '◐'
                | '◓'
                | '◑'
                | '◒'
                | '◴'
                | '◵'
                | '◶'
                | '◷'
        )
}

/// ASCII spinner frames: only masked as a row's first visible cell followed
/// by a blank (`| working`), never inside text or a path.
fn is_ascii_spinner(c: char) -> bool {
    matches!(c, '|' | '/' | '-' | '\\' | '*' | '·')
}

/// `row` with the cells an idle TUI keeps redrawing replaced by `#`:
/// spinner glyphs, clock times (`12:34`, `12:34:56`) and elapsed-time counters
/// (`12s`, `1.5s`, `250ms`, `3m`, `2h`). Everything else — streaming text, a
/// scrolling build, a progress fraction like `12/300` — is kept, so a real
/// change still changes the print.
pub fn mask_volatile(row: &str) -> String {
    mask_row(row, false)
}

/// [`mask_volatile`]; `keep_busy` leaves spinner glyphs and elapsed-time
/// counters in and masks only clock times (see [`busy_prints`]).
fn mask_row(row: &str, keep_busy: bool) -> String {
    let chars: Vec<char> = row.chars().collect();
    let mut out = String::with_capacity(row.len());
    let first_visible = chars.iter().position(|c| !c.is_whitespace());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let ascii_lead = Some(i) == first_visible
            && is_ascii_spinner(c)
            && chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        if !keep_busy && (is_spinner_glyph(c) || ascii_lead) {
            out.push('#');
            i += 1;
            continue;
        }
        let at_boundary = i == 0 || !chars[i - 1].is_alphanumeric();
        if c.is_ascii_digit() && at_boundary {
            if let Some((end, clock)) = time_token_end(&chars, i) {
                if clock || !keep_busy {
                    out.push('#');
                    i = end;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// End index (exclusive) of a clock or elapsed-time token starting at
/// `start` (a digit) and whether it is a clock, or `None` when the digits
/// there are something else.
fn time_token_end(chars: &[char], start: usize) -> Option<(usize, bool)> {
    let digits = |from: usize| {
        let mut j = from;
        while chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
            j += 1;
        }
        j
    };
    let boundary = |j: usize| chars.get(j).is_none_or(|c| !c.is_alphanumeric());
    let mut j = digits(start);
    // Clock: d{1,2}:dd(:dd)?
    if j - start <= 2 && chars.get(j) == Some(&':') {
        let m = digits(j + 1);
        if m - (j + 1) == 2 {
            let mut end = m;
            if chars.get(m) == Some(&':') {
                let s = digits(m + 1);
                if s - (m + 1) == 2 {
                    end = s;
                }
            }
            return boundary(end).then_some((end, true));
        }
        return None;
    }
    // Elapsed: d+(.d+)?(ms|s|m|h)
    if chars.get(j) == Some(&'.') && chars.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
        j = digits(j + 1);
    }
    for unit in ["ms", "s", "m", "h"] {
        let n = unit.len();
        let matches = unit
            .chars()
            .enumerate()
            .all(|(k, u)| chars.get(j + k) == Some(&u));
        if matches && boundary(j + n) {
            return Some((j + n, false));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_spinners_clocks_and_elapsed_counters() {
        assert_eq!(mask_volatile("⠋ Thinking… (12s)"), "# Thinking… (#)");
        assert_eq!(mask_volatile("⠙ Thinking… (13s)"), "# Thinking… (#)");
        assert_eq!(
            mask_volatile("✻ Working (1m 2s · esc)"),
            "# Working (# # · esc)"
        );
        assert_eq!(mask_volatile("  | waiting 1.5s"), "  # waiting #");
        assert_eq!(mask_volatile("took 250ms at 09:41:07"), "took # at #");
        assert_eq!(mask_volatile("12:05 ready"), "# ready");
    }

    #[test]
    fn busy_prints_see_spinners_and_counters_but_not_clocks() {
        let p = |r: &str| busy_prints(&[r.to_string()])[0];
        assert_ne!(
            p("⠋ Thinking… (12s)"),
            p("⠙ Thinking… (12s)"),
            "spinner frame"
        );
        assert_ne!(
            p("Thinking… (12s)"),
            p("Thinking… (13s)"),
            "elapsed counter"
        );
        assert_eq!(p("ready · 09:41:07"), p("ready · 09:41:08"), "wall clock");
        assert_eq!(p("ready · 09:41"), p("ready · 09:42"), "wall clock");
    }

    #[test]
    fn keeps_real_content_and_lookalikes() {
        for row in [
            "Compiling otto-sessions v0.3.0 (12/300)",
            "src/a-b/c.rs | 3 +-",
            "build 45% done",
            "v2s release",
            "x = a - b * c",
            "port 8080:80",
        ] {
            assert_eq!(mask_volatile(row), row, "{row}");
        }
    }

    #[test]
    fn prints_ignore_a_repaint_but_see_new_text() {
        let a = row_prints(&["⠋ Thinking… (12s)".into(), "> ".into()]);
        let b = row_prints(&["⠼ Thinking… (14s)".into(), "> ".into()]);
        assert_eq!(a, b, "spinner frame + elapsed tick is a repaint");
        let c = row_prints(&["⠋ Thinking… (12s)".into(), "> hello".into()]);
        assert_ne!(a[1], c[1], "typed / streamed text changes the print");
    }

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
