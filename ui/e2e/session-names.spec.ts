import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Session NAME THEMES end-to-end against the isolated test daemon:
//   - the /name-themes library (built-ins ≥10k capacity + per-user active)
//   - themed auto-naming of new agent sessions (unique among open sessions)
//   - custom themes + the "{name} #N" recycling fallback
//   - name-addressed relay ("ronaldo: …", "all: …", and the unaddressed fallback)
//   - delete a specific session
//   - the Settings → Session Names panel (render + pick + create)
//
// The active theme is per-USER (one root in the harness), so these run serially
// and each test sets the active theme it needs; session-name uniqueness is
// per-WORKSPACE so each naming test uses a fresh workspace.
test.describe.configure({ mode: 'serial' });

// The active theme is a single per-user value on the shared test daemon, so the
// 5 device projects must NOT run this file concurrently (they'd race on it).
// Pin the whole file to one project; the others skip it.
test.beforeEach(({}, testInfo) => {
  test.skip(
    testInfo.project.name !== 'iphone-portrait',
    'name-theme state is global to the user; run on a single project only',
  );
});

let ctx: APIRequestContext;
let base: string;

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
});

test.afterAll(async () => {
  await ctx.dispose();
});

const api = (p: string) => `${base}/api/v1${p}`;

async function getJson(url: string): Promise<any> {
  const r = await ctx.get(url);
  if (!r.ok()) throw new Error(`GET ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}
async function postJson(url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  if (!r.ok()) throw new Error(`POST ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}
async function putJson(url: string, data: unknown): Promise<any> {
  const r = await ctx.put(url, { data });
  if (!r.ok()) throw new Error(`PUT ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}

/** Create a shell agent session WITHOUT a title (so themed naming kicks in). */
async function newThemedSession(wsId: string): Promise<any> {
  return postJson(api(`/workspaces/${wsId}/sessions`), {
    kind: 'agent',
    provider: 'shell',
    cwd: '/tmp',
    meta: { origin: 'e2e' },
  });
}

test('library: built-ins are huge + fame-ordered, default active is footballers', async () => {
  const resp = await getJson(api('/name-themes'));
  const ids = (resp.themes as any[]).map((t) => t.id);
  expect(ids).toContain('footballers');
  expect(ids).toContain('scientists');

  const footballers = (resp.themes as any[]).find((t) => t.id === 'footballers');
  expect(footballers.kind).toBe('builtin');
  // The "10k at least per group" requirement.
  expect(footballers.capacity).toBeGreaterThanOrEqual(10_000);
  expect(footballers.sample.length).toBeGreaterThan(0);

  // A brand-new user defaults to the footballers theme (the "ronaldo" example).
  expect(resp.active).toBe('footballers');
});

test('themed auto-naming: new sessions get unique famous handles, not "shell #N"', async () => {
  await putJson(api('/name-themes/active'), { theme_id: 'footballers' });
  const wsId = await seedWorkspace(ctx, base);

  const a = await newThemedSession(wsId);
  const b = await newThemedSession(wsId);

  for (const s of [a, b]) {
    expect(s.title, 'should not fall back to numbered naming').not.toMatch(/^shell #/);
    expect(s.title.length).toBeGreaterThan(0);
    // The callable handle + full name are recorded for the resolver + UI.
    expect(s.meta.name_handle, 'name_handle recorded').toBeTruthy();
    expect(s.meta.name_full, 'name_full recorded').toBeTruthy();
  }
  // Unique among the workspace's open sessions.
  expect(a.title.toLowerCase()).not.toBe(b.title.toLowerCase());

  // Cleanup so we don't leak shell PTYs.
  for (const s of [a, b]) await ctx.delete(api(`/sessions/${s.id}`));
});

test('custom theme: recycles its names then suffixes "#2" when exhausted', async () => {
  const theme = await postJson(api('/name-themes'), {
    label: 'E2E Family',
    names: ['Dad', 'Mom'],
  });
  await putJson(api('/name-themes/active'), { theme_id: theme.id });
  const wsId = await seedWorkspace(ctx, base);

  const s1 = await newThemedSession(wsId);
  const s2 = await newThemedSession(wsId);
  const s3 = await newThemedSession(wsId);

  expect(s1.title).toBe('Dad');
  expect(s2.title).toBe('Mom');
  // Both base names taken → suffix scheme (relevant for OPEN sessions only).
  expect(s3.title).toBe('Dad #2');

  for (const s of [s1, s2, s3]) await ctx.delete(api(`/sessions/${s.id}`));
  // Reset active for the following tests.
  await putJson(api('/name-themes/active'), { theme_id: 'footballers' });
});

test('relay: address by name, broadcast with "all", and fall back when unaddressed', async () => {
  await putJson(api('/name-themes/active'), { theme_id: 'footballers' });
  const wsId = await seedWorkspace(ctx, base);

  const a = await newThemedSession(wsId);
  const b = await newThemedSession(wsId);
  const handleA: string = a.meta.name_handle;

  // "ronaldo: do X" → delivered only to session A.
  const r1 = await postJson(api(`/workspaces/${wsId}/relay`), {
    text: `${handleA}: list the files`,
  });
  expect(r1.unaddressed).toBe(false);
  expect(r1.broadcast).toBe(false);
  expect(r1.session_ids).toContain(a.id);
  expect(r1.session_ids).not.toContain(b.id);
  expect(r1.text).toBe('list the files');

  // "all: …" → broadcast to every live agent session.
  const r2 = await postJson(api(`/workspaces/${wsId}/relay`), { text: 'all: stand by' });
  expect(r2.broadcast).toBe(true);
  expect(r2.session_ids).toEqual(expect.arrayContaining([a.id, b.id]));

  // No recognizable name → unaddressed (UI falls back to the AI orchestrator).
  const r3 = await postJson(api(`/workspaces/${wsId}/relay`), {
    text: 'zzqqxx build the whole feature',
  });
  expect(r3.unaddressed).toBe(true);
  expect(r3.session_ids).toEqual([]);

  // Delete a SPECIFIC session and confirm it's gone.
  const delRes = await ctx.delete(api(`/sessions/${a.id}`));
  expect(delRes.ok()).toBeTruthy();
  const list = await getJson(api(`/workspaces/${wsId}/sessions`));
  const ids = (list.sessions ?? list).map((s: any) => s.id);
  expect(ids).not.toContain(a.id);

  await ctx.delete(api(`/sessions/${b.id}`));
});

test('settings panel: theme grid renders, picking one activates it, custom theme creates', async ({
  page,
}) => {
  await page.goto('/#/settings/session-names');
  await expect(page.getByRole('heading', { name: 'Session Names' })).toBeVisible({
    timeout: 15_000,
  });
  // On phone the Navigator drawer auto-opens (after hydration) and overlays the
  // content; dismiss it so the settings panel is interactable. The click
  // auto-waits for the ✕ to appear and is a no-op on wider viewports.
  await page
    .locator('.drawer.left .drawer-close')
    .click({ timeout: 5_000 })
    .catch(() => {});
  await expect(page.locator('.drawer-backdrop'))
    .toHaveCount(0, { timeout: 5_000 })
    .catch(() => {});

  // Built-in theme cards render.
  const footballers = page.locator('.theme-card', { hasText: 'Footballers' });
  await expect(footballers).toBeVisible();
  const scientists = page.locator('.theme-card', { hasText: 'Scientists' });
  await expect(scientists).toBeVisible();

  // Pick Scientists → it gains the Active badge.
  await scientists.click();
  await expect(scientists.locator('.badge-on')).toBeVisible({ timeout: 10_000 });

  // Create a custom theme via the form.
  const label = `UI Team ${Date.now()}`;
  await page.locator('#nt-label').fill(label);
  await page.locator('#nt-names').fill('Alice\nBob\nCarol');
  await page.getByRole('button', { name: 'Create theme' }).click();

  // It shows up as a selectable custom card.
  await expect(page.locator('.theme-card', { hasText: label })).toBeVisible({ timeout: 10_000 });

  // Reset to footballers so we leave the user in a known state.
  await page.locator('.theme-card', { hasText: 'Footballers' }).click();
  await expect(
    page.locator('.theme-card', { hasText: 'Footballers' }).locator('.badge-on'),
  ).toBeVisible({ timeout: 10_000 });
});
