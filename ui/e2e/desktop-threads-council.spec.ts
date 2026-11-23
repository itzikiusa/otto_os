import { test, expect, type Page } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Council in Threads (desktop-browser, MOCK mode — ui/src/lib/api/mock-runtime.ts
// serves the council routes and plays status events). A council is a linked
// group of normal threads:
//   1. "Ask several" in the new-thread stage: two agents picked, the send
//      says "Ask 2", one seat that cannot start fails the whole council (the
//      error names it, the picks stay); ⌘↵ starts it in the background (the
//      stage stays, a toast offers Show) and Show opens the seats in Split;
//   2. a seeded council: the seats nest under seat 1 in the sidebar, the
//      thread ⋯ opens them side by side, "Synthesize with" waits for every
//      seat and opens the judge as a pane, "Ask every seat…" sends the same
//      text to each seat (queued), and the Related chip lists the others.
// ─────────────────────────────────────────────────────────────────────────────

type Hooks = {
  seed: (row: Record<string, unknown>) => string;
  say: (sid: string, texts: string[]) => void;
  status: (sid: string, status: 'running' | 'ready') => void;
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

const at = (m: number): string => `2026-11-12T10:${String(m).padStart(2, '0')}:00Z`;
const council = (seat: number, role = 'seat') => ({ council: { id: 'cnc_e2e', seat, role }, read_only: true });

async function waitForMock(page: Page): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => typeof (window as unknown as { __mockRuntime?: Hooks }).__mockRuntime?.seed === 'function');
}

const hook = <K extends keyof Hooks>(page: Page, name: K, ...args: Parameters<Hooks[K]>): Promise<void> =>
  page.evaluate(([n, a]) => {
    const h = (window as unknown as { __mockRuntime: Record<string, (...x: unknown[]) => unknown> }).__mockRuntime;
    h[n as string](...(a as unknown[]));
  }, [name, args] as const);

/** The page ⋯'s item `name` (the focused thread's actions come first). */
async function threadMenu(page: Page, name: string | RegExp) {
  await page.getByTestId('threads-thread-menu').click();
  return page.locator('.ctx-menu').first().getByRole('menuitem', { name });
}

test('Ask several sends one prompt to two agents, all or none; ⌘↵ starts it in the background', async ({ page }) => {
  await waitForMock(page);
  await page.evaluate(() => { location.hash = '#/threads'; });
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible({ timeout: 15_000 });

  // Claude first, then "Ask several" and Codex picked in the agent menu.
  await stage.getByTestId('picker-agent').click();
  const picker = page.getByTestId('model-picker');
  await picker.getByTestId('model-picker-provider-claude').click();
  await page.keyboard.press('Escape');
  await stage.getByTestId('new-thread-ask-several').click();
  await expect(stage.getByTestId('new-thread-ask-several')).toHaveAttribute('aria-pressed', 'true');
  await expect(stage.getByTestId('new-thread-council-seats')).toContainText('Pick another model');
  await stage.getByTestId('picker-agent').click();
  await picker.getByTestId('model-picker-provider-codex').click();
  await picker.getByRole('button', { name: 'Default model' }).click();
  await page.keyboard.press('Escape');
  await expect(stage.getByTestId('picker-agent')).toContainText(/Claude( Code)?, Codex/);
  const send = stage.getByTestId('new-thread-send-btn');
  const input = stage.getByTestId('new-thread-input');
  await input.fill('Which cache should we use?');
  await expect(send).toHaveText(/Ask 2/);

  // A seat that cannot start fails the whole council and says which.
  await page.evaluate(() => { (window as unknown as { __mockRuntime: { councilFail?: string } }).__mockRuntime.councilFail = 'codex'; });
  await send.click();
  await expect(page.getByText(/seat 2 \(codex\)/).first()).toBeVisible();
  await expect(stage.getByTestId('new-thread-council-seats')).toBeVisible();

  // ⌘↵: both seats start in the background; the stage stays, a toast says so.
  await page.evaluate(() => { (window as unknown as { __mockRuntime: { councilFail?: string } }).__mockRuntime.councilFail = undefined; });
  await input.focus();
  await page.keyboard.press('ControlOrMeta+Enter');
  const toast = page.locator('.toast', { hasText: 'Started 2 threads' });
  await expect(toast).toBeVisible({ timeout: 15_000 });
  await expect(stage).toBeVisible();
  const body = await page.evaluate(() => (window as unknown as { __mockRuntime: { councils: unknown[] } }).__mockRuntime.councils.at(-1));
  expect(body).toMatchObject({ prompt: 'Which cache should we use?', seats: [{ provider: 'claude' }, { provider: 'codex' }] });

  // Show: the seats side by side, each a full thread with its own composer.
  await toast.getByRole('button', { name: 'Show' }).click();
  const canvas = page.getByTestId('threads-canvas');
  await expect(canvas).toHaveAttribute('data-layout', 'split');
  const panes = canvas.locator('[data-testid^="thread-pane-"]');
  await expect(panes).toHaveCount(2);
  await expect(panes.nth(0)).toContainText('Mock claude (seat 1)');
  await expect(panes.nth(1)).toContainText('Mock codex (seat 2)');
  await expect(panes.nth(1).getByRole('textbox').first()).toBeVisible();
});

test('a council is a group of threads: nested rows, side by side, synthesize and ask every seat from ⋯', async ({ page }, info) => {
  await waitForMock(page);
  await page.evaluate((rows) => {
    const m = (window as unknown as { __mockRuntime: Hooks }).__mockRuntime;
    for (const r of rows) m.seed(r);
  }, [
    { id: 'rt_s1', title: 'Cache plan · claude', provider: 'claude', last_active_at: at(9), meta: council(1) },
    { id: 'rt_s2', title: 'Cache plan · codex', provider: 'codex', parent_id: 'rt_s1', last_active_at: at(8), meta: council(2) },
    { id: 'rt_s3', title: 'Cache plan · gemini', provider: 'gemini', parent_id: 'rt_s1', last_active_at: at(7), status: 'working', meta: council(3) },
  ]);
  await hook(page, 'say', 'rt_s1', ['Use Redis.']);
  await hook(page, 'say', 'rt_s2', ['Use Memcached.']);
  await page.evaluate(() => { location.hash = '#/threads/rt_s1'; });
  await expect(page.getByTestId('runtime-thread-rt_s1')).toBeVisible({ timeout: 15_000 });

  // The seats nest under seat 1 like any child thread, each named by its agent.
  const sidebar = page.getByTestId('threads-sidebar');
  await expect(sidebar.getByTestId('thread-family-count-rt_s1')).toHaveText('· 2');
  await expect(sidebar.getByTestId('thread-row-rt_s2')).toHaveText(/codex/);
  await expect(sidebar.getByTestId('thread-row-rt_s2')).not.toHaveText(/Cache plan/);

  // The Related chip lists the other seats (one tap switches seats on a phone).
  await expect(page.getByTestId('threads-related-chip')).toHaveText(/2/);

  // ⋯ › Open council side by side: three seats in the Grid.
  await (await threadMenu(page, 'Open side by side')).click();
  const canvas = page.getByTestId('threads-canvas');
  await expect(canvas).toHaveAttribute('data-layout', 'grid');
  for (const id of ['rt_s1', 'rt_s2', 'rt_s3']) await expect(canvas.getByTestId(`thread-pane-${id}`)).toBeVisible();
  // Seats are peers: seat 3 working is not seat 1's background task.
  await expect(canvas.getByTestId('thread-pane-rt_s3')).toBeVisible();
  await expect(canvas.getByTestId('thread-pane-rt_s1')).not.toContainText(/background/i);

  // Seat 3 still works: Synthesize waits and says why.
  const synth = await threadMenu(page, 'Compare answers with');
  await expect(synth).toHaveAttribute('aria-disabled', 'true');
  await synth.hover();
  await expect(synth).toHaveAttribute('title', 'One model is still working');
  await page.screenshot({ path: info.outputPath('council-panes-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('council-panes-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });
  await page.keyboard.press('Escape');

  // Once every seat is done: Synthesize with ▸ Claude opens the judge as a pane.
  await hook(page, 'status', 'rt_s3', 'ready');
  await (await threadMenu(page, 'Compare answers with')).hover();
  await page.locator('.ctx-submenu').getByRole('menuitem', { name: /^Claude/ }).click();
  const judge = canvas.locator('[data-testid^="thread-pane-"]', { hasText: 'Mock synthesis' });
  await expect(judge).toBeVisible({ timeout: 15_000 });
  await expect(canvas.locator('[data-testid^="thread-pane-"]')).toHaveCount(4);
  await expect(judge).toContainText('Use Redis.');
  await page.screenshot({ path: info.outputPath('council-synthesis-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('council-synthesis-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });

  // ⋯ › Ask every seat…: one multi-line sheet, the text queued in each seat.
  await canvas.getByTestId('thread-pane-rt_s2').click();
  await (await threadMenu(page, 'Ask every model…')).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('textbox').fill('Why that one?');
  await dialog.getByRole('button', { name: 'Ask 3' }).click();
  await expect(page.locator('.toast', { hasText: 'Sent to 3 seats' })).toBeVisible();
  const prompts = await page.evaluate(() => (window as unknown as { __mockRuntime: { prompts: { id: string; text: string; delivery?: string }[] } }).__mockRuntime.prompts.filter((p) => p.text === 'Why that one?'));
  expect(prompts.map((p) => p.id).sort()).toEqual(['rt_s1', 'rt_s2', 'rt_s3']);
  expect(prompts.every((p) => p.delivery === 'queue')).toBe(true);
});
