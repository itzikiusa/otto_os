import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockedThread, shoot, stopFlow, threadWithTwoBackground, unsupportedFlow } from './threadsTaskStopFlow';

// ─────────────────────────────────────────────────────────────────────────────
// Threads: the person can stop background work and sees the truth about it
// (review C1–C4, D-06). A mocked thread whose turn ended with two background
// commands running; the per-task stop routes are mocked (the daemon under
// test predates them).
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base, 'Task Stop WS');
  await ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('stop one background task, Retry after a failure, asked state, Stop all; the row is calm, not working', async ({ page }, info) => {
  await mockedThread(page, workspaceId, 'stop-thread-a', 'Background stop');
  await threadWithTwoBackground(page, 'stop-thread-a');
  // D-06: the sidebar row is calm — background work running, the agent idle —
  // never "Working" (and not in the Working fold).
  const row = page.getByTestId('thread-row-stop-thread-a');
  await expect(row).toHaveAttribute('data-state', 'running');
  await expect(row).not.toHaveAttribute('data-class', 'working');
  await expect(row.getByTestId('thread-status-stop-thread-a')).not.toHaveText(/^Working/);
  await expect(row.locator('.tr-main')).toHaveAttribute('title', /2 background tasks running — the agent is idle/);
  await shoot(page, info, 'desktop-background-calm');
  await stopFlow(page, info, 'stop-thread-a', 'desktop');
});

test('a daemon without the stop route shows no dead Stop button', async ({ page }) => {
  await mockedThread(page, workspaceId, 'stop-thread-b', 'Old daemon');
  await threadWithTwoBackground(page, 'stop-thread-b');
  await unsupportedFlow(page, 'stop-thread-b');
});
