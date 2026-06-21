import { test, expect } from '@playwright/test';
import type { ChildProcess } from 'node:child_process';
import { apiCtx, seedWorkspace, seedGitRepo, seedRedis } from './seed';

// OPERATIONS specs: seed REAL data (a registered git repo + a redis DB
// connection) into the isolated test daemon, then verify it actually surfaces
// in the UI on both phone (iphone-portrait) and desktop (ipad-landscape) — i.e.
// the things a user "registered" are discoverable after the app activates the
// seeded workspace.
//
// Run with --workers=1 (single shared redis on a fixed port).

let workspaceId = '';
let repoId = '';
let redisConnId: string | null = null;
let redisProc: ChildProcess | null = null;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);

  // Register a local git repo (seedGitRepo inits one with many files + a commit)
  // — it lands in the GLOBAL repo list the Git page reads.
  ({ repoId } = await seedGitRepo(ctx, base, workspaceId));

  // Spawn an ephemeral redis-server + register a Dev connection. If redis isn't
  // available on this machine (or the daemon rejects it), seed returns null and
  // the DB test self-skips.
  try {
    const r = await seedRedis(ctx, base, workspaceId);
    if (r) {
      redisProc = r.proc;
      redisConnId = r.connId;
    }
  } catch {
    redisConnId = null;
  }

  await ctx.dispose();
});

test.afterAll(() => {
  // Kill the throwaway redis we spawned (best-effort).
  if (redisProc) {
    try {
      redisProc.kill('SIGKILL');
    } catch {
      /* already gone */
    }
  }
});

// Make the seeded workspace the active one before any page script runs — the
// workspace store reads this key on boot (ws.load → select). This is what lets
// the DB page load connections (its $effect is gated on ws.currentId).
test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
  }, workspaceId);
});

test('git: a registered repo shows up in the Git page', async ({ page }) => {
  await page.goto('/#/git');
  // Shell must boot first.
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  // With no tab open, the seeded repo renders in the landing repo grid
  // (.repo-name); if it had been opened it'd be a .git-tab-name. Either way its
  // name is in the DOM — assert the text is visible (poll: the global repo list
  // loads async on mount via git.loadAllRepos()).
  const repoName = page.getByText('e2e-repo', { exact: true });
  await expect(repoName.first()).toBeVisible({ timeout: 30_000 });
});

test('database: a registered redis connection shows up', async ({ page }) => {
  test.skip(redisConnId == null, 'redis-server unavailable — DB connection not seeded');

  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  // The DB Explorer's left sidebar lists connections (.conn-name within
  // .conn-list). Connections load via database.loadConnections() once
  // ws.currentId is set — which the seeded otto_workspace guarantees. Poll for
  // the seeded connection's name to appear.
  const conn = page.locator('.conn-list .conn-name', { hasText: 'e2e-redis' });
  await expect(conn.first()).toBeVisible({ timeout: 30_000 });
});

test('workspace: the seeded workspace is activated', async ({ page }, testInfo) => {
  await page.goto('/#/agents');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  // The Navigator lists workspaces; the active one is marked with `.active-ws`
  // and shows its name ("E2E WS"). Where the Navigator lives differs by device:
  //   - desktop/tablet (ipad-landscape): persistent in the sidebar, always shown.
  //   - phone (iphone-portrait): a LEFT drawer whose open-state is ui.railExpanded
  //     — which DEFAULTS to open on a fresh profile, so it's usually already up.
  // So: only open it if it isn't already showing the workspace item. (Toggling
  // blindly would CLOSE an already-open drawer.)
  const activeWs = page.locator('.nav-item.active-ws', { hasText: 'E2E WS' });
  const shownEarly = await activeWs
    .first()
    .isVisible()
    .catch(() => false);
  if (!shownEarly) {
    const openNav = page.getByRole('button', { name: 'Open navigator' });
    if (await openNav.isVisible().catch(() => false)) {
      // Raw click: opening the drawer immediately covers the button, which trips
      // Playwright's actionable-click "obscured" retry; dispatchEvent fires once.
      await openNav.dispatchEvent('click');
    }
  }

  // Assert the SEEDED workspace is the active one (not just present) — this is
  // the real signal that the app honoured otto_workspace on boot.
  await expect.poll(async () => activeWs.count(), { timeout: 30_000 }).toBeGreaterThan(0);
  await expect(activeWs.first()).toBeVisible({ timeout: 10_000 });

  void testInfo;
});
