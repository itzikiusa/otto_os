import { expect, type APIRequestContext } from '@playwright/test';

// Specs for daemon features newer than a prebuilt ottod fail here, at once,
// with what is missing — not later as a confusing timeout. Each probe changes
// nothing a spec can see.

const tooOld = (what: string): string =>
  `[e2e] daemon too old for this spec: it lacks ${what}. Run against an ottod built from this tree (` +
  '`cargo build -p ottod`, OTTO_E2E_BIN=<repo>/target/debug/ottod).';

/** PATCH /runtime-sessions/{id} `meta` (pins, History, "seen" on the thread). */
export async function requireThreadMeta(ctx: APIRequestContext, base: string): Promise<void> {
  const r = await ctx.patch(`${base}/api/v1/runtime-sessions/e2e-probe`, { data: { meta: { thread_pinned: true } } });
  // An older daemon only takes `{title}` and refuses the body itself.
  if (r.status() === 422 && /missing field/i.test(await r.text())) throw new Error(tooOld('thread meta on PATCH /runtime-sessions/{id}'));
}

/** POST /runtime-sessions/{id}/prompt `provider` (continue on another provider). */
export async function requireProviderSwitch(ctx: APIRequestContext, base: string, workspaceId: string, cwd: string): Promise<void> {
  const created = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/runtime-sessions`, { data: { provider: 'codex', cwd, title: 'e2e provider-switch probe' } });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = ((await created.json()) as { id: string }).id;
  try {
    // A new daemon refuses an unknown provider (400); an old one ignores the field and sends.
    const r = await ctx.post(`${base}/api/v1/runtime-sessions/${id}/prompt`, { data: { text: 'probe', provider: 'e2e-no-such-provider' } });
    if (r.ok()) throw new Error(tooOld('`provider` on POST /runtime-sessions/{id}/prompt'));
  } finally {
    await ctx.post(`${base}/api/v1/runtime-sessions/${id}/close`).catch(() => {});
  }
}
