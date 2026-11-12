import { test, expect, type Page } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The PR's agent thread, embedded in the PR workspace (desktop-browser, MOCK
// mode — ui/src/lib/api/mock-runtime.ts answers the runtime routes). Explain
// opens the thread in the left column instead of navigating away; each PR has
// its own thread; it maximises over the workspace and comes back; detaching
// leaves an "Open here again" placeholder; files and diff lines can be asked
// about, each message carrying the PR's context.
// ─────────────────────────────────────────────────────────────────────────────

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  });
  await page.setViewportSize({ width: 1600, height: 950 });
});

type MockLog = { creates: Record<string, unknown>[]; prompts: { id: string; text: string; access_mode: string }[] };
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
  // Folded until first used, and nothing is created by opening a PR.
  await expect(panel.getByRole('button', { name: 'Agent' })).toHaveAttribute('aria-expanded', 'false');
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
  expect(log.prompts[0].access_mode).toBe('approval-required');
  expect(log.prompts[0].text).toContain('cloned at /Users/dev/otto');
  expect(log.prompts[0].text).toContain('#51');

  // Maximise: the conversation takes the workspace; Back returns to the PR.
  await panel.getByTestId('embedded-thread-maximize').click();
  await expect(page.locator('.prw-detail')).toBeHidden();
  await expect(panel.getByTestId('embedded-runtime-thread')).toBeVisible();
  await panel.getByTestId('pr-agent-restore').click();
  await expect(page.locator('.prd-title')).toBeVisible();
  await expect(panel.getByTestId('embedded-runtime-thread')).toBeVisible();

  // Another PR: its own panel, empty — no thread until something is asked.
  await openPr(page, 42);
  const other = page.getByTestId('pr-agent-panel');
  await expect(other.getByRole('button', { name: 'Explain this PR' })).toBeVisible({ timeout: 10_000 });
  await expect(other.getByTestId('embedded-runtime-thread')).toHaveCount(0);
  expect((await mockLog(page)).creates).toHaveLength(1);

  // Back on #51 the same thread is found again (one thread per PR).
  await openPr(page, 51);
  await expect(panel.getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 10_000 });
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
  const files = page.getByRole('region', { name: 'Files in pull request #51' });
  const first = files.locator('.nav-file').first();
  const path = (await first.getAttribute('title')) ?? '';
  await first.locator('.nav-file-open').click({ button: 'right' });
  await page.locator('.ctx-menu').getByText('Explain this file', { exact: true }).click();
  await expect(page.getByTestId('pr-agent-panel').getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 10_000 });
  await expect.poll(async () => (await mockLog(page)).prompts.at(-1)?.text ?? '').toContain(path);

  // Lines: select two code rows of an expanded file, then the action beside it.
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
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

test('at 800 px tall the list, the files and the opened thread all fit the column', async ({ page }) => {
  // Folded on purpose last time: a new ask must still unfold it.
  await page.addInitScript(() => localStorage.setItem('otto_pr_agent_folded', '1'));
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto('/#/git');
  await openPr(page, 51);
  await menu(page, 'Explain this PR');
  const panel = page.getByTestId('pr-agent-panel');
  await expect(panel.getByTestId('embedded-runtime-thread')).toBeVisible({ timeout: 10_000 });
  const box = (await page.locator('.prw-agent').boundingBox())!;
  expect(box.height).toBeGreaterThanOrEqual(200);
  expect(box.y + box.height).toBeLessThanOrEqual(800);
  await expect(page.locator('.prw-list-main')).toBeVisible();
  await expect(page.getByRole('region', { name: 'Files in pull request #51' })).toBeVisible();
  expect((await page.locator('.prw-list-main').boundingBox())!.height).toBeGreaterThanOrEqual(140);
});
