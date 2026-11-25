import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { exerciseLongChat, seedLongChat, writeLongChat } from './long-chat-fixture';

// Phone: Chat on a long, real-shaped session (the 0.5.0 Chat switch answered
// "internal server error" on the owner's own daily session): Terminal ⇄ Chat
// opens on the newest turns fast, scrolling up pages older ones in, a live
// turn appends without a re-read, the composer sends.

let ctx: APIRequestContext;
let base = '';

test.beforeEach(async ({}, info) => {
  test.skip(!['iphone-portrait', 'android-phone'].includes(info.project.name), 'phone projects');
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
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
  const chat = writeLongChat();
  const id = await seedLongChat(ctx, base, wsId, chat);
  const m = await exerciseLongChat(page, id, chat, info.project.name, true);
  console.log(`[long-chat] ${JSON.stringify(m)}`);
  expect(m.firstPageBytes).toBeLessThan(3 << 20);
});
