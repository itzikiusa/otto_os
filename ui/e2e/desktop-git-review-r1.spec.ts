import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expectFullyInViewport, openPage } from './helpers';
import { apiCtx, seedWorkspace, tempDir } from './seed';
import { seedCardRepo, stubForge } from './git-commit-card-fixture';

// Git review round 1 (R1-013 · 015 · 016 · 031 · 045 · 067 · 030 · UX-G1):
// deep links that open the repo they name, a commit draft that survives a
// stop, a failure and a remount, bulk delete of a branch both locally and on
// its remote, the stash count after the browser drops one, a rate limit in
// words, Focus's empty copy, and repo tabs that never sit cut at the edge.

test.use({ serviceWorkers: 'block' });

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => {
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
  });
});

const activeTab = (page: Page) => page.locator('.git-tablist .git-tab.active');
const replaceAsk = (page: Page) => page.getByRole('dialog').filter({ hasText: 'Replace your title and description?' });
const go = (page: Page, hash: string) => page.evaluate((h) => (location.hash = h), hash);

async function twoRepos(page: Page, tag: string): Promise<[string, string]> {
  const a = await seedCardRepo(tag + '-a');
  const b = await seedCardRepo(tag + '-b');
  await stubForge(page);
  await openPage(page, 'git/' + a.id + '/graph');
  await expect(activeTab(page)).toContainText(tag + '-a', { timeout: 30_000 });
  return [a.id, b.id];
}
/** /repos/<fake> answers with repo `real` under the fake id (a duplicate row /git/repos dropped), after `delay` ms. */
async function dupLookup(page: Page, fake: string, real: string, delay = 0): Promise<void> {
  const { ctx, base } = await apiCtx();
  const all = (await (await ctx.get(base + '/api/v1/git/repos')).json()) as { id: string }[];
  const json = { ...all.find((r) => r.id === real), id: fake };
  await ctx.dispose();
  await page.route(new RegExp('/api/v1/repos/' + fake + '$'), async (route) => {
    await new Promise((res) => setTimeout(res, delay));
    await route.fulfill({ json });
  });
}

test('R1-013: #/git/<id> with no sub-tab opens that repo', async ({ page }) => {
  const [, b] = await twoRepos(page, 'r1-link1');
  await go(page, '#/git/' + b);
  await expect(activeTab(page)).toContainText('r1-link1-b', { timeout: 10_000 });
});

test('R1-013: a duplicate id\'s /pr/<n> opens the registered repo\'s pull requests', async ({ page }) => {
  const [a, b] = await twoRepos(page, 'r1-link2');
  const dup = '00000000-0000-0000-0000-0000000000d1';
  await dupLookup(page, dup, a);
  await go(page, '#/git/' + b + '/graph');
  await expect(activeTab(page)).toContainText('r1-link2-b', { timeout: 10_000 });
  await go(page, '#/git/' + dup + '/pr/7');
  await expect(activeTab(page)).toContainText('r1-link2-a', { timeout: 10_000 });
  await expect(page.locator('.git-views [role="tab"][aria-selected="true"]')).toContainText('Pull requests');
});

test('R1-013: a duplicate-id lookup that answers after you moved on goes nowhere', async ({ page }) => {
  const [a] = await twoRepos(page, 'r1-link3');
  const slow = '00000000-0000-0000-0000-0000000000d2';
  await dupLookup(page, slow, a, 1500);
  await go(page, '#/git/' + slow + '/graph');
  await go(page, '#/git/focus');
  await page.waitForTimeout(2500); // ui-guards: allow — the held lookup answers at 1.5 s
  await expect(page).toHaveURL(/#\/git\/focus$/);
});

test('R1-015: Keep mine does not silence the next draft', async ({ page }) => {
  const a = await seedCardRepo('r1-draft-retry');
  await stubForge(page, () => [], false);
  let call = 0;
  let release = (): void => {};
  const held = new Promise<void>((r) => (release = r));
  await page.route('**/draft-commit-message', async (route) => {
    const n = ++call;
    const nd = (lines: unknown[]) => route.fulfill({ contentType: 'application/x-ndjson', body: lines.map((l) => JSON.stringify(l)).join('\n') + '\n' });
    if (n === 1) { await held; return nd([{ done: { message: 'feat: agent text', from_staged: true } }]); }
    return nd([{ delta: 'feat: second' }, { done: { message: 'feat: second', from_staged: true } }]);
  });
  await page.goto('/#/git/' + a.id);
  const card = page.getByTestId('commit-card');
  const subject = card.locator('.subject-input');
  // The quiet open-time draft is running; the person types; "Keep mine".
  await expect(card.getByTestId('commit-card-drafting')).toBeVisible({ timeout: 30_000 });
  await subject.fill('mine');
  release();
  await replaceAsk(page).getByRole('button', { name: 'Keep mine' }).click();
  await expect(subject).toHaveValue('mine');
  // The next draft, by hand, still says it is ready.
  await subject.fill('');
  await card.getByRole('button', { name: 'Draft from the diff' }).click();
  await expect(subject).toHaveValue('feat: second', { timeout: 10_000 });
  await expect(page.getByText('Draft ready')).toBeVisible();
});

test('R1-015: a failed stream\'s own fragment is replaced on Retry, without asking', async ({ page }) => {
  const a = await seedCardRepo('r1-draft-retry2');
  await stubForge(page, () => [], false);
  let call = 0;
  await page.route('**/draft-commit-message', async (route) => {
    const nd = (lines: unknown[]) => route.fulfill({ contentType: 'application/x-ndjson', body: lines.map((l) => JSON.stringify(l)).join('\n') + '\n' });
    if (++call === 1) return nd([{ delta: 'feat: add' }, { error: 'boom' }]);
    return nd([{ delta: 'feat: ship it' }, { done: { message: 'feat: ship it\n\n- the body', from_staged: true } }]);
  });
  await page.goto('/#/git/' + a.id);
  const card = page.getByTestId('commit-card');
  const subject = card.locator('.subject-input');
  // The open-time draft wrote "feat: add" into the empty fields, then failed.
  await expect(card.getByTestId('load-error')).toContainText('boom', { timeout: 30_000 });
  await expect(subject).toHaveValue('feat: add');
  await card.getByRole('button', { name: 'Retry' }).click();
  await expect(subject).toHaveValue('feat: ship it', { timeout: 10_000 });
  await expect(replaceAsk(page)).toHaveCount(0);
});

async function slowDraftCard(page: Page, tag: string) {
  const a = await seedCardRepo(tag, 'hello world\nfake-draft: slow\n');
  await stubForge(page, () => [], false);
  await openPage(page, 'git/' + a.id);
  const card = page.getByTestId('commit-card');
  const subject = card.locator('.subject-input');
  const body = card.locator('.body-input');
  await expect(body).toHaveValue('- say hello world', { timeout: 20_000 });
  return { card, subject, body };
}

test('R1-015: Stop while the draft streams under typed text asks nothing and keeps the text', async ({ page }) => {
  const { card, subject, body } = await slowDraftCard(page, 'r1-draft-stop');
  // Stop while the draft streams under the person's own text.
  await subject.fill('my own summary');
  await body.fill('');
  await card.getByRole('button', { name: 'Commit options' }).click();
  await page.getByRole('menuitem', { name: 'Draft the message again' }).click();
  await page.waitForTimeout(2500); // ui-guards: allow — the first piece lands at ~1.5 s
  await card.getByRole('button', { name: 'Stop' }).click();
  await page.waitForTimeout(500); // ui-guards: allow — a confirm would open now
  await expect(replaceAsk(page)).toHaveCount(0);
  await expect(subject).toHaveValue('my own summary');
});

test('R1-045: Pull requests and back mid-stream: the draft keeps streaming into the fields', async ({ page }) => {
  const { card, subject, body } = await slowDraftCard(page, 'r1-draft-remount');
  await subject.fill('');
  await body.fill('');
  await card.getByRole('button', { name: 'Draft from the diff' }).click();
  await expect(subject).toHaveValue('feat(app): greet the world', { timeout: 10_000 });
  await page.locator('.git-views [role="tab"]', { hasText: 'Pull requests' }).click();
  await page.locator('.git-views [role="tab"]', { hasText: 'History' }).click();
  await expect(page.getByTestId('commit-card').locator('.body-input')).toHaveValue('- say hello world', { timeout: 15_000 });
  await expect(replaceAsk(page)).toHaveCount(0);
});

test('R1-045: a piece that streams while on Pull requests is caught up on return, and the end lands without asking', async ({ page }) => {
  const { card, subject, body } = await slowDraftCard(page, 'r1-draft-away');
  await subject.fill('');
  await body.fill('');
  await card.getByRole('button', { name: 'Draft from the diff' }).click();
  await expect(subject).toHaveValue('feat(app): greet the world', { timeout: 10_000 });
  await page.locator('.git-views [role="tab"]', { hasText: 'Pull requests' }).click();
  await page.waitForTimeout(2000); // ui-guards: allow — the body's first piece lands while away (1.5 s apart)
  await page.locator('.git-views [role="tab"]', { hasText: 'History' }).click();
  const back = page.getByTestId('commit-card');
  await expect(back.locator('.body-input')).toHaveValue(/^- say hello/, { timeout: 3000 });
  await expect(back.locator('.body-input')).toHaveValue('- say hello world', { timeout: 15_000 });
  await page.waitForTimeout(500); // ui-guards: allow — a confirm would open now
  await expect(replaceAsk(page)).toHaveCount(0);
});

test('R1-045: a draft that finishes while on Pull requests lands on return without asking', async ({ page }) => {
  const { card, subject, body } = await slowDraftCard(page, 'r1-draft-done-away');
  await subject.fill('');
  await body.fill('');
  await card.getByRole('button', { name: 'Draft from the diff' }).click();
  await expect(subject).toHaveValue('feat(app): greet the world', { timeout: 10_000 });
  await page.locator('.git-views [role="tab"]', { hasText: 'Pull requests' }).click();
  await page.waitForTimeout(6000); // ui-guards: allow — the rest streams and the draft ends while away
  await page.locator('.git-views [role="tab"]', { hasText: 'History' }).click();
  await expect(page.getByTestId('commit-card').locator('.body-input')).toHaveValue('- say hello world', { timeout: 10_000 });
  await page.waitForTimeout(500); // ui-guards: allow — a confirm would open now
  await expect(replaceAsk(page)).toHaveCount(0);
});

test('R1-016: local feat/x AND origin/feat/x — the force retry force-deletes the LOCAL branch', async ({ page }) => {
  const root = tempDir('otto-e2e-r1bd-');
  const origin = join(root, 'origin.git');
  const dir = join(root, 'work');
  execFileSync('git', ['init', '-q', '--bare', '-b', 'main', origin]);
  execFileSync('git', ['init', '-q', '-b', 'main', dir]);
  const git = (...x: string[]): void => void execFileSync('git', ['-C', dir, ...x], { stdio: 'ignore' });
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'E2E');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'a.txt'), 'a\n');
  git('add', '.');
  git('commit', '-q', '-m', 'init');
  git('remote', 'add', 'origin', origin);
  git('push', '-q', '-u', 'origin', 'main');
  git('checkout', '-q', '-b', 'feat/x');
  git('push', '-q', '-u', 'origin', 'feat/x');
  // Ahead of its upstream but merged into main: `git branch -d` refuses it.
  git('commit', '-q', '--allow-empty', '-m', 'x');
  git('checkout', '-q', 'main');
  git('merge', '-q', '--ff-only', 'feat/x');
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const r = await ctx.post(base + '/api/v1/workspaces/' + ws + '/repos', { data: { path: dir, name: 'r1-bulkdel' } });
  const id = ((await r.json()) as { id: string }).id;
  await ctx.dispose();
  await page.goto('/#/git/' + id + '/graph');
  await expect(page.locator('.refs-panel')).toBeVisible({ timeout: 30_000 });
  await page.getByRole('button', { name: 'Delete several local branches', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Delete branches' });
  await dialog.getByRole('checkbox', { name: 'Select feat/x', exact: true }).check();
  await dialog.getByRole('tab', { name: /^Remote/ }).click();
  await dialog.getByRole('checkbox', { name: 'Select origin/feat/x', exact: true }).check();
  await dialog.getByRole('button', { name: 'Delete 2 branches…', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete 1 remote branch', exact: true }).click();
  const rows = dialog.getByRole('list', { name: 'Delete results' }).getByRole('listitem');
  await expect(rows).toHaveCount(2, { timeout: 30_000 });
  await expect(rows.nth(0)).toContainText('needs force');
  await expect(rows.nth(1)).toContainText('origin/feat/x');
  await dialog.getByRole('button', { name: 'Force delete 1 unmerged…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Force delete 1', exact: true }).click();
  await expect(dialog.locator('.res.fail')).toHaveCount(0, { timeout: 15_000 });
  await expect(rows).toHaveCount(2);
  const heads = execFileSync('git', ['-C', dir, 'for-each-ref', '--format=%(refname:short)', 'refs/heads/']).toString();
  expect(heads.split('\n').filter(Boolean)).toEqual(['main']);
});

test('R1-031: dropping the last stash in the browser disables the header Pop', async ({ page }) => {
  const a = await seedCardRepo('r1-stash');
  execFileSync('git', ['-C', a.dir, 'stash', 'push', '-q', '--include-untracked', '-m', 'e2e stash']);
  writeFileSync(join(a.dir, 'app.txt'), 'still dirty\n');
  await stubForge(page);
  // The legacy Changes link opens History (on the WIP row).
  await openPage(page, 'git/' + a.id + '/changes');
  await expect(page.locator('.wip-row')).toHaveAttribute('aria-pressed', 'true', { timeout: 30_000 });
  const pop = page.locator('.git-header .ph-actions').getByRole('button', { name: /^Pop\b/ });
  await expect(pop).toBeEnabled({ timeout: 30_000 });
  await page.locator('.git-header .ph-actions').getByRole('button', { name: 'Stashes' }).click();
  await page.getByRole('menuitem', { name: /Browse stashes/ }).click();
  const browser = page.getByRole('dialog', { name: 'Stashes' });
  await browser.getByRole('button', { name: 'Drop…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Drop', exact: true }).click();
  await expect(page.getByText('Stash dropped')).toBeVisible({ timeout: 15_000 });
  await page.keyboard.press('Escape');
  await expect(pop).toBeDisabled();
});

test('R1-067: a forge rate limit says so, not "unreachable"; R1-030: Focus names the linked repos', async ({ page }) => {
  const a = await seedCardRepo('r1-ratelimit');
  await stubForge(page);
  await page.route(/\/api\/v1\/repos\/[^/]+\/prs\?/, (r) => r.fulfill({ status: 502, json: { code: 'upstream', message: 'github rate limited — retry in 90s' } }));
  await openPage(page, 'git/' + a.id + '/prs');
  await expect(page.getByText(/rate-limited this request/)).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText(/unreachable/i)).toHaveCount(0);
  await go(page, '#/git/focus');
  await expect(page.getByText('No repository is linked to a git account')).toBeVisible({ timeout: 20_000 });
});

test('UX-G1: open repo tabs shrink instead of sitting cut at the header edge; the verbs stay', async ({ page }) => {
  const clean = await seedCardRepo('payments-gateway-service-synced');
  execFileSync('git', ['-C', clean.dir, 'add', '-A']);
  execFileSync('git', ['-C', clean.dir, '-c', 'user.email=e2e@otto.local', '-c', 'user.name=E2E', 'commit', '-q', '-m', 'wip']);
  const ids = [clean.id, (await seedCardRepo('customer-onboarding-portal-dirty')).id];
  await stubForge(page);
  // Auto-fetch on (its fetches are stubbed): no "Auto-fetch paused" words in the row.
  await page.addInitScript(() => localStorage.removeItem('otto_git_auto_fetch'));
  for (const id of ids) {
    await openPage(page, 'git/' + id + '/graph');
    await expect(page.locator('.refs-panel')).toBeVisible({ timeout: 30_000 });
  }
  await page.waitForTimeout(500); // ui-guards: allow — the header's fit runs on the next frame
  // Tabs that fit are shown whole; a strip too tight turns into the "<repo> ▾" switcher (R2-001).
  await expect(page.locator('.git-header .git-tablist .git-tab.active:visible, .git-header [data-testid="git-repo-switch"]:visible').first()).toContainText('customer-onboarding-portal-dirty');
  const cut = await page.evaluate(() => {
    if (document.querySelector('.git-tabs.compact')) return [];
    const list = document.querySelector('.git-tablist')!.getBoundingClientRect();
    const outer = document.querySelector('.git-header .ph-tabs-inline')!.getBoundingClientRect();
    return Array.from(document.querySelectorAll('.git-tablist .git-tab')).flatMap((t) => {
      const r = t.getBoundingClientRect();
      const lo = Math.max(list.left, outer.left), hi = Math.min(list.right, outer.right);
      return r.left < lo - 0.5 || r.right > hi + 0.5 ? [t.textContent?.trim() + ' ' + [r.left, r.right, lo, hi].map(Math.round).join(',')] : [];
    });
  });
  expect(cut, 'tabs cut at the strip edge').toEqual([]);
  const actions = page.locator('.git-header .ph-actions');
  for (const v of [/^Fetch/, /^Pull\b/, /^(Push|Publish)\b/, /^Stash\b/, /^Pop\b/]) await expect(actions.getByRole('button', { name: v }).first()).toBeInViewport();
  await expect(actions.locator('.tbtn.icon-only'), 'every verb keeps its label at 1440 px').toHaveCount(0);
  await page.screenshot({ path: test.info().outputPath('git-tabs-light.png') });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: test.info().outputPath('git-tabs-dark.png') });
});

test('Git header matrix: Fetch · Pull · Push · Stash · Pop, ⋯, Focus, + and the active repo name are never clipped (1440/1280/1100 × right panel × 1/4 tabs × a long branch)', async ({ page }) => {
  test.setTimeout(180_000);
  const repos = [];
  for (const n of ['payments-gateway-service', 'customer-onboarding-portal', 'wallet-ledger-reconciliation', 'bo_common_ui']) repos.push(await seedCardRepo('hdr-' + n));
  const long = 'feature/PLAT-4821-wallet-gateway-retry-with-exponential-backoff-and-jitter';
  execFileSync('git', ['-C', repos[0].dir, 'checkout', '-q', '-b', long]);
  await stubForge(page);
  const active = repos[0].id;
  const verbs = [/^Fetch/, /^Pull\b/, /^Pull options$/, /^(Push|Publish)\b/, /^Stash\b/, /^Pop\b/, /^Stashes$/];
  for (const width of [1440, 1280, 1100]) {
    for (const panel of [true, false]) {
      for (const tabs of [1, 4]) {
        const what = width + 'px · panel ' + (panel ? 'open' : 'collapsed') + ' · ' + tabs + ' tab(s)';
        await page.setViewportSize({ width, height: 900 });
        await page.goto('/#/git');
        await page.evaluate(([open, ids, act]) => {
          localStorage.setItem('otto_right_open', open ? '1' : '0');
          localStorage.setItem('otto_right_tab', 'files');
          localStorage.setItem('otto_git_open_tabs', JSON.stringify({ openRepoIds: ids, activeRepoId: act, sub: {} }));
        }, [panel, repos.slice(0, tabs).map((r) => r.id), active] as const);
        await page.reload();
        const actions = page.locator('.git-header .ph-actions-wrap');
        await expect(actions.getByRole('button', { name: /^Fetch/ })).toBeVisible({ timeout: 30_000 });
        await page.waitForTimeout(400); // ui-guards: allow — the header's fit settles over a frame or two
        // The active repo stays named — a tab, or the compact "<repo> ▾" switcher — and Focus and + stay whole (R2-001).
        const repo = page.locator('.git-header .git-tablist .git-tab.active:visible, .git-header [data-testid="git-repo-switch"]:visible').first();
        await expect(repo, what + ': the active repo is named').toContainText('hdr-payments-gateway-service');
        const shown = await repo.evaluate((el) => { const n = el.querySelector<HTMLElement>('.git-tab-name, .git-repo-switch-name')!; return { w: n.clientWidth, full: n.scrollWidth }; });
        expect(shown.w, what + ': the active repo name is readable (' + shown.w + ' of ' + shown.full + ' px)').toBeGreaterThanOrEqual(Math.min(shown.full, 64));
        const targets = [...verbs.map((v) => actions.getByRole('button', { name: v }).first()), page.locator('.git-header .ph-more'), page.locator('.git-header .git-focus'), page.locator('.git-header .git-tab-new'), repo];
        for (const t of targets) {
          await expectFullyInViewport(page, t, what + ' · ' + (await t.getAttribute('aria-label') ?? (await t.textContent())?.trim()));
          const hit = await t.evaluate((el) => {
            const r = el.getBoundingClientRect();
            const at = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
            return !!at && (el === at || el.contains(at));
          });
          expect(hit, what + ': ' + (await t.textContent())?.trim() + ' is under the pointer, not clipped').toBe(true);
        }
        if (panel && tabs === 4) await page.screenshot({ path: test.info().outputPath('header-' + width + '-panel-open-4tabs-light.png') });
      }
    }
  }
});
