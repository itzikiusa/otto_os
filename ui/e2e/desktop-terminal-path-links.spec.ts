import { test, expect, type Page } from '@playwright/test';

// Printed relative paths resolve against the session's folders IN ORDER and
// open the first one that exists (lib/pathLinks). The session started in
// /work/a; the workspace's second repo is /work/b. A path that exists only in
// /work/b used to be joined onto /work/a and open a viewer for nothing.
const FILES = new Set(['/work/b/src/only-in-b.ts']);
const DIRS = new Set(['/work', '/work/a', '/work/b', '/work/b/marketing/videos/tour']);

async function fixture(page: Page, text: string) {
  const statted: string[] = [];
  await page.addInitScript(() => { localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture'); });
  await page.route('**/api/v1/**', async route => {
    const url = new URL(route.request().url()), path = url.searchParams.get('path') ?? '';
    if (url.pathname.endsWith('/fs/stat')) {
      statted.push(path);
      if (FILES.has(path)) return route.fulfill({ json: { path, is_dir: false, is_git_repo: false } });
      if (DIRS.has(path)) return route.fulfill({ json: { path, is_dir: true, is_git_repo: false } });
      return route.fulfill({ status: 404, json: { code: 'not_found', message: `no such path: ${path}` } });
    }
    if (url.pathname.endsWith('/workspaces/w/repos')) {
      return route.fulfill({ json: ['/work/a', '/work/b'].map((p, i) => ({ id: `r${i}`, workspace_id: 'w', name: p.split('/').at(-1), path: p, remote_url: null, provider: null, git_account_id: null, created_at: '2026-11-13T00:00:00Z' })) });
    }
    if (url.pathname.endsWith('/fs/read')) {
      if (!FILES.has(path)) return route.fulfill({ status: 404, json: { code: 'not_found', message: 'no such file' } });
      return route.fulfill({ json: { path, content: `CONTENT ${path}`, language: 'typescript', truncated: false } });
    }
    if (url.pathname.endsWith('/fs/browse')) {
      const entries = path === '/work/b/marketing/videos/tour' ? [{ name: 'scene-1.mp4', path: `${path}/scene-1.mp4`, is_dir: false, is_git_repo: false }] : [];
      return route.fulfill({ json: { path, parent: path.split('/').slice(0, -1).join('/') || '/', is_git_repo: false, entries } });
    }
    return route.fulfill({ json: [] });
  });
  await page.routeWebSocket(/\/ws\/term\//, socket => {
    socket.onMessage(message => {
      const frame = JSON.parse(String(message));
      if (frame.type === 'scrollback') socket.send(JSON.stringify({ type: 'scrollback', data: Buffer.from(text).toString('base64') }));
    });
  });
  await page.goto('/e2e/fixtures/path-links.html');
  await expect(page.locator('.xterm-rows')).toContainText('Read');
  return statted;
}

/** Point at the first character of `text` in the terminal and click it. */
async function clickText(page: Page, text: string, button: 'left' | 'right' = 'left') {
  const span = page.locator('.xterm-rows span').filter({ hasText: text }).first();
  let point = { x: NaN, y: NaN };
  // xterm replaces row spans during repaint: measure until the range is live.
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
        if (rect.width <= 0 || rect.height <= 0 || x < 0 || y < 0 || x >= window.innerWidth || y >= window.innerHeight) return null;
        return { x, y };
      }
      return null;
    }, text);
    if (measured) point = measured;
    return measured !== null;
  }, { message: `visible terminal text range for ${text}` }).toBe(true);
  await page.mouse.move(point.x, point.y);
  // xterm resolves the link under the pointer on mouse move; wait until it is
  // underlined as a link before clicking (no fixed sleep).
  await expect(page.locator('.xterm-screen')).toHaveClass(/xterm-cursor-pointer/);
  await page.mouse.click(point.x, point.y, { button });
}

test('a relative path opens from the folder that has it, not the session start folder', async ({ page }) => {
  const statted = await fixture(page, 'Read src/only-in-b.ts:4:2 now\r\n');
  await clickText(page, 'src/only-in-b.ts');
  await expect(page.getByLabel('Opened file')).toContainText('"path":"/work/b/src/only-in-b.ts","line":4,"col":2');
  await expect(page.locator('.cm-content')).toContainText('CONTENT /work/b/src/only-in-b.ts');
  // The start folder was asked first, and never opened.
  expect(statted[0]).toBe('/work/a/src/only-in-b.ts');
  await expect(page.locator('.cm-content')).not.toContainText('/work/a/');
});

test('a folder path opens the side panel at that folder, with no viewer', async ({ page }) => {
  await fixture(page, 'Read the clips in marketing/videos/tour/ for v0.2.2\r\n');
  await clickText(page, 'marketing/videos/tour/');
  await expect(page.getByLabel('Opened file')).toContainText('"path":"/work/b/marketing/videos/tour","dir":true');
  await expect(page.locator('.ft-root-path')).toHaveAttribute('title', '/work/b/marketing/videos/tour');
  await expect(page.getByText('scene-1.mp4')).toBeVisible();
  await expect(page.locator('.cm-content')).toHaveCount(0);
});

test('right-click on a folder link offers the side panel; Finder is hidden outside the desktop app', async ({ page }) => {
  await fixture(page, 'Read the clips in marketing/videos/tour/ now\r\n');
  await clickText(page, 'marketing/videos/tour/', 'right');
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitem', { name: 'Open in side panel' })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Copy path' })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Show in Finder' })).toHaveCount(0);
  // The right-click itself opened nothing.
  await expect(page.getByLabel('Opened file')).toHaveText('null');
  await menu.getByRole('menuitem', { name: 'Open in side panel' }).click();
  await expect(page.locator('.ft-root-path')).toHaveAttribute('title', '/work/b/marketing/videos/tour');
});

test('a path that exists nowhere shows a "not found" toast and opens no viewer', async ({ page }) => {
  await fixture(page, 'Read gone/nowhere.ts please\r\n');
  await clickText(page, 'gone/nowhere.ts');
  const toast = page.locator('.toast').filter({ hasText: 'Not found: gone/nowhere.ts' });
  await expect(toast).toBeVisible();
  await expect(toast).toContainText('Looked in 3 places');
  await expect(toast.getByRole('button', { name: 'Copy path' })).toBeVisible();
  await expect(page.getByLabel('Opened file')).toHaveText('null');
  await expect(page.locator('.cm-content')).toHaveCount(0);
});
