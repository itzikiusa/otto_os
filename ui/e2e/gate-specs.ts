// The BLOCKING e2e smoke gate (ci.yml `e2e-gate`): the specs below run on
// every PR with retries 0 + the flaky check, and a red result fails the
// workflow. The full suite stays advisory (`e2e-functional`, sharded).
//
// Admission rule: a spec joins only after a green history in the advisory
// shards (every test passed, none failed, in the latest CI runs) and only if
// it needs nothing beyond the isolated throwaway daemon — no docker DB stack,
// redis-server, ClickHouse, tour film or wall-clock budget. Seeded from the
// 2026-10-05 integration runs (37365325812 / 37368256555 shards, cross-checked
// against the PR #85 baseline failures). Promote more as they prove stable;
// a spec that turns flaky here moves back out (with an issue), never gets a
// retry.
//
// With OTTO_E2E_GATE=1, playwright.config.ts narrows the REAL
// `desktop-browser` (Chromium, 1280×800 shell) and `iphone-portrait` (WebKit)
// projects to these lists — never separate project names, which the specs'
// `info.project.name` guards would skip (S12-301). The gate fails on any
// skipped test and on fewer than GATE_MIN_EXPECTED passes
// (scripts/e2e-flaky-check.mjs --gate); ui/unit/e2eGateSpecs.test.ts rejects
// a gate spec whose project guard excludes its gate project.

/** desktop-*.spec.ts files run at desktop width (Chromium). */
export const DESKTOP_GATE_SPECS = [
  'desktop-accounts.spec.ts',
  'desktop-agents-retention.spec.ts',
  'desktop-api-postman-import.spec.ts',
  'desktop-api-run-recovery.spec.ts',
  'desktop-api-search.spec.ts',
  'desktop-assistant.spec.ts',
  'desktop-backup-archive.spec.ts',
  'desktop-browser-overlay-selector.spec.ts',
  'desktop-canvas-panel.spec.ts',
  'desktop-canvas-versions.spec.ts',
  'desktop-channels-rooms-health.spec.ts',
  'desktop-connections-hub.spec.ts',
  'desktop-personal-autonomy.spec.ts',
  'desktop-personal-documents.spec.ts',
  'desktop-platform-settings-recovery.spec.ts',
  'desktop-proof-packs.spec.ts',
  'desktop-proof-packs-v2.spec.ts',
  'desktop-review-agent-stop.spec.ts',
  'desktop-review5-mcp-audit-details.spec.ts',
  'desktop-review5-scheduled-draft.spec.ts',
  'desktop-review5-vault-backlinks.spec.ts',
  'desktop-review6-product-routing.spec.ts',
  'desktop-rpanel-tabs.spec.ts',
  'desktop-run-with-otto.spec.ts',
  'desktop-scheduled-tasks.spec.ts',
  'desktop-session-menu.spec.ts',
  'desktop-settings-run-history.spec.ts',
  'desktop-shell.spec.ts',
  'desktop-shell-reopen.spec.ts',
  'desktop-skills-lab.spec.ts',
  'desktop-tiled-maximize.spec.ts',
  'desktop-token-cleanup.spec.ts',
  'desktop-transcript-cache-live.spec.ts',
  'desktop-ux-git.spec.ts',
] as const;

/** Mobile specs run at iPhone portrait (WebKit). */
export const MOBILE_GATE_SPECS = ['theme.spec.ts', 'vault-mobile.spec.ts'] as const;

/** The projects the gate runs a spec on (desktop list / mobile list). */
export const DESKTOP_GATE_PROJECT = 'desktop-browser';
export const MOBILE_GATE_PROJECT = 'iphone-portrait';

/**
 * Floor for the gate's passed-test count: the gate fails when fewer pass, so a
 * spec that silently stops running (a new skip guard, a rename, a describe
 * that never registers) is caught. Equals the gate's test count — re-measure
 * with `OTTO_E2E_GATE=1 npx playwright test --list --project=desktop-browser
 * --project=iphone-portrait` when promoting a spec; lower it only when a spec
 * leaves the gate.
 */
export const GATE_MIN_EXPECTED = 166;

/** Exact-file matcher for a Playwright project's `testMatch`. */
export function gateMatcher(files: readonly string[]): RegExp {
  const esc = files.map((f) => f.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  return new RegExp(`(^|/)(${esc.join('|')})$`);
}
