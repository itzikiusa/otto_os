import { test, expect } from '@playwright/test';
import { apiCtx } from './seed';

test('real assistant history is bounded, pages without gaps and opens an older selection', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const endpoint = `${base}/api/v1/assistant/threads`;
  const ids: string[] = [];
  try {
    for (let i = 0; i < 105; i += 1) {
      const response = await ctx.post(endpoint, { data: { title: `Live history ${i}`, provider: 'codex' } });
      expect(response.ok(), await response.text()).toBeTruthy();
      ids.push((await response.json()).id);
    }
    const read = async (suffix: string) => {
      const response = await ctx.get(endpoint + suffix);
      expect(response.ok(), await response.text()).toBeTruthy();
      return await response.json() as { id: string }[];
    };
    const first = await read('');
    expect(first).toHaveLength(100);
    const second = await read('?limit=100&offset=100');
    expect(second).toHaveLength(5);
    const combined = [...first, ...second].map(row => row.id);
    expect(new Set(combined).size).toBe(105);
    expect(new Set(combined)).toEqual(new Set(ids));
    expect(await read('?limit=100&offset=105')).toEqual([]);
    const older = second[0].id;
    const index = ids.indexOf(older);
    await page.goto(`/#/assistant/${older}`);
    await expect(page.locator('[data-testid="page-header"] h1')).toHaveText(`Live history ${index}`);
    const more = page.getByRole('button', { name: /load more/i });
    await expect(more).toBeVisible();
    await more.click();
    await expect(more).toHaveCount(0);
    await expect(page.locator('[data-testid="page-header"] h1')).toHaveText(`Live history ${index}`);
  } finally { await ctx.dispose(); }
});
