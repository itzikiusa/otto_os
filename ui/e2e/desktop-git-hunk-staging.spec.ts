import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Hunk / line staging on the WIP panel (R3, desktop-browser only).
//
// The patch is rebuilt server-side from the daemon's own `git diff`, so these
// assertions are about BYTES on disk and in the index, not about what the UI
// drew: stage one of two hunks and the other must survive the commit; discard
// one hunk and only that hunk's lines come back (with a backup stash); a CRLF
// file staged by hunk must leave NO follow-up worktree diff — the classic
// symptom of a patch rebuilt from a `\r`-stripping parser.
//
// Each test seeds its OWN uniquely-named repo: fullyParallel runs them in
// separate workers, so a shared fixture would race.
// ─────────────────────────────────────────────────────────────────────────────

/** 20 numbered lines; `dirty` retouches lines 2 and 15 → two `-U3` hunks. */
function lines(dirty: boolean): string {
  return Array.from({ length: 20 }, (_, i) => {
    const n = i + 1;
    if (dirty && n === 2) return 'LINE TWO\n';
    if (dirty && n === 15) return 'LINE FIFTEEN\n';
    return `line ${n}\n`;
  }).join('');
}

/** Seed a git repo on disk and register it; returns the dir + the repo id. */
async function seedRepo(
  name: string,
  files: Record<string, string>,
  dirty: Record<string, string>,
): Promise<{ dir: string; repoId: string }> {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-hunk-'));
  const git = (...a: string[]) => execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  git('config', 'core.autocrlf', 'false');
  for (const [f, body] of Object.entries(files)) writeFileSync(join(dir, f), body);
  git('add', '.');
  git('commit', '-q', '-m', 'init');
  for (const [f, body] of Object.entries(dirty)) writeFileSync(join(dir, f), body);

  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, { data: { path: dir, name } });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  const repoId = (await r.json()).id as string;
  await ctx.dispose();
  return { dir, repoId };
}

/** Open the repo tab, select the WIP row, click `path` → its diff is shown. */
async function openWipFile(page: Page, name: string, path: string) {
  await openPage(page, 'git');
  const existingTab = page.locator('.git-tab-name', { hasText: name });
  if (await existingTab.count()) {
    await existingTab.first().click();
  } else {
    await page.locator('.git-tab-new').click();
    const menu = page.locator('.ctx-menu');
    await menu.locator('.ctx-search-input').fill(name);
    await menu.getByRole('menuitem', { name }).first().click();
  }
  await expect(page.locator('.rv-tabs')).toBeVisible();

  const wipRow = page.locator('.wip-row');
  await expect(wipRow).toBeVisible({ timeout: 15_000 });
  await wipRow.click();
  const panel = page.locator('.wip-panel');
  await expect(panel).toBeVisible();
  await panel.locator('.wp-name', { hasText: path }).first().click();
  await expect(panel.locator('.wp-diff')).toBeVisible();
  return panel;
}

/** `git` stdout for a repo, as a string. */
function gitOut(dir: string, ...a: string[]): string {
  return execFileSync('git', ['-C', dir, ...a], { encoding: 'utf8' });
}

test('stage one of two hunks → commit → the other hunk stays in WIP', async ({ page }) => {
  const name = 'e2e-hunk-stage-repo';
  const { dir } = await seedRepo(name, { 'two.txt': lines(false) }, { 'two.txt': lines(true) });
  const panel = await openWipFile(page, name, 'two.txt');

  // Two separated hunks, each with its own actions.
  const headers = panel.locator('.hunk-header');
  await expect(headers).toHaveCount(2);
  await headers.first().getByRole('button', { name: 'Stage hunk' }).click();

  // The file is now in BOTH trees — that is what "partial" means (so the badge
  // renders on the row in each tree).
  await expect(panel.locator('.chip.partial').first()).toBeVisible({ timeout: 10_000 });
  await expect(panel.locator('.wp-diff')).toBeVisible();

  const commitBtn = panel.locator('.btn.primary');
  await expect(commitBtn).toContainText('Commit (1)');
  await panel.locator('.subject-input').fill('feat: only the first hunk');
  await commitBtn.click();

  // Only hunk 1 landed; hunk 2 is still dirty, so the WIP row survives.
  await expect(page.locator('.graph-row', { hasText: 'feat: only the first hunk' })).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.locator('.wip-row')).toBeVisible();
  const committed = gitOut(dir, 'show', 'HEAD:two.txt');
  expect(committed).toContain('LINE TWO\n');
  expect(committed).toContain('line 15\n');
  expect(gitOut(dir, 'diff', '--', 'two.txt')).toContain('+LINE FIFTEEN');
});

test('discard one hunk restores only that hunk and keeps a backup stash', async ({ page }) => {
  const name = 'e2e-hunk-discard-repo';
  const { dir, repoId } = await seedRepo(
    name,
    { 'two.txt': lines(false) },
    { 'two.txt': lines(true) },
  );
  const panel = await openWipFile(page, name, 'two.txt');

  await panel.locator('.hunk-header').first().getByRole('button', { name: 'Discard hunk' }).click();
  const sheet = page.locator('.sheet');
  await expect(sheet).toContainText('backup stash');
  await sheet.getByRole('button', { name: 'Discard' }).click();

  // Only hunk 1 is reverted; hunk 2's change is untouched.
  await expect(page.locator('.wip-row')).toBeVisible({ timeout: 15_000 });
  await expect
    .poll(() => readFileSync(join(dir, 'two.txt'), 'utf8'), { timeout: 10_000 })
    .toContain('line 2\n');
  const after = readFileSync(join(dir, 'two.txt'), 'utf8');
  expect(after).toContain('LINE FIFTEEN\n');
  expect(after).not.toContain('LINE TWO\n');

  // The discarded bytes are recoverable from the listed backup stash.
  const { ctx, base } = await apiCtx();
  const stashes = await (await ctx.get(`${base}/api/v1/repos/${repoId}/stashes`)).json();
  expect(
    (stashes as { message: string }[]).some((s) =>
      s.message.includes('otto: backup before hunk discard'),
    ),
  ).toBe(true);
  await ctx.dispose();
});

test('a CRLF file staged by hunk leaves no follow-up worktree diff', async ({ page }) => {
  const name = 'e2e-hunk-crlf-repo';
  const base0 = 'c1\r\nc2\r\nc3\r\nc4\r\nc5\r\n';
  const { dir, repoId } = await seedRepo(
    name,
    { 'crlf.txt': base0 },
    { 'crlf.txt': 'c1\r\nC2\r\nc3\r\nc4\r\nc5\r\n' },
  );
  const panel = await openWipFile(page, name, 'crlf.txt');

  await panel.locator('.hunk-header').first().getByRole('button', { name: 'Stage hunk' }).click();
  await expect(panel.locator('.chip.partial')).toHaveCount(0, { timeout: 10_000 });

  // A patch rebuilt from the `\r`-stripping parser would stage LF content and
  // leave the whole file dirty again. Nothing may remain.
  const { ctx, base } = await apiCtx();
  await expect
    .poll(
      async () => {
        const r = await ctx.get(
          `${base}/api/v1/repos/${repoId}/diff?target=worktree&path=crlf.txt`,
        );
        return ((await r.json()) as { files: unknown[] }).files.length;
      },
      { timeout: 10_000 },
    )
    .toBe(0);
  await ctx.dispose();
  expect(gitOut(dir, 'show', ':crlf.txt')).toBe('c1\r\nC2\r\nc3\r\nc4\r\nc5\r\n');
});
