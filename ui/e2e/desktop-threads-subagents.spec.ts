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
  // Place 4: thread details' Subagents section (one source, childThreads.ts).
  const subs = page.getByTestId('details-subagents');
  if (await subs.count()) {
    await expect(subs).toContainText('Subagents · 2 working');
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await subs.screenshot({ path: test.info().outputPath(`details-subagents-desktop-${scheme}.png`), animations: 'disabled' });
    }
    await page.emulateMedia({ colorScheme: 'light' });
  }
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`subagents-row-desktop-${scheme}.png`), animations: 'disabled' });
  }
});

test('a subagent’s approval is answered in the parent, and one Stop stops the family', async ({ page }, info) => {
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
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`subagent-approval-desktop-${scheme}.png`), animations: 'disabled' });
  }
  await card.getByRole('button', { name: /^Approve/ }).first().click();
  const answers = () => page.evaluate(() => ((window as unknown as { __mockRuntime: { answers?: { id: string; decision: string }[] } }).__mockRuntime.answers ?? []));
  await expect.poll(answers).toEqual([expect.objectContaining({ id: '01M84S0000000000000000000A', decision: 'accept' })]);
  await expect(card).toHaveCount(0);

  await thread.getByTestId('runtime-bg-stop').click();
  const stops = () => page.evaluate(() => ((window as unknown as { __mockRuntime: { familyStops?: unknown[] } }).__mockRuntime.familyStops ?? []));
  await expect.poll(stops).toEqual([{ id: 'rt_ap_lead', background: true }]);
});

test('a subagent opens as a tab beside its parent; ⌘-click and the setting open it in the main view', async ({ page }, info) => {
  await delegated(page);
  const thread = page.getByTestId('runtime-thread-rt_sub_lead');
  await thread.getByTestId('runtime-child-group-toggle').click();
  await thread.getByTestId('runtime-child-row-01M84S0000000000000000000C').getByTestId('runtime-item-open').click();
  const pane = page.getByTestId('runtime-subagent-pane');
  await expect(pane).toBeVisible();
  await expect(pane.getByTestId('runtime-subagent-of')).toContainText('Subagent of Share links');
  await expect(pane.getByTestId('runtime-subagent-of')).toContainText('Share link docs');
  // Its first message reads as the task it was given, from its parent.
  await expect(pane.getByTestId('rt-from').first()).toContainText('Task from Share links');
  await expect(pane).not.toContainText('From lead');
  await expect(page.getByTestId('threads-tab-subagent')).toContainText('Share link docs');
  await expect(thread).toBeVisible();
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: info.outputPath(`subagent-tab-desktop-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });

  // ⌘-click: the main view, with the fixed "Subagent of …" line.
  await thread.getByTestId('runtime-child-row-01M84S0000000000000000000A').getByTestId('runtime-item-open').click({ modifiers: ['ControlOrMeta'] });
  await expect(page.locator('main, body').getByTestId('runtime-thread-01M84S0000000000000000000A').first()).toBeVisible();
  await expect(page.getByTestId('runtime-subagent-of').first()).toContainText('Subagent of Share links');
  await page.getByTestId('threads-back-to-parent').first().click();
  await expect(page.getByTestId('runtime-thread-rt_sub_lead')).toBeVisible();

  // The one per-device setting (Settings ▸ Threads) flips the default.
  await page.evaluate(() => {
    const k = 'otto_threads_chrome_v1';
    const v = JSON.parse(localStorage.getItem(k) ?? '{}');
    localStorage.setItem(k, JSON.stringify({ ...v, subagentsIn: 'main' }));
  });
  await delegated(page);
  await page.getByTestId('runtime-thread-rt_sub_lead').getByTestId('runtime-child-group-toggle').click();
  await page.getByTestId('runtime-thread-rt_sub_lead').getByTestId('runtime-child-row-01M84S0000000000000000000B').getByTestId('runtime-item-open').click();
  await expect(page.getByTestId('threads-back-to-parent').first()).toBeVisible();
  await expect(page.getByTestId('runtime-subagent-of').first()).toContainText('Share link tests');
});

test('a subagent’s question its parent’s agent did not answer shows in the parent as “Question from …”', async ({ page }) => {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.delegate === 'function');
  await page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: Hooks & { question: (sid: string, q: string) => void } }).__mockRuntime;
    m.seed({ id: 'rt_q_lead', title: 'Ship it', provider: 'claude', last_active_at: '2026-11-23T10:09:00Z' });
    m.delegate('rt_q_lead', [{ id: '01M84S0000000000000000000D', provider: 'codex', title: 'Pick the store', working: true }]);
    m.question('01M84S0000000000000000000D', 'Which database?');
    location.hash = '#/threads/rt_q_lead';
  });
  const thread = page.getByTestId('runtime-thread-rt_q_lead');
  await expect(thread).toBeVisible({ timeout: 15_000 });
  const card = thread.getByTestId('runtime-child-question');
  await expect(card).toContainText('Question from Pick the store');
  await expect(card).toContainText('Which database?');
  await expect(thread.getByTestId('runtime-child-approval')).toHaveCount(0);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: test.info().outputPath(`subagent-question-desktop-${scheme}.png`), animations: 'disabled' });
  }
});
