import { test, expect, type Page, type WebSocketRoute } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// The IDE's language-server client against a scripted server: `/ws/lsp` is answered in
// the page (routeWebSocket), so no real tsserver is needed. Covers the client lifecycle:
// re-attach after the socket drops, one `didChange` per edit with a split view, no
// duplicate lint marks after a tab swap, Problems for files no editor shows, and the
// server's own quick fixes and references taking precedence over Otto's heuristics.

const A = `const x = 1;\nconsole.log(x);\n`;
const B = `export const y = 2;\n`;

interface Conn { ws: WebSocketRoute; msgs: { method?: string; id?: number; params?: any }[]; closed: boolean }

/** Mock server: full-text sync; every line containing `BAD` is an error. */
class MockLsp {
  conns: Conn[] = [];
  /** Return a LocationLink on the retry, once the test ends its index job. */
  duringIndex = false;
  definitionUri?: string;
  pauseInitialize = false;
  acceptInitialize?: () => void;
  docs = new Map<string, string>();
  /** Extra publishes (uri → message) sent right after each `didOpen`. */
  ghosts: Array<{ uri: (openUri: string) => string; message: string }> = [];

  async install(page: Page): Promise<void> {
    await page.route('**/api/v1/lsp/capabilities', (r) =>
      r.fulfill({ json: { servers: [{ lang: 'typescript', available: true, command: 'mock-ts', install_command: null }] } }));
    await page.routeWebSocket(/\/ws\/lsp/, (ws) => {
      const conn: Conn = { ws, msgs: [], closed: false };
      this.conns.push(conn);
      ws.onClose(() => { conn.closed = true; });
      ws.onMessage((raw) => this.handle(conn, JSON.parse(String(raw))));
    });
  }

  private reply(c: Conn, id: number, result: unknown): void {
    c.ws.send(JSON.stringify({ jsonrpc: '2.0', id, result }));
  }

  private publish(c: Conn, uri: string, diagnostics: unknown[]): void {
    c.ws.send(JSON.stringify({ jsonrpc: '2.0', method: 'textDocument/publishDiagnostics', params: { uri, diagnostics } }));
  }

  private diagnose(c: Conn, uri: string): void {
    const lines = (this.docs.get(uri) ?? '').split('\n');
    this.publish(c, uri, lines.flatMap((l, i) => (l.includes('BAD')
      ? [{ range: { start: { line: i, character: 0 }, end: { line: i, character: l.length } }, severity: 1, source: 'mock', message: `bad on line ${i + 1}` }]
      : [])));
  }

  private handle(c: Conn, m: { method?: string; id?: number; params?: any }): void {
    c.msgs.push(m);
    const uri: string | undefined = m.params?.textDocument?.uri;
    switch (m.method) {
      case 'initialize':
        this.acceptInitialize = () => this.reply(c, m.id!, { capabilities: { textDocumentSync: 1, definitionProvider: true, referencesProvider: true, codeActionProvider: { codeActionKinds: ['quickfix'] } } });
        if (!this.pauseInitialize) this.acceptInitialize();
        return;
      case 'textDocument/didOpen':
        this.docs.set(uri!, m.params.textDocument.text);
        this.diagnose(c, uri!);
        for (const g of this.ghosts) this.publish(c, g.uri(uri!), [{ range: { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } }, severity: 1, message: g.message }]);
        return;
      case 'textDocument/didChange':
        this.docs.set(uri!, m.params.contentChanges[m.params.contentChanges.length - 1].text);
        this.diagnose(c, uri!);
        return;
      case 'textDocument/definition':
        if (this.definitionUri || this.pauseInitialize || (this.duringIndex && this.count('textDocument/definition') > 1)) {
          const start = { line: 0, character: 13 };
          const range = { start, end: { line: 0, character: 14 } };
          this.reply(c, m.id!, [{ targetUri: this.definitionUri ?? uri!.replace('lsp-mock-a.ts', 'lsp-mock-b.ts'), targetRange: range, targetSelectionRange: range }]);
        } else {
          this.reply(c, m.id!, null);
        }
        return;
      case 'textDocument/references':
        this.reply(c, m.id!, [0, 1].map((line) => ({ uri, range: { start: { line, character: line === 0 ? 6 : 12 }, end: { line, character: line === 0 ? 7 : 13 } } })));
        return;
      case 'textDocument/codeAction': {
        const line = m.params.range.start.line;
        this.reply(c, m.id!, [
          { title: 'Mock: refactor (never a fix)', kind: 'refactor.move', edit: { changes: { [uri!]: [{ range: m.params.range, newText: '' }] } } },
          { title: 'Mock: fix BAD', kind: 'quickfix', isPreferred: true, edit: { changes: { [uri!]: [{ range: { start: { line, character: 0 }, end: { line, character: 3 } }, newText: 'OK_' }] } } },
        ]);
        return;
      }
      default:
        if (m.id != null && m.method) this.reply(c, m.id, null);
    }
  }

  live(): Conn[] { return this.conns.filter((c) => !c.closed); }
  count(method: string, c?: Conn): number {
    return (c ? [c] : this.conns).reduce((n, x) => n + x.msgs.filter((m) => m.method === method).length, 0);
  }
  openedUri(name: string): string {
    for (const c of this.conns) for (const m of c.msgs) if (m.method === 'textDocument/didOpen' && m.params.textDocument.uri.endsWith(`/${name}`)) return m.params.textDocument.uri;
    return '';
  }
}

// A service worker would answer the capabilities fetch before `page.route` sees it.
test.use({ serviceWorkers: 'block' });
// One worker: the seeded scratch files are shared.

test.describe.configure({ mode: 'serial' });

let ws = '';

// Every test starts from the seeded text (autosave writes the edits back).
test.beforeEach(async () => {
  const { ctx, base } = await apiCtx();
  ws ||= await seedWorkspace(ctx, base, 'LSP mock');
  for (const [path, content] of [['lsp-mock-a.ts', A], ['lsp-mock-b.ts', B]]) {
    const put = await ctx.put(`${base}/api/v1/scratches/file`, { data: { path, content } });
    expect(put.ok()).toBe(true);
  }
  await ctx.dispose();
});

async function openIde(page: Page): Promise<MockLsp> {
  const lsp = new MockLsp();
  await lsp.install(page);
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  await page.goto('/#/ide?scratches=1');
  await expect(page.getByTestId('ide-target-switcher')).toContainText('Scratches');
  return lsp;
}

async function openFile(page: Page, lsp: MockLsp, name: string): Promise<void> {
  await page.getByTestId('ide-project-tree').getByText(name, { exact: true }).dblclick();
  await expect.poll(() => lsp.openedUri(name), { timeout: 15_000 }).not.toBe('');
}

const editor = (page: Page, marker: string) => page.locator('.cm-editor').filter({ hasText: marker }).first();

/** Type `text` on a new line after the line holding `after`. */
async function typeLineAfter(page: Page, marker: string, after: string, text: string): Promise<void> {
  await editor(page, marker).locator('.cm-line', { hasText: after }).first().click();
  await page.keyboard.press('End');
  await page.keyboard.press('Enter');
  await page.keyboard.type(text);
}

/** LSP plugin ids carried by the error marks (a duplicate shows two on one range). */
async function lintPluginIds(page: Page, marker: string): Promise<string[]> {
  return editor(page, marker).evaluate((ed) =>
    [...ed.querySelectorAll('.cm-lintRange-error')].flatMap((el) => [...el.classList].filter((c) => /^\d+(\.\d+)?$/.test(c))));
}


test('a dropped /ws/lsp socket re-attaches: a new error is reported without reopening the tab', async ({ page }) => {
  const lsp = await openIde(page);
  await openFile(page, lsp, 'lsp-mock-a.ts');
  await expect.poll(() => lsp.live().length).toBe(1);
  lsp.conns[0].ws.close({ code: 1006, reason: 'server gone' });
  // The reconnect sequence dials again (1 s backoff) and reopens the document.
  await expect.poll(() => lsp.conns.length, { timeout: 15_000 }).toBeGreaterThan(1);
  const fresh = lsp.conns[lsp.conns.length - 1];
  await expect.poll(() => lsp.count('textDocument/didOpen', fresh), { timeout: 15_000 }).toBe(1);
  await typeLineAfter(page, 'console.log(x)', 'console.log(x)', 'BAD();');
  await expect(editor(page, 'console.log(x)').locator('.cm-lintRange-error')).toHaveCount(1, { timeout: 10_000 });
  await expect.poll(() => lsp.count('textDocument/didChange', fresh)).toBeGreaterThan(0);
});

test('a split view of one file sends each edit once', async ({ page }) => {
  const lsp = await openIde(page);
  await openFile(page, lsp, 'lsp-mock-a.ts');
  await page.getByTestId('ide-split-right-btn').click();
  await expect(page.locator('.cm-editor').filter({ hasText: 'console.log(x)' })).toHaveCount(2);
  await page.waitForTimeout(1500); // ui-guards: allow — both views attached, the mirrored buffer settled
  const before = lsp.count('textDocument/didChange');
  await editor(page, 'console.log(x)').locator('.cm-line', { hasText: 'const x' }).click();
  await page.keyboard.press('End');
  await page.keyboard.type('Z');
  await page.waitForTimeout(2500); // ui-guards: allow — absence: no second didChange once the other pane mirrors the edit
  expect(lsp.count('textDocument/didChange') - before).toBe(1);
});

test('switching tabs away and back leaves one set of lint marks', async ({ page }) => {
  const lsp = await openIde(page);
  await openFile(page, lsp, 'lsp-mock-a.ts');
  await typeLineAfter(page, 'console.log(x)', 'console.log(x)', 'BAD();');
  await expect(editor(page, 'console.log(x)').locator('.cm-lintRange-error')).toHaveCount(1, { timeout: 10_000 });
  await openFile(page, lsp, 'lsp-mock-b.ts');
  await page.getByTestId('ide-tab-item').filter({ hasText: 'lsp-mock-a.ts' }).locator('.ide-tab-btn').click();
  await expect(editor(page, 'console.log(x)').locator('.cm-lintRange-error')).toHaveCount(1, { timeout: 10_000 });
  await page.waitForTimeout(1000); // ui-guards: allow — absence: no second set of marks once the re-attached plugin published
  expect(await lintPluginIds(page, 'console.log(x)')).toHaveLength(1);
});

test('diagnostics for a file no editor shows are listed under Problems', async ({ page }) => {
  const lsp = await openIde(page);
  lsp.ghosts.push({ uri: (open) => open.replace(/[^/]+$/, 'lsp-mock-ghost.ts'), message: 'ghost file error' });
  await openFile(page, lsp, 'lsp-mock-a.ts');
  await page.locator('button', { hasText: /^\s*Problems/ }).last().click();
  await expect(page.getByTestId('ide-problems-panel')).toContainText('ghost file error', { timeout: 10_000 });
});

test('⌥⏎ lists the server’s quick fix first (no refactors) and applies it; Find Usages asks the server', async ({ page }) => {
  const lsp = await openIde(page);
  const textRefs: string[] = [];
  page.on('request', (r) => { if (r.url().includes('/ide/lsp/references')) textRefs.push(r.url()); });
  await openFile(page, lsp, 'lsp-mock-a.ts');
  await typeLineAfter(page, 'console.log(x)', 'console.log(x)', 'BAD();');
  await expect(editor(page, 'console.log(x)').locator('.cm-lintRange-error')).toHaveCount(1, { timeout: 10_000 });
  await page.keyboard.press('Home');
  await page.keyboard.press('Alt+Enter');
  const options = page.getByTestId('ide-intention-popover').getByTestId('ide-intention-option');
  await expect(options.first()).toContainText('Mock: fix BAD');
  await expect(page.getByTestId('ide-intention-popover')).not.toContainText('never a fix');
  await options.first().click();
  await expect(editor(page, 'console.log(x)')).toContainText('OK_();');

  await editor(page, 'console.log(x)').locator('.cm-line', { hasText: 'const x' }).click();
  await page.keyboard.press('Home');
  for (let i = 0; i < 6; i++) await page.keyboard.press('ArrowRight');
  await page.keyboard.press('Alt+F7');
  const usages = page.getByTestId('ide-usages-panel');
  await expect(usages).toContainText('2 references', { timeout: 10_000 });
  await expect(usages.getByTestId('ide-usages-text-match')).toHaveCount(0);
  expect(textRefs).toEqual([]);
});

test('the lint tooltip lists only quick fixes, as themed buttons in light and dark', async ({ page }) => {
  const lsp = await openIde(page);
  await openFile(page, lsp, 'lsp-mock-a.ts');
  await typeLineAfter(page, 'console.log(x)', 'console.log(x)', 'BAD();');
  const mark = editor(page, 'console.log(x)').locator('.cm-lintRange-error');
  await expect(mark).toHaveCount(1, { timeout: 10_000 });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.mouse.move(0, 0);
    await mark.hover();
    const action = page.locator('.cm-tooltip-lint .cm-diagnosticAction');
    await expect(action).toHaveText(['Mock: fix BAD'], { timeout: 10_000 });
    const [bg, want] = await action.evaluate((el) => {
      const probe = document.createElement('span');
      probe.style.backgroundColor = 'var(--accent-soft)';
      el.parentElement!.appendChild(probe);
      const out = [getComputedStyle(el).backgroundColor, getComputedStyle(probe).backgroundColor];
      probe.remove();
      return out;
    });
    expect(bg).toBe(want);
    if (process.env.OTTO_E2E_LINT_SHOTS) await page.screenshot({ path: `${process.env.OTTO_E2E_LINT_SHOTS}/lint-tooltip-${scheme}.png` });
  }
});

test('a definition with no target names the symbol and leaves Find Usages closed', async ({ page }) => {
  const lsp = await openIde(page);
  let fallback = 0;
  await page.route('**/api/v1/ide/lsp/definition', (route) => {
    fallback++;
    return route.fulfill({ json: { symbol: 'absentSymbol', locations: [] } });
  });
  await openFile(page, lsp, 'lsp-mock-a.ts');
  await typeLineAfter(page, 'console.log(x)', 'console.log(x)', 'absentSymbol();');
  await editor(page, 'absentSymbol').getByText('absentSymbol', { exact: true }).click({ modifiers: ['Meta'] });
  await expect(page.getByText(/no definition found.*absentSymbol/i)).toBeVisible();
  expect(fallback).toBe(1);
  expect(lsp.count('textDocument/definition')).toBe(1);
  expect(lsp.count('textDocument/references')).toBe(0);
});

test('an unresolved Rust dependency explains the unavailable language server', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const name = 'lsp-no-server.rs';
  const file = await ctx.put(`${base}/api/v1/scratches/file`, {
    data: { path: name, content: 'pub type Count = AtomicUsize;\n' },
  });
  expect(file.ok()).toBe(true);
  await ctx.dispose();
  await page.route('**/api/v1/lsp/capabilities', (route) => route.fulfill({
    json: { servers: [{ lang: 'rust', available: false, command: 'rust-analyzer', install_command: 'rustup component add rust-analyzer' }] },
  }));
  await page.route('**/api/v1/ide/lsp/definition', (route) =>
    route.fulfill({ json: { symbol: 'AtomicUsize', locations: [] } }));
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), ws);
  await page.goto('/#/ide?scratches=1');
  await page.getByTestId('ide-project-tree').getByText(name, { exact: true }).dblclick();
  await editor(page, 'AtomicUsize').getByText('AtomicUsize', { exact: true }).click({ modifiers: ['Meta'] });
  await expect(page.getByText(/rust-analyzer is unavailable/i)).toBeVisible();
});

test('a failed definition lookup shows the error', async ({ page }) => {
  const lsp = await openIde(page);
  await page.route('**/api/v1/ide/lsp/definition', (route) =>
    route.fulfill({ status: 503, json: { error: 'Cannot read project index' } }));
  await openFile(page, lsp, 'lsp-mock-a.ts');
  await typeLineAfter(page, 'console.log(x)', 'console.log(x)', 'absentSymbol();');
  await editor(page, 'absentSymbol').getByText('absentSymbol', { exact: true }).click({ modifiers: ['Meta'] });
  await expect(page.getByText(/couldn’t look up the definition/i)).toBeVisible();
});

test('definition retries once when indexing finishes and opens the LocationLink', async ({ page }) => {
  const lsp = await openIde(page);
  lsp.duringIndex = true;
  await page.route('**/api/v1/ide/lsp/definition', (route) =>
    route.fulfill({ json: { symbol: 'lateSymbol', locations: [] } }));
  await openFile(page, lsp, 'lsp-mock-a.ts');
  await typeLineAfter(page, 'console.log(x)', 'console.log(x)', 'lateSymbol();');
  const progress = (kind: 'begin' | 'end') => lsp.live()[0].ws.send(JSON.stringify({
    jsonrpc: '2.0', method: '$/progress', params: { token: 'index', value: { kind, title: 'Indexing project' } },
  }));
  progress('begin');
  const token = editor(page, 'lateSymbol').getByText('lateSymbol', { exact: true });
  await token.click({ modifiers: ['Meta'] });
  await expect.poll(() => lsp.count('textDocument/definition')).toBe(1);
  await expect(page.getByText(/still indexing/i)).toBeVisible();
  progress('end');
  await expect(editor(page, 'export const y').locator('.cm-content')).toContainText('export const y');
  expect(lsp.count('textDocument/definition')).toBe(2);
  expect(lsp.count('textDocument/references')).toBe(0);
});

test('a definition requested during startup continues after the handshake', async ({ page }) => {
  const lsp = await openIde(page);
  lsp.pauseInitialize = true;
  await page.route('**/api/v1/ide/lsp/definition', (route) =>
    route.fulfill({ json: { symbol: 'log', locations: [] } }));
  await page.getByTestId('ide-project-tree').getByText('lsp-mock-a.ts', { exact: true }).dblclick();
  await expect.poll(() => lsp.count('initialize')).toBe(1);
  const token = editor(page, 'console.log(x)').getByText('log', { exact: true });
  await token.click({ modifiers: ['Meta'] });
  await expect(page.getByText(/waiting for the language server/i)).toBeVisible();
  lsp.acceptInitialize!();
  await expect(editor(page, 'export const y').locator('.cm-content')).toContainText('export const y');
  expect(lsp.count('textDocument/definition')).toBe(1);
});

test('a definition outside the project opens read-only on the original target', async ({ page }) => {
  const lsp = await openIde(page);
  await openFile(page, lsp, 'lsp-mock-a.ts');
  const project = new URL(lsp.openedUri('lsp-mock-a.ts')).pathname.replace(/\/[^/]+$/, '');
  const outside = '/tmp/otto-definition-fixture/sdk/library.ts';
  lsp.definitionUri = 'file://' + outside;
  let requestedRoot = '';
  await page.route('**/api/v1/ide/fs/file?**', (route) => {
    const query = new URL(route.request().url()).searchParams;
    if (query.get('path') !== outside) return route.fallback();
    requestedRoot = query.get('root') ?? '';
    return route.fulfill({ json: {
      path: outside, content: 'export const LibraryDefinition = 42;\n',
      language: 'typescript', size: 37, sha256: 'sdk-fixture', read_only: true, truncated: false,
    } });
  });
  await editor(page, 'console.log(x)').getByText('log', { exact: true }).click({ modifiers: ['Meta'] });
  await expect(editor(page, 'LibraryDefinition').locator('.cm-content')).toContainText('LibraryDefinition');
  expect(requestedRoot.replace(/^\/private\/tmp\//, '/tmp/')).toBe(project.replace(/^\/private\/tmp\//, '/tmp/'));
  await expect(page.getByText('Read-only', { exact: true }).first()).toBeVisible();
});
