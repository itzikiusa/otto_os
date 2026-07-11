import { expect, test } from '@playwright/test';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { expectNoHorizontalOverflow, openPage } from './helpers';

// Vault v3 (the docs home) — desktop vertical, against the ISOLATED daemon and
// a REAL on-disk markdown bundle: register → tree → open note (reading view,
// wikilinks, tags) → backlinks → edit + autosave → search (FTS + operators) →
// quick switcher → rename rewrites links ON DISK → trash → OKF validation.
// The API half runs first (serial) so UI failures don't mask engine failures.

let workspaceId = '';
let vaultId = 0;
let vaultDir = '';

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const seeded = await seedVaultDir(ctx, base, workspaceId);
  vaultId = seeded.vaultId;
  vaultDir = seeded.dir;
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ({ wsId, vId }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem(`otto_vault_last:${wsId}`, String(vId));
      localStorage.setItem('otto_rail_expanded', '0');
    },
    { wsId: workspaceId, vId: vaultId },
  );
});

// ---------------------------------------------------------------------------
// API vertical
// ---------------------------------------------------------------------------

test('api: index, links, backlinks, search, switcher, okf', async () => {
  const { ctx, base } = await apiCtx();
  const v1 = `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}`;

  // Note read: meta + resolved outgoing links.
  const note = await (await ctx.get(`${v1}/note?path=services%2Fauth-api.md`)).json();
  expect(note.meta.okf_type).toBe('Service');
  expect(note.meta.tags).toContain('jwt');
  expect(note.meta.aliases).toContain('The Auth Service');
  const targets = note.outgoing.map((l: { dst_path: string | null }) => l.dst_path);
  expect(targets).toContain('services/orders-api.md');
  expect(targets).toContain('runbooks/deploy.md');
  expect(targets).toContain(null); // [[Missing Note]] stays unresolved (legal)

  // Backlinks with context.
  const bl = await (await ctx.get(`${v1}/backlinks?path=services%2Forders-api.md`)).json();
  expect(bl.map((b: { path: string }) => b.path)).toContain('services/auth-api.md');

  // FTS search + operator filter.
  const hits = await (
    await ctx.post(`${v1}/search`, { data: { query: 'charging customer card' } })
  ).json();
  expect(hits[0].path).toBe('services/orders-api.md');
  const tagged = await (await ctx.post(`${v1}/search`, { data: { query: 'tag:oncall' } })).json();
  expect(tagged).toHaveLength(1);
  expect(tagged[0].path).toBe('runbooks/deploy.md');

  // Switcher matches aliases.
  const sw = await (await ctx.get(`${v1}/switcher?q=the%20auth%20service`)).json();
  expect(sw.some((h: { alias: string | null }) => h.alias === 'The Auth Service')).toBe(true);

  // Graph: compact arrays, valid edge indices, ghost for the broken link.
  const g = await (await ctx.get(`${v1}/graph?ghosts=true&tags=true`)).json();
  expect(g.paths.length).toBeGreaterThanOrEqual(3);
  expect(g.edges.length % 2).toBe(0);
  expect(Math.max(...g.edges)).toBeLessThan(g.paths.length);
  expect(g.flags.some((f: number) => f & 1)).toBe(true); // ghost

  // OKF: conformant fixture, broken-link warning present.
  const rep = await (await ctx.post(`${v1}/okf/validate`, { data: {} })).json();
  expect(rep.conformant).toBe(true);
  expect(rep.warnings.some((w: { rule: string }) => w.rule === 'W2')).toBe(true);

  await ctx.dispose();
});

test('api: write, conflict, rename rewrites links on disk, trash', async () => {
  const { ctx, base } = await apiCtx();
  const v1 = `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}`;

  // Create (if_hash "" = must-not-exist) — parent folder auto-created.
  const meta = await (
    await ctx.put(`${v1}/note`, {
      data: {
        path: 'notes/from api.md',
        content: '---\ntype: Reference\ntitle: From API\ndescription: Written over HTTP.\n---\n\nSee [[auth-api]].\n',
        if_hash: '',
      },
    })
  ).json();
  expect(meta.title).toBe('From API');
  expect(existsSync(join(vaultDir, 'notes/from api.md'))).toBe(true);

  // Stale hash → 409.
  const conflict = await ctx.put(`${v1}/note`, {
    data: { path: 'notes/from api.md', content: 'clobber', if_hash: 'deadbeef' },
  });
  expect(conflict.status()).toBe(409);

  // Rename target file → the three referencing notes are rewritten ON DISK.
  const ren = await (
    await ctx.post(`${v1}/rename`, {
      data: { from: 'services/auth-api.md', to: 'services/identity-api.md' },
    })
  ).json();
  expect(ren.links_updated).toBeGreaterThanOrEqual(3);
  expect(readFileSync(join(vaultDir, 'services/orders-api.md'), 'utf8')).toContain(
    'identity-api.md',
  );
  expect(readFileSync(join(vaultDir, 'runbooks/deploy.md'), 'utf8')).toContain(
    '[[identity-api]]',
  );
  // Rename back so the UI half sees the original names.
  await ctx.post(`${v1}/rename`, {
    data: { from: 'services/identity-api.md', to: 'services/auth-api.md' },
  });

  // Trash: file moves under .trash/, never deleted.
  const del = await ctx.delete(`${v1}/note?path=notes%2Ffrom%20api.md`);
  expect(del.status()).toBe(204);
  expect(existsSync(join(vaultDir, 'notes/from api.md'))).toBe(false);
  expect(existsSync(join(vaultDir, '.trash/notes/from api.md'))).toBe(true);

  await ctx.dispose();
});

// ---------------------------------------------------------------------------
// UI vertical
// ---------------------------------------------------------------------------

test('ui: tree, reading view, wikilink nav, backlinks, status bar', async ({ page }) => {
  await openPage(page, 'vault');
  await expectNoHorizontalOverflow(page);

  // Tree shows folders + reserved index.md; open a note through it.
  const tree = page.locator('.tree');
  await expect(tree.getByText('services', { exact: true })).toBeVisible({ timeout: 15_000 });
  await tree.getByText('services', { exact: true }).click();
  await tree.getByText('auth-api', { exact: true }).click();

  // Reading view: rendered heading + tag chip + resolved and unresolved links.
  const read = page.locator('.read');
  await expect(read.getByRole('heading', { name: 'Overview' })).toBeVisible();
  await expect(read.locator('span.tag', { hasText: '#jwt' })).toBeVisible();
  await expect(read.locator('a.internal-link.unresolved')).toHaveCount(1);

  // Status bar: backlinks/words/chars.
  await expect(page.locator('.vault-statusbar')).toContainText('words');
  await expect(page.locator('.vault-statusbar')).toContainText('backlinks');

  // Wikilink navigation → deploy runbook opens, backlinks list the linker.
  await read.locator('a.internal-link', { hasText: 'the deploy runbook' }).click();
  await expect(page.locator('.crumbs')).toContainText('deploy');
  await expect(page.locator('.right .item .t', { hasText: 'Auth API' })).toBeVisible();
});

test('ui: edit + autosave persists to disk', async ({ page }) => {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await tree.getByText('runbooks', { exact: true }).click();
  await tree.getByText('deploy', { exact: true }).click();

  // Switch to edit mode and type at the end of the doc.
  await page.locator('.mode-btn').click();
  const editor = page.locator('.cm-content');
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+ArrowDown' : 'Control+End');
  await page.keyboard.type('\nAutosaved line from e2e.');
  // Autosave debounce is 800ms; wait for the save to land on disk.
  await expect
    .poll(() => readFileSync(join(vaultDir, 'runbooks/deploy.md'), 'utf8'), { timeout: 10_000 })
    .toContain('Autosaved line from e2e.');
});

test('ui: search panel, tags panel, quick switcher', async ({ page }) => {
  await openPage(page, 'vault');

  // Search with FTS.
  await page.locator('.left-modes button[title="Search"]').click();
  await page.locator('.search input').fill('charging customer');
  await page.locator('.search input').press('Enter');
  await expect(page.locator('.hit .t', { hasText: 'Orders API' })).toBeVisible();
  await page.locator('.hit .t', { hasText: 'Orders API' }).click();
  await expect(page.locator('.crumbs')).toContainText('orders-api');

  // Tags panel → tag click runs a tag: search.
  await page.locator('.left-modes button[title="Tags"]').click();
  await expect(page.locator('.tag-row .tag', { hasText: '#oncall' })).toBeVisible();
  await page.locator('.tag-row .tag', { hasText: '#oncall' }).click();
  await expect(page.locator('.search input')).toHaveValue('tag:oncall');

  // Quick switcher via alias.
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+o' : 'Control+o');
  const sw = page.locator('.panel input');
  await sw.fill('the auth service');
  await expect(page.locator('.hit .via', { hasText: 'Auth API' }).first()).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(page.locator('.crumbs')).toContainText('auth-api');
});

test('ui: OKF panel validates and reports', async ({ page }) => {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await tree.getByText('services', { exact: true }).click();
  await tree.getByText('auth-api', { exact: true }).click();

  // OKF chip visible on an OKF vault; validate from the right panel.
  await expect(page.locator('.okf-chip')).toBeVisible();
  await page.locator('.right .hdr', { hasText: 'OKF' }).click();
  await page.locator('.okf-actions .mini', { hasText: 'Validate' }).click();
  await expect(page.locator('.right')).toContainText('OKF v0.1 conformant');
  // Broken-link warning surfaces as W2.
  await expect(page.locator('.finding.warn .t', { hasText: 'W2' }).first()).toBeVisible();
});
