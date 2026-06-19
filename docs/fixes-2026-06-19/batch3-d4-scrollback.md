# Batch-3 D4 — Terminal scrollback survives reattach

## Problem

Terminal scrollback history was **lost on every reattach/reconnect**. The
client (`ui/src/lib/components/Terminal.svelte`) sends
`{ type: "scrollback", lines: 2000 }` on each (re)connect, but the server's
`ClientFrame::Scrollback` handler ignored the requested `lines` and replied with
only `PtyHandle::screen_snapshot()` — the visible 24-row screen. Everything that
had scrolled off above the viewport vanished whenever a client reconnected.

## Approach: `snapshot_with_history(lines)` on the PTY

Added `PtyHandle::snapshot_with_history(lines)` in
`crates/otto-pty/src/lib.rs`. It produces, in one coherent payload:

1. **Up to `lines` rows of scrollback history** (the rows that scrolled off
   *above* the visible screen), emitted as plain text — one `\r\n`-terminated
   line each. Writing these scrolls them up into the client xterm's own
   scrollback buffer (the client xterm keeps 10,000 lines).
2. **The current-screen frame**, identical to `screen_snapshot()`: leading
   `\x1b[2J\x1b[H` followed by `screen.state_formatted()` (full formatting,
   cursor, attrs, input mode). This reproduces the live screen — TUI input box
   included — in one frame with no replay flicker.

`lines == 0` short-circuits to exactly the bare `screen_snapshot()` (verified by
a test asserting byte-for-byte equality).

### How history rows are read from vt100

vt100 0.16 only exposes the *visible* window, plus a scrollback cursor:

- `Screen::set_scrollback(d)` shifts the viewport up by `d` rows and **clamps
  `d` to the actual retained history depth**.
- `Screen::scrollback()` reads back the (clamped) offset.
- `Screen::rows(0, cols)` returns the currently-visible rows as plain text.

So the implementation:

1. Saves the current offset (`0` in normal operation).
2. Probes retained history depth: `set_scrollback(usize::MAX)` then read back
   `scrollback()` → `total_history` (self-clamping; no separate capacity query
   needed).
3. `take = lines.min(total_history)` — caps by both the client request and the
   emulator's retained history (the parser keeps 1000 lines).
4. For `d` in `take..=1` (oldest→newest), `set_scrollback(d)` and take
   `rows(0, cols).next()` — at offset `d` the **first** visible row is exactly
   the row `d` positions above the live screen's top
   (`visible_rows()` does `skip(scrollback_len - d)`), so this yields the most
   recent `take` history rows in display order.
5. Restores the saved offset, then appends the current-screen frame.

### How `lines` is honored

- `ws.rs` passes the client's requested `lines` straight through to
  `snapshot_with_history(lines)`.
- The PTY caps it with `lines.min(total_history)`, so over-asking (the client
  sends 2000; the parser retains ≤1000) returns all available history without
  error and never over-reads.
- When a client sends `lines: 0`, `ws.rs` substitutes
  `DEFAULT_ATTACH_HISTORY_LINES` (1000) so a zero from a minimal client still
  restores ample context rather than collapsing to the bare screen.

### How double-rendering is avoided

History holds **only** rows that scrolled off above the live screen — never the
visible rows. The visible viewport is drawn exactly once, by the current-screen
frame at the end. The `\x1b[2J\x1b[H` that precedes it clears the terminal
**grid**, not the client xterm's scrollback, so the history text we just wrote
remains scrolled up in the client's history while the live screen redraws fresh
below it. No visible row is emitted twice.

## Files changed

- `crates/otto-pty/src/lib.rs` — new `snapshot_with_history(lines)`; unit test
  `snapshot_with_history_keeps_offscreen_lines_without_duplicating_visible`.
- `crates/otto-sessions/src/ws.rs` — `ClientFrame::Scrollback { lines }` now
  honors `lines` and calls `snapshot_with_history`; added
  `DEFAULT_ATTACH_HISTORY_LINES` (the `#[allow(dead_code)]` on `lines` is gone
  since the field is now used).

## Tests

New unit test in `otto-pty`: prints `LINE_0001..LINE_0080` (far more than one
24-row screen) via `sh`, waits for child exit + reader drain, then asserts:

- `LINE_0001` (scrolled off the top) **is present** in the history-inclusive
  snapshot.
- `LINE_0080` (in the visible viewport) appears **exactly once** — proves no
  double-render between history and the live screen.
- `LINE_0001` is **absent** from the bare `screen_snapshot()` — confirms it is
  genuinely history, not part of the live screen.
- `snapshot_with_history(0) == screen_snapshot()` byte-for-byte.

## Verification

- `cargo check -p otto-pty -p otto-sessions` — OK.
- `cargo clippy -p otto-pty -p otto-sessions --all-targets -- -D warnings` —
  clean (exit 0).
- `cargo test -p otto-pty -p otto-sessions` — all pass (otto-pty 3/3 incl. the
  new test; otto-sessions 16 unit + e2e pass, folder-trust tests ignored as
  designed).

### Pre-existing failure outside my files (not fixed, per instructions)

`cargo clippy --workspace --all-targets -- -D warnings` fails to compile
`otto-dbviewer` (unused-imports: `Capabilities`, `ObjectDetail`, …) — that crate
is being actively edited by another agent (its `src/{driver,service,types}.rs`
and `drivers/mysql.rs` show as modified) and is unrelated to D4. Left untouched.
