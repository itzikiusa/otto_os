import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

test.use({ serviceWorkers: 'block' });
for (const kind of ['discovery', 'refinement'] as const) {
  for (const scenario of ['clear', 'late-failure'] as const) {
    test(`${kind} chat preserves the latest draft decision: ${scenario}`, async ({ page }) => {
      const { ctx, base } = await apiCtx();
      const ws = await seedWorkspace(ctx, base);
      const title = `R16 ${kind} ${scenario} ${Date.now()}`;
      const draft = await ctx.post(`${base}/api/v1/workspaces/${ws}/product/drafts`, { data: { title } });
      expect(draft.ok()).toBeTruthy(); const sid = (await draft.json()).story.id;
      const endpoint = kind === 'discovery' ? 'discovery-chats' : 'refinement-threads';
      const selector = kind === 'discovery' ? '.discovery-chat' : '.refine-chat';
      const row = (id: string) => ({ id, story_id: sid, workspace_id: ws, title: `Conversation ${id}`, status: 'active', model: null, created_at: '2026-10-08T00:00:00Z', updated_at: '2026-10-08T00:00:00Z' });
      await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), ws);
      await page.context().route(`**/product/stories/${sid}/${endpoint}`, r => r.fulfill({ json: ['A', 'B'].map(row) }));
      await page.context().route(`**/product/${endpoint}/*`, r => {
        const id = r.request().url().split('/').at(-1)!;
        return r.fulfill({ json: { [kind === 'discovery' ? 'chat' : 'thread']: row(id), messages: [] } });
      });
      let release!: () => void; let requested!: () => void;
      const held = new Promise<void>(r => release = r), started = new Promise<void>(r => requested = r);
      await page.context().route(`**/product/${endpoint}/A/messages`, async r => { requested(); await held; await r.fulfill({ status: 503, json: { code: 'upstream', message: 'Fixture send failed' } }); });
      await openPage(page, 'product');
      await page.locator('.story-row', { hasText: title }).click();
      await page.getByRole('tab', { name: 'Discover', exact: true }).click();
      await page.locator('.sub-tab-strip .st', { hasText: kind === 'discovery' ? 'Chat' : 'Refine' }).click();
      const input = page.locator(`${selector} textarea`);
      await expect(input).toBeVisible();
      await input.fill('Old A text');
      if (scenario === 'late-failure') {
        await page.locator(selector).getByRole('button', { name: 'Send', exact: true }).click();
        await started;
      }
      await page.getByRole('button', { name: /Conversation B/ }).click();
      await expect(input).toHaveValue('');
      await page.getByRole('button', { name: /Conversation A/ }).click();
      await input.fill(scenario === 'clear' ? '' : 'Newer A draft');
      await page.getByRole('button', { name: /Conversation B/ }).click();
      if (scenario === 'late-failure') {
        const delivered = page.waitForResponse(`**/product/${endpoint}/A/messages`);
        release(); await (await delivered).finished();
      }
      await page.getByRole('button', { name: /Conversation A/ }).click();
      await expect(input).toHaveValue(scenario === 'clear' ? '' : 'Newer A draft');
      await ctx.dispose();
    });
  }
}
