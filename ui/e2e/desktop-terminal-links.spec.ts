import { test, expect, type Page } from '@playwright/test';
async function fixture(page: Page, text: string | ((cols: number) => string) = 'Read ui/src/App.svelte:2:3\r\n', query = '', anyRenderer = false) {
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async route => {
    const url = new URL(route.request().url()), path = url.searchParams.get('path') ?? '';
    if (url.pathname.endsWith('/fs/read')) {
      if (path.startsWith('/slow')) await new Promise(r => setTimeout(r, 450));
      return route.fulfill({ json: { path, content: `FIRST\nCONTENT ${path}\nTHIRD`, language: 'typescript', truncated: false } });
    }
    if (url.pathname.endsWith('/fs/browse')) {
      if (path === '/work') return route.fulfill({ status: 400, json: { code: 'invalid', message: 'workspace unavailable' } });
      if (path.startsWith('/slow')) await new Promise(r => setTimeout(r, 650));
      return route.fulfill({ json: { path, parent: '/', is_git_repo: false, entries: [{ name: 'sibling.ts', path: `${path}/sibling.ts`, is_dir: false, is_git_repo: false }] } });
    }
    return route.fulfill({ json: [] });
  });
  await page.routeWebSocket(/\/ws\/term\//, socket => {
    let cols = 80;
    socket.onMessage(message => {
      const frame = JSON.parse(String(message));
      if (frame.type === 'resize') cols = frame.cols;
      if (frame.type === 'scrollback') socket.send(JSON.stringify({ type: 'scrollback', data: Buffer.from(typeof text === 'string' ? text : text(cols)).toString('base64') }));
    });
  });
  await page.goto(`/e2e/fixtures/terminal-links.html${query}`);
  if (anyRenderer) await expect.poll(() => page.evaluate(() => (window as unknown as ProbeWindow).__ottoTermProbe.some(p => !p.disposed && p.text().includes('Read')))).toBe(true);
  else await expect(page.locator('.xterm-rows')).toContainText('Read');
}
type ProbeWindow = Window & { __ottoTermProbe: { disposed: boolean; text(): string; renderer(): string }[]; __opened: string[] };
async function clickText(page: Page, text: string) {
  const span = page.locator('.xterm-rows span').filter({ hasText: text }).first();
  let point = { x: NaN, y: NaN };
  // xterm replaces row spans during repaint. A handle resolved as visible can
  // detach before evaluate, making its text Range report an all-zero rect.
  await expect.poll(async () => {
    const measured = await span.evaluate((el, needle) => {
      if (!el.isConnected) return null;
      const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
      let node;
      while ((node = walker.nextNode())) {
        const index = node.textContent?.indexOf(needle) ?? -1;
        if (index < 0) continue;
        const range = document.createRange();
        range.setStart(node, index);
        range.setEnd(node, index + 1);
        const rect = range.getBoundingClientRect();
        const x = rect.x + rect.width / 2, y = rect.y + rect.height / 2;
        if (!node.isConnected || ![x, y, rect.width, rect.height].every(Number.isFinite)
          || rect.width <= 0 || rect.height <= 0
          || x < 0 || y < 0 || x >= window.innerWidth || y >= window.innerHeight) return null;
        return { x, y };
      }
      return null;
    }, text);
    if (measured) point = measured;
    return measured !== null;
  }, { message: `connected, visible terminal text range for ${text}` }).toBe(true);
  await page.mouse.move(point.x, point.y);
  await page.waitForTimeout(80); // Preserve the existing hover interval after geometry readiness.
  await page.mouse.click(point.x, point.y);
}
test('external explicit file is visible despite unavailable workspace, and preserves workspace identity', async ({ page }) => {
  await fixture(page);
  await page.getByRole('button', { name: 'External file' }).click();
  await expect(page.locator('.cm-content')).toContainText('CONTENT /outside/Application Support/report.ts');
  await expect(page.locator('.ft-root-path')).toHaveAttribute('title', '/outside/Application Support');
  await expect(page.getByLabel('Workspace state')).toHaveText('w:/work');
  await page.getByRole('button', { name: 'No workspace' }).click();
  await page.getByRole('button', { name: 'External file' }).click();
  await expect(page.locator('.cm-content')).toContainText('CONTENT /outside/Application Support/report.ts');
});
test('latest explicit file and folder win over older in-flight reads', async ({ page }) => {
  await fixture(page);
  await page.getByRole('button', { name: 'Slow file' }).click();
  await page.getByRole('button', { name: 'Latest file' }).click();
  await expect(page.locator('.cm-content')).toContainText('/fast/latest.ts');
  await page.waitForTimeout(800);
  await expect(page.locator('.cm-content')).toContainText('/fast/latest.ts');
  await expect(page.locator('.ft-root-path')).toHaveAttribute('title', '/fast');
});
test('relative terminal reference and OSC8 full path open through existing Files viewer', async ({ page }) => {
  await fixture(page, 'Read ui/src/App.svelte:2:3\r\n\x1b]8;;file:///outside/Application%20Support/real.ts#L2\x07short.ts\x1b]8;;\x07\r\n/root/git_fetch\r\n');
  await clickText(page, 'ui/src/App.svelte');
  await expect(page.getByLabel('Opened file')).toContainText('"path":"/work/ui/src/App.svelte","line":2,"col":3');
  await clickText(page, 'short.ts');
  await expect(page.getByLabel('Opened file')).toContainText('/outside/Application Support/real.ts');
  await expect(page.locator('.cm-content')).toContainText('/outside/Application Support/real.ts');
  const before = await page.getByLabel('Opened file').textContent();
  await clickText(page, '/root/git_fetch');
  await expect(page.getByLabel('Opened file')).toHaveText(before!);
});
test('share terminal does not dispatch file references using logged-in owner state', async ({ page }) => {
  await fixture(page, 'Read ui/src/App.svelte:2:3\r\n', '?share=1');
  await clickText(page, 'ui/src/App.svelte');
  await expect(page.getByLabel('Opened file')).toHaveText('null');
});

test('wrapped path and HTTP URL clicks use full targets', async ({ page }) => {
  const path = `ui/${'nested/'.repeat(22)}wrap.ts`;
  await fixture(page, `Read ${path}:2\r\nhttps://example.invalid/report?q=yes\r\n`);
  // The browser external-link path uses window.open; intercept its first
  // navigation at context scope so the popup stays entirely inside the fixture.
  await page.context().route('https://example.invalid/**', route => route.fulfill({
    contentType: 'text/html', body: '<title>External URL fixture</title>',
  }));
  await clickText(page, 'wrap.ts');
  await expect(page.getByLabel('Opened file')).toContainText(`/work/${path}`);
  const opened = page.waitForEvent('popup');
  await clickText(page, 'https://example.invalid/');
  const popup = await opened;
  await expect(popup).toHaveURL('https://example.invalid/report?q=yes');
  await popup.close();
});

test('readable file stays visible when its own parent cannot be listed', async ({ page }) => {
  await fixture(page);
  await page.route('**/api/v1/fs/browse?**', route => route.fulfill({ status: 400, json: { code: 'invalid', message: 'parent listing denied by OS' } }));
  await page.getByRole('button', { name: 'External file' }).click();
  await expect(page.locator('.cm-content')).toContainText('/outside/Application Support/report.ts');
  await expect(page.getByTestId('load-error')).toContainText('parent listing denied by OS');
});


test('unquoted Application Support path opens the complete absolute target', async ({ page }) => {
  await page.setViewportSize({ width: 1950, height: 850 });
  const path = '/Users/itziklavon/Library/Application Support/Otto/snips/01M311BBDY6KH1PAE81J9PTTY4.png';
  await fixture(page, `Read ${path}\r\n`, '?wide=1');
  await clickText(page, 'Support/Otto');
  await expect(page.getByLabel('Opened file')).toContainText(`"path":"${path}"`);
});

for (const wide of [true, false]) test(`actual Codex cyan CRLF path opens either fragment (${wide ? 'wide' : 'mixed soft/hard'})`, async ({ page }) => {
  await page.setViewportSize({ width: 1950, height: 850 });
  await fixture(page, 'Read installation status (\x1b[36m/Users/itziklavon/Library/Logs/Otto/uncommitted-20260921-\x1b[0m\r\n  \x1b[36mcorrections/status.md\x1b[0m).\r\n', wide ? '?wide=1' : '');
  const path = '/Users/itziklavon/Library/Logs/Otto/uncommitted-20260921-corrections/status.md';
  await clickText(page, '/Users/itziklavon');
  await expect(page.getByLabel('Opened file')).toContainText(`"path":"${path}"`);
  await clickText(page, 'corrections/status.md');
  await expect(page.getByLabel('Opened file')).toContainText(`"path":"${path}"`);
  await expect(page.getByLabel('Opened file')).toContainText('"n":2');
});

test('Claude bold cursor-addressed continuation opens the complete relative path', async ({ page }) => {
  await page.setViewportSize({ width: 1950, height: 850 });
  const prefix = '../../../private/tmp/claude-501/-Users-itziklavon-games-management';
  const suffix = '/9cb793be-ed41-4083-b30b-4fc9bac2c8d9/scratchpad/ui-design-spec.md';
  await fixture(page, cols => `Read ${' '.repeat(Math.max(0, cols - 5 - 16 - prefix.length))}Referenced file \x1b[1m${prefix}\x1b[2;6H${suffix}\x1b[22m\r\n`, '?wide=1');
  await clickText(page, '../../../private/tmp');
  await expect(page.getByLabel('Opened file')).toContainText(`"path":"/work/${prefix}${suffix}"`);
  await clickText(page, '/9cb793be');
  await expect(page.getByLabel('Opened file')).toContainText(`"path":"/work/${prefix}${suffix}"`);
  await expect(page.getByLabel('Opened file')).toContainText('"n":2');
});

// Real agent TUIs (claude, codex, agy) turn mouse reporting on and Claude
// prints OSC 8 hyperlinks; desktop panes draw on WebGL. A URL must open on a
// plain click, ⌘-click and ⌃-click in every one of those states (⌃-click used
// to be swallowed as "the link menu's", which only file links have).
// Renderer-agnostic: cells are located through the parsed buffer
// (__ottoTermProbe), not the DOM renderer's row spans.
const WRAPPED = `https://example.invalid/wrap/${'w'.repeat(150)}/wrapEND`;
const URL_TEXT = 'Read links\r\n'
  + 'plain https://example.invalid/plain?q=1 done\r\n'
  + '\x1b]8;;https://example.invalid/osc8\x07osc label\x1b]8;;\x07\r\n'
  + `${WRAPPED}\r\n`;
for (const renderer of ['dom', 'webgl'] as const) for (const mode of [0, 1000, 1002, 1003]) {
  test(`URL links open on click, ⌘-click and ⌃-click (${renderer}, mouse reporting ${mode || 'off'})`, async ({ page }) => {
    await page.addInitScript((dom) => {
      // The functional suite pins DOM (global-setup); the app draws on WebGL.
      if (dom) localStorage.setItem('otto.term.renderer', 'dom');
      else localStorage.removeItem('otto.term.renderer');
      const w = window as unknown as ProbeWindow;
      w.__ottoTermProbe = [];
      w.__opened = [];
      window.open = (url?: string | URL) => { w.__opened.push(String(url)); return null; };
    }, renderer === 'dom');
    let cols = 0;
    await fixture(page, c => { cols = c; return (mode ? `\x1b[?${mode}h\x1b[?1006h` : '') + URL_TEXT; }, '', true);
    const active = await page.evaluate(() => (window as unknown as ProbeWindow).__ottoTermProbe.find(p => !p.disposed)!.renderer());
    test.skip(active !== renderer, `${renderer} renderer unavailable in this engine (got ${active})`);
    // The TUI really owns the mouse: xterm hands clicks/drags to the app.
    if (mode) await expect(page.locator('.xterm')).toHaveClass(/enable-mouse-events/);
    const box = (await page.locator('.xterm-screen').boundingBox())!;
    const clickCell = async (needle: string, modifier?: 'Meta' | 'Control') => {
      const at = await page.evaluate((n) => {
        const rows = (window as unknown as ProbeWindow).__ottoTermProbe.find(p => !p.disposed)!.text().split('\n');
        const row = rows.findIndex(r => r.includes(n));
        return { row, col: rows[row]?.indexOf(n) ?? -1, rows: rows.length };
      }, needle);
      expect(at.row, `${needle} on screen`).toBeGreaterThanOrEqual(0);
      const x = box.x + (at.col + 1.5) * (box.width / cols), y = box.y + (at.row + 0.5) * (box.height / at.rows);
      await page.mouse.move(x - 3, y);
      await page.mouse.move(x, y);
      await expect(page.locator('.xterm-screen')).toHaveClass(/xterm-cursor-pointer/); // the hover found the link
      if (modifier) await page.keyboard.down(modifier);
      await page.mouse.click(x, y);
      if (modifier) await page.keyboard.up(modifier);
    };
    const expected: string[] = [];
    for (const modifier of [undefined, 'Meta', 'Control'] as const) {
      for (const [needle, url] of [
        ['example.invalid/plain', 'https://example.invalid/plain?q=1'],
        ['osc label', 'https://example.invalid/osc8'],
        ['wrapEND', WRAPPED],
      ]) {
        await clickCell(needle, modifier);
        expected.push(url);
        await expect.poll(() => page.evaluate(() => (window as unknown as ProbeWindow).__opened), { message: `${needle} ${modifier ?? 'click'}` }).toEqual(expected);
      }
    }
  });
}
