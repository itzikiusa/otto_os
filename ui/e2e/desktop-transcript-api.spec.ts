import { test, expect, type APIRequestContext } from '@playwright/test';
import { rmSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { isDesktopProject } from './perf';
import { writeChatTranscript } from './chat-fixture';

// The daemon's transcript surface (GET /sessions/{id}/transcript, the
// workspace keep-alive). The session Chat view that used to host these checks
// was removed in favour of Threads; the endpoints stay (Threads reads them), so
// their contract keeps its own API-only spec.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const dirs: string[] = [];

async function seedTranscriptSession(title: string): Promise<string> {
  const { dir, path, root } = writeChatTranscript();
  dirs.push(dir);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: root, meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: path } },
  });
  if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
  return (await r.json()).id as string;
}

test.beforeEach(async ({}, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop projects only');
  ({ ctx, base } = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
});

test.afterEach(async () => {
  await ctx?.dispose();
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

test('the daemon fills per-turn usage from the transcript, thinking tokens included', async () => {
  const id = await seedTranscriptSession('Usage');
  const r = await ctx.get(`${base}/api/v1/sessions/${id}/transcript`);
  expect(r.ok()).toBeTruthy();
  const body = (await r.json()) as {
    turns: { role: string; usage?: { input_tokens: number; output_tokens: number; thinking_tokens: number } | null }[];
  };
  const assistant = body.turns.filter((t) => t.role === 'assistant');
  expect(assistant.length).toBeGreaterThan(0);
  // Every assistant turn carries usage; user turns never do.
  for (const t of assistant) expect(t.usage?.input_tokens ?? 0).toBeGreaterThan(0);
  for (const t of body.turns.filter((t) => t.role === 'user')) expect(t.usage ?? null).toBeNull();
  // The fixture's thinking records report output_tokens_details.thinking_tokens.
  expect(assistant.some((t) => (t.usage?.thinking_tokens ?? 0) > 0)).toBeTruthy();
});

test('the workspace keep-alive arms the tails of every live session you can read', async () => {
  await seedTranscriptSession('Keep-alive');
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/transcript/touch`);
  expect(r.status(), await r.text()).toBe(200);
  const body = (await r.json()) as { armed: number };
  expect(body.armed).toBeGreaterThanOrEqual(1);
});
