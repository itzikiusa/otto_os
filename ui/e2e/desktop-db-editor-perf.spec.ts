import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// DB Explorer — large pasted script stays responsive (perf regression guard).
//
// The report: ~200 single-line INSERTs (50–500 KB) in the query editor made the
// whole app laggy. Causes fixed, each guarded here:
//   • unwrapped long highlighted lines re-laid out every rendered line per key
//     → the editor soft-wraps by default (`.cm-lineWrapping`), with a toggle;
//   • completion shipped the WHOLE buffer (prefix + suffix) per request and
//     fired inside VALUES data → requests carry only the current statement and
//     never fire inside string literals;
//   • switching query tabs rebuilt (re-parsed) the editor and dropped undo.
// Runs in WebKit (≈ Tauri's WKWebView). The typing budget is deliberately
// generous (loaded CI machines) — the structural assertions catch regressions.
// Uses the Docker-free mocked connection (db-mock.ts).
// ─────────────────────────────────────────────────────────────────────────────

// `serviceWorkers: 'block'`: once sw.js claims the page, WebKit's fetches go
// through the worker and `page.route` never sees them — the DB mock (and the
// completion recorder below) silently miss and the real daemon dials the fake
// host instead.
test.use({ browserName: 'webkit', serviceWorkers: 'block' });

let workspaceId = '';
let connId = '';
const CONN = `mock-perf-${Math.random().toString(36).slice(2, 8)}`;

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  connId = await seedMockDbConnection(ctx, base, workspaceId, CONN);
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

/** 200 one-line INSERTs, 6 tuples each (~150 KB) — the worst case measured. */
function wideInserts(n = 200): string {
  const out: string[] = [];
  for (let i = 0; i < n; i++) {
    const vals: string[] = [];
    for (let r = 0; r < 6; r++) {
      vals.push(
        `(${i * 10 + r}, 'user_${i}_${r}', 'user${i}_${r}@example.com', 'ACTIVE', '2024-01-01 12:00:00', ${(r * 3.1).toFixed(2)}, 'EUR', 'MT', 'note ${r}; not a statement end')`,
      );
    }
    out.push(
      `INSERT INTO players (id, login, email, status, created_at, balance, currency, country, note) VALUES ${vals.join(', ')};`,
    );
  }
  return out.join('\n');
}

interface CompletionReq {
  prefix: number;
  suffix: number;
  /** The request's text before the cursor (tail only — for word assertions). */
  tail: string;
}

/** When set, the mocked daemon flags its answer `truncated` (item cap hit). */
let answerTruncated = false;
/** Artificial /completion latency (ms) — a cold schema snapshot build. */
let completionDelay = 0;

async function openEditor(page: Page, seen: CompletionReq[]): Promise<void> {
  await mockDbRoutes(page, connId);
  // Registered after mockDbRoutes, so it wins for /completion: record what each
  // request carries and answer with a realistic item list.
  const items = Array.from({ length: 200 }, (_, i) => ({
    label: `${['customers', 'country', 'created_at', 'city'][i % 4]}_${i}`,
    kind: ['table', 'column', 'column', 'keyword'][i % 4],
    detail: null,
    score: i % 7,
  }));
  await page.route(new RegExp(`/connections/${connId}/db/completion$`), async (route) => {
    const body = (route.request().postDataJSON() ?? {}) as { prefix?: string; suffix?: string };
    seen.push({
      prefix: (body.prefix ?? '').length,
      suffix: (body.suffix ?? '').length,
      tail: (body.prefix ?? '').slice(-40),
    });
    const reply = answerTruncated ? { items, truncated: true } : { items };
    if (completionDelay) await new Promise((r) => setTimeout(r, completionDelay));
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(reply) });
  });
  await page.goto('/#/database');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const c = page.locator('.conn-list .conn-name', { hasText: CONN });
  await expect(c.first()).toBeVisible({ timeout: 30_000 });
  await c.first().click();
  await expect(page.locator('.qe-edit .cm-content')).toBeVisible({ timeout: 20_000 });
}

/** Paste through the real DOM path (what a native ⌘V fires on the editor) and
 *  time it to the first painted frame. */
async function paste(page: Page, text: string): Promise<number> {
  return page.evaluate(async (t) => {
    const el = document.querySelector('.qe-edit .cm-content') as HTMLElement;
    el.focus();
    const dt = new DataTransfer();
    dt.setData('text/plain', t);
    const t0 = performance.now();
    el.dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
    await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
    return performance.now() - t0;
  }, text);
}

/** Editor facts, read (never written) through the CM view. */
async function cm(page: Page): Promise<{ len: number; head: number; text: string }> {
  return page.evaluate(() => {
    const el = document.querySelector('.qe-edit .cm-content') as HTMLElement & {
      cmTile?: { root?: { view?: { state: { doc: { length: number; toString(): string }; selection: { main: { head: number } } } } } };
    };
    const view = el.cmTile?.root?.view;
    if (!view) throw new Error('no CodeMirror view');
    return { len: view.state.doc.length, head: view.state.selection.main.head, text: view.state.doc.toString() };
  });
}

/** Type `text` one key at a time; per key, keydown → first task after the next
 *  animation frame (≈ the frame the keystroke is painted in). */
async function typeTimed(page: Page, text: string): Promise<number[]> {
  await page.evaluate(() => {
    const w = window as unknown as { __keys: number[]; __probe?: boolean };
    w.__keys = [];
    if (w.__probe) return;
    w.__probe = true;
    addEventListener(
      'keydown',
      () => {
        const t0 = performance.now();
        requestAnimationFrame(() => setTimeout(() => w.__keys.push(performance.now() - t0), 0));
      },
      true,
    );
  });
  for (const ch of text) {
    await page.keyboard.type(ch);
    await page.waitForTimeout(40);
  }
  await page.waitForTimeout(200);
  return page.evaluate(() => (window as unknown as { __keys: number[] }).__keys);
}

const pct = (xs: number[], p: number): number => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor(p * s.length))] ?? 0;
};

test('200 pasted INSERTs: wrapped, responsive, completion scoped to the statement', async ({ page }) => {
  test.setTimeout(120_000);
  const seen: CompletionReq[] = [];
  await openEditor(page, seen);

  // Soft-wrap is on by default (the fix for the per-keystroke relayout).
  await expect(page.locator('.qe-edit .cm-content.cm-lineWrapping')).toHaveCount(1);

  const script = wideInserts();
  const pasteMs = await paste(page, script);
  expect((await cm(page)).len).toBe(script.length);
  expect(pasteMs, `paste → first frame ${pasteMs.toFixed(0)} ms`).toBeLessThan(400);

  // "Run all" appears once the (idle-debounced) analysis settles.
  await expect(page.getByRole('button', { name: 'Run all' })).toBeVisible({ timeout: 5_000 });

  // Type inside a VALUES string literal near the top: pure data entry — no
  // completion request may fire (it used to on every word and after every `, `).
  await page.evaluate(() => {
    const el = document.querySelector('.qe-edit .cm-content') as HTMLElement & {
      cmTile?: { root?: { view?: { state: { doc: { toString(): string } }; dispatch(s: unknown): void; focus(): void } } };
    };
    const view = el.cmTile!.root!.view!;
    const at = view.state.doc.toString().indexOf("'user_3_1") + 3; // inside the quotes
    view.dispatch({ selection: { anchor: at } });
    view.focus();
  });
  const before = seen.length;
  const inString = await typeTimed(page, 'abc def, xyz');
  await page.waitForTimeout(400);
  expect(seen.length - before, 'no completion requests while typing string data').toBe(0);

  // Type a new statement at the end: identifiers → completion runs, but each
  // request carries only the current statement, never the ~150 KB buffer.
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.press('Enter');
  const atEnd = await typeTimed(page, 'SELECT cou FROM cus');
  await expect.poll(() => seen.length, { timeout: 5_000 }).toBeGreaterThan(before);
  for (const r of seen) {
    expect(r.prefix, 'completion prefix is bounded').toBeLessThanOrEqual(8192);
    expect(r.suffix, 'completion suffix is bounded').toBeLessThanOrEqual(2048);
  }
  const doc = await cm(page);
  expect(Math.max(...seen.map((r) => r.prefix + r.suffix))).toBeLessThan(doc.len / 10);

  // The popup mounts a bounded number of rows (`maxRenderedOptions`): mounting
  // every matching option of a 200-item answer was the one 49–160 ms frame
  // left while typing identifiers. ~50 options match `cus`; ≤ 40 render.
  await page.keyboard.press('Control+Space');
  const options = page.locator('.cm-tooltip-autocomplete li');
  await expect(options.first()).toBeVisible({ timeout: 5_000 });
  expect(await options.count(), 'rendered completion rows').toBeLessThanOrEqual(40);
  await page.keyboard.press('Escape');

  // Generous per-key budget (WebKit): wrapped typing measured ~3–6 ms/key here
  // vs 20–41 ms unwrapped; the bound only catches a gross regression.
  const keys = [...inString, ...atEnd];
  const p50 = pct(keys, 0.5);
  const p95 = pct(keys, 0.95);
  test.info().annotations.push({
    type: 'perf',
    description: `paste ${pasteMs.toFixed(0)} ms; key→frame p50 ${p50.toFixed(1)} ms, p95 ${p95.toFixed(1)} ms (n=${keys.length}); completion requests ${seen.length}, max body ${Math.max(...seen.map((r) => r.prefix + r.suffix))} chars`,
  });
  expect(p50, `key→frame p50 ${p50.toFixed(1)} ms`).toBeLessThan(34);
  expect(p95, `key→frame p95 ${p95.toFixed(1)} ms`).toBeLessThan(100);
});

test('Wrap toggle turns wrapping off live and is remembered', async ({ page }) => {
  const seen: CompletionReq[] = [];
  await openEditor(page, seen);
  const wrapped = page.locator('.qe-edit .cm-content.cm-lineWrapping');
  await expect(wrapped).toHaveCount(1);
  const toggle = page.locator('label.qe-wrap');
  await toggle.click();
  await expect(wrapped).toHaveCount(0);
  await page.reload();
  await expect(page.locator('.qe-edit .cm-content')).toBeVisible({ timeout: 30_000 });
  await expect(wrapped).toHaveCount(0);
  await page.locator('label.qe-wrap').click();
  await expect(wrapped).toHaveCount(1);
});

test('switching query tabs keeps the editor state (text, undo) instead of rebuilding', async ({ page }) => {
  const seen: CompletionReq[] = [];
  await openEditor(page, seen);
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.press('Delete');
  await page.keyboard.insertText('SELECT 1');
  await page.waitForTimeout(600); // let the edit land as its own undo event
  await page.keyboard.type(' + 2');
  expect((await cm(page)).text).toBe('SELECT 1 + 2');

  await page.locator('.qe-tab-new').click();
  await expect(page.locator('.qe-tab')).toHaveCount(2);
  expect((await cm(page)).text).toBe('');
  await page.keyboard.insertText('SELECT 3');

  await page.locator('.qe-tab').first().click();
  expect((await cm(page)).text).toBe('SELECT 1 + 2');
  // The tab's undo history survived the round trip (a rebuild wiped it).
  await content.click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.press('ControlOrMeta+Z');
  await expect.poll(async () => (await cm(page)).text).toBe('SELECT 1');

  await page.locator('.qe-tab').nth(1).click();
  expect((await cm(page)).text).toBe('SELECT 3');
});

test('completion: one request per word, re-ask when truncated, smooth popup, dialect-aware strings', async ({ page }) => {
  test.setTimeout(90_000);
  answerTruncated = false;
  const seen: CompletionReq[] = [];
  await openEditor(page, seen);
  const content = page.locator('.qe-edit .cm-content');
  await content.click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.press('Delete');

  // (1) A word typed while its (slow — a cold snapshot build) request is in
  // flight is served by that ONE request: CodeMirror keeps the query alive and
  // filters its answer via `validFor`. It used to abort + re-request on every
  // keystroke (one request per ~100 ms pause, each cancelled by the next key).
  await page.keyboard.insertText('SELECT * FROM ');
  await page.waitForTimeout(300);
  completionDelay = 900;
  const before = seen.length;
  for (const ch of 'customers') {
    await page.keyboard.type(ch);
    await page.waitForTimeout(100);
  }
  await expect(page.locator('.cm-tooltip-autocomplete li').first()).toBeVisible({ timeout: 5_000 });
  completionDelay = 0;
  const perWord = seen.length - before;
  expect(perWord, `requests for one typed word: ${perWord}`).toBeLessThanOrEqual(2);

  // (2) The popup's first frames stay smooth: max rAF gap while it is open.
  const maxGap = await page.evaluate(async () => {
    let last = performance.now();
    let worst = 0;
    const until = last + 500;
    while (performance.now() < until) {
      await new Promise((r) => requestAnimationFrame(() => r(null)));
      const now = performance.now();
      worst = Math.max(worst, now - last);
      last = now;
    }
    return worst;
  });
  expect(maxGap, `max frame gap with the popup open ${maxGap.toFixed(1)} ms`).toBeLessThan(50);
  expect(await page.locator('.cm-tooltip-autocomplete li').count()).toBeLessThanOrEqual(40);
  await page.keyboard.press('Escape');

  // (3) A `truncated` answer is NOT reused: the next keystroke asks again with
  // the longer word (the daemon capped the list for the shorter one).
  answerTruncated = true;
  await page.keyboard.insertText(' WHERE ');
  await page.waitForTimeout(300);
  await page.keyboard.type('c');
  await expect.poll(() => seen.at(-1)?.tail.endsWith('WHERE c') ?? false, { timeout: 5_000 }).toBe(true);
  const afterFirst = seen.length;
  await page.keyboard.type('o');
  await expect.poll(() => seen.length, { timeout: 5_000 }).toBeGreaterThan(afterFirst);
  expect(seen.at(-1)?.tail.endsWith('WHERE co')).toBe(true);
  answerTruncated = false;
  await page.keyboard.press('Escape');

  // (4) MySQL dialect: `'O\'Brien …` is still ONE string literal, so typing
  // inside it never asks for completion (the standard dialect ended the
  // string at `\'` and completed the "identifiers" after it).
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.press('Delete');
  await page.keyboard.insertText("SELECT * FROM customers WHERE name = 'O\\'Brien");
  await page.waitForTimeout(300);
  const inString = seen.length;
  for (const ch of ' and friends') {
    await page.keyboard.type(ch);
    await page.waitForTimeout(30);
  }
  await page.waitForTimeout(400);
  expect(seen.length - inString, "no completion inside 'O\\'Brien …").toBe(0);
});
