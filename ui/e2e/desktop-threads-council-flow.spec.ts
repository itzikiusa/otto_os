import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Several threads toward one goal, with REAL fake-agent turns (otto-fake-claude
// / otto-fake-codex echo; `slow <s>` keeps a seat working; `ask` = an approval):
//   • a 3-seat council side by side gets the council strip: one chip per seat
//     with its state, Synthesize disabled WITH the reason while a seat works,
//     Stop all, Synthesize → a synthesis pane, and "Continue with this answer"
//     opens the new-thread stage with the answer quoted — nothing sent;
//   • three ordinary threads in Grid get the pane strip (one needs you), and
//     "Message every pane…" reaches each of them;
//   • Related › Message › "All open threads in this project" reaches a
//     structured thread; the Grid's add slot lists a structured thread.
// Screenshots (OTTO_E2E_SHOTS/fix-T-COUNCIL/) at 1280×800, light and dark.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-council-flow-'));
  writeFileSync(join(dir, 'README.md'), '# Council flow\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Council flow WS', root_path: dir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('council-flow-init')) return;
    sessionStorage.setItem('council-flow-init', '1');
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_threads_v1');
  }, wsId);
});

async function shot(page: Page, name: string): Promise<void> {
  const out = process.env.OTTO_E2E_SHOTS;
  if (!out) return;
  mkdirSync(join(out, 'fix-T-COUNCIL'), { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(out, 'fix-T-COUNCIL', `desktop-${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

async function thread(title: string, provider = 'codex'): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider, cwd: dir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

const sessionCount = async (): Promise<number> =>
  ((await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as unknown[]).length;

/** Show `ids` side by side (the store's persisted panes), first focused. */
async function showPanes(page: Page, ids: string[]): Promise<void> {
  await page.goto(`/#/threads/${ids[0]}`);
  await expect(page.getByTestId(`runtime-thread-${ids[0]}`)).toBeVisible({ timeout: 30_000 });
  await page.evaluate((p) => {
    const prev = JSON.parse(localStorage.getItem('otto_threads_v1') ?? '{}');
    localStorage.setItem('otto_threads_v1', JSON.stringify({ ...prev, paneIds: p, focusedSessionId: p[0], layoutMode: p.length > 2 ? 'grid' : 'split' }));
  }, ids);
  await page.reload();
  for (const id of ids) await expect(page.getByTestId(`thread-pane-${id}`)).toBeVisible({ timeout: 30_000 });
}

test('a council runs from the strip: seat states, Synthesize with its reason, Stop all, synthesis, Continue', async ({ page }) => {
  test.setTimeout(180_000);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions/council`, {
    data: { prompt: 'Which cache should we use?', title: 'Cache choice', cwd: dir, seats: [{ provider: 'claude' }, { provider: 'claude', effort: 'high' }, { provider: 'codex' }] },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const seats = ((await r.json()).seats as { id: string }[]).map((s) => s.id);
  await showPanes(page, seats);

  const strip = page.getByTestId('threads-pane-strip');
  await expect(strip).toBeVisible();
  await expect(strip).toHaveAttribute('data-council', /.+/);
  await expect(strip.getByTestId('pane-strip-title')).toContainText('Compare models · Cache choice');
  for (const [i, id] of seats.entries()) {
    const chip = strip.getByTestId(`pane-chip-${id}`);
    await expect(chip).toContainText(`${i + 1} · `);
    await expect(chip).toHaveAttribute('data-state', 'answered', { timeout: 30_000 });
  }
  // Two seats of one agent are told apart in their pane headers.
  await expect(page.getByTestId(`pane-title-${seats[0]}`)).toHaveText(/^1 · /);
  await expect(page.getByTestId(`pane-title-${seats[1]}`)).toHaveText(/^2 · .*High/);
  const synth = strip.getByTestId('pane-strip-synthesize');
  await expect(synth).toHaveClass(/primary/);

  // Seat 1 works again: Synthesize waits and says for whom.
  const pane1 = page.getByTestId(`thread-pane-${seats[0]}`);
  const composer = pane1.getByTestId('runtime-composer');
  await composer.fill('slow 60');
  await composer.press('Enter');
  await expect(strip.getByTestId(`pane-chip-${seats[0]}`)).toHaveAttribute('data-state', 'working', { timeout: 20_000 });
  await expect(synth).toHaveAttribute('aria-disabled', 'true');
  await expect(synth).not.toHaveClass(/primary/);
  await expect(synth).toHaveAttribute('title', 'Waiting for model 1');
  await expect(strip.getByTestId('pane-strip-why')).toHaveText('Waiting for model 1');
  await shot(page, 'council-strip-mid-run');

  // Stop all stops the working seat.
  await strip.getByTestId('pane-strip-stop').click();
  await expect(strip.getByTestId(`pane-chip-${seats[0]}`)).not.toHaveAttribute('data-state', 'working', { timeout: 20_000 });
  await expect(strip.getByTestId('pane-strip-stop')).toHaveCount(0);

  // Synthesize: the judge opens as a pane beside the seats and answers.
  await expect(synth).toHaveAttribute('aria-disabled', 'false');
  await synth.click();
  const panes = page.getByTestId('threads-canvas').locator('[data-testid^="thread-pane-"]');
  await expect(panes).toHaveCount(4, { timeout: 30_000 });
  const judgeChip = strip.locator('[data-testid^="pane-chip-"]', { hasText: 'Compared' });
  await expect(judgeChip).toHaveAttribute('data-state', 'answered', { timeout: 40_000 });
  await expect(page.locator('.toast', { hasText: /conflict/i })).toHaveCount(0);
  const cont = strip.getByTestId('pane-strip-continue');
  await expect(cont).toBeVisible();
  await expect(cont).toHaveClass(/primary/);
  await expect(strip.locator('.primary')).toHaveCount(1);
  await shot(page, 'council-strip-finished');

  // Continue with this answer: the stage, the answer quoted, nothing sent.
  const before = await sessionCount();
  await cont.click();
  const stage = page.getByTestId('new-thread-stage');
  await expect(stage).toBeVisible();
  const input = stage.getByTestId('new-thread-input');
  await expect(input).toHaveValue(/^> .+/);
  await expect(input).toHaveValue(/\n\n$/);
  expect(await sessionCount()).toBe(before);

  // The agent picker names the provider and lays effort out as one row.
  await stage.getByTestId('picker-agent').click();
  const picker = page.getByTestId('model-picker');
  await expect(picker.getByTestId('model-picker-provider-name')).not.toBeEmpty();
  const effort = picker.getByTestId('picker-effort');
  if (await effort.count()) {
    const box = (await effort.boundingBox())!;
    expect(box.height).toBeLessThan(80);
  }
  await expectFullyInViewport(page, picker, 'model picker');
  await shot(page, 'model-picker');
  await page.keyboard.press('Escape');
});

test('three threads in Grid: the pane strip, Message every pane, broadcast, and the add slot', async ({ page }) => {
  test.setTimeout(150_000);
  const a = await thread('Alpha build');
  const b = await thread('Beta docs');
  const c = await thread('Gamma deploy');
  for (const id of [a, b]) await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text: 'hello' } });
  await ctx.post(`${base}/api/v1/runtime-sessions/${c}/prompt`, { data: { text: 'ask' } });
  await showPanes(page, [a, b, c]);

  const strip = page.getByTestId('threads-pane-strip');
  await expect(strip).toBeVisible();
  await expect(strip).not.toHaveAttribute('data-council', /.+/);
  await expect(strip.locator('[data-testid^="pane-chip-"]')).toHaveCount(3);
  await expect(strip.getByTestId(`pane-chip-${c}`)).toHaveAttribute('data-state', 'needs-you', { timeout: 20_000 });
  await expect(strip.getByTestId(`pane-chip-${a}`)).toHaveAttribute('data-state', 'answered', { timeout: 20_000 });
  // A chip focuses its pane.
  await strip.getByTestId(`pane-chip-${b}`).click();
  await expect(page.getByTestId(`thread-pane-${b}`)).toHaveAttribute('aria-current', 'true');
  await shot(page, 'pane-strip-three');

  // Message every pane: a paragraph, the count said, each pane receives it.
  await strip.getByTestId('pane-strip-message').click();
  const sheet = page.getByTestId('pane-message-sheet');
  await expect(sheet.getByTestId('pane-message-intro')).toContainText('3 threads will receive it');
  await sheet.getByTestId('pane-message-input').fill('note from the strip\nsecond line');
  await page.getByTestId('pane-message-send').click();
  await expect(page.locator('.toast', { hasText: 'Sent to 3 threads' })).toBeVisible({ timeout: 15_000 });
  for (const id of [a, b]) await expect(page.getByTestId(`thread-pane-${id}`)).toContainText('echo: note from the strip', { timeout: 20_000 });
  await expect(page.getByTestId(`thread-pane-${c}`)).toContainText('note from the strip');

  // Related › Message › All open threads in this project reaches a structured thread.
  await page.getByTestId(`thread-pane-${a}`).getByRole('button', { name: 'More thread actions' }).click();
  await page.locator('.ctx-menu').first().getByRole('menuitem', { name: 'Related threads…' }).click();
  await page.getByTestId('related-message-any').click();
  await page.locator('.ctx-menu').first().getByRole('menuitem', { name: /All open threads in this project/ }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('textbox').fill('broadcast check');
  await dialog.getByRole('button', { name: /^Send to \d+$/ }).click();
  await expect(page.locator('.toast', { hasText: /Sent to \d+ threads?/ }).last()).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId(`thread-pane-${b}`)).toContainText('broadcast check', { timeout: 20_000 });

  // The Grid's add slot offers a structured thread not shown yet.
  const d = await thread('Delta review');
  await page.reload();
  const slot = page.getByTestId('threads-grid-picker');
  await expect(slot.getByTestId(`threads-add-${d}`)).toBeVisible({ timeout: 30_000 });
  await slot.getByTestId(`threads-add-${d}`).click();
  await expect(page.getByTestId(`thread-pane-${d}`)).toBeVisible();
  // Four panes at 1280×800: each keeps a usable composer and one marked focus.
  for (const id of [a, b, c, d]) {
    const pane = page.getByTestId(`thread-pane-${id}`);
    await expect(pane.getByTestId('runtime-composer')).toBeVisible();
  }
  await expect(page.locator('[data-testid^="thread-pane-"][aria-current="true"]')).toHaveCount(1);
  await shot(page, 'grid-four');
});
