// Shared by desktop-threads-task-stop.spec.ts and threads-task-stop-mobile.spec.ts:
// a mocked Claude thread whose turn ended with two background commands still
// running, and the per-task stop routes mocked with `page.route` (the daemon
// under test predates them): Stop one, Retry after a 5xx, "Asked the
// agent", Stop all through the confirm, and an older daemon's 404 that hides
// every Stop instead of leaving a dead button.
import { expect, type Page, type Route, type TestInfo } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { expectNoHorizontalOverflow } from './helpers';

// Review screenshots are opt-in: set OTTO_E2E_SHOTS to a folder to keep them.
const SHOTS = process.env.OTTO_E2E_SHOTS ? `${process.env.OTTO_E2E_SHOTS}/threads` : '';

/** A fully mocked Claude thread, idle, with two running background commands
 *  (the daemon under test predates the stop routes and its fake agent). */
export async function mockedThread(page: Page, workspaceId: string, id: string, title: string): Promise<void> {
  const ts = new Date(Date.now() - 60_000).toISOString();
  const background_tasks = [
    { id: 'b1', kind: 'command', label: 'npm run dev', started_at: ts },
    { id: 'b2', kind: 'command', label: 'tail -f app.log', started_at: new Date(Date.now() - 30_000).toISOString() },
  ];
  const row = {
    id, workspace_id: workspaceId, provider: 'claude', title, cwd: '/tmp', status: 'idle', access_mode: 'approval-required',
    created_by: 'root', created_at: ts, last_active_at: ts, last_sequence: 3, parent_id: null, meta: {}, model: null,
    background_tasks, capabilities: { steer: true, images: true, files: true },
  };
  await page.addInitScript((w) => {
    localStorage.setItem('otto_workspace', w as string);
    localStorage.removeItem('otto_threads_v1');
  }, workspaceId);
  await page.route(`**/api/v1/workspaces/${workspaceId}/runtime-sessions`, (r) => r.fulfill({ json: [row] }));
  await page.route('**/api/v1/runtime-sessions', (r) => r.fulfill({ json: [row] }));
  await page.route(`**/api/v1/runtime-sessions/${id}`, (r) => r.fulfill({ json: row }));
  await page.route(`**/api/v1/runtime-sessions/${id}/events**`, (r) => r.fulfill({ json: {
    kind: 'snapshot', snapshot_sequence: 3, snapshot: {
      provider_session: { status: 'ready', cwd: '/tmp', model: null, updated_at: ts, last_error: null, background_tasks },
      provider_thread: null,
      turns: [{ id: 't1', ordinal: 1, status: 'completed', started_at: ts, completed_at: ts }],
      turn_items: [
        { id: 'u1', turn_id: 't1', ordinal: 1, status: 'completed', title: null, started_at: ts, completed_at: ts, updated_at: ts, type: 'user_message', text: 'start the dev server and tail the log' },
        { id: 'a1', turn_id: 't1', ordinal: 2, status: 'completed', title: null, started_at: ts, completed_at: ts, updated_at: ts, type: 'assistant_message', message_id: 'a1', text: 'Both are running in the background.', streaming: false },
      ],
      runtime_requests: [], plans: [],
    },
  } }));
  await page.route(`**/api/v1/runtime-sessions/${id}/draft`, (r) => r.fulfill({ json: { revision: 0, draft: null } }));
}

type Mode = { status: number; body: unknown };

/** Mock both stop routes; returns the posted paths and a setter for the next answers. */
export async function mockStops(page: Page): Promise<{ posts: string[]; one: (m: Mode) => void; all: (m: Mode) => void }> {
  const posts: string[] = [];
  let one: Mode = { status: 200, body: {} };
  let all: Mode = { status: 200, body: { results: [] } };
  const reply = (route: Route, m: Mode) => route.fulfill({ status: m.status, contentType: 'application/json', body: JSON.stringify(m.body) });
  await page.route(/\/runtime-sessions\/[^/]+\/tasks\/stop$/, (route) => {
    posts.push(new URL(route.request().url()).pathname);
    return reply(route, all);
  });
  await page.route(/\/runtime-sessions\/[^/]+\/tasks\/[^/]+\/stop$/, (route) => {
    posts.push(new URL(route.request().url()).pathname);
    return reply(route, one);
  });
  return { posts, one: (m) => (one = m), all: (m) => (all = m) };
}

/** Open the thread, start two background commands; returns the thread locator. */
export async function threadWithTwoBackground(page: Page, id: string) {
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await expect(thread.getByText('Both are running in the background.')).toBeVisible({ timeout: 15_000 });
  await expect(thread.getByTestId('runtime-background')).toHaveAttribute('data-count', '2', { timeout: 15_000 });
  // The turn ended: idle with background work is NOT a running turn.
  await expect(thread.getByTestId('runtime-working')).toHaveCount(0);
  await expect(thread.getByTestId('runtime-stop')).toHaveCount(0);
  await expect(thread.getByTestId('runtime-background-stop-all')).toBeVisible();
  return thread;
}

export async function shoot(page: Page, info: TestInfo, name: string): Promise<void> {
  if (SHOTS) mkdirSync(SHOTS, { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/${name}-${scheme}.png`, animations: 'disabled' });
    await page.screenshot({ path: info.outputPath(`${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

/** The whole per-task stop flow on an open thread with two running background commands. */
export async function stopFlow(page: Page, info: TestInfo, id: string, label: string): Promise<void> {
  const mock = await mockStops(page);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await thread.getByTestId('runtime-background-show').click();
  const panel = page.getByTestId('thread-agents-tasks');
  await expect(panel).toBeVisible();
  const running = panel.locator('[data-testid="agents-tasks-row"][data-status="running"]');
  await expect(running).toHaveCount(2);
  await expect(running.getByTestId('agents-tasks-stop')).toHaveCount(2);
  const [first, second] = [running.nth(0), running.nth(1)];

  // A 5xx: the reason inline, and Retry.
  mock.one({ status: 500, body: { code: 'internal', message: 'driver crashed' } });
  await first.getByRole('button', { name: 'Stop npm run dev' }).click();
  await expect(first.getByTestId('agents-tasks-stop-note')).toHaveText('Couldn’t stop: driver crashed');
  const retry = first.getByRole('button', { name: /^Retry stopping/ });
  await expect(retry).toBeVisible();
  expect(mock.posts.filter((p) => /\/tasks\/[^/]+\/stop$/.test(p)).length).toBe(1);
  await shoot(page, info, `${label}-stop-failed`);

  // Retry → a native stop: "Stopping…" until the provider settles it.
  mock.one({ status: 200, body: { task_id: 'b', outcome: 'native' } });
  await retry.click();
  await expect(first.getByTestId('agents-tasks-stopping')).toBeVisible();
  await expect(first.getByTestId('agents-tasks-stop')).toHaveCount(0);

  // The second one: Otto could only ask the agent — the row stays Running and says so.
  mock.one({ status: 200, body: { task_id: 'b', outcome: 'asked' } });
  await second.getByTestId('agents-tasks-stop').click();
  await expect(second.getByTestId('agents-tasks-stop-note')).toHaveText('Asked the agent to stop it');
  await expect(second).toHaveAttribute('data-status', 'running');
  expect(mock.posts.filter((p) => /\/tasks\/[^/]+\/stop$/.test(p)).length).toBe(3);
  await expectNoHorizontalOverflow(page);
  await shoot(page, info, `${label}-stop-states`);

  // Stop all: confirmed, naming how many, one POST to …/tasks/stop.
  mock.all({ status: 200, body: { results: [{ task_id: 'x', outcome: 'native' }, { task_id: 'y', outcome: 'native' }] } });
  // On a phone Agents & tasks is a drawer over the thread: close it first.
  const drawer = page.getByRole('dialog', { name: 'Thread details' });
  if (await drawer.isVisible()) {
    await page.keyboard.press('Escape');
    await expect(drawer).toHaveCount(0);
  }
  await thread.getByTestId('runtime-background-stop-all').click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Stop 2 background tasks?');
  await shoot(page, info, `${label}-stop-all-confirm`);
  await dialog.getByRole('button', { name: 'Stop 2' }).click();
  await expect.poll(() => mock.posts.filter((p) => p.endsWith('/tasks/stop')).length).toBe(1);
}

/** An older daemon answers the stop route 404 "no such route": every Stop disappears (no dead button). */
export async function unsupportedFlow(page: Page, id: string): Promise<void> {
  await page.route(/\/runtime-sessions\/[^/]+\/tasks\/[^/]+\/stop$/, (route) =>
    route.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ code: 'not_found', message: `no such route: ${new URL(route.request().url()).pathname}` }) }),
  );
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await thread.getByTestId('runtime-background-show').click();
  const panel = page.getByTestId('thread-agents-tasks');
  const stops = panel.getByTestId('agents-tasks-stop');
  await expect(stops).toHaveCount(2);
  await stops.first().click();
  await expect(stops).toHaveCount(0);
  await expect(thread.getByTestId('runtime-background-stop-all')).toHaveCount(0);
  await expect(page.getByText('This Otto build can’t stop background tasks')).toBeVisible();
}
