import { test, expect, type Page, type Route } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Route fixtures must bypass the mobile service worker.
test.use({ serviceWorkers: 'block' });

// Real isolated repository; only errors, races and forge responses are stubbed.
let repoId = '';
let workspaceId = '';
let firstSha = '';
let secondSha = '';
let repoDir = '';
let worktreePath = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-ux-git-'));
  repoDir = dir;
  worktreePath = join(mkdtempSync(join(tmpdir(), 'otto-ux-linked-')), 'review-worktree');
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'ux@otto.local');
  git('config', 'user.name', 'UX audit');
  git('config', 'commit.gpgsign', 'false');
  writeFileSync(join(dir, 'first.txt'), 'first version\n');
  git('add', '.'); git('commit', '-qm', 'First audit commit');
  firstSha = git('rev-parse', 'HEAD');
  writeFileSync(join(dir, 'second.txt'), 'second version\n');
  git('add', '.'); git('commit', '-qm', 'Second audit commit');
  secondSha = git('rev-parse', 'HEAD');
  git('branch', 'feature/a-long-branch-for-keyboard-actions');
  for (let n = 0; n < 30; n++) git('branch', `release/long-reference-${n}`);
  writeFileSync(join(dir, 'first.txt'), 'stash contents\n');
  git('stash', 'push', '-m', 'Audit saved changes');
  git('branch', 'origin/collision');
  git('update-ref', 'refs/remotes/origin/collision', firstSha);
  git('remote', 'add', 'origin', 'https://github.com/otto-test/ux-audit.git');
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/repos`, { data: { path: dir, name: 'UX audit repository' } });
  expect(response.ok()).toBeTruthy();
  repoId = (await response.json()).id;
  await ctx.dispose();
});
test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_git_auto_fetch', JSON.stringify({ enabled: false }));
  }, workspaceId);
});
async function openRepo(page: Page, tab = 'graph') {
  await page.goto(`/#/git/${repoId}/${tab}`);
  await expect(page.locator('.rv-tabs')).toBeVisible();
}
function diff(path: string) {
  return { files: [{ path, old_path: null, is_binary: false, hunks: [{ header: '@@ -0,0 +1 @@', lines: [{ origin: 'add', content: path, old_line: null, new_line: 1 }] }] }] };
}
function pr(number: number, state = 'open') {
  return { number, title: `${state} request ${number}`, state, author: 'audit', source_branch: 'feature/a', target_branch: 'main', url: `https://github.com/otto-test/ux-audit/pull/${number}`, updated_at: '2026-09-25T08:00:00Z', draft: false };
}


const longPath = `src/${'long-directory/'.repeat(10)}reviewed-file.ts`;
async function prFixtures(page: Page, inline = false) {
  await page.route('**/api/v1/repos/*/prs/*', async route => {
    const path = new URL(route.request().url()).pathname;
    if (/\/prs\/\d+$/.test(path)) {
      const number = Number(path.split('/').at(-1));
      await route.fulfill({ json: { ...pr(number), title: `PR ${number}: ${'Long pull request description '.repeat(4)}`, source_branch: `feature/${'long-branch-'.repeat(9)}`, description_md: '## Change\n\n' + 'Readable summary of the change. '.repeat(50), approved_by: [], reviewers: [], mergeable: true, comments: [{ id: 'comment-1', author: 'reviewer', body: 'Discussion text '.repeat(30), path: inline ? longPath : null, line: inline ? 1 : null, created_at: '2026-09-25T08:00:00Z', replies: [], resolved: false }] } });
    } else await route.fallback();
  });
  await page.route('**/api/v1/repos/*/prs/*/diff', r => r.fulfill({ json: diff(longPath) }));
  await page.route('**/api/v1/repos/*/prs/*/commits', r => r.fulfill({ json: [{ sha: secondSha, message: 'Long commit summary '.repeat(10), author: 'Audit author', date: '2026-09-25T08:00:00Z' }] }));
  await page.route('**/api/v1/repos/*/prs/*/reviews', r => r.fulfill({ json: [{ id: 'review-1', repo_id: repoId, pr_number: 1, status: 'done', error: null, comments: [{ id: 'finding-1', review_id: 'review-1', path: longPath, line: 1, severity: 'bug', body: 'Preserve the current account when accepting this response. '.repeat(8), state: 'draft', posted: false, created_at: '2026-09-25T08:00:00Z' }], agents: [], created_at: '2026-09-25T08:00:00Z', verdict: 'request_changes', blocker_count: 1 }] }));
  await page.route('**/api/v1/reviews/review-1/findings*', r => r.fulfill({ json: [] }));
  await page.route('**/api/v1/reviews/review-1/merge-readiness', r => r.fulfill({ json: { unresolved_blocker_count: 1, unresolved_total: 1, resolved_count: 0, ci_status: 'success', approvals: 0, mergeable: true } }));
}

test('summary reply survives Files and Commits subtab round trips', async ({ page }) => {
  await prFixtures(page);
  await page.goto(`/#/git/${repoId}/pr/1`);
  const thread = page.locator('.cmt').first();
  await thread.getByRole('button', {name: 'Reply', exact: true}).click();
  await thread.getByPlaceholder('Reply…').fill('Keep this reply while I inspect the diff');
  await page.getByRole('tab', {name: 'Files', exact: true}).click();
  await expect(page.locator('.prd-diff')).toBeVisible();
  await page.getByRole('tab', {name: 'Commits', exact: true}).click();
  await page.getByRole('tab', {name: 'Summary', exact: true}).click();
  await expect(thread.getByPlaceholder('Reply…')).toHaveValue('Keep this reply while I inspect the diff');
  await page.evaluate(id => { location.hash = `#/git/${id}/pr/2`; }, repoId);
  await page.getByRole('button', {name: 'Keep editing', exact: true}).click();
  await expect(thread.getByPlaceholder('Reply…')).toHaveValue('Keep this reply while I inspect the diff');
});

test('graph leading branch chip stays inside its cell at 2048px', async ({ page }, testInfo) => {
  await page.setViewportSize({width: 2048, height: 900});
  execFileSync('git', ['-C', repoDir, 'switch', '-c', 'feature/customer-experience-2026']);
  await openRepo(page);
  const cell = page.locator('.graph-row-head .branch-cell');
  await expect(cell.locator('.head-badge')).toBeVisible();
  await page.screenshot({path: testInfo.outputPath('head-label-before.png')});
  const bounds = await cell.evaluate(el => {
    const cell = el.getBoundingClientRect();
    return [...el.querySelectorAll('.ref-select, .head-badge')].map(item => ({left: item.getBoundingClientRect().left - cell.left, right: item.getBoundingClientRect().right - cell.right}));
  });
  expect(bounds.every(b => b.left >= -1 && b.right <= 1), JSON.stringify(bounds)).toBe(true);
});

const themes = [
  {theme: 'native', scheme: 'light', width: 1440, rtl: false},
  {theme: 'native', scheme: 'dark', width: 834, rtl: false},
  {theme: 'warm', scheme: 'light', width: 390, rtl: false},
  {theme: 'warm', scheme: 'dark', width: 390, rtl: true},
  {theme: 'pro-dark', scheme: 'dark', width: 1440, rtl: true},
];
function run(status = 'done') {
  return {id: 'review-4', repo_id: repoId, pr_number: null, status, error: null,
    comments: [{id: 'finding-4', review_id: 'review-4', path: longPath, line: 1, severity: 'bug', body: 'Keep the active repository identity during delayed responses. '.repeat(8), state: 'draft', posted: false, created_at: '2026-09-25T08:00:00Z'}],
    agents: [], created_at: '2026-09-25T08:00:00Z'};
}
for (const variant of themes) test(`history composition ${variant.theme} ${variant.scheme} ${variant.width}`, async ({page}, testInfo) => {
  await page.setViewportSize({width: variant.width, height: 900});
  await page.addInitScript(v => {localStorage.setItem('otto_theme', v.theme); localStorage.setItem('otto_scheme', v.scheme);}, variant);
  await page.route('**/api/v1/repos/*/local-reviews', route => route.fulfill({json: Array.from({length: 24}, (_, i) => ({...run(), id: `run-${i}`}))}));
  await openRepo(page, 'review');
  if (variant.rtl) await page.evaluate(() => {document.documentElement.dir = 'rtl';});
  await page.locator('.lrp-history-toggle').click();
  await expect(page.locator('.lrp-history-run-header')).toHaveCount(24);
  await page.screenshot({path: testInfo.outputPath('history-before.png')});
  const top = await page.locator('.lrp-toolbar').boundingBox();
  const history = await page.locator('.lrp-history').boundingBox();
  expect(history!.y - top!.y - top!.height).toBeLessThan(150);
  await page.locator('.lrp-history-run-header').first().click();
  await expect(page.locator('.lrp-history-run-body')).toContainText('Keep the active repository');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({path: testInfo.outputPath('history-expanded.png')});
});

test('real linked worktree opens and registers its own tab without switching the main checkout', async ({page}) => {
  const before = execFileSync('git', ['-C', repoDir, 'branch', '--show-current'], {encoding: 'utf8'}).trim();
  execFileSync('git', ['-C', repoDir, 'worktree', 'add', '-b', 'review/linked-checkout', worktreePath]);
  await openRepo(page);
  await page.getByRole('button', {name: /WORKTREES/}).click();
  const row = page.locator('.ref-row.is-worktree').filter({hasText: 'review/linked-checkout'});
  await expect(row).toBeVisible();
  await row.click();
  await expect(page.locator('.git-tab.active .git-tab-branch-name')).toHaveText('review/linked-checkout');
  expect(execFileSync('git', ['-C', repoDir, 'branch', '--show-current'], {encoding: 'utf8'}).trim()).toBe(before);
  const {ctx, base} = await apiCtx();
  const repos = await (await ctx.get(`${base}/api/v1/workspaces/${workspaceId}/repos`)).json();
  expect(repos.some((r: {path: string}) => r.path.replace('/private', '') === worktreePath.replace('/private', ''))).toBe(true);
  await ctx.dispose();
});

test('cancelled active PR review cannot be resurrected by an older poll', async ({page}) => {
  await prFixtures(page);
  const active = {...run('running'), pr_number: 1, comments: [], agents: [
    {name: 'Correctness reviewer', provider: 'claude', model: '', status: 'running', session_id: null, findings: []},
    {name: 'Summarizer', provider: 'claude', model: '', status: 'pending', session_id: null, findings: []},
  ]};
  await page.route('**/api/v1/repos/*/prs/1/reviews', r => r.fulfill({json: [active]}));
  let poll: Route | undefined;
  await page.route('**/api/v1/repos/*/prs/1/review', r => {poll = r;});
  await page.route('**/api/v1/reviews/review-4/cancel', r => r.fulfill({json: {...active, status: 'cancelled'}}));
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.getByRole('tab', {name: 'Review', exact: true}).click();
  await expect(page.locator('.rp-agent').first()).toContainText('Correctness reviewer');
  await expect.poll(() => !!poll).toBe(true);
  await page.getByTestId('review-cancel').click();
  await expect(page.getByTestId('review-cancelled')).toBeVisible();
  await poll!.fulfill({json: active});
  await page.waitForTimeout(150);
  await expect(page.getByTestId('review-cancelled')).toBeVisible();
});

test('local review finishes and hands only selected findings to a synthetic agent', async ({page}) => {
  await page.route('**/api/v1/repos/*/local-reviews', r => r.fulfill({json: []}));
  await page.route('**/api/v1/repos/*/local-review', r => r.fulfill({json: run()}));
  await page.route('**/api/v1/reviews/review-4/findings*', r => r.fulfill({json: []}));
  let body: unknown;
  let launchFails = true;
  const session = {id: 'synthetic-handoff', workspace_id: workspaceId, title: 'Selected review fixes', provider: 'claude', kind: 'agent', status: 'idle', cwd: repoDir, meta: {}, archived: false, created_at: '2026-09-25T08:00:00Z', last_active_at: '2026-09-25T08:00:00Z'};
  await page.route('**/api/v1/sessions/synthetic-handoff', r => r.fulfill({json: session}));
  await page.routeWebSocket('**/ws/term/**', socket => { socket.onMessage(() => {}); });
  await page.route('**/api/v1/reviews/review-4/handoff', r => {
    body = r.request().postDataJSON();
    return launchFails ? r.fulfill({status: 503, json: {code: 'upstream', message: 'Synthetic launch failure'}}) : r.fulfill({json: session});
  });
  await openRepo(page, 'review');
  await page.getByRole('button', {name: 'Review changes', exact: true}).click();
  await expect(page.locator('.lrp-chk')).toBeChecked();
  await page.getByRole('button', {name: 'Send to agent (1)', exact: true}).click();
  await page.getByRole('menuitem', {name: 'claude', exact: true}).click();
  await expect.poll(() => body).toEqual({provider: 'claude', comment_ids: ['finding-4']});
  await expect(page.getByText('Handoff failed', {exact: true})).toBeVisible();
  await expect(page.locator('.lrp-chk')).toBeChecked();
  launchFails = false;
  await page.getByRole('button', {name: 'Send to agent (1)', exact: true}).click();
  await page.getByRole('menuitem', {name: 'claude', exact: true}).click();
  await expect(page).toHaveURL(/#\/agents\/synthetic-handoff$/);
  await expect(page.locator('.tab').filter({hasText: 'Selected review fixes'}).first()).toBeVisible();
});

test('creating a branch from a selected commit keeps that SHA while HEAD moves behind the dialog', async ({page}) => {
  await openRepo(page);
  await page.locator(`.graph-row[data-sha="${secondSha}"] .graph-select`).click({button: 'right'});
  await page.getByRole('menuitem', {name: 'Create branch here…', exact: true}).click();
  const dialog = page.getByRole('dialog', {name: 'Create branch', exact: true});
  await dialog.getByRole('textbox').fill('review/pinned-selection');
  execFileSync('git', ['-C', repoDir, 'commit', '--allow-empty', '-qm', 'Head moved while branch dialog stayed open']);
  await dialog.getByRole('button', {name: 'Create', exact: true}).click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => execFileSync('git', ['-C', repoDir, 'rev-parse', 'review/pinned-selection'], {encoding: 'utf8'}).trim()).toBe(secondSha);
});

test('local review disambiguates equal local and remote branch names', async ({page}) => {
  const errors: string[] = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.route('**/api/v1/repos/*/local-reviews', r => r.fulfill({json: []}));
  const bases: string[] = [];
  await page.route('**/api/v1/repos/*/local-review', r => {
    bases.push(r.request().postDataJSON().base);
    return r.fulfill({json: {...run(), comments: []}});
  });
  await page.route('**/api/v1/reviews/review-4/findings*', r => r.fulfill({json: []}));
  await openRepo(page, 'review');
  await page.getByLabel('Compare to').selectOption({label: 'origin/collision (local)'});
  await page.getByRole('button', {name: 'Review changes', exact: true}).click();
  await expect.poll(() => bases).toEqual(['refs/heads/origin/collision']);
  await page.getByLabel('Compare to').selectOption({label: 'origin/collision (remote)'});
  await page.getByRole('button', {name: 'Review again', exact: true}).click();
  await expect.poll(() => bases).toEqual(['refs/heads/origin/collision', 'refs/remotes/origin/collision']);
  expect(errors).toEqual([]);
});

test('active PR agent output and progress update without remounting the terminal', async ({page}, testInfo) => {
  await prFixtures(page);
  const active = {...run('running'), pr_number: 1, comments: [], agents: [
    {name: 'Correctness reviewer', provider: 'claude', model: '', status: 'running', session_id: 'synthetic-review-session', findings: [], note: 'Inspecting the current selection'},
    {name: 'Summarizer', provider: 'claude', model: '', status: 'pending', session_id: null, findings: []},
  ]};
  let sendEvent: ((data: string) => void) | undefined;
  await page.routeWebSocket('**/ws/events*', socket => {sendEvent = data => socket.send(data);});
  let attachments = 0;
  await page.routeWebSocket('**/ws/term/**', socket => {
    attachments++;
    socket.onMessage(raw => {
      if (JSON.parse(String(raw)).type === 'scrollback') socket.send(JSON.stringify({type: 'scrollback', epoch: 1, data: Buffer.from('Synthetic live reviewer output\r\nChecking request identity\r\n').toString('base64')}));
    });
  });
  await page.route('**/api/v1/repos/*/prs/1/reviews', r => r.fulfill({json: [active]}));
  await page.route('**/api/v1/repos/*/prs/1/review', r => r.fulfill({json: active}));
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.getByRole('tab', {name: 'Review', exact: true}).click();
  await page.locator('.rp-agent').first().getByRole('button', {name: 'Open', exact: true}).click();
  await expect(page.locator('.rp-term')).toContainText('Synthetic live reviewer output');
  active.agents[0].note = 'Verified the delayed response; checking cancellation next';
  await expect.poll(() => !!sendEvent).toBe(true);
  sendEvent!(JSON.stringify({type: 'review_changed', workspace_id: workspaceId, review_id: 'review-4', status: 'running'}));
  await expect(page.locator('.rp-agent').first()).toContainText('checking cancellation next');
  await expect(page.locator('.rp-term')).toContainText('Synthetic live reviewer output');
  expect(attachments).toBe(1);
  await page.screenshot({path: testInfo.outputPath('active-review-stream.png')});
});


test('inline reply survives switching from Files to Summary and back', async ({page}) => {
  await prFixtures(page, true);
  await page.goto(`/#/git/${repoId}/pr/1`);
  await page.getByRole('tab', {name: 'Files', exact: true}).click();
  const thread = page.locator('.prd-diff .cmt').first();
  await thread.getByRole('button', {name: 'Reply', exact: true}).click();
  await thread.getByPlaceholder('Reply…').fill('Keep the inline review explanation');
  await page.getByRole('tab', {name: 'Summary', exact: true}).click();
  await expect(thread).toBeHidden();
  await page.getByRole('tab', {name: 'Files', exact: true}).click();
  await expect(thread.getByPlaceholder('Reply…')).toHaveValue('Keep the inline review explanation');
});
