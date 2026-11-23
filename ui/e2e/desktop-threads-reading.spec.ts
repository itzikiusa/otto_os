import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// The Threads reading experience on a mocked, finished thread (the shape every
// provider's driver emits: user / assistant messages, commands, file changes
// with a unified diff or old/new text): the 760 px reading column in reading
// type, the finished turn's one-line work fold, the end-of-turn changes card
// and the composer card. OTTO_E2E_SHOTS=<dir> keeps light/dark screenshots.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });
const SHOTS = process.env.OTTO_E2E_SHOTS ?? '';
let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base, 'Reading WS');
  await ctx.dispose();
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

const T0 = Date.parse('2026-11-23T10:00:00Z');
const at = (s: number) => new Date(T0 + s * 1000).toISOString();
const item = (id: string, ordinal: number, s: number, e: number, body: Record<string, unknown>) => ({
  id, turn_id: 't1', ordinal, status: 'completed', title: null, started_at: at(s), completed_at: at(e), updated_at: at(e), ...body,
});
const ANSWER = [
  'Offline edits now survive a reload and sync when the connection returns.',
  '',
  '- **Queue first.** Every edit is written to IndexedDB before it is sent, in `queue.ts`.',
  '- **Replay in order.** On reconnect the queue drains oldest first and stops at the first failure.',
  '- **Conflicts.** Last write wins per note. The server timestamp breaks ties.',
  '',
  'I tested it by hand too: typed with the network off, reloaded, went back online, and the note saved. Want me to open a pull request?',
].join('\n');
const DIFF = '--- a/src/sync/queue.ts\n+++ b/src/sync/queue.ts\n@@ -1,2 +1,5 @@\n import { db } from "./db";\n-export const queue = [];\n+export const queue = new Queue(db);\n+queue.replay();\n+queue.onConflict(lastWriteWins);\n+export default queue;\n';
export const ITEMS = [
  item('u1', 1, 0, 0, { type: 'user_message', text: 'Notes typed offline are lost on reload. Queue edits locally and sync them when we reconnect.' }),
  item('c1', 2, 1, 20, { type: 'command_execution', input: 'rg -n "saveNote" src', output: 'src/notes.ts:12', exit_code: 0 }),
  item('c2', 3, 20, 40, { type: 'command_execution', input: 'cat src/notes.ts', output: '…', exit_code: 0 }),
  item('f1', 4, 40, 60, { type: 'file_change', file_name: '/tmp/harbor/src/sync/queue.ts', diff_str: DIFF }),
  item('f2', 5, 60, 80, { type: 'file_change', file_name: '/tmp/harbor/src/notes.ts', old_str: 'save(n)\n', new_str: 'queue.push(n)\nreturn n\n' }),
  item('f3', 6, 80, 90, { type: 'file_change', file_name: '/tmp/harbor/README.md' }),
  item('c3', 7, 90, 120, { type: 'command_execution', input: 'npm test -- sync', output: 'ok 12 passed', exit_code: 0 }),
  item('c4', 8, 120, 134, { type: 'command_execution', input: 'npm run lint', output: 'ok', exit_code: 0 }),
  item('a1', 9, 134, 134, { type: 'assistant_message', message_id: 'a1', text: ANSWER, streaming: false }),
];

export async function mockThread(page: Page, id: string, items: unknown[], turns: unknown[]): Promise<void> {
  const row = {
    id, workspace_id: workspaceId, provider: 'claude', title: 'Sync offline edits on reconnect', cwd: '/tmp/harbor', status: 'idle',
    access_mode: 'approval-required', created_by: 'root', created_at: at(0), last_active_at: at(134), last_sequence: 3, parent_id: null, meta: {}, model: null,
    capabilities: { steer: true, images: true, files: true, efforts: ['low', 'medium', 'high'] },
  };
  await page.addInitScript((w) => {
    localStorage.setItem('otto_workspace', w as string);
    localStorage.removeItem('otto_threads_v1');
  }, workspaceId);
  await page.route(`**/api/v1/workspaces/${workspaceId}/runtime-sessions`, (r) => r.fulfill({ json: [row] }));
  await page.route('**/api/v1/runtime-sessions', (r) => r.fulfill({ json: [row] }));
  await page.route(`**/api/v1/runtime-sessions/${id}`, (r) => r.fulfill({ json: row }));
  await page.route(`**/api/v1/runtime-sessions/${id}/draft`, (r) => r.fulfill({ json: { revision: 0, draft: null } }));
  await page.route(`**/api/v1/runtime-sessions/${id}/events**`, (r) => r.fulfill({ json: {
    kind: 'snapshot', snapshot_sequence: 3, snapshot: {
      provider_session: { status: 'ready', cwd: '/tmp/harbor', model: null, updated_at: at(134), last_error: null },
      provider_thread: null, turns, turn_items: items, runtime_requests: [], plans: [],
    },
  } }));
}

async function shoot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: `${SHOTS}/${name}-${scheme}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

const TURNS = [{ id: 't1', ordinal: 1, status: 'completed', started_at: at(0), completed_at: at(134) }];

test('a finished thread reads as one calm column', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await mockThread(page, 'read-1', ITEMS, TURNS);
  await page.goto('/#/threads/read-1');
  const thread = page.getByTestId('runtime-thread-read-1');
  await expect(thread.getByText('Offline edits now survive')).toBeVisible({ timeout: 30_000 });
  // R1: one 760 px column in reading type.
  const prose = thread.locator('.rt-assistant .md-body').first();
  expect(await prose.evaluate((el) => getComputedStyle(el).fontSize)).toBe('15px');
  const col = (await thread.locator('.rt-col').boundingBox())!;
  const dock = (await thread.locator('.rt-composer').boundingBox())!;
  expect(col.width).toBeLessThanOrEqual(760 + 2 * 24 + 1);
  expect(Math.abs(dock.x + dock.width / 2 - (col.x + col.width / 2))).toBeLessThan(2);
  // R2: the finished turn is one line that says what was done.
  await expect(thread.locator('[data-testid^="runtime-fold-"] > summary')).toHaveText(/^\s*Worked for 2m 13s · ran 4 commands and edited 3 files\s*$/);
  // R3: the end-of-turn changes card, names only where an edit carries no text.
  const card = thread.getByTestId('runtime-turn-changes');
  await expect(card).toContainText('3 changed files');
  await expect(card).toContainText('+6');
  await expect(card).toContainText('−2');
  await card.getByRole('button', { name: /3 changed files/ }).click();
  await expect(card.locator('li')).toHaveText([/src\/sync\/queue\.ts\s*\+4\s*−1/, /src\/notes\.ts\s*\+2\s*−1/, /^\s*README\.md\s*$/]);
  // R5: times show on hover only (still in the accessibility tree).
  const time = thread.getByTestId('runtime-assistant-time');
  await page.mouse.move(5, 500);
  await expect(time).toHaveCSS('opacity', '0');
  await thread.locator('.rt-assistant').hover();
  await expect(time).toHaveCSS('opacity', '1');
  // R4: the composer row keeps only "+", the non-default access, the model and the primary.
  await expect(thread.getByTestId('runtime-plan')).toHaveCount(0);
  await thread.getByTestId('runtime-plus').click();
  const menu = page.locator('.ctx-menu');
  await expect(menu.getByRole('menuitem', { name: /Attach an image or PDF/ })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /Mention a file or agent/ })).toBeVisible();
  await expect(menu.getByRole('menuitemcheckbox', { name: /Access: Supervised/ })).toBeVisible();
  await page.keyboard.press('Escape');
  await shoot(page, 'desktop');
  await card.getByTestId('runtime-turn-changes-open').click();
  await expect(page.getByTestId('threads-inspector').or(page.locator('[data-testid^="thread-changes"]')).first()).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(thread.getByText('Offline edits now survive')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await shoot(page, 'phone');
});

test('while a turn runs, its finished steps fold to "N earlier steps" and the live one shows', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const live = ITEMS.slice(0, 8).map((i) => (i.id === 'c4' ? { ...i, status: 'running', completed_at: null } : i));
  await mockThread(page, 'read-2', live, [{ id: 't1', ordinal: 1, status: 'running', started_at: at(0), completed_at: null }]);
  await page.goto('/#/threads/read-2');
  const thread = page.getByTestId('runtime-thread-read-2');
  const earlier = thread.getByTestId('runtime-earlier-steps');
  await expect(earlier).toContainText('6 earlier steps', { timeout: 30_000 });
  await expect(thread.locator('[data-item-type="command_execution"]').filter({ hasText: 'npm run lint' })).toBeVisible();
  const older = thread.locator('[data-item-type="command_execution"]').filter({ hasText: 'npm test -- sync' });
  await expect(older).toBeHidden();
  await earlier.locator('summary').click();
  await expect(older).toBeVisible();
  await shoot(page, 'running');
});
