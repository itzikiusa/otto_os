import { test, expect, type Page } from '@playwright/test';

// Emoji in Threads (desktop-browser, MOCK mode — ui/src/lib/api/mock-runtime.ts):
//   1. the composer turns `:) ` into 🙂 on the space; Backspace deletes the
//      space, the next one restores `:)`; never inside inline code;
//   2. `:roc` lists shortcodes, Enter inserts the emoji (no send);
//   3. agent replies show `:rocket:` as 🚀 — or as written with the
//      "Show emoji in agent replies" setting off.

type Hooks = {
  seed: (row: Record<string, unknown>) => string;
  say: (sid: string, texts: string[]) => void;
};

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
  });
  await page.setViewportSize({ width: 1400, height: 900 });
});

async function openThread(page: Page): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.seed === 'function');
  await page.evaluate(() => {
    (window as unknown as { __mockRuntime: Hooks }).__mockRuntime.seed({ id: 'rt_emoji', title: 'Emoji thread' });
    location.hash = '#/threads';
  });
  await expect(page.getByTestId('runtime-thread-rt_emoji')).toBeVisible({ timeout: 15_000 });
}

test('emoticons convert on space and Backspace restores them', async ({ page }) => {
  await openThread(page);
  const box = page.getByTestId('runtime-composer');
  await box.click();

  await box.pressSequentially('hi :)');
  await expect(box).toHaveValue('hi :)');
  await box.press(' ');
  await expect(box).toHaveValue('hi 🙂 ');
  await box.press('Backspace');
  await expect(box).toHaveValue('hi 🙂');
  await box.press('Backspace');
  await expect(box).toHaveValue('hi :)');
  await box.pressSequentially(' and <3 ');
  await expect(box).toHaveValue('hi 🙂 and ❤️ ');

  await box.fill('');
  await box.pressSequentially('`x :) ');
  await expect(box).toHaveValue('`x :) ');
  await box.fill('');
  await box.pressSequentially('a:) ');
  await expect(box).toHaveValue('a:) ');
});

test('`:` shortcodes list emoji and Enter inserts one', async ({ page }) => {
  await openThread(page);
  const box = page.getByTestId('runtime-composer');
  await box.click();
  await box.pressSequentially('ship it :rocke');
  const pop = page.getByTestId('emoji-pop');
  await expect(pop).toBeVisible();
  await expect(pop.getByRole('option', { name: /:rocket:/ })).toBeVisible();
  await box.press('Enter');
  await expect(box).toHaveValue('ship it 🚀 ');
  await expect(pop).toHaveCount(0);

  await box.pressSequentially(':smi');
  await expect(pop).toBeVisible();
  await box.press('Escape');
  await expect(pop).toHaveCount(0);
  await expect(box).toHaveValue('ship it 🚀 :smi');
});

test('agent replies render shortcodes, never emoticons', async ({ page }) => {
  await openThread(page);
  await page.evaluate(() => (window as unknown as { __mockRuntime: Hooks }).__mockRuntime.say('rt_emoji', ['Shipped :rocket: `:fire:` :)']));
  const thread = page.getByTestId('runtime-thread-rt_emoji');
  await expect(thread.getByText('Shipped 🚀')).toBeVisible();
  await expect(thread.locator('code', { hasText: ':fire:' })).toBeVisible();
  await expect(thread.getByText('🙂')).toHaveCount(0);
});

test('with "Show emoji in agent replies" off, shortcodes stay as written', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('otto_agent_emoji', '0'));
  await openThread(page);
  await page.evaluate(() => (window as unknown as { __mockRuntime: Hooks }).__mockRuntime.say('rt_emoji', ['Shipped :rocket:']));
  await expect(page.getByTestId('runtime-thread-rt_emoji').getByText('Shipped :rocket:')).toBeVisible();
});
