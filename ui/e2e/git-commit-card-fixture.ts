// Shared fixture for the commit card specs (desktop-git-commit-card.spec.ts,
// git-commit-card-mobile.spec.ts): a dirty repository whose origin names a
// GitHub URL (so Otto offers pull requests) while pushes land in a local bare
// repository (`remote.origin.pushurl`) — nothing leaves the machine.
import { expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

export interface CardRepo {
  id: string;
  dir: string;
  bare: string;
  branch: string;
}

export async function seedCardRepo(name: string): Promise<CardRepo> {
  const root = mkdtempSync(join(tmpdir(), 'otto-e2e-card-'));
  const bare = join(root, 'remote.git');
  const dir = join(root, 'work');
  const branch = 'feat/commit-card';
  execFileSync('git', ['init', '-q', '--bare', '-b', 'main', bare]);
  execFileSync('git', ['init', '-q', '-b', 'main', dir]);
  const git = (...a: string[]): void => void execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'app.txt'), 'hello\n');
  git('add', '.');
  git('commit', '-q', '-m', 'init');
  git('remote', 'add', 'origin', `https://github.com/acme/${name}.git`);
  git('remote', 'set-url', '--push', 'origin', bare);
  git('push', '-q', '-u', 'origin', 'main');
  git('checkout', '-q', '-b', branch);
  git('push', '-q', '-u', 'origin', branch);
  // The work: one edit and one new file.
  writeFileSync(join(dir, 'app.txt'), 'hello world\n');
  writeFileSync(join(dir, 'notes.txt'), 'todo\n');
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/repos`, { data: { path: dir, name } });
  if (!r.ok()) throw new Error(`repo seed failed: ${r.status()} ${await r.text()}`);
  const id = ((await r.json()) as { id: string }).id;
  await ctx.dispose();
  return { id, dir, bare, branch };
}

/** The forge side, offline: a GitHub account (so the PR lookup runs), no open
 *  PR yet, a drafted message, and fetches that never reach github.com. */
export async function stubForge(page: Page, prs: () => unknown[] = () => []): Promise<void> {
  await page.route('**/api/v1/git/accounts', (r) => r.fulfill({ json: [{ id: 'acc1', provider: 'github', label: 'me', username: 'me' }] }));
  await page.route(/\/api\/v1\/repos\/[^/]+\/prs\?/, (r) => r.fulfill({ json: { items: prs(), has_more: false, page: 1, per_page: 50 } }));
  await page.route('**/draft-commit-message', (r) =>
    r.fulfill({ json: { message: 'feat: ship the commit card\n\n- one primary that follows state', from_staged: true } }),
  );
  await page.route(/\/api\/v1\/repos\/[^/]+\/fetch$/, (r) => r.fulfill({ status: 502, json: { code: 'upstream', message: 'offline in tests' } }));
}

/** The commit that reached the bare remote's branch. */
export function remoteSubject(repo: CardRepo): string {
  return execFileSync('git', ['--git-dir', repo.bare, 'log', '-1', '--format=%s', repo.branch]).toString().trim();
}

export async function expectChangesFirst(page: Page): Promise<void> {
  await expect(page.locator('.rv-tab[aria-selected="true"]')).toContainText('Changes', { timeout: 15_000 });
  const card = page.getByTestId('commit-card');
  await expect(card).toBeVisible();
  await expect(card.getByTestId('commit-card-included')).toHaveText('2 files included');
  // Drafted from the diff (the stubbed drafting agent), ready to send.
  await expect(card.locator('.subject-input')).toHaveValue('feat: ship the commit card', { timeout: 15_000 });
  await expect(card).toContainText('Drafted from the diff');
}
