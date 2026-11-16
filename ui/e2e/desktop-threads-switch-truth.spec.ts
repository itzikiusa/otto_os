import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// "Moving between providers, not sure it is passed on" (review F6, F7, A-9),
// on mocked threads (the daemon under test predates the new fields):
//  - a model / effort / access change is a timeline row, and the turn's
//    status line names the model that actually ran;
//  - a child a PERSON asked answers to them: its card offers "Send the answer
//    to Claude", which queues that answer into the parent thread;
//  - at 320 px the composer's model and access chips stay readable.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

// Review screenshots are opt-in: set OTTO_E2E_SHOTS to a folder to keep them.
const SHOTS = process.env.OTTO_E2E_SHOTS ? `${process.env.OTTO_E2E_SHOTS}/threads` : '';
let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base, 'Switch Truth WS');
  await ctx.dispose();
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

const ts = '2026-11-16T10:00:00Z';
const item = (id: string, ordinal: number, turn: string, body: Record<string, unknown>) => ({
  id, turn_id: turn, ordinal, status: 'completed', title: null, started_at: ts, completed_at: ts, updated_at: ts, ...body,
});
const row = (id: string, title: string, extra: Record<string, unknown> = {}) => ({
  id, workspace_id: workspaceId, provider: 'claude', title, cwd: '/tmp', status: 'idle', access_mode: 'approval-required',
  created_by: 'root', created_at: ts, last_active_at: ts, last_sequence: 3, parent_id: null, meta: {}, model: null,
  capabilities: { steer: true, images: true, files: true, efforts: ['low', 'medium', 'high'] }, ...extra,
});

async function mock(page: Page, rows: Record<string, unknown>[], snaps: Record<string, { turns: unknown[]; turn_items: unknown[] }>): Promise<{ id: string; body: Record<string, unknown> }[]> {
  await page.addInitScript((w) => {
    localStorage.setItem('otto_workspace', w as string);
    localStorage.removeItem('otto_threads_v1');
  }, workspaceId);
  await page.route(`**/api/v1/workspaces/${workspaceId}/runtime-sessions`, (r) => r.fulfill({ json: rows }));
  await page.route('**/api/v1/runtime-sessions', (r) => r.fulfill({ json: rows }));
  for (const r of rows) {
    const id = r.id as string;
    await page.route(`**/api/v1/runtime-sessions/${id}`, (route) => route.fulfill({ json: r }));
    await page.route(`**/api/v1/runtime-sessions/${id}/draft`, (route) => route.fulfill({ json: { revision: 0, draft: null } }));
    await page.route(`**/api/v1/runtime-sessions/${id}/events**`, (route) => route.fulfill({ json: {
      kind: 'snapshot', snapshot_sequence: 3, snapshot: {
        provider_session: { status: 'ready', cwd: '/tmp', model: null, updated_at: ts, last_error: null },
        provider_thread: null, ...snaps[id], runtime_requests: [], plans: [],
      },
    } }));
  }
  const sent: { id: string; body: Record<string, unknown> }[] = [];
  await page.route('**/api/v1/runtime-sessions/*/prompt', (route) => {
    sent.push({ id: route.request().url().split('/').at(-2)!, body: route.request().postDataJSON() });
    // The mock never emits later events, so the answer must not name a newer
    // sequence (the view would keep catching up to it).
    return route.fulfill({ json: { sequence: 3, queued_id: 'q9' } });
  });
  return sent;
}

test('a model switch shows in the timeline and the turn names what ran; a person-asked answer can be handed over', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const parent = row('switch-parent', 'Switch parent', { model: 'claude-sonnet-4-5', effort: 'high' });
  const child = row('switch-child', 'Ask Codex', { provider: 'codex', parent_id: 'switch-parent', meta: { child_of: { id: 'switch-parent', by: 'person' } } });
  const sent = await mock(page, [parent, child], {
    'switch-parent': {
      turns: [{ id: 't1', ordinal: 1, status: 'completed', started_at: ts, completed_at: ts, model: 'claude-sonnet-4-5', effort: 'high' }],
      turn_items: [
        item('u1', 1, 't1', { type: 'user_message', text: 'review the plan' }),
        item('selection:1', 2, 't1', { type: 'system_notice', title: 'Model', message: 'Model: opus → claude-sonnet-4-5' }),
        item('n1', 3, 't1', { type: 'system_notice', message: 'Asked Codex [otto-thread:switch-child:started]' }),
        item('a1', 4, 't1', { type: 'assistant_message', message_id: 'a1', text: 'Done reviewing.', streaming: false }),
      ],
    },
    'switch-child': {
      turns: [{ id: 'k1', ordinal: 1, status: 'completed', started_at: ts, completed_at: ts }],
      turn_items: [
        item('ku', 1, 'k1', { type: 'user_message', text: 'what do you think?' }),
        item('ka', 2, 'k1', { type: 'assistant_message', message_id: 'ka', text: 'Use a queue.', streaming: false }),
      ],
    },
  });
  await page.goto('/#/threads/switch-parent');
  const thread = page.getByTestId('runtime-thread-switch-parent');
  await expect(thread).toBeVisible({ timeout: 30_000 });

  // F7: the change is a quiet row, the status line names the model that ran,
  // the chip shows the stored effort.
  await expect(thread.getByTestId('runtime-setting-change')).toHaveText('Model: opus → claude-sonnet-4-5');
  await expect(thread.getByText('Model: opus → claude-sonnet-4-5')).toHaveCount(1); // not also a work row
  await expect(thread.getByTestId('runtime-ran-on')).toHaveText('Ran on claude-sonnet-4-5 · High');
  await expect(thread.getByTestId('runtime-model')).toContainText('High');

  // F6: the answer came back to the person; one click hands it to Claude.
  const card = thread.getByTestId('runtime-child-switch-child');
  await expect(card).toHaveAttribute('data-state', 'done');
  const hand = thread.getByTestId('runtime-child-handover-switch-child');
  await expect(hand).toHaveText('Send the answer to Claude');
  await hand.click();
  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0].id).toBe('switch-parent');
  expect(String(sent[0].body.text)).toContain('Use a queue.');
  expect(sent[0].body.delivery).toBe('queue');
  await expect(hand).toHaveText('Sent to Claude');
  await expect(card).toHaveAttribute('data-state', 'done');
  if (SHOTS) mkdirSync(SHOTS, { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/desktop-switch-truth-${scheme}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });

  // A-9: at 320 px the model and access chips keep their words (not just "…").
  await page.setViewportSize({ width: 320, height: 720 });
  for (const testid of ['runtime-model', 'runtime-access']) {
    const chip = thread.getByTestId(testid);
    await expect(chip).toBeVisible();
    await expectFullyInViewport(page, chip);
    const clipped = await chip.evaluate((el) => [...el.querySelectorAll<HTMLElement>('.ellipsis, span')].some((s) => s.scrollWidth > s.clientWidth + 1));
    expect(clipped, `${testid} label is truncated at 320 px`).toBe(false);
  }
  await expectNoHorizontalOverflow(page);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/narrow-320-chips-${scheme}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
});
