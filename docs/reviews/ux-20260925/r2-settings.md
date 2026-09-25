# Reviewer 6/10 — round 2 Settings, MCP, Plugins, Skills Lab

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`. No commits, subagents, real plugin installs, provider operations, or outward MCP tool calls. Only four owned module components and `ui/e2e/desktop-ux-r2-settings.spec.ts` changed. The parent owns integration/global checks and the remaining mandatory rounds.

## Independent recheck of round 1

All six `desktop-ux-settings.spec.ts` tests passed again against the current release daemon: MCP keyboard focus; failed aggregate membership loads block edits and Retry works; repeated Enter mints only one token; phone plugin list Retry; every available Settings section remains reachable/titled at desktop and phone; seeded skill search/Files view. Also reran the three existing MCP default-view tests, including a real isolated-daemon per-workspace session-attachment write that preserves the other workspace's setting.

## Confirmed defects repaired

1. **P1 — later file response overwrites the selected Skills file.** `ui/src/modules/skills-lab/SkillEditor.svelte:59`. Hold first-file GET, open second file, release first GET. Red browser run showed `# Delayed first document` beneath the second-file selection. Loads now use a generation and render loading instead of allowing editing against stale content. Retry remains available on failure.
2. **P1 — Save marked text typed during an in-flight write as saved.** `SkillEditor.svelte:118`, `:249`. Hold Save, type newer text, release Save. Red test found Save disabled despite the server having only the older submitted text. Save now snapshots path/text, acknowledges exactly that snapshot, and feeds the live draft to CodeEditor so its prop synchronization cannot replace later typing. Green test checks the daemon's actual persisted file before and after a second keyboard Save.
3. **P2 — bulk workspace-role update reported success after all writes failed.** `ui/src/modules/settings/Users.svelte:120`, `:149`. All PUTs return 503; after all rows finish, old code still reports “Set to viewer in all workspaces” (red expected count 0, actual 1). Individual writes now return their result; bulk reports the actual count and failure, retaining successful rows. Final test covers all-failed, partially failed, and retry: retry sends exactly one request for the remaining failed workspace.
4. **P2 — a delayed attachment save leaked workspace A's value into workspace B.** `ui/src/modules/mcp/OttoServerHome.svelte:158`, `:170`, `:251`. Start Alpha detach, switch to Beta through the command palette, then complete Alpha's response. Red browser run showed Beta unchecked. Loads/writes now check their request generation and workspace; pending state resets on switch; the checkbox is keyed to its workspace and disabled until its setting is known. Final test waits for the old response to finish before checking Beta.
5. **P2 — long plugin version collapsed the plugin name to zero width on phone.** `ui/src/modules/settings/PluginsSettings.svelte:284`. Synthetic long name/version at 375px made the name hidden and metadata spill across actions. Name now has a separate line, metadata wraps and version stays bounded with its full value in a title. Test checks visible name, nonzero usable width and internal row bounds; screenshot visually confirms name, status and actions.
6. **P2 — tablet RTL Skills file sidebar left a roughly 70px editor.** `SkillEditor.svelte:415`, `:366`. The old viewport breakpoint did not account for the narrow detail pane beside two navigation columns at 834px. Visually reproduced in `/tmp/otto-ux-r2-settings-skills-tablet-before.png`. The Files layout now responds to the existing `skilldetail` container width; its header can wrap. All five visual variants assert an editor wider than 200px. The repaired tablet image shows a full-width editor below files.
7. **P2 — plugin source/version paths reordered under RTL.** `PluginsSettings.svelte:129`, `:169`, `:174`. Viewed tablet screenshot showed a reordered source placeholder and truncated version suffix on the leading side. Source input, source metadata and version now explicitly keep LTR direction. Repaired RTL screenshot viewed; the retained visual spec also checks computed direction on reruns.

## Verified workflows and limits

- Appearance: Warm + Light survives reload; Pro Dark disables scheme controls and explains why. Every Settings section's navigation/title verified again.
- Permissions: failed provider permission write restores the saved checkbox state; retry writes the new value; reload preserves it.
- Users: membership read failure prevents replacement writes; all/partial bulk write failure is honest and retry only writes remaining failures.
- Tokens: repeated Enter still causes one mint request (round-one regression).
- Plugins: load error/Retry/empty, populated long metadata, remove confirmation describes file retention, Escape cancels without mutation, enable failure preserves Disabled, install failure preserves source and inline invalid state, repeated Enter is guarded, Enter retries. Install/enable requests are intercepted with failures, never real execution.
- Skills: search/select/files, delayed request ordering, snapshot save with later typing, actual saved bytes, keyboard Save, failed-file Retry, cancel draft discard, Revert. These exercise the shared CodeEditor after the other reviewer's events/Vite dependency repair.
- MCP: main view/subroutes, keyboard focus, per-workspace persistence, delayed save across workspace switch, catalog-load Retry, failed exposure update rollback. No external tools invoked.
- Twenty route-correct Chromium screenshots cover Appearance, loaded Plugins, MCP catalog and loaded Skills Files in Native light desktop, Native dark phone, Warm light RTL tablet, Warm dark desktop, Pro Dark phone. Screenshots use synthetic selected skill/plugin records; no real account/session/usage images are offered as evidence.
- Native theme means browser-rendered Native tokens, not physical Tauri validation. Six focused flows additionally passed actual Playwright iPhone WebKit; no physical VoiceOver session. No production cloud writes. Unverified workflows remain: deeper Groups & access edits; provider/account setup; backup/restore actions; governed external server discovery/tool testing/policies/approvals; Skills review/evaluation/provider-copy changes and cross-skill response races. These are coverage limits, not asserted defects.

## Commands and outcomes

From `worktree/ui`, common environment:

```sh
OTTO_E2E_SLOT=ux2settings OTTO_E2E_PORT=7845 OTTO_E2E_PW_PORT=5345 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

- `npx playwright test e2e/desktop-ux-r2-settings.spec.ts e2e/desktop-ux-settings.spec.ts e2e/desktop-mcp-cp-default-view.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-2-settings-results`: **18 passed (3.2m)**. Log `/tmp/otto-ux-2-settings.log`.
- Final strengthened R2 spec: `npx playwright test e2e/desktop-ux-r2-settings.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-2-settings-final`: **11 passed (1.8m)**. Log `/tmp/otto-ux-2-settings-final.log`.
- Red evidence: `/tmp/otto-ux-2-settings-red` and `/tmp/otto-ux-2-settings-red2`, matching `.log` files. Initial bulk test needed a selector correction because toast dedup adds `×2`; red2 then caught the actual false-success defect. Initial MCP test also exposed the stale optimistic checkbox before completion; strengthened final test awaits the completed old save.
- Phone WebKit: `npx playwright test e2e/desktop-ux-r2-settings.spec.ts --grep 'long plugin|appearance choices|failed permission|plugin install|Skills failed|MCP catalog' --project=iphone-portrait --workers=1 --output=/tmp/otto-ux-2-settings-webkit-final`: **6 passed (26.5s)**. Log `/tmp/otto-ux-2-settings-webkit-final.log`. Initial `/tmp/otto-ux-2-settings-webkit` had 4 passed and 2 helper failures: `uncheck`/`setChecked` expected a lasting changed value, whereas fast failure rollback correctly restored it before the helper returned. The final test uses `click` and still asserts rollback and retry. No production change was needed.
- `.last-run.json` in combined, final desktop and final WebKit result directories reports `status: passed` and no failed tests.
- `npx tsc --noEmit -p tsconfig.e2e.json`: passed.
- `node scripts/ui-guards.mjs`: passed (803 files; no new debt). No baseline increase.
- Scoped `git diff --check`: passed. Full npm check intentionally left to parent.

## Visual evidence actually viewed

Prefix `/tmp/otto-ux-r2-settings-`; names below include `.png`:

- `plugins-phone`, `plugins-native-dark-phone`, `plugins-warm-dark`, `plugins-warm-light-tablet-rtl` (before and after LTR correction).
- `appearance-native-light`, `appearance-native-dark-phone`, `appearance-pro-dark-phone`.
- `skills-native-light`, `skills-native-dark-phone`, `skills-warm-light-tablet-rtl` (before and after container repair), `skills-warm-dark`, `skills-pro-dark-phone`.
- `mcp-warm-dark`, `mcp-pro-dark-phone`, `mcp-warm-light-tablet-rtl`.

Observed improvements: phone plugin identity/status/actions remain distinct; tablet Skills no longer squeezes editing to a strip; code keeps LTR ordering within mirrored layout; consistent headers/theme accents/readable form hierarchy. Dense MCP descriptions and tablet three-column content still require more scanning than desktop.

## Scores after repairs

Scores judge the verified UI and describe coverage limits rather than applying a blanket penalty for avoiding production writes.

| Family | Layout/readability | Interaction | Accessibility | States/recovery | Responsive | Mean |
|---|---:|---:|---:|---:|---:|---:|
| Settings preferences/permissions/tokens | 9.5 | 9.5 | 9.3 | 9.5 | 9.4 | 9.44 |
| Users membership/bulk roles | 9.3 | 9.5 | 9.2 | 9.5 | 9.2 | 9.34 |
| MCP built-in server/catalog | 9.4 | 9.5 | 9.3 | 9.4 | 9.3 | 9.38 |
| MCP external servers/activity | 9.2 | 9.1 | 9.2 | 9.1 | 9.2 | 9.16 |
| Plugins | 9.4 | 9.5 | 9.3 | 9.5 | 9.4 | 9.42 |
| Skills Files/drafts | 9.4 | 9.5 | 9.3 | 9.5 | 9.3 | 9.40 |

| Visual variant | Appearance | Plugins | MCP catalog | Skills Files |
|---|---:|---:|---:|---:|
| Native light desktop | 9.5 | 9.5 | 9.4 | 9.5 |
| Native dark phone | 9.4 | 9.4 | 9.3 | 9.4 |
| Warm light tablet RTL | 9.3 | 9.3 | 9.3 | 9.3 |
| Warm dark desktop | 9.5 | 9.5 | 9.4 | 9.4 |
| Pro Dark phone | 9.4 | 9.4 | 9.3 | 9.4 |

Parent separately reports the integrated npm check passed with 0 errors/0 warnings and 454 unit tests passed; those are parent-owned checks.

No whole-scope 9.5 claim. The next round should deepen the unverified workflows listed above rather than repeat the same surface navigation. No reproduced feasible defect was deferred merely to the next round.
