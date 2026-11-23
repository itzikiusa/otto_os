import { test, expect, type Page } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';

// Subagents of a thread (MOCK mode — mock-runtime.ts `delegate`): the
// agent's delegations of one turn are ONE timeline row (stacked provider
// icons, "3 subagents", the tally in words) that opens to one row each, and
// the line under the timeline reads "Waiting on 2 subagents · <titles> ·
// Stop"; the strip above the composer keeps only commands and timers.

type Hooks = {
  seed: (row: Record<string, unknown>) => string;
  say: (sid: string, texts: string[]) => void;
  delegate: (parent: string, kids: { id: string; provider: string; title: string; working?: boolean; answer?: string }[]) => void;
};

test.beforeEach(async ({ page }, ti) => {
  test.skip(!/^(iphone|android)/.test(ti.project.name), 'phones only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
  });
});

async function delegated(page: Page): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.delegate === 'function');
  await page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: Hooks }).__mockRuntime;
    m.seed({ id: 'rt_sub_lead', title: 'Share links', provider: 'claude', last_active_at: '2026-11-23T10:09:00Z' });
    m.say('rt_sub_lead', ['Okay, opening Codex, Gemini and Muse.']);
    m.delegate('rt_sub_lead', [
      { id: '01M84S0000000000000000000A', provider: 'codex', title: 'Share link API', working: true },
      { id: '01M84S0000000000000000000B', provider: 'gemini', title: 'Share link tests', working: true },
      { id: '01M84S0000000000000000000C', provider: 'muse', title: 'Share link docs', answer: 'Docs written: README and help page.' },
    ]);
    location.hash = '#/threads/rt_sub_lead';
  });
  await expect(page.getByTestId('runtime-thread-rt_sub_lead')).toBeVisible({ timeout: 15_000 });
}

test('the subagent row and the waiting line on a phone, by touch', async ({ page }, info) => {
  await delegated(page);
  const thread = page.getByTestId('runtime-thread-rt_sub_lead');
  const head = thread.getByTestId('runtime-child-group-toggle');
  await expect(head).toContainText('3 subagents');
  const box = await head.boundingBox();
  expect(box!.height).toBeGreaterThanOrEqual(44);
  await head.tap();
  await expect(thread.locator('[data-testid^="runtime-child-row-"]')).toHaveCount(3);
  await thread.getByTestId('runtime-child-row-01M84S0000000000000000000C').locator('button[aria-expanded]').tap();
  await expect(thread.getByTestId('runtime-child-answer-01M84S0000000000000000000C')).toContainText('README and help page');
  await expect(thread.getByTestId('runtime-bg-waiting')).toContainText('Waiting on 2 subagents');
  await expectNoHorizontalOverflow(page);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`subagents-row-phone-${scheme}.png`), animations: 'disabled' });
  }
});
