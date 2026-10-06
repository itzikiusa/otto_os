import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// `/ws/term/{id}?view=1` (docs/contracts/ws.md §1, r3-05-01 follow-up): a
// VIEW-ONLY attach to a suspended agent session must not spawn its CLI. The
// first real keystroke on that socket resumes it (and is not delivered);
// emulator replies (`user: false`) never do. A classic attach still resumes on
// open. Drives the daemon directly — the resume decision is server-side — with
// the harness's fake `claude` (global-setup.ts) as the "CLI".
//
// Desktop-browser project only (daemon behaviour; one engine is enough).

let ctx: APIRequestContext;
let base: string;
let token: string;

const b64 = (s: string) => Buffer.from(s, 'utf8').toString('base64');

interface Sock {
  frames: Array<Record<string, unknown>>;
  send: (frame: unknown) => void;
  close: () => void;
}

async function attach(id: string, view: boolean): Promise<Sock> {
  const url = `${base.replace('http', 'ws')}/ws/term/${id}${view ? '?view=1' : ''}`;
  // The bearer rides in the otto-bearer subprotocol — `?token=` is refused.
  const ws = new WebSocket(url, ['otto-bearer', token]);
  const frames: Array<Record<string, unknown>> = [];
  ws.onmessage = (ev) => {
    if (typeof ev.data === 'string') frames.push(JSON.parse(ev.data) as Record<string, unknown>);
  };
  await new Promise<void>((resolve, reject) => {
    ws.onopen = () => resolve();
    ws.onerror = () => reject(new Error('ws error'));
  });
  return { frames, send: (f) => ws.send(JSON.stringify(f)), close: () => ws.close() };
}

async function status(id: string): Promise<string> {
  const r = await ctx.get(`${base}/api/v1/sessions/${id}`);
  expect(r.ok()).toBeTruthy();
  return ((await r.json()) as { status: string }).status;
}

/** No live process: `reconnectable` (idle sweep, daemon restart) or `exited`
 *  (the app-quit kill used here) — both resume on a classic attach. */
const DORMANT = ['reconnectable', 'exited'];
const dormant = async (id: string) => DORMANT.includes(await status(id));

async function suspendAll(id: string): Promise<void> {
  expect((await ctx.post(`${base}/api/v1/app/kill-sessions`)).ok()).toBeTruthy();
  await expect.poll(() => dormant(id), { timeout: 15_000 }).toBe(true);
}

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  token = c.token;
});

test.afterEach(async () => {
  await ctx?.dispose();
});

test('a view-only attach never resumes the CLI; the first real keystroke does', async () => {
  const wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'claude', title: 'ViewOnly', cwd: '/tmp' },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = ((await r.json()) as { id: string }).id;
  await expect
    .poll(async () => ((await (await ctx.get(`${base}/api/v1/sessions/${id}`)).json()) as { provider_session_id: string | null }).provider_session_id, { timeout: 15_000 })
    .not.toBeNull();
  await suspendAll(id);

  // 1. View-only: attach, wait well past the attach path — nothing spawned.
  const view = await attach(id, true);
  await expect.poll(() => DORMANT.includes(String(view.frames.find((f) => f.type === 'status')?.status))).toBe(true);
  await new Promise((res) => setTimeout(res, 1500));
  expect(await dormant(id)).toBe(true);

  // 2. An emulator reply (DA answer) is not the user: still dormant.
  view.send({ type: 'input', data: b64('\x1b[?1;2c'), user: false });
  await new Promise((res) => setTimeout(res, 1500));
  expect(await dormant(id)).toBe(true);
  expect(view.frames.some((f) => f.type === 'status' && f.status === 'running')).toBe(false);

  // 3. A real keystroke wakes it and moves this socket onto the new process.
  view.send({ type: 'input', data: b64('x'), user: true });
  await expect.poll(() => view.frames.some((f) => f.type === 'status' && f.status === 'running'), { timeout: 15_000 }).toBe(true);
  await expect.poll(() => dormant(id), { timeout: 15_000 }).toBe(false);
  expect(view.frames.some((f) => f.type === 'scrollback' && Number(f.epoch) > 0)).toBe(true);
  view.close();

  // 4. Control: a classic attach still resumes on open.
  await suspendAll(id);
  const classic = await attach(id, false);
  await expect.poll(() => dormant(id), { timeout: 15_000 }).toBe(false);
  classic.close();
});
