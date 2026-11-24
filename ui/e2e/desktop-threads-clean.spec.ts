import { test, expect, type Page } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// A calmer Threads (desktop-browser, MOCK mode — ui/src/lib/api/mock-runtime.ts
// serves the rows), after T3 Code's layout:
//   1. the toolbar keeps four controls (sidebar, title, right panel, ⋯); the
//      project card, terminal drawer, layout and New thread moved into ⋯;
//   2. subagents live in the details card; relay / rooms are ONE ⋯ item
//      and popover (no Peers tab, no "To:" bar, no header chip);
//   3. the right panel's + menu has no Terminal / Peers, and no Connections
//      until the thread touched a database;
//   4. a split pane's header has no phone button (⋯ › Continue on phone);
//   5. the composer shows its draft state only on failure;
//   6. all of it fits a phone with no sideways scroll.
// ─────────────────────────────────────────────────────────────────────────────

type Hooks = { seed: (row: Record<string, unknown>) => string };

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(() => {
    localStorage.setItem('otto_mock', '1');
    localStorage.setItem('otto_workspace', 'wsp_otto');
    localStorage.removeItem('otto_threads_v1');
  });
  await page.setViewportSize({ width: 1400, height: 900 });
});

const at = (h: number): string => `2026-11-12T${String(h).padStart(2, '0')}:00:00Z`;

async function openThreads(page: Page, rows: Record<string, unknown>[], ready = 'threads-sidebar'): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.seed === 'function');
  await page.evaluate((rs) => {
    for (const r of rs) (window as unknown as { __mockRuntime: Hooks }).__mockRuntime.seed(r);
  }, rows);
  await page.evaluate(() => { location.hash = '#/threads'; });
  await expect(page.getByTestId(ready)).toBeVisible({ timeout: 15_000 });
}

const LEAD = { id: 'rt_lead', title: 'Lead thread', provider: 'claude', last_active_at: at(10) };
const WORKER = { id: 'rt_worker', title: 'Codex worker', provider: 'codex', parent_id: 'rt_lead', status: 'working', last_active_at: at(9) };
const SOLO = { id: 'rt_solo', title: 'Solo thread', last_active_at: at(8) };

test('the toolbar keeps four controls; the rest is one ⋯ away', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await openThreads(page, [LEAD, WORKER, SOLO]);
  await expect(page.getByTestId('runtime-thread-rt_lead')).toBeVisible({ timeout: 15_000 });

  for (const gone of ['threads-back', 'threads-forward', 'threads-new-btn-top', 'threads-toggle-card', 'threads-layout-switch', 'threads-toggle-terminal', 'threads-continue-phone']) {
    await expect(page.getByTestId(gone)).toHaveCount(0);
  }
  for (const kept of ['threads-toggle-sidebar', 'threads-breadcrumb', 'threads-toggle-inspector', 'threads-thread-menu']) {
    await expect(page.getByTestId(kept)).toBeVisible();
  }

  await page.getByTestId('threads-thread-menu').click();
  const menu = page.locator('.ctx-menu');
  await expectFullyInViewport(page, menu, 'toolbar ⋯ menu');
  // New thread is the sidebar's pencil while the sidebar shows; layout and
  // the thread details card sit in View.
  await expect(menu.getByRole('menuitem', { name: 'Send to another thread…' })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'New thread' })).toHaveCount(0);
  await expect(menu.getByRole('menuitemcheckbox', { name: 'Terminal drawer' })).toHaveCount(1);
  await menu.getByRole('menuitem', { name: 'View' }).click();
  const view = page.locator('.ctx-submenu');
  for (const name of ['Thread details', 'Single thread', 'Split', 'Grid']) {
    await expect(view.getByRole('menuitemcheckbox', { name })).toBeVisible();
  }
  // The details card floats by default; ⋯ › View › Thread details hides it.
  await expect(view.getByRole('menuitemcheckbox', { name: 'Thread details' })).toBeChecked();
  await view.getByRole('menuitemcheckbox', { name: 'Thread details' }).click();
  await expect(page.getByTestId('thread-details-card')).toHaveCount(0);
  // Light and dark.
  await page.screenshot({ path: test.info().outputPath('threads-clean-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: test.info().outputPath('threads-clean-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });
});

test('subagents sit in the details card; ⋯ › Send to another thread… holds relay and rooms', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await openThreads(page, [LEAD, WORKER, SOLO]);
  await expect(page.getByTestId('runtime-thread-rt_lead')).toBeVisible({ timeout: 15_000 });
  // No old strip above the conversation, no Peers bar in the pane, no header chip.
  await expect(page.locator('[data-testid^="pane-peer-bar-"]')).toHaveCount(0);
  await expect(page.getByTestId('threads-related-chip')).toHaveCount(0);

  // The details card lists the subagent with its status word.
  const subs = page.getByTestId('thread-details-card').getByTestId('details-subagents');
  await expect(subs).toContainText('Subagents · 1 working');
  const sub = subs.getByTestId('details-subagent');
  await expect(sub).toContainText('Codex worker');
  await expect(sub).toHaveAttribute('data-word', 'Working');

  // ⋯ › Send to another thread…: one popover under ⋯ (relay, open beside, rooms).
  const more = page.getByTestId('threads-thread-menu');
  await more.click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Send to another thread…' }).click();
  const pop = page.getByTestId('threads-related-popover');
  await expectFullyInViewport(page, pop, 'send to another thread popover');
  await expect(pop.getByTestId('related-row-rt_worker')).toContainText('Codex worker');
  for (const action of ['related-message-any', 'related-start-room']) await expect(pop.getByTestId(action)).toBeVisible();
  // Escape closes it and hands focus back to ⋯.
  await pop.focus();
  await page.keyboard.press('Escape');
  await expect(pop).toHaveCount(0);
  await expect(more).toBeFocused();

  // Open beside: the subagent joins a split; its pane header has no phone button.
  await more.click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Send to another thread…' }).click();
  await pop.getByRole('button', { name: 'Open Codex worker beside' }).click();
  await expect(page.getByTestId('thread-pane-rt_worker')).toBeVisible();
  await expect(page.locator('[data-testid^="pane-continue-phone-"]')).toHaveCount(0);
  await expect(page.locator('[data-testid^="pane-terminal-btn-"]').first()).toBeVisible();

  // A solo thread: no Subagents section; ⋯ still opens the popover.
  await more.click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'View' }).click();
  await page.locator('.ctx-submenu').getByRole('menuitemcheckbox', { name: 'Single thread' }).click();
  await page.getByTestId('thread-row-rt_solo').locator('.tr-main').click();
  await expect(page.getByTestId('runtime-thread-rt_solo')).toBeVisible();
  await expect(page.getByTestId('thread-details-card')).toBeVisible();
  await expect(page.getByTestId('details-subagents')).toHaveCount(0);
  await more.click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Send to another thread…' }).click();
  await expect(page.getByTestId('threads-related-popover')).toContainText('No other threads yet');
});

test('the right panel offers no Terminal, Peers or (without DB work) Connections', async ({ page }) => {
  await openThreads(page, [SOLO]);
  await expect(page.getByTestId('runtime-thread-rt_solo')).toBeVisible({ timeout: 15_000 });
  await page.getByTestId('threads-toggle-inspector').click();
  const launcher = page.getByTestId('threads-surface-launcher');
  await expect(launcher).toBeVisible();
  await expect(page.getByTestId('threads-surface-terminal')).toHaveCount(0);
  await page.getByTestId('threads-surface-add').click();
  const menu = page.locator('.ctx-menu');
  await expect(menu.getByRole('menuitem', { name: /^Files/ })).toBeVisible();
  for (const gone of [/Terminal/, /Peers/, /Connections/]) await expect(menu.getByRole('menuitem', { name: gone })).toHaveCount(0);
  await page.keyboard.press('Escape');
});

test('the composer is quiet: no "Saved" line while the draft is fine', async ({ page }) => {
  await openThreads(page, [SOLO]);
  const thread = page.getByTestId('runtime-thread-rt_solo');
  await expect(thread).toBeVisible({ timeout: 15_000 });
  await thread.getByTestId('runtime-composer').fill('a draft');
  const status = thread.getByTestId('runtime-draft-status');
  await expect(status).not.toHaveAttribute('data-state', 'error');
  for (const word of ['Saved', 'Saving…', 'Restoring…']) await expect(thread.getByText(word, { exact: true })).toHaveCount(0);
  // One model chip; effort lives inside its picker.
  await expect(thread.getByTestId('runtime-model')).toHaveCount(1);
});

test('phone: the toolbar, ⋯ and the send-to popover fit with no sideways scroll', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  // At phone width the sidebar is a drawer: the page opens on the thread.
  await openThreads(page, [LEAD, WORKER], 'runtime-thread-rt_lead');
  await expectNoHorizontalOverflow(page);
  await page.getByTestId('threads-thread-menu').click();
  await expectFullyInViewport(page, page.locator('.ctx-menu'), 'toolbar ⋯ menu (phone)');
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Send to another thread…' }).click();
  await expectFullyInViewport(page, page.getByTestId('threads-related-popover'), 'send-to popover (phone)');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: test.info().outputPath('threads-clean-phone.png'), animations: 'disabled' });
});
