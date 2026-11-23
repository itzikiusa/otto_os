// A finished structured thread as every provider's driver shapes it (user /
// assistant messages, commands, file changes with a unified diff or old/new
// text, one without text), mocked over the e2e daemon's routes. Shared by
// desktop-threads-reading.spec.ts and threads-reading-mobile.spec.ts.
import type { Page } from '@playwright/test';

// Relative to now, so a running turn's elapsed reads as its own (~2 min).
const T0 = Date.now() - 140_000;
export const at = (s: number) => new Date(T0 + s * 1000).toISOString();
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

export async function mockThread(page: Page, workspaceId: string, id: string, items: unknown[], turns: unknown[]): Promise<void> {
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


export const TURNS = [{ id: 't1', ordinal: 1, status: 'completed', started_at: at(0), completed_at: at(134) }];
