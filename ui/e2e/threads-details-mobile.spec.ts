import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Thread details on a phone / tablet (ThreadDetailsCard.svelte), with real
// touch (tap): the card never floats over the conversation there — the
// details drawer (right panel) heads with it, every row is a finger-sized
// target, its menus fit, and a subagent row opens Agents & tasks. A Claude
// thread on the isolated daemon (otto-fake-claude: `agent <s>`).

const PROJECTS = ['iphone-portrait', 'iphone-landscape', 'ipad-portrait'];

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';
const git = (...a: string[]): string => execFileSync('git', ['-c', 'user.email=e2e@otto', '-c', 'user.name=e2e', ...a], { cwd: dir }).toString();

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'harbor-m-'));
  writeFileSync(join(dir, 'README.md'), '# harbor\n');
  git('init', '-q', '-b', 'main');
  git('add', '.');
  git('commit', '-qm', 'init');
  git('checkout', '-qb', 'feat/phone');
  writeFileSync(join(dir, 'queue.ts'), 'export const queue = [];\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Harbor phone', root_path: dir } });
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

async function shots(page: Page, name: string): Promise<void> {
  const out = process.env.THRC_SHOTS;
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: out ? `${out}/${name}-${scheme}.png` : test.info().outputPath(`${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

test('phone: thread details open as the details drawer, by touch', async ({ page }, info) => {
  test.skip(!PROJECTS.includes(info.project.name), 'phone and tablet projects');
  test.setTimeout(120_000);
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'claude', cwd: dir, title: `Phone details ${info.project.name}` } });
  const id = (await created.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const conv = page.getByTestId(`runtime-thread-${id}`);
  await expect(conv).toBeVisible({ timeout: 30_000 });
  const composer = conv.getByTestId('runtime-composer');
  await composer.tap();
  await composer.fill('agent 90');
  await conv.getByTestId('runtime-send').tap();
  await expect(conv.getByText("I'm waiting for the PR sweep to come back.")).toBeVisible({ timeout: 20_000 });

  // Never floating over the conversation here.
  await expect(page.getByTestId('thread-details-card')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);

  await page.getByTestId('threads-toggle-inspector').tap();
  const card = page.getByTestId('thread-details-card');
  await expect(card).toBeVisible();
  await expectFullyInViewport(page, card, 'details card (drawer)');
  await expect(card.getByTestId('details-branch')).toContainText('feat/phone', { timeout: 15_000 });
  await expect(card.getByTestId('details-git-action')).toHaveAttribute('data-kind', 'commit');
  const sub = card.getByTestId('details-subagent');
  await expect(sub).toHaveAttribute('data-word', 'Working');
  // Finger-sized targets.
  for (const el of [card.getByTestId('details-open-in'), card.getByTestId('details-branch'), card.getByTestId('details-git-action'), card.getByTestId('details-changes'), sub.getByRole('button', { name: /^PR sweep Working/ })]) {
    expect((await el.boundingBox())!.height, 'touch target').toBeGreaterThanOrEqual(44);
  }
  await expectNoHorizontalOverflow(page);
  await shots(page, `details-${info.project.name}`);

  // Open in ▾ fits the screen.
  await card.getByTestId('details-open-in').tap();
  await expectFullyInViewport(page, page.locator('.ctx-menu'), 'Open in menu (phone)');
  await page.keyboard.press('Escape');

  // A subagent row brings Agents & tasks into view.
  await sub.getByRole('button', { name: /^PR sweep Working/ }).tap();
  await expect(page.getByTestId('thread-agents-tasks')).toBeInViewport();
  await expectNoHorizontalOverflow(page);
});
