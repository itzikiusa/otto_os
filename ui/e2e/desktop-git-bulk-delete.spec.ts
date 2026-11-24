import { test, expect, type Page, type Request } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { chmodSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, tempDir } from './seed';

// Deleting several branches is ONE request, whichever way it starts (the
// LOCAL / REMOTE headers' trash, a row's "Delete multiple branches…",
// "Select merged", local + remote, two remotes) — never the single route in
// a loop. Real-shaped: a clone of a local bare origin whose pre-receive hook
// sleeps 300 ms per push (a network round trip), eight feature branches with
// their remote copies, a second remote, an unmerged branch.

test.use({ serviceWorkers: 'block' });
const FEATURES = Array.from({ length: 8 }, (_, i) => 'feature/PLAT-48' + (20 + i) + '-wallet-retry');

interface Seeded { id: string; dir: string; origin: string; upstream: string }

async function seed(tag: string): Promise<Seeded> {
  const root = tempDir('otto-e2e-bulkdel-');
  const origin = join(root, 'origin.git');
  const upstream = join(root, 'upstream.git');
  const dir = join(root, 'work');
  for (const bare of [origin, upstream]) {
    execFileSync('git', ['init', '-q', '--bare', '-b', 'main', bare]);
    writeFileSync(join(bare, 'hooks', 'pre-receive'), '#!/bin/sh\nsleep 0.3\n');
    chmodSync(join(bare, 'hooks', 'pre-receive'), 0o755);
  }
  execFileSync('git', ['init', '-q', '-b', 'main', dir]);
  const git = (...a: string[]): void => void execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'a.txt'), 'a\n');
  git('add', '.');
  git('commit', '-q', '-m', 'init');
  git('remote', 'add', 'origin', origin);
  git('remote', 'add', 'upstream', upstream);
  git('push', '-q', '-u', 'origin', 'main');
  for (const b of FEATURES) git('branch', b);
  git('push', '-q', '-u', 'origin', ...FEATURES);
  for (const b of FEATURES) git('branch', '--set-upstream-to=origin/' + b, b);
  git('push', '-q', 'upstream', 'main:legacy-1', 'main:legacy-2');
  git('checkout', '-q', '-b', 'wip/unmerged');
  git('commit', '-q', '--allow-empty', '-m', 'not merged anywhere');
  git('checkout', '-q', 'main');
  git('fetch', '-q', '--all');
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(base + '/api/v1/workspaces/' + wsId + '/repos', { data: { path: dir, name: tag + '-' + Date.now().toString(36) } });
  if (!r.ok()) throw new Error('seed: ' + r.status());
  const id = ((await r.json()) as { id: string }).id;
  await ctx.dispose();
  return { id, dir, origin, upstream };
}

const heads = (gitDir: string): string[] =>
  execFileSync('git', ['--git-dir', gitDir, 'for-each-ref', '--format=%(refname:short)', 'refs/heads/']).toString().split('\n').filter(Boolean);

function countDeletes(page: Page): { bulk: Request[]; single: Request[]; refs: () => number } {
  const bulk: Request[] = [];
  const single: Request[] = [];
  let refs = 0;
  page.on('request', (r) => {
    if (r.method() === 'POST' && /\/repos\/[^/]+\/branches\/delete$/.test(r.url())) bulk.push(r);
    if (r.method() === 'POST' && /\/repos\/[^/]+\/branch\/delete$/.test(r.url())) single.push(r);
    if (r.method() === 'GET' && /\/repos\/[^/]+\/(refs|overview)(\?|$)/.test(r.url())) refs++;
  });
  return { bulk, single, refs: () => refs };
}

async function openHistory(page: Page, id: string): Promise<void> {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => {
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
  });
  await page.goto('/#/git/' + id + '/graph');
  await expect(page.locator('.refs-panel')).toBeVisible({ timeout: 30_000 });
}

test.beforeEach(({}, info) => test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only'));

test('LOCAL trash → Select merged → with their remote copies: one request, says what it does, per-branch results', async ({ page }) => {
  const s = await seed('bulkdel-a');
  const n = countDeletes(page);
  await openHistory(page, s.id);
  // Hold the response so the in-flight state can be read.
  let release = (): void => {};
  const held = new Promise<void>((r) => (release = r));
  await page.route(/\/branches\/delete$/, async (route) => {
    await held;
    await route.continue();
  });
  await page.getByRole('button', { name: 'Delete several local branches', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Delete branches' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Select merged', exact: true }).click();
  await dialog.getByText('Also delete their remote branches').click();
  await dialog.getByRole('button', { name: 'Delete 8 branches…', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete 8 remote branches', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Deleting 8 branches…' })).toBeVisible();
  const refsBefore = n.refs();
  const t0 = Date.now();
  release();
  const results = dialog.getByRole('list', { name: 'Delete results' }).getByRole('listitem');
  await expect(results).toHaveCount(8, { timeout: 30_000 });
  console.log('BULKDEL-UI 8 local+remote ms', Date.now() - t0);
  await expect(dialog.locator('.res.fail')).toHaveCount(0);
  expect(n.bulk.length, 'one bulk request').toBe(1);
  expect(n.single.length, 'never the single route').toBe(0);
  expect((n.bulk[0].postDataJSON() as { items: unknown[] }).items).toHaveLength(8);
  expect(heads(s.origin).sort()).toEqual(['main']);
  expect(heads(join(s.dir, '.git')).sort()).toEqual(['main', 'wip/unmerged']);
  // The list updates once at the end (one refresh pass), not a refetch per branch.
  await page.waitForLoadState('networkidle').catch(() => {});
  expect(n.refs() - refsBefore).toBeLessThanOrEqual(3);
});

test('REMOTE trash opens on Remote; branches on two remotes go in one request', async ({ page }) => {
  const s = await seed('bulkdel-b');
  const n = countDeletes(page);
  await openHistory(page, s.id);
  await page.getByRole('button', { name: 'Delete several remote branches', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Delete branches' });
  await expect(dialog.getByRole('tab', { name: /^Remote/ })).toHaveAttribute('aria-selected', 'true');
  for (const b of ['upstream/legacy-1', 'upstream/legacy-2', 'origin/' + FEATURES[0], 'origin/' + FEATURES[1]]) {
    await dialog.getByRole('checkbox', { name: 'Select ' + b, exact: true }).check();
  }
  await dialog.getByRole('button', { name: 'Delete 4 branches…', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete 4 remote branches', exact: true }).click();
  await expect(dialog.getByRole('list', { name: 'Delete results' }).getByRole('listitem')).toHaveCount(4, { timeout: 30_000 });
  await expect(dialog.locator('.res.fail')).toHaveCount(0);
  expect(n.bulk.length).toBe(1);
  expect(n.single.length).toBe(0);
  expect(heads(s.upstream)).toEqual([]);
  expect(heads(s.origin)).not.toContain(FEATURES[0]);
  // Locals untouched by a remote-only delete.
  expect(heads(join(s.dir, '.git'))).toContain(FEATURES[0]);
});

test('a row\'s "Delete multiple branches…" with an unmerged branch: one request, the unmerged one forced after asking', async ({ page }) => {
  const s = await seed('bulkdel-c');
  const n = countDeletes(page);
  await openHistory(page, s.id);
  await page.locator('.refs-panel').getByText('unmerged', { exact: true }).first().click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Delete multiple branches…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Delete branches' });
  await dialog.getByRole('checkbox', { name: 'Select ' + FEATURES[2], exact: true }).check();
  await dialog.getByRole('button', { name: 'Delete 2 branches…', exact: true }).click();
  await page.getByRole('dialog', { name: 'Force-delete unmerged branches' }).getByRole('button', { name: 'Force delete 1' }).click();
  await expect(dialog.getByRole('list', { name: 'Delete results' }).getByRole('listitem')).toHaveCount(2, { timeout: 30_000 });
  await expect(dialog.locator('.res.fail')).toHaveCount(0);
  expect(n.bulk.length).toBe(1);
  expect(n.single.length).toBe(0);
  expect(heads(join(s.dir, '.git')).sort()).not.toContain('wip/unmerged');
});

test('timing: the bulk route for 8 local + remote branches against a 300 ms remote', async () => {
  const s = await seed('bulkdel-t');
  const { ctx, base } = await apiCtx();
  const items = FEATURES.map((name) => ({ name, remote: 'origin' }));
  const t0 = Date.now();
  const r = await ctx.post(base + '/api/v1/repos/' + s.id + '/branches/delete', { data: { items } });
  const ms = Date.now() - t0;
  expect(r.ok(), await r.text()).toBeTruthy();
  const body = (await r.json()) as { results: { local_deleted: boolean; remote_deleted: boolean; error: string | null }[] };
  expect(body.results.every((x) => x.local_deleted && x.remote_deleted && !x.error)).toBe(true);
  console.log('BULKDEL-ROUTE 8 local+remote ms', ms);
  await ctx.dispose();
});
