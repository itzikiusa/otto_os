// A thread with one changed file (src/a.ts, a two-line rewrite) mocked over
// the e2e daemon, for the review-notes specs (desktop and phone).
import { type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';

const SHOTS = process.env.OTTO_E2E_SHOTS ?? '';

export async function setup(page: Page, opts: { inspector: boolean }): Promise<{ sent: string[]; diff: { lines: { origin: string; content: string; old_line: number | null; new_line: number | null }[] } }> {
  const { ctx, base } = await apiCtx();
  const workspaceId = await seedWorkspace(ctx, base);
  await ctx.dispose();
  const ts = '2026-11-07T08:00:00Z';
  const sessions = [{ id: 'notes-alpha', workspace_id: workspaceId, provider: 'claude', title: 'Notes alpha', cwd: '/tmp/review-notes', status: 'idle', access_mode: 'approval-required', created_by: 'root', created_at: ts, last_active_at: ts, last_sequence: 1, parent_id: null, meta: {}, model: null }];
  const repo = { id: 'notes-repo', workspace_id: workspaceId, name: 'Notes repo', path: '/tmp/review-notes', remote_url: null, provider: null, forge: null, git_account_id: null, created_at: ts };
  await page.addInitScript(({ workspaceId, inspector }) => {
    localStorage.setItem('otto_workspace', workspaceId);
    if (!sessionStorage.getItem('notes-init')) {
      sessionStorage.setItem('notes-init', '1');
      localStorage.removeItem('otto_threads_review_v1');
      localStorage.setItem('otto_threads_v1', JSON.stringify({ focusedSessionId: 'notes-alpha', paneIds: ['notes-alpha'], rightOpen: inspector, inspectorTab: 'changes' }));
      localStorage.setItem('otto_threads_chrome_v1', JSON.stringify({ surfaces: ['changes'] }));
    }
  }, { workspaceId, inspector: opts.inspector });
  await page.route(`**/api/v1/workspaces/${workspaceId}/runtime-sessions`, (r) => r.fulfill({ json: sessions }));
  await page.route('**/api/v1/runtime-sessions', (r) => r.fulfill({ json: sessions }));
  await page.route('**/api/v1/runtime-sessions/notes-alpha', (r) => r.fulfill({ json: sessions[0] }));
  await page.route('**/api/v1/runtime-sessions/notes-*/events**', (r) => r.fulfill({ json: { kind: 'snapshot', snapshot_sequence: 1, snapshot: { provider_session: { status: 'ready', cwd: repo.path, model: null, updated_at: ts, last_error: null }, provider_thread: null, turns: [], turn_items: [], runtime_requests: [], plans: [] } } }));
  await page.route('**/api/v1/workspaces/*/repos', (r) => r.fulfill({ json: [repo] }));
  await page.route('**/api/v1/workspaces/*/repos/detect', (r) => r.fulfill({ json: repo }));
  await page.route('**/api/v1/repos/notes-repo/status', (r) => r.fulfill({ json: { branch: 'feature', upstream: null, ahead: 0, behind: 0, changes: [{ path: 'src/a.ts', orig_path: null, kind: 'modified' }] } }));
  const diff = { lines: [
    { origin: 'context', content: 'const a = 1;', old_line: 8, new_line: 8 },
    { origin: 'del', content: 'save(note);', old_line: 9, new_line: null },
    { origin: 'add', content: 'queue.push(note);', old_line: null, new_line: 9 },
    { origin: 'add', content: 'return note;', old_line: null, new_line: 10 },
  ] };
  await page.route('**/api/v1/repos/notes-repo/diff?*', (r) => r.fulfill({ json: { files: [{ path: 'src/a.ts', old_path: null, is_binary: false, added: 2, deleted: 1, hunks: [{ header: '@@ -8,2 +8,3 @@', lines: diff.lines }] }] } }));
  await page.route('**/api/v1/runtime-sessions/notes-*/draft', (r) => r.fulfill({ json: { revision: 0, draft: null } }));
  const sent: string[] = [];
  await page.route('**/api/v1/runtime-sessions/notes-*/prompt', (r) => {
    sent.push(r.request().postDataJSON().text);
    return r.fulfill({ json: { sequence: 1 } });
  });
  return { sent, diff };
}

export async function shoot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  for (const s of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: s });
    await page.screenshot({ path: `${SHOTS}/${name}-${s}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

