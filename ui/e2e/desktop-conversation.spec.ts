import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { existsSync, readdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { apiCtx, seedWorkspace } from './seed';

// Desktop BROWSER: the conversation view (docs/design/conversation-view.md
// §5.1/§5.2). The session's chat is rebuilt from a provider transcript on disk,
// so the spec points a seeded session at one of Track A's redacted fixtures
// (`crates/otto-transcript/fixtures/claude/*.jsonl`) through the `OTTO_E2E=1`
// hook `meta.e2e_transcript_path` (§8 of the design doc) and asserts:
//   • user + assistant turns render,
//   • a tool step expands to its output,
//   • the global "Show system" toggle reveals the per-turn system notes and
//     persists,
//   • the Terminal · Chat · Split choice persists across a reload.
// Only meaningful on the desktop-browser project; self-skips elsewhere, and
// when no fixture is checked in yet.

test.setTimeout(120_000);

const FIXTURE_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '../../crates/otto-transcript/fixtures/claude');

/** Fixture path: explicit override, else Track A's recommended
 *  `01-basic-tools.jsonl` (user + assistant turns, Bash + Edit calls WITH
 *  results, attachment + stop-hook system notes), else any Claude fixture. */
function pickFixture(): string | null {
  const env = process.env.OTTO_E2E_TRANSCRIPT_FIXTURE;
  if (env) return env;
  if (!existsSync(FIXTURE_DIR)) return null;
  const preferred = join(FIXTURE_DIR, '01-basic-tools.jsonl');
  if (existsSync(preferred)) return preferred;
  const files = readdirSync(FIXTURE_DIR, { withFileTypes: true })
    .filter((d) => d.isFile() && d.name.endsWith('.jsonl'))
    .map((d) => join(FIXTURE_DIR, d.name));
  return files[0] ?? null;
}

let ctx: APIRequestContext;
let base: string;
let wsId = '';
let sessionId = '';

const conv = (page: Page) => page.locator('.conv[data-loaded="true"]');

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const fixture = pickFixture();
  test.skip(!fixture, 'no transcript fixture under crates/otto-transcript/fixtures/claude yet (Track A)');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'shell',
      title: 'ConvFixture',
      cwd: '/tmp',
      // `nested_provider` tells the resolver which provider the shell wraps
      // (a bare `shell` session is provider_unsupported).
      meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: fixture },
    },
  });
  if (!r.ok()) throw new Error(`seed session → ${r.status()} ${await r.text()}`);
  sessionId = (await r.json()).id as string;

  // The daemon must actually fold the fixture — otherwise every UI assertion
  // below would fail for the wrong reason.
  const t = await ctx.get(`${base}/api/v1/sessions/${sessionId}/transcript`);
  expect(t.ok(), `GET transcript → ${t.status()}`).toBeTruthy();
  const body = (await t.json()) as { unavailable_reason: string | null; turns: unknown[] };
  expect(body.unavailable_reason, 'fixture must resolve (see §8 e2e hook)').toBeNull();
  expect(body.turns.length).toBeGreaterThan(0);

  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.pane-head', { hasText: 'ConvFixture' }).first()).toBeVisible({ timeout: 20_000 });
  // Terminal is the default for every session (§5.1); the chat is opt-in, so
  // every spec below switches to it explicitly (persisted per session).
  await expect(page.locator('.pane-body[data-view="terminal"]')).toBeVisible({ timeout: 20_000 });
  await page.locator('.view-seg button', { hasText: 'Chat' }).click();
});

test.afterEach(async () => {
  await ctx?.dispose();
});

test('sanitizer renders hostile markdown HTML inert (WebFetch output path)', async ({ page }) => {
  // No TS unit runner in this repo — Vite serves the module, so exercise the
  // real sanitizer in the browser with the bypasses the review named.
  const out = await page.evaluate(async () => {
    // Runtime path (Vite dev serves the source tree); kept out of TS resolution.
    const modPath = ['/src', 'lib', 'sanitize.ts'].join('/');
    const mod = (await import(/* @vite-ignore */ modPath)) as { sanitizeHtml: (h: string) => string };
    const cases = [
      '<section><img src=x onerror="window.__pwned=1"></section>',
      '<noscript><img src=x onerror="window.__pwned=1"></noscript>',
      '<button onclick="window.__pwned=1"><b onmouseover="window.__pwned=1">x</b></button>',
      '<a href="java\tscript:alert(1)">a</a><a href="jAvaScript&#x3a;alert(1)">b</a>',
      '<a href="vbscript:x">c</a><img src="data:text/html,x"><img src="data:image/svg+xml,x">',
      '<img src="data:image/png;base64,AAAA"><a href="https://ok.example/?x=1">ok</a><a href="#frag">f</a>',
      '<div><section><article><img src=x onerror=1 onload=1></article></section></div>',
      '<svg onload=1><script>1</script></svg><math><mi xlink:href="javascript:1">m</mi></math>',
    ];
    const html = cases.map((c) => mod.sanitizeHtml(c)).join('\n');
    const host = document.createElement('div');
    host.innerHTML = html;
    document.body.appendChild(host);
    await new Promise((r) => setTimeout(r, 200));
    const attrs = Array.from(host.querySelectorAll('*')).flatMap((e) =>
      Array.from(e.attributes).map((a) => `${e.tagName.toLowerCase()}.${a.name}=${a.value}`),
    );
    host.remove();
    return { html, attrs, pwned: (window as unknown as { __pwned?: number }).__pwned ?? 0 };
  });
  expect(out.pwned).toBe(0);
  expect(out.html).not.toMatch(/on[a-z]+=/i);
  expect(out.html).not.toMatch(/script:|vbscript|<script|<svg|<math|<section|<noscript|<button|data:text|svg\+xml/i);
  // Allowed survivors still there.
  expect(out.attrs).toContain('img.src=data:image/png;base64,AAAA');
  expect(out.attrs).toContain('a.href=https://ok.example/?x=1');
  expect(out.attrs).toContain('a.rel=noopener noreferrer');
  expect(out.attrs).toContain('a.href=#frag');
});

test('chat renders user + assistant turns from the transcript', async ({ page }) => {
  // beforeEach asserted the Terminal default and switched to Chat (§5.1).
  await expect(page.locator('.pane-body[data-view="chat"]')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.view-seg button.active', { hasText: 'Chat' })).toBeVisible();

  const view = conv(page);
  await expect(view).toBeVisible({ timeout: 20_000 });
  await expect(view.locator('.turn[data-role="user"]').first()).toBeVisible();
  await expect(view.locator('.turn[data-role="assistant"]').first()).toBeVisible();
  // Header stats come from the same fold.
  await expect(view.locator('.conv-head .stats')).toContainText(/\d+ turns/);
});

test('a tool step expands to its output', async ({ page }) => {
  const view = conv(page);
  await expect(view).toBeVisible({ timeout: 20_000 });

  // Steps sit behind a "Worked for … · N steps" header unless the response had
  // a single call — open every group first, then the first tool row.
  const groups = view.locator('.steps:not(.single) .steps-head');
  const n = await groups.count();
  for (let i = 0; i < n; i++) {
    const g = groups.nth(i);
    if ((await g.getAttribute('aria-expanded')) !== 'true') await g.click();
  }
  const row = view.locator('.step-row').first();
  await expect(row, 'fixture must contain at least one tool_call').toBeVisible();
  await expect(row).toHaveAttribute('aria-expanded', 'false');
  await row.click();
  await expect(row).toHaveAttribute('aria-expanded', 'true');
  const body = view.locator('.step .step-body').first();
  await expect(body).toBeVisible();
  // Output <pre> / windowed list / diff / pending — one of them renders.
  await expect(body.locator('.out, .out-vlist, .diff-wrap, .pending, .file-chip').first()).toBeVisible();
});

test('Show system toggles the per-turn system notes and persists', async ({ page }) => {
  const view = conv(page);
  await expect(view).toBeVisible({ timeout: 20_000 });
  const toggle = view.locator('.sys-toggle input');
  await expect(toggle).not.toBeChecked();
  const notesBefore = await view.locator('.sys-list').count();
  expect(notesBefore).toBe(0);

  await toggle.check();
  // Turns that carry system notes reveal them (the chip count tells us how many).
  const chips = view.locator('.sys-chip');
  if ((await chips.count()) > 0) {
    await expect(view.locator('.sys-list').first()).toBeVisible();
    await expect(view.locator('.sys-note').first()).toBeVisible();
  }

  // Global + persisted (localStorage) — survives a reload.
  await page.reload();
  await expect(conv(page)).toBeVisible({ timeout: 20_000 });
  await expect(conv(page).locator('.sys-toggle input')).toBeChecked();
  await conv(page).locator('.sys-toggle input').uncheck();
  await expect(conv(page).locator('.sys-list')).toHaveCount(0);
});

test('Terminal · Chat · Split persists across reload; Split shows chat beside the terminal', async ({ page }) => {
  await expect(page.locator('.pane-body[data-view="chat"]')).toBeVisible({ timeout: 20_000 });

  await page.locator('.view-seg button', { hasText: 'Terminal' }).click();
  await expect(page.locator('.pane-body[data-view="terminal"]')).toBeVisible();
  await expect(page.locator('.conv')).toHaveCount(0);
  await page.reload();
  await expect(page.locator('.pane-body[data-view="terminal"]')).toBeVisible({ timeout: 20_000 });

  // 1280px ≥ 1200px → Split is offered: chat + splitter + terminal.
  await page.locator('.view-seg button', { hasText: 'Split' }).click();
  const body = page.locator('.pane-body[data-view="split"]');
  await expect(body).toBeVisible();
  await expect(body.locator('.pane-chat')).toBeVisible();
  await expect(body.locator('.pane-splitter')).toBeVisible();
  await expect(body.locator('.pane-term')).toBeVisible();
  const chat = await body.locator('.pane-chat').boundingBox();
  const term = await body.locator('.pane-term').boundingBox();
  expect(chat && term && chat.x + chat.width <= term.x + 1).toBeTruthy();

  await page.reload();
  await expect(page.locator('.pane-body[data-view="split"]')).toBeVisible({ timeout: 20_000 });

  // Below 1200px Split degrades to Chat and the Split tab disappears.
  await page.setViewportSize({ width: 1100, height: 800 });
  await expect(page.locator('.pane-body[data-view="chat"]')).toBeVisible();
  await expect(page.locator('.view-seg button', { hasText: 'Split' })).toHaveCount(0);
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(page.locator('.pane-body[data-view="split"]')).toBeVisible();

  // ⌘⇧C cycles (split → terminal at 1280px).
  await page.locator('.pane-head', { hasText: 'ConvFixture' }).click();
  await page.keyboard.press('Meta+Shift+C');
  await expect(page.locator('.pane-body[data-view="terminal"]')).toBeVisible();
});

test('composer is a multi-line box that never scrolls sideways', async ({ page }) => {
  const view = conv(page);
  await expect(view).toBeVisible({ timeout: 20_000 });
  const ta = view.locator('.composer textarea');
  await expect(ta).toBeVisible();
  const box = await ta.boundingBox();
  expect(box && box.height >= 56, `textarea height ${box?.height}`).toBeTruthy();
  // The placeholder is long: it must wrap, not hide behind a horizontal bar.
  const overflow = await ta.evaluate((el) => el.scrollWidth - el.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
  await ta.fill('one\ntwo\nthree\nfour\nfive\nsix');
  const grown = await ta.boundingBox();
  expect(grown && box && grown.height > box.height).toBeTruthy();
});

test('search finds turns, walks matches and copy buttons exist per turn', async ({ page }) => {
  const view = conv(page);
  await expect(view).toBeVisible({ timeout: 20_000 });
  await view.locator('.conv-head button[title^="Search"]').click();
  const input = view.locator('.search-in');
  await expect(input).toBeFocused();
  await input.fill('PR');
  const n = view.locator('.search-n');
  await expect(n).toHaveAttribute('data-search-hits', /^[1-9]\d*$/);
  await expect(view.locator('.turn.hit').first()).toBeVisible();
  await input.press('Enter');
  await expect(view.locator('.turn.current')).toHaveCount(1);
  await input.press('Escape');
  await expect(view.locator('.search-in')).toHaveCount(0);
  await expect(view.locator('.turn.hit')).toHaveCount(0);
  // Copy affordance on every turn with text (revealed on hover).
  const turns = view.locator('.turn[data-role]');
  expect(await view.locator('.turn .copy-btn').count()).toBeGreaterThan(0);
  await turns.first().hover();
  await expect(turns.first().locator('.copy-btn')).toBeVisible();
});

test('an edit step shows +/− stats and a colored diff; touch keeps the tail armed', async ({ page }) => {
  const view = conv(page);
  await expect(view).toBeVisible({ timeout: 20_000 });
  const groups = view.locator('.steps:not(.single) .steps-head');
  const n = await groups.count();
  for (let i = 0; i < n; i++) {
    const g = groups.nth(i);
    if ((await g.getAttribute('aria-expanded')) !== 'true') await g.click();
  }
  const edit = view.locator('.step[data-tool="edit"]').first();
  await expect(edit, 'fixture must contain an Edit call').toBeVisible();
  await expect(edit.locator('.step-stats .add')).toContainText(/\+\d+/);
  await expect(edit.locator('.step-stats .del')).toContainText(/\d+/);
  await edit.locator('.step-row').click();
  await expect(edit.locator('.diff-wrap')).toBeVisible();
  // Keep-alive: the seeded session is live (shell PTY), so touch re-arms → 204.
  const r = await ctx.post(`${base}/api/v1/sessions/${sessionId}/transcript/touch`);
  expect(r.status(), await r.text()).toBe(204);
});

test('typing / opens slash-command completion; images attach as thumbnails', async ({ page }) => {
  const view = conv(page);
  await expect(view).toBeVisible({ timeout: 20_000 });
  const ta = view.locator('.composer textarea');
  // The composer never lets the OS autocorrect/predict over it.
  await ta.click();
  await expect(ta).toHaveAttribute('autocorrect', 'off');
  await expect(ta).toHaveAttribute('spellcheck', 'false');
  await ta.fill('/comp');
  const pop = view.locator('[data-slash-pop]');
  await expect(pop).toBeVisible();
  await expect(pop.locator('.cmd-row.active .cmd-name')).toHaveText('/compact');
  await ta.press('Tab');
  await expect(ta).toHaveValue('/compact ');
  await expect(pop).toHaveCount(0);
  await ta.press('Escape');
  await ta.fill('');

  // Drop a 1×1 PNG on the composer → uploaded to the inbox, shown as a thumbnail,
  // and removable; the textarea stays free of `[Image: …]` text.
  await view.locator('.composer').evaluate((el) => {
    const b64 = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const file = new File([bytes], 'dot.png', { type: 'image/png' });
    const dt = new DataTransfer();
    dt.items.add(file);
    el.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
  });
  const thumb = view.locator('.composer .thumb');
  await expect(thumb).toHaveCount(1, { timeout: 15_000 });
  await expect(thumb.locator('img')).toBeVisible();
  await expect(ta).toHaveValue('');
  await expect(view.locator('.composer .send')).toBeEnabled();
  await thumb.locator('.thumb-x').click();
  await expect(thumb).toHaveCount(0);
});

test('an unsent chat draft survives leaving and returning; the status row shows the cwd', async ({ page }) => {
  const view = conv(page);
  await expect(view).toBeVisible({ timeout: 20_000 });
  await expect(view.locator('[data-status-line] .cwd')).toHaveText('/tmp');
  const ta = view.locator('.composer textarea');
  await ta.fill('half-written thought');
  // Away (Terminal view unmounts the chat) and back.
  await page.locator('.view-seg button', { hasText: 'Terminal' }).click();
  await expect(page.locator('.conv')).toHaveCount(0);
  await page.locator('.view-seg button', { hasText: 'Chat' }).click();
  await expect(conv(page).locator('.composer textarea')).toHaveValue('half-written thought', { timeout: 20_000 });
  // A reload keeps it too (sessionStorage).
  await page.reload();
  await expect(conv(page).locator('.composer textarea')).toHaveValue('half-written thought', { timeout: 20_000 });
  await conv(page).locator('.composer textarea').fill('');
});

test('the workspace keep-alive arms the tails of every live session you can read', async () => {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/transcript/touch`);
  expect(r.status(), await r.text()).toBe(200);
  const body = (await r.json()) as { armed: number };
  expect(body.armed).toBeGreaterThanOrEqual(1);
});
