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

test('a subagent’s approval is answered in the parent, and one Stop stops the family', async ({ page }) => {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.delegate === 'function');
  await page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: Hooks & { approval: (sid: string, cmd: string) => void } }).__mockRuntime;
    m.seed({ id: 'rt_ap_lead', title: 'Ship it', provider: 'claude', last_active_at: '2026-11-23T10:09:00Z' });
    m.delegate('rt_ap_lead', [
      { id: '01M84S0000000000000000000A', provider: 'codex', title: 'Run the migration', working: true },
      { id: '01M84S0000000000000000000B', provider: 'gemini', title: 'Write the notes', working: true },
    ]);
    m.approval('01M84S0000000000000000000A', 'npm run migrate');
    location.hash = '#/threads/rt_ap_lead';
  });
  const thread = page.getByTestId('runtime-thread-rt_ap_lead');
  await expect(thread).toBeVisible({ timeout: 15_000 });
  const card = thread.getByTestId('runtime-child-approval');
  await expect(card).toContainText('Approval needed from Run the migration');
  await expect(card).toContainText('npm run migrate');
  await card.getByRole('button', { name: /^Approve/ }).first().tap();
  const answers = () => page.evaluate(() => ((window as unknown as { __mockRuntime: { answers?: { id: string; decision: string }[] } }).__mockRuntime.answers ?? []));
  await expect.poll(answers).toEqual([expect.objectContaining({ id: '01M84S0000000000000000000A', decision: 'accept' })]);
  await expect(card).toHaveCount(0);

  await thread.getByTestId('runtime-bg-stop').tap();
  const stops = () => page.evaluate(() => ((window as unknown as { __mockRuntime: { familyStops?: unknown[] } }).__mockRuntime.familyStops ?? []));
  await expect.poll(stops).toEqual([{ id: 'rt_ap_lead', background: true }]);
});

test('a subagent opens as a sheet over its parent on a phone, by touch', async ({ page }, info) => {
  await delegated(page);
  const thread = page.getByTestId('runtime-thread-rt_sub_lead');
  await thread.getByTestId('runtime-child-group-toggle').tap();
  await thread.getByTestId('runtime-child-row-01M84S0000000000000000000C').getByTestId('runtime-item-open').tap();
  const pane = page.getByTestId('runtime-subagent-pane');
  await expect(pane).toBeVisible();
  await expect(pane.getByTestId('runtime-subagent-of')).toContainText('Subagent of Share links');
  await expectNoHorizontalOverflow(page);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`subagent-tab-phone-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
  await pane.getByTestId('threads-back-to-parent').tap();
  await expect(page.getByTestId('runtime-thread-rt_sub_lead')).toBeVisible();
});
