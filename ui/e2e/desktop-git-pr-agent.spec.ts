import { test, expect, type Page } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The PR's agent thread, embedded in the PR workspace (desktop-browser, MOCK
// mode — ui/src/lib/api/mock-runtime.ts answers the runtime routes). Explain
// opens the thread in the workspace's Detail pane (the Agent view) instead of
// navigating away; each PR has its own thread; it widens and narrows;
// detaching leaves an "Open here again" placeholder; files and diff lines can
// be asked about, each message carrying the PR's context. Before a thread
// exists the pane is the Threads start composer: agent, model and access are
// picked there, and New thread brings it back.
// ─────────────────────────────────────────────────────────────────────────────

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  });
  await page.setViewportSize({ width: 1600, height: 950 });
});

type MockLog = { creates: Record<string, unknown>[]; prompts: { id: string; text: string; access_mode: string; delivery?: string }[] };
// Review screenshots are opt-in: set OTTO_E2E_SHOTS to a folder to keep them.
const SHOTS = process.env.OTTO_E2E_SHOTS ? `${process.env.OTTO_E2E_SHOTS}/embed` : '';
const mockLog = (page: Page): Promise<MockLog> =>
  page.evaluate(() => (window as unknown as { __mockRuntime?: MockLog }).__mockRuntime ?? { creates: [], prompts: [] });

async function openPr(page: Page, n: number): Promise<void> {
  await page.evaluate((num) => { location.hash = `#/git/rep_otto/pr/${num}`; }, n);
  await expect(page.locator(`.prd[data-pr="${n}"] .prd-title`)).toBeVisible({ timeout: 15_000 });
}

async function menu(page: Page, item: string): Promise<void> {
  await page.getByRole('button', { name: 'More pull request actions' }).click();
  await page.locator('.ctx-menu').getByText(item, { exact: true }).click();
}

test('Explain opens the PR’s thread in place; another PR has its own; maximise and back; detach', async ({ page }) => {
  await page.goto('/#/git');
  await openPr(page, 51);
  const panel = page.getByTestId('pr-agent-panel');
  // The pane opens on the details until the agent is used; opening a PR creates nothing.
  await expect(panel).toHaveCount(0);
  await expect(page.getByTestId('pr-details-card')).toBeVisible();
  expect((await mockLog(page)).creates).toHaveLength(0);

  await menu(page, 'Explain this PR');
  await expect(panel.getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 10_000 });
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/git');
  await expect(page.locator('.prd-title')).toBeVisible(); // the PR stays beside it
  // The thread mounts as soon as it is created; the prompt follows it.
  await expect.poll(async () => (await mockLog(page)).prompts.length).toBe(1);
  const log = await mockLog(page);
  expect(log.creates).toHaveLength(1);
  expect(log.creates[0]).toMatchObject({
    source: 'pr-agent',
    origin: { kind: 'pr', id: 'rep_otto#51' },
    cwd: '/Users/dev/otto',
    access_mode: 'approval-required',
  });
  // The access mode rides on the create (the first message follows it).
  expect(log.prompts[0].text).toContain('cloned at /Users/dev/otto');
  expect(log.prompts[0].text).toContain('#51');

  // Maximise: the pane widens to all the room the PR leaves; ⋯ › Narrow brings it back.
  const pane = page.getByTestId('pr-pane');
  const w0 = (await pane.boundingBox())!.width;
  await panel.getByTestId('embedded-thread-maximize').click();
  await expect.poll(async () => (await pane.boundingBox())!.width).toBeGreaterThan(w0 + 100);
  await expect(page.locator('.prd-title')).toBeVisible();
  await pane.getByRole('button', { name: 'Agent · PR #51 actions' }).click();
  await page.locator('.ctx-menu').getByText('Narrow', { exact: true }).click();
  await expect.poll(async () => (await pane.boundingBox())!.width).toBeLessThan(w0 + 2);
  await expect(panel.getByTestId('embedded-runtime-thread')).toBeVisible();

  // Another PR: its own panel, empty — no thread until something is asked.
  await openPr(page, 42);
  const other = page.getByTestId('pr-agent-panel');
  await expect(other.getByRole('button', { name: 'Explain this PR' })).toBeVisible({ timeout: 10_000 });
  await expect(other.getByTestId('embedded-runtime-draft')).toBeVisible();
  await expect(other.locator('[data-testid^="runtime-thread-"]')).toHaveCount(0);
  expect((await mockLog(page)).creates).toHaveLength(1);

  // Back on #51 the same thread is found again (one thread per PR).
  await openPr(page, 51);
  await expect(panel.locator('[data-testid^="runtime-thread-"]')).toBeVisible({ timeout: 10_000 });
  expect((await mockLog(page)).creates).toHaveLength(1);

  // Detach: it opens in Threads; the PR keeps an "Open here again" placeholder.
  await panel.getByTestId('embedded-thread-detach').click();
  await expect.poll(() => page.evaluate(() => location.hash)).toMatch(/^#\/threads\//);
  await openPr(page, 51);
  await expect(panel.getByRole('button', { name: 'Open here again' })).toBeVisible();
  await panel.getByRole('button', { name: 'Open here again' }).click();
  await expect(panel.getByTestId('embedded-runtime-thread')).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

test('Fix asks first, names the folder, and runs with edits allowed', async ({ page }) => {
  await page.goto('/#/git');
  await openPr(page, 51);
  await menu(page, 'Fix findings with the agent…');
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('/Users/dev/otto');
  await expect(dialog).toContainText('does not commit, push, or post');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  expect((await mockLog(page)).creates).toHaveLength(0);

  await menu(page, 'Fix findings with the agent…');
  await page.getByRole('dialog').getByRole('button', { name: 'Start fixing' }).click();
  await expect(page.getByTestId('pr-agent-panel').getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 10_000 });
  await expect.poll(async () => (await mockLog(page)).prompts.length).toBe(1);
  const log = await mockLog(page);
  expect(log.creates[0]).toMatchObject({ access_mode: 'auto-accept-edits' });
  expect(log.prompts[0].text).toContain('You may edit files in /Users/dev/otto only');
});

test('Explain a file from the Files panel and lines from the diff', async ({ page }) => {
  await page.goto('/#/git');
  await openPr(page, 51);
  await page.getByRole('tab', { name: 'Files', exact: true }).click(); // the tree shows beside Files
  const files = page.getByRole('region', { name: 'Files in pull request #51' });
  const first = files.locator('.nav-file').first();
  const path = (await first.getAttribute('title')) ?? '';
  await first.locator('.nav-file-open').click({ button: 'right' });
  await page.locator('.ctx-menu').getByText('Explain this file', { exact: true }).click();
  await expect(page.getByTestId('pr-agent-panel').getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 10_000 });
  await expect.poll(async () => (await mockLog(page)).prompts.at(-1)?.text ?? '').toContain(path);

  // Lines: select two code rows of an expanded file, then the action beside it.
  await page.getByRole('tab', { name: 'Files', exact: true }).click();
  await page.locator('.prd-code .dfile-head').first().click();
  const rows = page.locator('.prd-code .vrow[data-rk]');
  await expect(rows.nth(1)).toBeVisible();
  await page.evaluate(() => {
    const els = document.querySelectorAll('.prd-code .vrow[data-rk] .code');
    const r = document.createRange();
    r.setStart(els[0], 0);
    r.setEnd(els[1], els[1].childNodes.length);
    const sel = document.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(r);
  });
  const pick = page.getByTestId('pr-agent-line-pick');
  await expect(pick).toBeVisible();
  await expect(pick).toContainText(/Explain (old )?lines? \d+/);
  await pick.click();
  await expect.poll(async () => (await mockLog(page)).prompts.at(-1)?.text ?? '').toMatch(/explain (old )?lines? \d+/);
  expect((await mockLog(page)).creates).toHaveLength(1); // the same thread
});

test('at 1440×800 an ask opens the closed pane beside the PR; the list and the files keep their column', async ({ page }) => {
  // Closed on purpose last time: a new ask must still open it.
  await page.addInitScript(() => localStorage.setItem('otto.pane.git.pr.view', ''));
  await page.setViewportSize({ width: 1440, height: 800 });
  await page.goto('/#/git');
  await openPr(page, 51);
  await expect(page.getByTestId('pr-pane')).toHaveCount(0);
  await menu(page, 'Explain this PR');
  const panel = page.getByTestId('pr-agent-panel');
  await expect(panel.getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 10_000 });
  await expect(page.getByRole('dialog', { name: 'Agent · PR #51' })).toHaveCount(0); // inline, not a sheet
  const box = (await page.getByTestId('pr-pane').boundingBox())!;
  expect(box.y + box.height).toBeLessThanOrEqual(800);
  await expect(page.locator('.pr-search')).toBeVisible();
  await page.getByRole('tab', { name: 'Files', exact: true }).click(); // the tree shows beside Files
  await expect(page.getByRole('region', { name: 'Files in pull request #51' })).toBeVisible();
  await expect(panel.getByTestId('embedded-runtime-thread')).toBeVisible();
});

test('before a thread the dock is the start composer: pick the agent, model and access, then ask; New thread starts over', async ({ page }) => {
  await page.goto('/#/git');
  await openPr(page, 51);
  const panel = page.getByTestId('pr-agent-panel');
  await page.getByTestId('pr-agent-open').click();
  const draft = panel.getByTestId('embedded-runtime-draft');
  await expect(draft).toBeVisible({ timeout: 10_000 });
  await expect(draft.getByRole('button', { name: 'Explain this PR' })).toBeVisible();
  await expect(draft.getByTestId('runtime-access')).toContainText('Supervised');
  await expect(draft.getByTestId('embedded-draft-access-hint')).toContainText('asks before');
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/pr-dock-draft-light.png`, animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/pr-dock-draft-dark.png`, animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });

  // Another agent and a non-default model, in the one picker Threads uses.
  await draft.getByTestId('picker-agent').click();
  const picker = page.getByTestId('model-picker');
  await expect(picker).toBeVisible();
  await expectFullyInViewport(page, picker, 'model picker');
  await picker.getByTestId('model-picker-provider-codex').click();
  const row = picker.locator('.mp-row:not([disabled])').filter({ hasNotText: 'Default model' }).first();
  await row.click();
  if (await picker.isVisible()) await page.keyboard.press('Escape');
  await expect(draft.getByTestId('picker-agent')).toHaveAttribute('aria-label', /Codex/);

  // Edits allowed: asked first, and the footer says what it now means.
  await draft.getByTestId('runtime-access').click();
  await page.locator('.ctx-menu').getByText('Auto-accept edits', { exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Allow edits' }).click();
  await expect(draft.getByTestId('runtime-access')).toContainText('Auto-accept edits');
  await expect(draft.getByTestId('embedded-draft-access-hint')).toContainText('changes files without asking');

  await draft.getByTestId('new-thread-input').fill('Is the retry loop bounded?');
  await draft.getByTestId('new-thread-input').press('Enter');
  await expect(panel.getByTestId('embedded-runtime-thread').locator('[data-testid^="runtime-thread-"]')).toBeVisible({ timeout: 10_000 });
  await expect.poll(async () => (await mockLog(page)).prompts.length).toBe(1);
  const log = await mockLog(page);
  expect(log.creates).toHaveLength(1);
  expect(log.creates[0]).toMatchObject({ provider: 'codex', access_mode: 'auto-accept-edits', source: 'pr-agent', origin: { kind: 'pr', id: 'rep_otto#51' } });
  expect(typeof log.creates[0].model).toBe('string');
  expect(log.prompts[0].text).toContain('Is the retry loop bounded?');
  expect(log.prompts[0].text).toContain('cloned at /Users/dev/otto'); // the PR's context rides along
  // The thread's own chip shows the mode it really runs in.
  await expect(panel.getByTestId('runtime-access')).toContainText('Auto-accept edits');

  // New thread: this one closes, the start composer comes back.
  await panel.getByTestId('embedded-thread-new').click();
  await expect(panel.getByTestId('embedded-runtime-draft')).toBeVisible();
  await expect(panel.getByTestId('runtime-access')).toContainText('Supervised');
  await expectNoHorizontalOverflow(page);
});

test('at 1100 px (the pane is a sheet) the start composer keeps both chips reachable', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 800 });
  await page.goto('/#/git');
  await openPr(page, 51);
  const panel = page.getByTestId('pr-agent-panel');
  await page.getByTestId('pr-agent-open').click();
  await expect(page.getByRole('dialog', { name: 'Agent · PR #51' })).toBeVisible();
  const draft = panel.getByTestId('embedded-runtime-draft');
  await expect(draft).toBeVisible({ timeout: 10_000 });
  for (const id of ['picker-agent', 'runtime-access', 'new-thread-send-btn']) {
    await expectFullyInViewport(page, draft.getByTestId(id), id);
    const b = (await draft.getByTestId(id).boundingBox())!;
    const d = (await draft.boundingBox())!;
    expect(b.x + b.width).toBeLessThanOrEqual(d.x + d.width + 1);
  }
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/pr-dock-narrow-light.png`, animations: 'disabled' });
  await draft.getByTestId('runtime-access').click();
  await expectFullyInViewport(page, page.locator('.ctx-menu'), 'access menu');
  await page.keyboard.press('Escape');
  await expectNoHorizontalOverflow(page);
});
