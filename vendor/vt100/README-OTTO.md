# Vendored fork of `vt100` 0.18.1 (MIT)

Vendored from crates.io `vt100-0.18.1` and applied as a `[patch.crates-io]`
in the root `Cargo.toml`. Upstream files are kept byte-identical except for
`src/grid.rs`, `src/row.rs` and `src/screen.rs`, where every change is marked
`OTTO PATCH`. (Our additions are formatted with upstream's `max_width = 78`:
`rustfmt --edition 2024 --config max_width=78 vendor/vt100/src/lib.rs`. The
crate is not a workspace member, so `cargo fmt --all` never touches it.)

History: the fork started on 0.16.2 with five patches. 0.18 upstreamed two of
them outright (Patch 1, Patch 4) and part of a third (the height half of
Patch 3), so the 0.18.1 port carries less code; the behavior Otto relies on
is unchanged.

## Patch 1: top-anchored scroll regions feed scrollback — NOW UPSTREAM

Inline-viewport TUIs (codex / ratatui `Terminal::insert_before`) emit finished
transcript lines by scrolling a scroll region anchored at the top of the
screen. vt100 ≤ 0.16 only banked scrolled-off rows when NO region was active,
so reattach snapshots (`PtyHandle::snapshot_with_history`) had zero codex
history. Upstream 0.18 `Grid::scroll_up` banks rows whenever
`scroll_top == 0`, which is exactly the fork's rule (interior regions still
discard). No Otto code remains for this; the regression test still guards it.

## Patch 2: full-width, reflowable scrollback replay

- `src/screen.rs` — `Screen::scrollback_rows_formatted(take) -> String`:
  formatted scrollback replay that emits every row at its full stored width
  and joins soft-wrapped rows into logical lines the client terminal re-wraps
  at its own width (reflowable on later resizes). Cursor moves are
  column-relative only, so the stream is safe in a scrolled client. (0.18:
  returns `String` like every other upstream formatter; it was `Vec<u8>`.)
- `src/grid.rs` — `pub(crate) Grid::scrollback_rows()` read-only accessor.
- `src/row.rs` — `Row::cols()` widened to `pub(crate)`.

Upstream 0.18's new `Screen::contents_formatted_basic` / `on_scroll` capture
is SGR-only, unjoined and not width-independent, so it does not replace this.

## Patch 3: reflow-on-resize (the big one)

`src/grid.rs` — `Grid::set_size` on a scrollback-bearing grid (the primary
screen, capacity > 0) is diverted to `set_size_reflow`, which REFLOWS like
xterm.js / ghostty / tmux instead of upstream's truncate-and-pad on width
changes: scrollback + visible rows are joined into logical lines on their
soft-wrap flags, re-split at the new width (wide-char pairs kept atomic), the
viewport is bottom-anchored, and the cursor stays glued to its logical
position. Height-only changes move rows between grid and scrollback
(shrink: drop blank rows below the cursor, then push top rows into
scrollback; grow: pull rows back from scrollback, then add blank rows at the
bottom). The alternate screen (no scrollback) takes upstream's path — real
terminals don't reflow the alt buffer either.

Why: truncation made every resize destructive (content cut at the new width)
and desynced this emulator from the reflowing xterm.js in the UI — inline
TUIs (claude/codex) repaint assuming reflow semantics, so their erase
sequences landed on the wrong rows here, leaving duplicate/stale fragments
in scrollback and snapshots. Verified end-to-end by a headless xterm client
driving a real daemon + real `claude` through resize storms (clean), plus
`vt100_reflow_*` / `vt100_height_*` tests in `crates/otto-pty`.
Support: `Row::{into_cells, from_cells, pad_to, content_len, is_blank}`.

0.18 reconciliation:
- Upstream 0.18 now does bottom-anchored height changes itself, but only for
  height, it pads blank rows at the TOP when a capped scrollback can't cover
  the growth (its `unbounded_len` bookkeeping), and it still truncates on
  width changes. The fork keeps its own height path so behavior is
  identical to the 0.16 fork; `unbounded_len` is kept roughly in step but is
  only read by upstream's path, which the primary screen never takes.
- The Otto branch runs BEFORE upstream's new same-size early return, so a
  same-size `set_size` still resets the scroll region (as before).
- The scrollback view offset is now reset to 0 on resize (upstream 0.18 /
  xterm behavior; the fork used to clamp it). Reflow renumbers every row, so
  an old offset was meaningless anyway, and otto-pty never scrolls the view.
- Rows pushed off the top of the capped scrollback during a resize are handed
  to the `on_row` callback, so `Parser::set_size` / `Callbacks::on_scroll`
  keep upstream's contract. otto-pty uses no callbacks.

## Patch 4: CSI 3 J erases scrollback — NOW UPSTREAM

codex's SIGWINCH repaint is `ESC[r ESC[H ESC[2J ESC[3J` + a reprint of its
whole transcript; without 3J every resize banked a duplicate transcript (the
"essay appears ×7" bug). Upstream 0.18 implements `ed` mode 3
(`Grid::erase_all_scrollback`, also resetting the view offset), so the fork's
`Grid::erase_scrollback` is gone. Regression:
`codex_3j_resize_repaint_keeps_single_transcript_copy` in `crates/otto-pty`.

## Patch 5: compact, shared scrollback rows + a runtime cap

`src/grid.rs` / `src/row.rs` / `src/screen.rs` (perf review 01 F2,
daemon-core F9):

- Scrollback is `VecDeque<Arc<Row>>`. A row entering scrollback (scroll-up,
  height shrink, reflow) goes through `Row::trim_for_scrollback`, used in
  place of upstream 0.18's own `Row::trim` at its call site in
  `Grid::scroll_up`: a NON-wrapped row drops its trailing `Cell::default()`
  cells (at least one cell is kept — the formatters index `cells[start]`);
  wrapped rows stay full (their blanks are positions inside a logical line,
  which upstream's trim-everything would lose and reflow would then
  misjoin). Rows pulled back onto the grid are re-padded (`pad_to`).
  4000 rows of 20-char lines at 200 cols drop from ~25.6 MB to ~2.4 MB.
- `Arc` makes `Screen::clone()` (otto-pty's snapshot capture, done under the
  parser lock) refcount bumps for the history instead of a deep copy;
  reflow/height growth take rows back with `Arc::try_unwrap` (clone only
  when a capture still shares them), and upstream's
  `grow_visible_scrollback` uses `Arc::make_mut` only on rows that must grow.
- `Screen::set_scrollback_capacity(n)` / `primary_scrollback_capacity()`
  change / read the PRIMARY screen's cap at runtime (otto-pty keeps a
  shallower history for long-unviewed terminals). (0.18 rename: these were
  `set_scrollback_len` / `scrollback_len` in the 0.16 fork; upstream 0.18
  now uses `scrollback_len()` for the ROW COUNT and `scrollback_capacity()`
  for the ACTIVE screen's cap, which is 0 on the alternate screen.)
  `Screen::scrollback_stats()` reports `(rows, cells)` for memory accounting.

## Patch 6: ECMA-48 / xterm cursor, repeat, tab, IRM, and DECAWM sequences (`agy` / BubbleTea v2)

`src/perform.rs` / `src/screen.rs` / `src/grid.rs`:

- `CSI Ps b` (`REP` — Repeat Preceding Graphic Character): upstream 0.18 added
  `REP`, `HPA` (`` CSI Ps ` ``) and `HVP` (`CSI Pl;Pc f`); Patch 6 bounds `REP`
  to `rows × cols` and clears `last_char` on non-graphic cursor/erase/mode
  sequences per ECMA-48 so BubbleTea v2's `cursedRenderer` (`agy`) never repeats
  stale glyphs across cursor jumps.
- `CSI Ps a` (`HPR`), `CSI Ps e` (`VPR`), `CSI Ps I` (`CHT`), `CSI Ps Z` (`CBT`),
  and `CSI s` / `CSI u` (`SCOSC` / `SCORC` when parameterless).
- `CSI 4 h` / `CSI 4 l` (`IRM` — Insert/Replace Mode) and `CSI ? 7 h` /
  `CSI ? 7 l` (`DECAWM` — Auto-Wrap Mode; default enabled, preserved across
  `state_formatted` / `input_mode_diff`).

## Upgrading

Re-vendor the new upstream version (copy `src/`, `Cargo.toml`,
`CHANGELOG.md`, `README.md`, `.cargo_vcs_info.json`; keep this file and
`LICENSE`), then re-apply every change marked `OTTO PATCH` — `diff -r` the
pristine previous version's `src/` against this directory to get the exact
patch set. Check upstream's CHANGELOG for behavior it now provides natively
(as 0.18 did for Patches 1 and 4). Bump `vt100` in the root `Cargo.toml`
and run `cargo update -p vt100`. Regression tests live in `crates/otto-pty`
(`snapshot_with_history_captures_scroll_region_history`,
`history_replay_joins_soft_wrapped_rows_for_client_reflow`,
`history_replay_survives_narrowing_without_truncation`,
`codex_3j_resize_repaint_keeps_single_transcript_copy`,
`history_cap_shrinks_and_regrows`,
`short_lines_at_wide_grid_keep_a_small_scrollback`, `vt100_reflow_*`,
`vt100_height_*`, `agy_cursed_renderer_sequences_survive_snapshot_roundtrip`).
The published crate ships no `tests/` directory, so only its one unit test and
doc-tests can be run standalone (copy the directory out of the workspace and
add an empty `[workspace]` table).
