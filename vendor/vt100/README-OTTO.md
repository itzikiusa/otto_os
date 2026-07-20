# Vendored fork of `vt100` 0.16.2 (MIT)

Vendored verbatim from crates.io `vt100-0.16.2` and applied as a
`[patch.crates-io]` in the root `Cargo.toml`, with ONE behavioral change:

## Patch: top-anchored scroll regions feed scrollback

`src/grid.rs`, `Grid::scroll_up`: upstream only pushes a scrolled-off row into
scrollback when **no scroll region is active**. Inline-viewport TUIs — codex
(ratatui `Terminal::insert_before`) most importantly — emit finished transcript
lines by setting a scroll region anchored at the top of the screen and scrolling
it up. Under upstream semantics every one of those rows is discarded, so Otto's
reattach snapshot (`PtyHandle::snapshot_with_history`) had **zero codex
history** — reconnecting a codex session lost all scrollback. (tmux has the same
upstream-style behavior; this is the known "codex loses scrollback in tmux"
failure class.)

The patch pushes rows into scrollback when `scroll_top == 0` regardless of the
bottom margin, matching what xterm.js does live in the browser — so the replayed
history equals what the user saw before disconnecting. Rows scrolled out of an
*interior* region (`scroll_top > 0`) are still discarded, as everywhere else.

## Patch 2: full-width, reflowable scrollback replay

- `src/screen.rs` — new `Screen::scrollback_rows_formatted(take)`: formatted
  scrollback replay that emits every row at its full stored width (scrollback
  rows are never resized by `set_size`, so nothing is truncated after a
  narrowing resize) and joins soft-wrapped rows into logical lines the client
  terminal re-wraps at its own width (reflowable on later resizes). Cursor
  moves are column-relative only, so the stream is safe in a scrolled client.
- `src/grid.rs` — new `pub(crate) Grid::scrollback_rows()` read-only accessor.
- `src/row.rs` — `Row::cols()` widened to `pub(crate)`.

## Patch 3: reflow-on-resize (the big one)

`src/grid.rs` — `Grid::set_size` on a scrollback-bearing grid (the primary
screen) now REFLOWS like xterm.js / ghostty / tmux instead of upstream's
truncate-and-pad: scrollback + visible rows are joined into logical lines on
their soft-wrap flags, re-split at the new width (wide-char pairs kept
atomic), the viewport is bottom-anchored, and the cursor stays glued to its
logical position. Height changes move rows between grid and scrollback
(upstream truncated the BOTTOM, eating the prompt). The alternate screen
(`scrollback_len == 0`) keeps upstream behavior — real terminals don't
reflow the alt buffer either.

Why: upstream's truncation made every resize destructive (content cut at the
new width) and desynced this emulator from the reflowing xterm.js in the UI —
inline TUIs (claude/codex) repaint assuming reflow semantics, so their
erase sequences landed on the wrong rows here, leaving duplicate/stale
fragments in scrollback and snapshots. Verified end-to-end by a headless
xterm client driving a real daemon + real `claude` through resize storms
(clean), plus `vt100_reflow_*` / `vt100_height_*` tests in `crates/otto-pty`.
Support: `Row::{into_cells, from_cells, pad_to, content_len, is_blank}`.

## Upgrading

Re-vendor the new upstream version and re-apply the changes marked
`OTTO PATCH` (the `Grid::scroll_up` conditional, `Screen::
scrollback_rows_formatted`, `Grid::scrollback_rows`, `Row::cols`
visibility). Regression tests live in `crates/otto-pty`
(`snapshot_with_history_captures_scroll_region_history`,
`history_replay_joins_soft_wrapped_rows_for_client_reflow`,
`history_replay_survives_narrowing_without_truncation`).
