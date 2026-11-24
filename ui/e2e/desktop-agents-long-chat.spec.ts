import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { budgetMs } from './perf';
import { exerciseLongChat, seedLongChat, writeLongChat } from './long-chat-fixture';

// Chat on a long, real-shaped session (the 0.5.0 Chat switch answered
// "internal server error" on the owner's own daily session): Terminal ⇄ Chat
// opens on the newest turns fast, scrolling up pages older ones in, a live
// turn appends without a re-read, the composer sends.

let ctx: APIRequestContext;
let base = '';

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  ({ ctx, base } = await apiCtx());
});
test.afterEach(async () => {
  await ctx?.dispose();
});

test('a 60 MB session opens in Chat on its newest turns and pages back', async ({ page }, info) => {
  test.setTimeout(180_000);
  const wsId = await seedWorkspace(ctx, base);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsId);
  const chat = writeLongChat();
  const id = await seedLongChat(ctx, base, wsId, chat);
  const m = await exerciseLongChat(page, id, chat, info.project.name);
  console.log(`[long-chat] ${JSON.stringify(m)}`);
  expect(m.firstPaintMs, 'Chat paints its first turn fast').toBeLessThan(budgetMs(2_000));
  expect(m.firstPageBytes).toBeLessThan(3 << 20);
});
