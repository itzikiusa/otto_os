// Shared fixture for the commit card specs (desktop-git-commit-card.spec.ts,
// git-commit-card-mobile.spec.ts): a dirty repository whose origin names a
// GitHub URL (so Otto offers pull requests) while pushes land in a local bare
// repository (`remote.origin.pushurl`) — nothing leaves the machine.
import { expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { openPage } from './helpers';
import { apiCtx, seedWorkspace, tempDir } from './seed';

export interface CardRepo {
  id: string;
  dir: string;
  bare: string;
  branch: string;
}

export async function seedCardRepo(name: string, edit = 'hello world\n'): Promise<CardRepo> {
  const root = tempDir('otto-e2e-card-');
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
  writeFileSync(join(dir, 'app.txt'), edit);
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
export async function stubForge(page: Page, prs: () => unknown[] = () => [], draft = true): Promise<void> {
  await page.route('**/api/v1/git/accounts', (r) => r.fulfill({ json: [{ id: 'acc1', provider: 'github', label: 'me', username: 'me' }] }));
  await page.route(/\/api\/v1\/repos\/[^/]+\/prs\?/, (r) => r.fulfill({ json: { items: prs(), has_more: false, page: 1, per_page: 50 } }));
  if (draft) await page.route('**/draft-commit-message', (r) =>
    r.fulfill({ json: { message: 'feat: ship the commit card\n\n- one primary that follows state', from_staged: true } }),
  );
  await page.route(/\/api\/v1\/repos\/[^/]+\/fetch(\?|$)/, (r) => r.fulfill({ status: 502, json: { code: 'upstream', message: 'offline in tests' } }));
}

/** The commit that reached the bare remote's branch. */
export function remoteSubject(repo: CardRepo): string {
  return execFileSync('git', ['--git-dir', repo.bare, 'log', '-1', '--format=%s', repo.branch]).toString().trim();
}

/** A dirty repository opens on History with its WIP row selected: the commit
 *  card over every change, included, the message drafted. */
export async function expectWipFirst(page: Page): Promise<void> {
  await expect(page.locator('.rv-tab[aria-selected="true"]')).toHaveText('History', { timeout: 15_000 });
  const wip = page.locator('.wip-row');
  await expect(wip).toHaveAttribute('aria-pressed', 'true', { timeout: 15_000 });
  const card = page.getByTestId('commit-card');
  await expect(card).toBeVisible();
  // The count is said once, on the WIP row (design review 2, H4): the card
  // says all is included, the Included header carries the +/− totals, and no
  // lone "+1 −1" line floats between them.
  await expect(wip).toContainText('2 files changed');
  await expect(card.getByTestId('commit-card-included')).toHaveText('all files included');
  await expect(page.locator('[data-wp-section="staged"] .wp-sec-count')).toHaveCount(0);
  await expect(page.getByTestId('wip-section-lines')).toHaveText(/\+1\s*−1/);
  await expect(page.locator('.wp-totals')).toHaveCount(0);
  // Drafted from the diff (the stubbed drafting agent), ready to send.
  await expect(card.locator('.subject-input')).toHaveValue('feat: ship the commit card', { timeout: 15_000 });
  await expect(card).toContainText('Drafted from the diff');
}

/** "Draft from the diff" against the daemon's real headless draft (the e2e
 *  `claude` is otto-fake-claude): the message streams into the fields with
 *  "Drafting… · Stop" beside it, Stop keeps what arrived, and the finished
 *  draft commits and pushes. `fake-draft: slow` in the diff paces the
 *  stream (four pieces, 1.5 s apart). */
export async function expectDraftStreams(page: Page, name: string): Promise<void> {
  const repo = await seedCardRepo(name, 'hello world\nfake-draft: slow\n');
  await stubForge(page, () => [], false);
  await openPage(page, `git/${repo.id}`);
  const card = page.getByTestId('commit-card');
  const subject = card.locator('.subject-input');
  const body = card.locator('.body-input');
  const drafting = card.getByTestId('commit-card-drafting');
  // Opening drafts by itself (openPage waits that one out); drafting by hand
  // after clearing the fields is the one watched here.
  await expect(body).toHaveValue('- say hello world', { timeout: 20_000 });
  const again = async (): Promise<void> => {
    await subject.fill('');
    await body.fill('');
    await card.getByRole('button', { name: 'Draft from the diff' }).click();
  };
  await again();
  // The subject arrives before the body, while "Drafting… · Stop" shows.
  await expect(subject).toHaveValue('feat(app): greet the world', { timeout: 10_000 });
  await expect(drafting).toBeVisible();
  await expect(body).toHaveValue('');
  await expect(body).toHaveValue('- say hello world', { timeout: 10_000 });
  await expect(drafting).toHaveCount(0);
  await expect(card.locator('.wc-commit')).toBeEnabled();
  // Stopped mid-stream: what arrived stays.
  await again();
  await expect(subject).toHaveValue('feat(app): greet the world', { timeout: 10_000 });
  await card.getByRole('button', { name: 'Stop' }).click();
  await expect(drafting).toHaveCount(0);
  await page.waitForTimeout(4000); // ui-guards: allow — proves nothing more streams in after Stop
  await expect(subject).toHaveValue('feat(app): greet the world');
  await expect(body).toHaveValue('');
  await expect(card).toContainText('Drafted from the diff');
  await card.locator('.wc-commit').click();
  await expect.poll(() => remoteSubject(repo), { timeout: 20_000 }).toBe('feat(app): greet the world');
}

/** A draft that fails says why in one line, with Retry — never an idle card. */
export async function expectDraftFailureSaysWhy(page: Page, name: string): Promise<void> {
  const repo = await seedCardRepo(name, 'hello world\nfake-draft: fail\n');
  await stubForge(page, () => [], false);
  await openPage(page, `git/${repo.id}`);
  const card = page.getByTestId('commit-card');
  const why = card.getByTestId('load-error');
  await expect(why).toContainText(/Couldn’t draft the message\. .*Not logged in/, { timeout: 20_000 });
  await expect(card.getByRole('button', { name: 'Draft from the diff' })).toHaveCount(0);
  await card.getByRole('button', { name: 'Retry' }).click();
  await expect(why).toContainText('Not logged in', { timeout: 20_000 });
  await expect(card.getByTestId('commit-card-drafting')).toHaveCount(0);
}
