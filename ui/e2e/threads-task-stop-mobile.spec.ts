import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
import { mockedThread, shoot, stopFlow, threadWithTwoBackground, unsupportedFlow } from './threadsTaskStopFlow';

// The same per-task stop flow as desktop-threads-task-stop.spec.ts, on a phone:
// every Stop / Retry / Stop all is reachable at phone width and nothing
// scrolls sideways; the thread whose turn ended says it waits on background
// work instead of looking like a running turn.

test.use({ serviceWorkers: 'block' });

let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base, 'Task Stop Phone WS');
  await ctx.dispose();
});

test('phone: stop one, Retry, asked, Stop all — all reachable', async ({ page }, info) => {
  await mockedThread(page, workspaceId, 'stop-thread-m', 'Phone background stop');
  const thread = await threadWithTwoBackground(page, 'stop-thread-m');
  await expect(thread.getByTestId('runtime-bg-waiting')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await shoot(page, info, `${info.project.name}-background-calm`);
  await stopFlow(page, info, 'stop-thread-m', info.project.name);
});

test('phone: no dead Stop button on a daemon without the route', async ({ page }) => {
  await mockedThread(page, workspaceId, 'stop-thread-n', 'Phone old daemon');
  await threadWithTwoBackground(page, 'stop-thread-n');
  await unsupportedFlow(page, 'stop-thread-n');
});
