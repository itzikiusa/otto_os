import { test, expect, type Page } from '@playwright/test';
import { resolve } from 'node:path';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage, expectFullyInViewport } from './helpers';

// Conversation view — Track C surfaces (docs/design/conversation-view.md §5.3–5.5):
//   • History (`#/history`) lists a seeded session and opens it READ-ONLY.
//   • Activity panel: "+ Add task" → `POST /sessions/{id}/tasks`; the row shows
//     the `from board` badge and survives a reload (persisted server-side).
//   • Mission Control: a session-backed card carries the done/total task strip
//     and `+ Sub-task` POSTs a task.
//   • Outputs: the fixture's `Write` blocks surface as artifacts with a preview.
//
// Runs against the isolated e2e daemon (global-setup, `OTTO_E2E=1`). The History
// row is a `claude` session pointed at a redacted Claude fixture through the e2e
// hook `meta.e2e_transcript_path` (design doc §8, A → B) — the daemon folds
// that file as the session's transcript. CLAUDE_BIN is a nonexistent binary
// there, so the PTY exits at once; the row (kind agent, provider claude) is what
// History lists and what the board-task gate (`nudgeable`) accepts.
// `10-write-structured-patch` carries Write blocks (incl. a `.md`) → artifacts.
const FIXTURE = resolve(process.cwd(), '..', 'crates/otto-transcript/fixtures/claude/10-write-structured-patch.jsonl');

let wsA = '';
let boardId = '';
let claudeId = '';
let claudeTitle = '';
let base = '';
let token = '';

test.beforeAll(async () => {
  const a = await apiCtx();
  base = a.base;
  token = a.token;
  wsA = await seedWorkspace(a.ctx, a.base);
  await seedShellSession(a.ctx, a.base, wsA); // a plain live shell (never a History row)
  // Mission Control only lists LIVE sessions (running/working/idle) and the
  // only session that stays alive on the e2e daemon is a shell — so the board
  // card is a shell stamped the way the capture scan stamps one that ran
  // `claude` (`meta.nested_provider`), which the board-task gate accepts.
  const b = await a.ctx.post(`${a.base}/api/v1/workspaces/${wsA}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'shell',
      title: 'E2E Board Shell',
      cwd: '/tmp',
      meta: { origin: 'e2e', nested_provider: 'claude' },
    },
  });
  if (b.ok()) boardId = ((await b.json()) as { id: string }).id;
  claudeTitle = `E2E History ${Date.now().toString(36)}`;
  const r = await a.ctx.post(`${a.base}/api/v1/workspaces/${wsA}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'claude',
      title: claudeTitle,
      cwd: '/tmp',
      meta: { origin: 'e2e', e2e_transcript_path: FIXTURE },
    },
  });
  if (r.ok()) claudeId = ((await r.json()) as { id: string }).id;
  await a.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_right_open', '1');
    localStorage.setItem('otto_right_tab', 'activity');
    localStorage.setItem('otto_view_mode', 'tabs');
  }, wsA);
});

/** Open the fixture-backed claude session (the one board tasks are accepted for). */
async function openClaude(page: Page, tab: 'Activity' | 'Outputs'): Promise<void> {
  await openPage(page, 'agents');
  await page.getByRole('button', { name: new RegExp(claudeTitle) }).first().click();
  await expect(page.locator('.rpanel')).toBeVisible();
  await page.locator('.rpanel').getByRole('tab', { name: tab, exact: true }).click();
}

async function tasksOf(page: Page, sid: string): Promise<{ title: string; source: string }[]> {
  const resp = await page.request.get(`${base}/api/v1/workspaces/${wsA}/sessions/${sid}/tasks`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  expect(resp.ok()).toBe(true);
  return (await resp.json()) as { title: string; source: string }[];
}

test.describe('history page', () => {
  test('lists the seeded claude session and opens it read-only', async ({ page }) => {
    expect(claudeId, 'the fixture-backed session row was created').not.toBe('');
    await openPage(page, 'history');
    const list = page.getByTestId('history-list');
    await expect(list).toBeVisible();

    // The seeded session is a row (title, or its first prompt) under its cwd group.
    const row = page.locator(`[data-testid="history-row"][data-session-id="${claudeId}"]`);
    await expect(row).toBeVisible({ timeout: 15_000 });
    await expect(row.locator('.glyph.claude')).toBeVisible();

    // Search narrows the list to it.
    await page.getByTestId('history-search').fill(claudeTitle);
    await expect(row).toBeVisible();

    // Opening it mounts the read-only conversation + updates the deep link.
    await row.click();
    await expect(page).toHaveURL(new RegExp(`#/history/${claudeId}$`));
    const conv = page.getByTestId('history-conversation');
    await expect(conv).toBeVisible();
    // The conversation is mounted read-only and actually folded the fixture:
    // at least one user turn is rendered, and there is no composer.
    await expect(conv.locator('.conv[data-readonly="true"]')).toBeVisible();
    await expect(conv.locator('.turn[data-role="user"]').first()).toBeVisible({ timeout: 15_000 });
    await expect(conv.locator('.turn[data-role="assistant"]').first()).toBeVisible();
    await expect(conv.locator('textarea')).toHaveCount(0);

    // Header actions are present; the transcript path is copyable.
    await expect(page.getByRole('button', { name: /Copy path/ })).toBeVisible();
    await expect(page.getByRole('button', { name: /Open folder/ })).toBeVisible();

    // Row context menu (ctxMenu store) — clamped into the viewport per AGENTS.md.
    await row.click({ button: 'right' });
    const menu = page.locator('.ctx-menu');
    await expectFullyInViewport(page, menu, 'history row context menu');
    await expect(menu.getByRole('menuitem', { name: /Resume in Otto|Open in Otto/ })).toBeVisible();
    await expect(menu.getByRole('menuitem', { name: 'Archive' })).toBeVisible();
    await page.locator('.ctx-backdrop').click({ position: { x: 2, y: 2 } });
    await expect(menu).toBeHidden();
  });

  test('is reachable from the Agents header and the sidebar', async ({ page }) => {
    await openPage(page, 'agents');
    await page.getByTestId('agents-history-btn').click();
    await expect(page).toHaveURL(/#\/history$/);
    await expect(page.getByTestId('history-page')).toBeVisible();
  });
});

test.describe('tasks from the board', () => {
  test('add task via the Activity panel → row with the board badge, persists after reload', async ({ page }) => {
    expect(claudeId, 'the fixture-backed session row was created').not.toBe('');
    await openClaude(page, 'Activity');
    const panel = page.locator('.rpanel');
    await panel.getByTestId('add-task-btn').click();
    const title = `Board task ${Date.now().toString(36)}`;
    await panel.getByLabel('Task title').fill(title);
    await panel.getByLabel('Task description').fill('added from the e2e suite');
    await panel.getByTestId('add-task-form').getByRole('button', { name: 'Add', exact: true }).click();

    const row = panel.locator('[data-testid="task-row"]', { hasText: title });
    await expect(row).toBeVisible();
    await expect(row).toHaveAttribute('data-source', 'user');
    await expect(row.locator('.badge.board')).toHaveText(/from board/i);
    await expect(panel.getByTestId('task-count')).toHaveText(/0\/1/);

    // Server-side truth: the row came back from POST /sessions/{id}/tasks.
    const tasks = await tasksOf(page, claudeId);
    expect(tasks.some((t) => t.title === title && t.source === 'user')).toBe(true);

    // Persists across a reload (REST load, not just the optimistic insert).
    await page.reload();
    await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
    await page.getByRole('button', { name: new RegExp(claudeTitle) }).first().click();
    await panel.getByRole('tab', { name: 'Activity', exact: true }).click();
    await expect(panel.locator('[data-testid="task-row"]', { hasText: title })).toBeVisible();
    await expect(panel.locator('[data-testid="task-row"]', { hasText: title }).locator('.badge.board')).toBeVisible();
  });

  test('Mission Control card shows the done/total strip and + Sub-task POSTs a task', async ({ page }) => {
    expect(boardId, 'the board shell session row was created').not.toBe('');
    // The work-queue board is the Agents view in `mission` mode (`#/mission-control`
    // is the separate work-graph module).
    await page.addInitScript(() => localStorage.setItem('otto_view_mode', 'mission'));
    await openPage(page, 'agents');
    await expect(page.locator('.mission')).toBeVisible();
    // Every session-backed card carries the strip.
    await expect(page.getByTestId('task-strip').first()).toBeVisible();
    const card = page.locator(`li.item[data-session-id="${boardId}"]`);
    await expect(card, 'the live board shell is on the board').toBeVisible();
    await expect(card.getByTestId('task-strip-count')).toHaveText(/^\d+\/\d+$/);

    // + Sub-task → inline input → POST /sessions/{id}/tasks.
    const before = (await tasksOf(page, boardId)).length;
    await card.getByTestId('subtask-btn').click();
    const title = `Sub-task ${Date.now().toString(36)}`;
    await card.getByLabel('Sub-task title').fill(title);
    await card.getByLabel('Sub-task title').press('Enter');
    await expect(card.getByLabel('Sub-task title')).toHaveCount(0);
    await expect(card.getByTestId('task-strip-count')).toHaveText(new RegExp(`/${before + 1}$`));
    const tasks = await tasksOf(page, boardId);
    expect(tasks.some((t) => t.title === title && t.source === 'user')).toBe(true);
  });
});

test.describe('outputs panel', () => {
  test('lists the fixture artifacts and previews the markdown one', async ({ page }) => {
    expect(claudeId, 'the fixture-backed session row was created').not.toBe('');
    await openClaude(page, 'Outputs');
    const panel = page.getByTestId('outputs-panel');
    await expect(panel).toBeVisible();
    // The fixture's Write blocks (a .json, a .md, two .mjs) fold into artifacts.
    const rows = panel.getByRole('option');
    await expect(rows.first()).toBeVisible({ timeout: 15_000 });
    expect(await rows.count()).toBeGreaterThanOrEqual(2);
    const md = rows.filter({ has: page.locator('.alabel', { hasText: /\.md$/ }) }).first();
    await expect(md).toBeVisible();
    await md.click();
    const preview = page.getByTestId('outputs-preview');
    await expect(preview).toBeVisible();
    await expect(preview.locator('.ptitle')).toHaveText(/\.md$/);
    // The bytes route serves a redacted fixture path only when the daemon can
    // resolve it; either the sanitized markdown renders or the panel says why.
    await expect(preview.locator('.pbody.md, .pbody.err').first()).toBeVisible({ timeout: 15_000 });
  });
});
