import { test, expect, type Page } from '@playwright/test';

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
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
  });
  await page.setViewportSize({ width: 1600, height: 1000 });
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

test('one row for the turn’s subagents and one waiting line', async ({ page }, info) => {
  await delegated(page);
  const thread = page.getByTestId('runtime-thread-rt_sub_lead');
  const groups = thread.getByTestId('runtime-child-group');
  await expect(groups).toHaveCount(1);
  const head = groups.getByTestId('runtime-child-group-toggle');
  await expect(head).toContainText('3 subagents');
  await expect(groups.getByTestId('runtime-child-group-summary')).toHaveText('2 working · 1 done');
  // Folded: no per-subagent rows until opened.
  await expect(thread.locator('[data-testid^="runtime-child-row-"]')).toHaveCount(0);
  await head.click();
  await expect(head).toHaveAttribute('aria-expanded', 'true');
  await expect(thread.locator('[data-testid^="runtime-child-row-"]')).toHaveCount(3);
  const done = thread.getByTestId('runtime-child-row-01M84S0000000000000000000C');
  await expect(done).toContainText('Done');
  await expect(done).toContainText('Docs written');
  await expect(thread.getByTestId('runtime-child-row-01M84S0000000000000000000A')).toContainText('Working');
  // The full answer stays expandable inline.
  await done.locator('button[aria-expanded]').click();
  await expect(thread.getByTestId('runtime-child-answer-01M84S0000000000000000000C')).toContainText('README and help page');

  // One line at the end of the timeline; subagents are not "background tasks".
  const wait = thread.getByTestId('runtime-bg-waiting');
  await expect(wait).toContainText('Waiting on 2 subagents');
  await expect(wait).toContainText('Share link API, Share link tests');
  await expect(wait.getByTestId('runtime-bg-stop')).toBeVisible();
  await expect(thread.getByTestId('runtime-background')).toHaveCount(0);
  await expect(thread).not.toContainText('background task');
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`subagents-row-desktop-${scheme}.png`), animations: 'disabled' });
  }
});
