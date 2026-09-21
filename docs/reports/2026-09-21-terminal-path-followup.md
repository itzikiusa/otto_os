# Terminal file paths — follow-up to installed September 21 corrections

The first link patch passed simple path and soft-wrap tests but missed the actual provider output shown in the user's follow-up screenshots.

## Confirmed causes

- An unquoted absolute path containing `Application Support` was split at its space. The suffix `Support/...` was incorrectly resolved from the session cwd.
- Codex rendered the displayed installation-status path in cyan across a hard CRLF plus indentation. The terminal detector joined only xterm soft wraps, so the leading fragment was not clickable and the trailing fragment became a different relative path.
- Claude rendered a bold `../../../private/tmp/.../scratchpad/ui-design-spec.md` across cursor-positioned rows. The trailing fragment was likewise treated as a standalone relative path.

Read-only snapshots confirmed the exact formatting. These examples contain no retained OSC 8 destination. Metadata checks confirmed all three original full paths exist. No file contents or live sessions were modified during this diagnosis.

## Verification target

Use the real terminal component with synthetic copies of the relevant formatting, covering the space-containing path and both providers' wrapped paths. Clicking each displayed fragment must select the complete intended file. Separate paths and unrelated prose must not be joined. Run Chromium and WebKit checks, alongside the existing relative-path, OSC 8, symlink, shared-session and file-viewer regressions.

General terminal text cannot always reveal an omitted directory or an arbitrary filename boundary. Explicit native destinations and delimited paths remain preferable when available. This patch must not infer unrelated paths from neighboring output.

## Results

- The three reported cases were reproduced as failing regressions before the fix.
- Rooted paths containing spaces now claim their entire range before relative suffix detection. Sentence punctuation and a subsequent explicit root terminate the candidate.
- Soft-wrapped rows are assembled before bounded hard-row recognition. Reconstructed targets retain separate clickable ranges for each physical row, excluding continuation indentation.
- Claude's bold relative-directory plus slash-continuation format remains recognizable when replayed into a wider terminal. Painted trailing spaces do not manufacture a right-margin path.
- All 153 UI unit tests passed, including 14 focused link tests. All 20 real-terminal browser cases passed (10 Chromium, 10 WebKit). Type checks passed with zero errors/warnings; diff check passed.
- Separately replayed the actual captured Claude terminal snapshot against isolated mock file endpoints: clicking the displayed `ui-design-spec.md` selected the full intended path in Chromium and WebKit.
- Independent correctness review approved the final implementation. No backend or API changes were needed for these reported cases.

## Installation

The rebuilt signed app is installed only after validation and source freeze. Check `~/Library/Logs/Otto/uncommitted-20260921-terminal-paths/status.md` and `installed.json` for the final installation outcome, binary hashes and backup location. No commits or pushes are authorized or performed.
