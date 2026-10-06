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
// playwright.config.ts turns these lists into the `desktop-gate` (Chromium,
// 1280×800 shell) and `iphone-gate` (WebKit, iPhone portrait) projects.

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

/** Exact-file matcher for a Playwright project's `testMatch`. */
export function gateMatcher(files: readonly string[]): RegExp {
  const esc = files.map((f) => f.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  return new RegExp(`(^|/)(${esc.join('|')})$`);
}
