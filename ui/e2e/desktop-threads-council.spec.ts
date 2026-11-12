import { test, expect, type Page } from '@playwright/test';

// ─────────────────────────────────────────────────────────────────────────────
// Council in Threads (desktop-browser, MOCK mode — ui/src/lib/api/mock-runtime.ts
// serves the council routes and plays status events):
//   1. "Ask several" in the new-thread stage: two agents picked, the send
//      says "Ask 2", one seat that cannot start fails the whole council (the
//      error names it, the picks stay), the retry opens the council view;
//   2. a seeded council: one sidebar row ("Council · 3") that unfolds into
//      its seats, Synthesize disabled with a reason while a seat works, the
//      synthesis as the last column, "Continue here" and back.
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

test('Ask several sends one prompt to two agents, all or none', async ({ page }) => {
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
  await expect(stage.getByTestId('picker-agent')).toContainText(/Claude, Codex/);
  const send = stage.getByTestId('new-thread-send-btn');
  await stage.getByTestId('new-thread-input').fill('Which cache should we use?');
  await expect(send).toHaveText(/Ask 2/);

  // A seat that cannot start fails the whole council and says which.
  await page.evaluate(() => { (window as unknown as { __mockRuntime: { councilFail?: string } }).__mockRuntime.councilFail = 'codex'; });
  await send.click();
  await expect(page.getByText(/seat 2 \(codex\)/).first()).toBeVisible();
  await expect(stage.getByTestId('new-thread-council-seats')).toBeVisible();

  // Retry: both seats start; the council view shows them side by side.
  await page.evaluate(() => { (window as unknown as { __mockRuntime: { councilFail?: string } }).__mockRuntime.councilFail = undefined; });
  await send.click();
  const view = page.getByTestId('council-view');
  await expect(view).toBeVisible({ timeout: 15_000 });
  await expect(view.getByTestId('council-column-1')).toContainText('Mock claude (seat 1)');
  await expect(view.getByTestId('council-column-2')).toContainText('Mock codex (seat 2)');
  const body = await page.evaluate(() => (window as unknown as { __mockRuntime: { councils: unknown[] } }).__mockRuntime.councils.at(-1));
  expect(body).toMatchObject({ prompt: 'Which cache should we use?', seats: [{ provider: 'claude' }, { provider: 'codex' }] });
});

test('a council is one sidebar row; Synthesize waits for every seat; Continue here and back', async ({ page }, info) => {
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
  await page.evaluate(() => { location.hash = '#/threads/rt_s1'; });

  const view = page.getByTestId('council-view');
  await expect(view).toBeVisible({ timeout: 15_000 });
  await expect(view.getByText('Council · 3')).toBeVisible();
  for (const n of [1, 2, 3]) await expect(view.getByTestId(`council-column-${n}`)).toBeVisible();

  // One sidebar row with the seats folded under it.
  const sidebar = page.getByTestId('threads-sidebar');
  const row = sidebar.getByTestId('thread-council-cnc_e2e');
  await expect(row).toContainText('Council · 3');
  await expect(sidebar.getByTestId('thread-row-rt_s2')).toHaveCount(0);
  await sidebar.getByTestId('thread-council-fold-cnc_e2e').click();
  await expect(sidebar.getByTestId('thread-row-rt_s2')).toBeVisible();
  await expect(sidebar.getByTestId('thread-row-rt_s3')).toBeVisible();

  // Seat 3 still works: Synthesize is disabled and says why.
  const synth = view.getByTestId('council-synthesize');
  await expect(synth).toBeDisabled();
  await expect(view.getByTestId('council-synth-blocker')).toHaveText('Seat 3 is still working');
  await page.screenshot({ path: info.outputPath('council-light.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: info.outputPath('council-dark.png'), animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });

  await hook(page, 'status', 'rt_s3', 'ready');
  await expect(synth).toBeEnabled();
  await expect(synth).toHaveText(/Synthesize with Claude/);
  await synth.click();
  await expect(view.getByTestId('council-column-synthesis')).toContainText('Mock synthesis', { timeout: 15_000 });

  // In the columns the other seats are the council, not "linked threads":
  // no Related chip in the header, no "N linked" chip in a column.
  await expect(page.getByTestId('threads-related-chip')).toHaveCount(0);
  await expect(view.getByTestId('runtime-linked')).toHaveCount(0);

  // Continue here: seat 2 as a plain thread; the header's "Back to council"
  // (and the council row) bring the columns back.
  await view.getByTestId('council-continue-rt_s2').click();
  await expect(page.getByTestId('council-view')).toHaveCount(0);
  await expect(page.getByTestId('runtime-thread-rt_s2')).toBeVisible();
  await page.getByTestId('threads-back-to-council').click();
  await expect(page.getByTestId('council-view')).toBeVisible();
  await expect(page.getByTestId('threads-back-to-council')).toHaveCount(0);
  await page.getByTestId('council-view').getByTestId('council-continue-rt_s2').click();
  await expect(page.getByTestId('runtime-thread-rt_s2')).toBeVisible();
  await row.locator('.tr-main').click();
  await expect(page.getByTestId('council-view')).toBeVisible();
});
