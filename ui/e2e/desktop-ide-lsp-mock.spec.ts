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
        this.reply(c, m.id!, { capabilities: { textDocumentSync: 1, referencesProvider: true, codeActionProvider: { codeActionKinds: ['quickfix'] } } });
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
