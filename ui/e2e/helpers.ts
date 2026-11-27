import { expect, type Locator, type Page, type Request } from '@playwright/test';
import { appendFileSync, mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import AxeBuilder from '@axe-core/playwright';

// The top-level routable pages (hash router: #/<module>). `share` is excluded
// (it needs a scoped token) and is covered separately. Every sidebar module must
// be listed here (or exempted, with a reason, in unit/pagesCoverage.test.ts) so
// the axe / RTL / theme sweeps reach it.
export const PAGES = [
  'agents',
  'threads',
  'rooms',
  'rooms/recaps',
  'home',
  'api',
  'aws',
  'brokers',
  'connections',
  'database',
  'git',
  'walkthroughs',
  'insights',
  'kubernetes',
  'containers',
  'settings/plugins',
  'product',
  'settings',
  'skills-eval',
  'usage',
  'vault',
  'workflows',
  'loops',
  'canvas',
  'design',
  'design/brand',
  'design/learned',
  'browser',
  'mcp',
  'scheduled-tasks',
  'personal-agents',
  'ide',
] as const;

export type PageId = (typeof PAGES)[number];

/** Navigate to a hash route and wait for the shell + content to settle. */
/** The DB workbench's view (Query / Builder / Structure / Diagram / Dashboards)
 *  lives in the header's ⋯ (a non-Query view shows as a chip that returns). */
export async function setDbView(page: Page, view: string): Promise<void> {
  await page.getByTestId('page-header').getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitemcheckbox', { name: view, exact: true }).click();
}

export async function openPage(page: Page, id: string): Promise<void> {
  await page.goto(`/#/${id}`);
  // The app shell is always present once booted.
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  // Let layout/reflow + first data fetch settle.
  await page.waitForLoadState('networkidle').catch(() => {});
}

/**
 * Open the New Session sheet with ⌘T, falling back to the TabBar + button only
 * if the shortcut never reaches the app. The sheet is lazy-loaded
 * (`{#await import(...)}` in App.svelte), so it appears a beat after the key
 * press: an instant `isVisible()` check reads false, and the fallback click then
 * lands on the sheet's own backdrop once it mounts. Wait for the sheet first.
 */
export async function openNewSessionSheet(page: Page): Promise<Locator> {
  const dialog = page.locator('.sheet[role="dialog"][aria-label="New session"]');
  await page.keyboard.press('Meta+t');
  const opened = await dialog
    .waitFor({ state: 'visible', timeout: 10_000 })
    .then(() => true, () => false);
  if (!opened) await page.getByTitle('New session', { exact: true }).click();
  await expect(dialog).toBeVisible();
  return dialog;
}

/** Every control takes a finger on a phone: 44 × 44 px (the owner's rule; a
 *  hit area may exceed the glyph, so this measures the control's own box). */
export async function expectFingerSized(...targets: Locator[]): Promise<void> {
  for (const t of targets) {
    const b = await t.boundingBox();
    const name = (await t.getAttribute('aria-label')) ?? (await t.innerText()).trim();
    expect(b, `${name} is on the page`).not.toBeNull();
    expect(Math.round(b!.height), `${name} height`).toBeGreaterThanOrEqual(44);
    expect(Math.round(b!.width), `${name} width`).toBeGreaterThanOrEqual(44);
  }
}

/**
 * Open the API page and make sure the request editor is showing. A workspace
 * with nothing in it (no saved requests, history or edited tab) opens on the
 * "Create your first request" onboarding state; this clicks through it.
 */
export async function openApiEditor(page: Page): Promise<void> {
  await openPage(page, 'api');
  // The editor shows (aria-busy) while the workspace's lists are in flight; an
  // untouched empty workspace swaps it for onboarding once they settle. Decide
  // only after that, or the onboarding check races the swap.
  const url = page.getByLabel('Request URL', { exact: true });
  const onboarding = page.getByText('Create your first request', { exact: true });
  await expect(url.or(onboarding).first()).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.api-page[aria-busy="true"]')).toHaveCount(0, { timeout: 15_000 });
  if (await onboarding.isVisible()) {
    await page.getByRole('button', { name: 'New request', exact: true }).first().click();
  }
  await expect(url).toBeVisible();
}

/**
 * Assert the page does not overflow the viewport horizontally. A small
 * tolerance absorbs sub-pixel rounding. This is the core check for the "content
 * runs past the right edge / is clipped" class of bugs.
 */
export async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  const overflow = await page.evaluate(() => {
    const el = document.documentElement;
    return el.scrollWidth - el.clientWidth;
  });
  expect(overflow, 'horizontal overflow (px) past the viewport').toBeLessThanOrEqual(2);
}

/**
 * Assert the main content pane actually has height — this is what catches the
 * "lower pane collapses to ~0" class of bugs (blank terminal, missing DB
 * results). `.center` is the content pane in both desktop and mobile shells.
 */
export async function expectContentHasHeight(page: Page, min = 120): Promise<void> {
  const box = await page.locator('.center').first().boundingBox();
  expect(box, 'content pane (.center) should be present').not.toBeNull();
  expect(box!.height, 'content pane height').toBeGreaterThan(min);
  expect(box!.width, 'content pane width').toBeGreaterThan(0);
}

/**
 * Assert a floating element (menu / dropdown / popover / typeahead) is FULLY
 * inside the viewport. This is the core check for the "popup rendered
 * off-screen / clipped" class of bugs: flip-style positioning (`top = y - h`)
 * goes negative when the popup is taller than the window, leaving the whole
 * thing unreachable. Popups with data-driven lists must be tested with enough
 * items to overflow the window (they should clamp + scroll internally) — see
 * desktop-git-add-menu.spec.ts.
 */
export async function expectFullyInViewport(
  page: Page,
  locator: import('@playwright/test').Locator,
  what = 'floating element',
): Promise<void> {
  await expect(locator).toBeVisible();
  // Measure where the element SETTLES: an entrance animation (a drawer sliding
  // in, a menu popping in) moves it through off-screen positions on the way.
  await locator.evaluate((el) =>
    Promise.all(
      document
        .getAnimations()
        .filter((a) => a.playState === 'running' && a.effect instanceof KeyframeEffect && a.effect.target instanceof Element && a.effect.target.contains(el))
        .map((a) => a.finished.catch(() => undefined)),
    ),
  );
  const box = await locator.boundingBox();
  const viewport = page.viewportSize()!;
  expect(box, `${what} should render`).not.toBeNull();
  expect(box!.y, `${what} top must be inside the viewport`).toBeGreaterThanOrEqual(0);
  expect(box!.x, `${what} left must be inside the viewport`).toBeGreaterThanOrEqual(0);
  expect(box!.y + box!.height, `${what} bottom must be inside the viewport`).toBeLessThanOrEqual(
    viewport.height,
  );
  expect(box!.x + box!.width, `${what} right must be inside the viewport`).toBeLessThanOrEqual(
    viewport.width,
  );
}

/**
 * The DB explorer opens Mongo results in Vertical view by default (and any
 * engine's result past the auto-Vertical column threshold); grid-shaped
 * assertions opt back in with this. Waits for the results toolbar (the view
 * switch only exists once a result with columns is on screen), so it is safe to
 * call straight after Run; a no-op when the switch never appears (an error /
 * empty result — the caller's own assertion reports that) or already shows Grid.
 * Picking Grid is remembered on the tab, so later runs in the same tab stay Grid.
 */
export async function ensureGridView(page: Page, timeout = 20_000): Promise<void> {
  const seg = page.locator('.view-seg');
  const shown = await seg.waitFor({ state: 'visible', timeout }).then(
    () => true,
    () => false,
  );
  if (!shown) return;
  const on = await seg.locator('.vs.on').textContent();
  if ((on ?? '').trim() !== 'Grid') await seg.locator('.vs', { hasText: 'Grid' }).click();
  await expect(seg.locator('.vs.on')).toHaveText('Grid');
}

/** Serious-impact axe rules each page is allowed to still have (S19-304),
 *  ratchet-style like ui-guards-baseline.json: a page with an entry fails on
 *  any serious rule NOT listed; a page without one is critical-only until a
 *  baseline is recorded for it. Record / refresh with
 *  `OTTO_A11Y_RECORD=1 npm run test:e2e -- pages.spec.ts theme.spec.ts rtl.spec.ts`
 *  then `node scripts/a11y-baseline.mjs` (merges test-results/a11y-record.jsonl).
 *  Only ever shrink an entry by hand; never add a rule to make a test pass. */
const A11Y_BASELINE: Record<string, string[]> = (() => {
  try {
    return JSON.parse(readFileSync(join(process.cwd(), 'e2e/a11y-baseline.json'), 'utf8')) as Record<string, string[]>;
  } catch {
    return {};
  }
})();

/** The baseline key for the page on screen: its hash route without a query. */
export function a11yKey(url: string): string {
  const hash = new URL(url).hash.replace(/^#\/?/, '');
  return hash.split('?')[0] || 'root';
}

/**
 * Run an axe-core accessibility scan. Fails on any `critical` violation, and
 * on any `serious` one not in the page's ratcheted allowlist
 * (e2e/a11y-baseline.json; see A11Y_BASELINE). Returns the full violation list
 * so callers can additionally inspect `serious` ones.
 */
export async function expectAccessible(
  page: Page,
  opts: { key?: string } = {},
): Promise<Awaited<ReturnType<AxeBuilder['analyze']>>['violations']> {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'])
    .analyze();
  const critical = results.violations.filter((v) => v.impact === 'critical');
  expect(
    critical,
    `critical a11y violations: ${critical.map((v) => v.id).join(', ')}`,
  ).toEqual([]);
  const key = opts.key ?? a11yKey(page.url());
  const serious = [...new Set(results.violations.filter((v) => v.impact === 'serious').map((v) => v.id))].sort();
  if (process.env.OTTO_A11Y_RECORD) {
    mkdirSync(join(process.cwd(), 'test-results'), { recursive: true });
    appendFileSync(join(process.cwd(), 'test-results/a11y-record.jsonl'), `${JSON.stringify({ key, serious })}\n`);
  } else if (A11Y_BASELINE[key]) {
    const allowed = new Set(A11Y_BASELINE[key]);
    const fresh = serious.filter((id) => !allowed.has(id));
    expect(fresh, `new serious a11y violations on "${key}" (not in e2e/a11y-baseline.json): ${fresh.join(', ')}`).toEqual([]);
  }
  return results.violations;
}

/**
 * Run a ⌘K command by title. On desktop ⌘K focuses the floating bar (the one
 * command surface — FloatingBar.svelte); the row is picked by its visible
 * title and the bar closes once the command runs.
 */
export async function runBarCommand(page: Page, title: string): Promise<void> {
  const input = page.getByRole('combobox', { name: 'Ask Otto or search commands' });
  await page.keyboard.press('Meta+k');
  await expect(input).toBeFocused({ timeout: 10_000 });
  await page.keyboard.type(title);
  await page
    .getByTestId('floating-bar')
    .getByRole('option')
    .filter({ hasText: title })
    .filter({ hasNotText: 'Ask Otto' })
    .first()
    .click();
  // Raw locator: once a terminal takes focus the bar docks and its input
  // leaves the accessibility tree.
  await expect(page.getByTestId('floating-bar').locator('input.input-main')).not.toBeFocused({
    timeout: 10_000,
  });
}

/** Show a Side-panel tab by name: click its rail icon unless it is already
 *  the one shown (a second click would collapse the panel to the rail). */
export async function openRightPanelTab(page: Page, name: string): Promise<void> {
  const btn = page.locator('.rstrip').getByRole('button', { name, exact: true });
  await expect(btn).toBeVisible();
  if ((await btn.getAttribute('aria-pressed')) !== 'true') await btn.click();
  await expect(btn).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.rpanel .rpanel-title')).toHaveText(name);
}

/** The flattened PNG a snip annotated-save POSTs, as base64. The editor sends
 *  a raw `image/png` body (docs/contracts/api.md, `POST /snips/{id}/annotated`),
 *  not the legacy `{data_b64}` JSON — assert that framing, then hand back the
 *  bytes in the form the specs decode. */
export function snipAnnotatedPng(request: Request): string {
  expect(request.headers()['content-type']).toBe('image/png');
  const body = request.postDataBuffer();
  expect(body, 'annotated save must carry a body').not.toBeNull();
  expect(body!.subarray(0, 8).toString('hex'), 'annotated save must be a PNG').toBe('89504e470d0a1a0a');
  return body!.toString('base64');
}

/** The tour film's chapter `title` as the Help page labels its button
 *  (`m:ss Title`, `timeLabel` in src/modules/help/guide.ts) plus its start in
 *  seconds — read from the bundled film.json so the playback specs follow a
 *  re-cut film instead of pinning the old timestamps. */
export function tourChapter(title: string): { label: string; start: number } {
  const film = JSON.parse(readFileSync(join(process.cwd(), 'src/lib/walkthroughs/film.json'), 'utf8')) as {
    chapters: { title: string; start: number }[];
  };
  const chapter = film.chapters.find((c) => c.title === title);
  if (!chapter) throw new Error(`film.json has no chapter titled ${JSON.stringify(title)}`);
  const s = Math.floor(chapter.start);
  return { label: `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')} ${title}`, start: chapter.start };
}

/** Path of a real tour MP4 (OTTO_E2E_TOUR_VIDEO — CI generates a synthetic
 *  one); playback specs skip without it. */
export const tourVideoPath = process.env.OTTO_E2E_TOUR_VIDEO;

/**
 * The select-all chord CodeMirror binds for THIS page. CodeMirror's `Mod` is
 * Meta on Mac or iOS — the same test as @codemirror/view's `browser.mac`
 * (`/Mac/` platform, or an Apple-vendor engine with a `Mobile/` UA or touch
 * points) — Control otherwise. Playwright's `ControlOrMeta` follows the HOST OS instead, so on a
 * Linux runner it sent Control+A to the iPhone (WebKit) project, which
 * CodeMirror ignores — 19 iPhone DB tests failed on that alone (S12-304).
 */
export async function editorSelectAll(page: Page): Promise<string> {
  const apple = await page.evaluate(() => {
    const ios =
      /Apple Computer/.test(navigator.vendor) &&
      (/Mobile\/\w+/.test(navigator.userAgent) || navigator.maxTouchPoints > 2);
    return ios || /Mac/.test(navigator.platform);
  });
  return apple ? 'Meta+A' : 'Control+A';
}

/** /meta with every agent CLI's detection set to `found` (version `version`
 *  when found), so a spec of the setup prompt never depends on what the host
 *  or the harness's stand-ins put on PATH (gemini/agy/grok fakes, a real pi or
 *  opencode). A registered agent without a tool row gets one. */
export function withAgentClis<M extends { providers?: string[]; tools: { name: string }[] }>(meta: M, found: boolean, version: string | null): M {
  const agents = (meta.providers ?? []).filter((p) => p !== 'shell');
  const row = (name: string) => ({ name, found, version: found ? version : null });
  const tools = meta.tools.map((t) => (agents.includes(t.name) ? { ...t, ...row(t.name) } : t));
  for (const a of agents) if (!tools.some((t) => t.name === a)) tools.push(row(a));
  return { ...meta, tools };
}

/** The workspace `.mcp.json` several sessions share never holds a credential:
 *  the session's token appears nowhere in the file, and the otto entry
 *  forwards each bridge variable by REFERENCE (`${OTTO_MCP_TOKEN:-}`, which
 *  each agent CLI expands from its own env) — never a value. */
export function expectMcpJsonHoldsNoCredential(raw: string, token: string): void {
  expect(token.length, 'the captured session token is not empty').toBeGreaterThan(0);
  expect(raw.includes(token), 'the session token is never written into .mcp.json').toBe(false);
  const env = (JSON.parse(raw) as { mcpServers?: Record<string, { env?: Record<string, string> }> }).mcpServers?.otto?.env ?? {};
  for (const [name, value] of Object.entries(env)) {
    expect(value, `${name} is a reference, not a literal`).toMatch(/^\$\{[A-Z][A-Z0-9_]*(:-)?\}$/);
  }
}

/** ⌘K, then wait until the floating bar's input has focus: the bar is a lazy
 *  chunk, so on a busy machine ⌘K right after load can land before it is
 *  mounted, and keys typed meanwhile go to the page (chords), not the bar. */
export async function openFloatingBar(page: Page): Promise<void> {
  await page.keyboard.press('Meta+k');
  await expect(page.getByRole('combobox', { name: 'Ask Otto or search commands' })).toBeFocused();
}

/** Menus list what the connection's access decisions allow at the moment they
 *  open, and on a busy daemon a decision can still be loading (an item is
 *  missing or disabled until it lands). Reopen until `name` is there and
 *  enabled, then pick it. */
export async function pickMenuItem(page: Page, open: () => Promise<unknown>, name: string | RegExp): Promise<void> {
  const item = page.getByRole('menuitem', { name, exact: typeof name === 'string' });
  await expect(async () => {
    if (await page.getByRole('menu', { name: 'Context menu' }).isVisible()) await page.keyboard.press('Escape');
    await open();
    await expect(item).toBeEnabled({ timeout: 1_000 });
  }).toPass({ timeout: 20_000 });
  await item.click();
}

// ── The Database workbench (three rows: header · query tabs · editor) ────────

/** The workbench says it is connected in words, with the server's version
 *  ("Connected · MongoDB 8.2.1 · 14 ms"). The version comes from the health
 *  probe, which runs only once the capabilities and the schema root have
 *  loaded, so this also means the engine's capabilities are in (what the old
 *  engine chip proved). The detail is hidden on a phone, so read its text. */
export async function expectDbConnected(page: Page, engine: 'MySQL' | 'PostgreSQL' | 'MongoDB' | 'Redis' | 'ClickHouse'): Promise<void> {
  await expect(page.getByTestId('db-conn-state')).toContainText(new RegExp(`^\\s*Connected\\s*·\\s*${engine} \\d`), { timeout: 20_000 });
}

/** Pick `name` from the workbench header's one ⋯ (views, the query's
 *  Format / Save / Query plan, the connection's actions). */
export async function pickDbMenuItem(page: Page, name: string | RegExp): Promise<void> {
  await pickMenuItem(page, () => page.getByTestId('page-header').getByRole('button', { name: 'More actions' }).first().click(), name);
}

/** One click on a table shows its rows; its structure opens from its menu. */
export async function openStructureOf(page: Page, label: Locator): Promise<void> {
  await pickMenuItem(page, () => label.click({ button: 'right' }), 'Open structure');
}

/** The results toolbar's export menu (copy, download, Export all rows…,
 *  Import file…): its Export button, which a phone folds into the toolbar's ⋯. */
export function resultsExportButton(page: Page): Locator {
  const bar = page.locator('.grid-toolbar');
  return (page.viewportSize()?.width ?? 0) <= 640
    ? bar.getByRole('button', { name: 'More result actions', exact: true })
    : bar.getByTitle('Copy, download, export all rows, or import a file', { exact: true });
}

/** The connection list is its own page (#/connections); the workbench's back
 *  button goes there. */
export async function showConnectionList(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'All connections', exact: true }).click();
  await expect(page.locator('.conn-list')).toBeVisible({ timeout: 15_000 });
}

/** The open connection's tables: the side pane's Tables tab, or on a phone the
 *  sheet the header's Tables button opens (with the same Tables · Saved ·
 *  History tabs). */
export async function showDbTables(page: Page): Promise<void> {
  if ((page.viewportSize()?.width ?? 0) <= 640) {
    await page.getByTestId('page-header').getByRole('button', { name: 'Tables', exact: true }).click();
  }
  await page.getByRole('tablist', { name: 'Sidebar view' }).getByRole('tab', { name: 'Tables', exact: true }).click();
  await expect(page.locator('.schema-tree')).toBeVisible({ timeout: 15_000 });
}

/** A statement that removes data without a filter (deleteMany({}), DELETE
 *  without WHERE, DROP…) asks first on an unguarded connection; say Run. */
export async function confirmDestructiveRun(page: Page): Promise<void> {
  const dialog = page.getByRole('dialog').filter({ hasText: /Run (a destructive statement|destructive statements)\?/ });
  await dialog.getByRole('button', { name: 'Run', exact: true }).click();
  await expect(dialog).toBeHidden();
}

/** Threads' list on screen: a phone shows it as the page (‹ Threads in the
 *  header goes back to it); desktop / tablet show the column again if it was
 *  hidden (the header's "Show Threads"). */
export async function showThreadList(page: Page): Promise<void> {
  const list = page.getByTestId('threads-sidebar');
  const back = page.getByRole('button', { name: 'Back to Threads' });
  const show = page.getByTestId('show-list-pane');
  await expect(list.or(back).or(show).first()).toBeVisible({ timeout: 15_000 });
  if (await list.isVisible()) return;
  if (await back.isVisible()) await back.click();
  else await show.click();
  await expect(list).toBeVisible();
}

/** Hide Threads' list column (desktop / tablet): its ⋯ › Hide Threads. */
export async function hideThreadList(page: Page): Promise<void> {
  const list = page.getByTestId('threads-sidebar');
  await list.getByRole('button', { name: 'Threads actions' }).click();
  await page.getByRole('menuitem', { name: /^Hide Threads/ }).click();
  await expect(list).toBeHidden();
}
