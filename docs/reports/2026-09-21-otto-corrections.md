# Otto corrections — September 21, 2026

These corrections extend the [September 20 enhancements](2026-09-20-otto-enhancements.md). All changes remain uncommitted. No pushes or pull requests were made.

## What to try

1. **Workspace context:** right-click a workspace → Workspace context, or open Settings → Workspace context. Save its goal, shared instructions, memory, decisions, references and artifacts. Preview them for an agent provider. New and restarted agent sessions inherit this context. Existing running conversations are not sent a new turn. There is no separate Projects navigation or session project picker. Existing Swarm records and scoped project memberships remain intact.
2. **Folder access:** enable Show hidden and open `.ssh` or `.git`. General file browsing and reading now follow the OS account running Otto, including accessible symlinks. Otto no longer applies its extra root/secret-name lists to these two endpoints. macOS permission denials remain effective and show the attempted path. Authentication and share/MCP endpoint scopes remain enforced.
3. **Terminal links:** click an HTTP(S) URL, `ui/src/App.svelte:42:3`, `../Cargo.toml#L8`, an absolute file path, or a native terminal hyperlink. URLs open the browser; files open the Files panel. Relative files resolve from the session's saved working folder; macOS resolves symlinks and parent-directory segments. Quoted paths/native hyperlinks support spaces; soft-wrapped paths retain the full target. Shortened basenames cannot reveal a nested directory that the CLI omitted; a missing-file error shows the exact attempted path.
4. **Files outside the workspace:** open an absolute file outside the current workspace. The Files panel browses its parent while displaying the file independently of directory listing. The viewer still works with no workspace or an unreadable parent folder. The workspace and session working folder do not change. A newer click wins over a delayed older file response.

## Validation

- Workspace context: 59 context library tests, 4 persistence tests, and a session create/restart regression passed. A regression protects shared instructions from a large memory section consuming the instruction budget.
- Filesystem: 12 focused Rust tests passed, including real temporary-file OS denials, synthetic secret-named files, hidden entries, symlinks, size bounds, nonregular files, authentication and token endpoint scopes.
- Browser regressions: 3 workspace context cases, 6 terminal/Files cases, 1 filesystem picker case and 2 existing personal-document/path-picker cases passed. Browser API fixtures are isolated from live user data; terminal cases drive real xterm rendering and clicks.
- 146 UI unit tests passed. Type checks passed with zero errors/warnings. Production UI build passed with existing bundle-size/import notices.
- Full-workspace all-target Clippy passed with warnings denied. `git diff --check` passed.
- A real temporary symlink regression verifies that text links and native file hyperlinks preserve OS parent-directory semantics.
- Independent reviews covered workspace persistence/context assembly, editor/preview stale-response handling, filesystem behavior, and terminal/file-viewer behavior.
- Full Rust workspace tests were not run because existing unrelated Git fixtures create commits. No real credential files, provider logins, databases or remote SSH targets were used for feature tests.

## Build and installation

Installation is the final step after all checks and review findings are resolved. The detached installer records signed binary hashes, the full uncommitted source fingerprint, backup location, new app/daemon processes and health verification under:

`~/Library/Logs/Otto/uncommitted-20260921-corrections/`

`status.md` and `installed.json` in that directory are authoritative for installation completion. The previous app and SQLite state are backed up after the daemon stops. No database rollback is performed automatically.
