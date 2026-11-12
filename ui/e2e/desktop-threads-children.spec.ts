import { test, expect, type Page } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Threads inside a thread (desktop-browser, MOCK mode — ui/src/lib/api/
// mock-runtime.ts serves POST /runtime-sessions/{id}/children and writes the
// parent's notices):
//   1. "Ask another agent" in a thread's composer: Codex picked as a chip, the
//      access it gets in words, Send asks it (never this thread's agent); the
//      child's live card shows in the parent's timeline and expands its answer
//      inline; the sidebar nests it under the parent ("· 1", foldable);
//      "Open beside" splits; its finished turn comes back as one line;
//   2. typing `@cod` offers Codex as a chip;
//   3. a full group is refused in words above the composer.
// ─────────────────────────────────────────────────────────────────────────────

type Hooks = {
  seed: (row: Record<string, unknown>) => string;
  say: (sid: string, texts: string[]) => void;
  childDone: (sid: string, line: string) => void;
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

async function openLead(page: Page): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.seed === 'function');
  await page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: Hooks }).__mockRuntime;
    m.seed({ id: 'rt_lead', title: 'Fix the build', provider: 'claude', last_active_at: '2026-11-12T10:09:00Z' });
    m.say('rt_lead', ['The build fails in the linker step.']);
    location.hash = '#/threads/rt_lead';
  });
  await expect(page.getByTestId('runtime-thread-rt_lead')).toBeVisible({ timeout: 15_000 });
}

test('Ask another agent opens a linked child shown live in the parent and nested in the sidebar', async ({ page }) => {
  await openLead(page);
  const thread = page.getByTestId('runtime-thread-rt_lead');

  await thread.getByTestId('runtime-ask-agent').click();
  const picker = page.getByTestId('model-picker');
  await picker.getByTestId('model-picker-provider-codex').click();
  await picker.getByRole('button', { name: 'Default model' }).click();
  const chips = thread.getByTestId('runtime-ask-targets');
  await expect(chips).toContainText('Codex');
  await expect(chips).toContainText('asks before it edits, like this thread');

  await thread.getByTestId('runtime-composer').fill('Why does the linker fail?');
  await thread.getByTestId('runtime-send').click();

  // Asked as a new linked thread — not sent to this thread's agent.
  const read = () => page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: { asks?: { id: string; prompt: string; targets: { provider: string }[] }[]; prompts: { id: string }[] } }).__mockRuntime;
    return { asks: m.asks ?? [], prompts: m.prompts.filter((p) => p.id === 'rt_lead').length };
  });
  // The ask is sent asynchronously after the click: wait for it to land.
  await expect.poll(async () => (await read()).asks.length).toBe(1);
  const sent = await read();
  expect(sent.asks[0]).toMatchObject({ id: 'rt_lead', prompt: 'Why does the linker fail?', targets: [{ provider: 'codex' }] });
  expect(sent.prompts).toBe(0);
  await expect(chips).toHaveCount(0);
  await expect(thread.getByTestId('runtime-composer')).toHaveValue('');

  // The live card in the parent's timeline: provider, state, the answer inline.
  const card = thread.locator('[data-testid^="runtime-child-row-"]').first();
  await expect(card).toBeVisible();
  await expect(card).toContainText('Done');
  await card.locator('button[aria-expanded]').click();
  await expect(thread.locator('[data-testid^="runtime-child-answer-"]')).toContainText('Mock codex: here is my answer.');

  // The sidebar nests the child under its parent, foldable.
  await expect(page.getByTestId('thread-family-count-rt_lead')).toHaveText('· 1');
  const childId = (await card.getAttribute('data-testid'))!.replace('runtime-child-row-', '');
  await expect(page.getByTestId(`thread-row-${childId}`)).toBeVisible();
  await page.getByTestId('thread-family-fold-rt_lead').click();
  await expect(page.getByTestId(`thread-row-${childId}`)).toHaveCount(0);
  await page.getByTestId('thread-family-fold-rt_lead').click();
  await expect(page.getByTestId(`thread-row-${childId}`)).toBeVisible();

  // Open beside: parent and child side by side.
  await card.getByTestId('runtime-item-beside').click();
  await expect(page.getByTestId(`runtime-thread-${childId}`)).toBeVisible();
  await expect(page.getByTestId('runtime-thread-rt_lead')).toBeVisible();

  // Its finished turn comes back to the parent as one line with Open.
  await page.evaluate((id) => (window as unknown as { __mockRuntime: Hooks }).__mockRuntime.childDone(id, 'The linker needs -lm.'), childId);
  await expect(thread.getByTestId(`runtime-child-notice-${childId}`)).toContainText('finished: The linker needs -lm.');
});

test('typing @cod offers Codex as a chip', async ({ page }) => {
  await openLead(page);
  const thread = page.getByTestId('runtime-thread-rt_lead');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('check this @cod');
  const menu = thread.getByTestId('runtime-ask-mentions');
  await expect(menu).toBeVisible();
  await expect(menu.getByRole('option').first()).toContainText('Codex');
  await composer.press('Enter');
  await expect(thread.getByTestId('runtime-ask-targets')).toContainText('Codex');
  await expect(composer).toHaveValue('check this ');
});

test('a full group is refused in words above the composer', async ({ page }) => {
  await openLead(page);
  await page.evaluate(() => {
    const m = (window as unknown as { __mockRuntime: Hooks }).__mockRuntime;
    for (let i = 0; i < 8; i += 1) m.seed({ id: `rt_full${i}`, title: `Worker ${i}`, provider: 'codex', parent_id: 'rt_lead' });
  });
  const thread = page.getByTestId('runtime-thread-rt_lead');
  const composer = thread.getByTestId('runtime-composer');
  await composer.fill('one more @gem');
  await composer.press('Enter');
  await composer.fill('Another look please');
  await thread.getByTestId('runtime-send').click();
  await expect(thread.locator('.rt-banner')).toContainText('close one before asking another agent');
  await expect(thread.getByTestId('runtime-ask-targets')).toBeVisible();
});
