import { test, expect, type Page, type Locator, type APIRequestContext } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, platform, arch, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { apiCtx } from './seed';
import {
  editorSelectAll,
  expectAccessible,
  expectContentHasHeight,
  expectFullyInViewport,
  expectNoHorizontalOverflow,
} from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Otto Full IDE — Real-Browser End-to-End, Usability, A11y & Visual Suite
//
// Exercises every screen & capability against the isolated throwaway `ottod`:
//   1. Sidebar navigation to `#/ide` + auto-loading seeded polyglot workspace
//      (Go, Python, TypeScript/JS, Rust, Java).
//   2. File tree browse, create file, edit, `⌘S` save, optimistic `expected_sha256`
//      conflict safety, Structure symbol jump, and `⇧⇧` Search Everywhere
//      (Files · Symbols · Text + multi-file Replace preview).
//   3. Multi-line selection & simultaneous before/after (prefix + suffix) line
//      editing (both multi-cursor simultaneous typing AND explicit Prefix/Suffix
//      bar), asserting exact resulting source text on disk & in CodeMirror.
//   4. Gutter Run & Debug detection across all 5 languages (Go `TestAdd`,
//      Python `test_calc`, JS `test(...)`, Rust `#[test]`, Java `@Test`) +
//      executing a real test via the Run panel with structured pass result.
//   5. Live DAP Debugging (`dlv dap` / Node inspector): line + conditional
//      breakpoints, paused execution line highlight + inline ghost variable
//      values, Call Stack frames, Variables tree, Step Over, and Evaluate REPL.
//   6. Toolchains & SDK Downloader modal: host OS & CPU arch badge + all 5
//      ecosystems (`Java`, `Go`, `Rust`, `Node`, `Python`) installed &
//      arch-matched downloadable catalogs.
//   7. Environments & Run Configurations modal: project env vars, Keychain/file
//      secret env var masking (`has_secret`), saved Run Config persistence.
//   8. Remote SSH Target modal: saved SSH profiles from `/connections` & target
//      switching UI.
//   9. Share with Friends (2 isolated Browser Contexts): Host creates room,
//      Friend joins via capability-token link with zero owner credentials,
//      live peer presence, multi-cursor sync, Viewer -> Editor role toggle,
//      Follow mode, and instant Revoke.
//  10. Runtime IDE Plugins modal: zero-rebuild install of
//      `examples/ide-plugins/kotlin-ide`, immediate `.kt` language & gutter
//      test activation, and live Disable / Enable toggling.
//  11. Visual, Responsive & Accessibility sweep across Desktop Dark, Desktop
//      Light, Desktop RTL (`dir="rtl"`), iPad Portrait/Landscape & iPhone
//      Portrait (`expectNoHorizontalOverflow`, `expectFullyInViewport`, axe).
// ─────────────────────────────────────────────────────────────────────────────

const SCREENSHOT_DIR =
  '/Users/itziklavon/.gemini/antigravity-cli/brain/4814dd52-6e20-428a-88b9-51174ddd8f0e/scratch/screenshots';
const KOTLIN_PLUGIN_DIR = resolve(process.cwd(), '..', 'examples', 'ide-plugins', 'kotlin-ide');

interface IdeTargetSpec {
  kind: 'local' | 'ssh';
  workspace_id?: string | null;
  root: string;
  connection_id?: string | null;
}

interface IdeGutterResp {
  items: Array<{
    line: number;
    kind: string;
    label: string;
    language: string;
  }>;
}

interface IdeDebugSnapshot {
  id: string;
  state: string;
  paused_file?: string | null;
  paused_line?: number | null;
  active_thread_id?: number | null;
  active_frame_id?: number | null;
  stack_frames?: Array<{ id: number; name: string; path?: string | null; line: number }>;
  scopes?: Array<{
    name: string;
    variables_reference: number;
    variables?: Array<{ name: string; value: string }>;
  }>;
  inline_values?: Array<{ line: number; name: string; value: string }>;
}

let base = '';
let token = '';
let workspaceId = '';
let workspaceRoot = '';
let sshConnId = '';

function seedPolyglotDirectory(dir: string): void {
  mkdirSync(join(dir, 'calc'), { recursive: true });
  mkdirSync(join(dir, 'cmd', 'demo'), { recursive: true });
  mkdirSync(join(dir, 'pyapp'), { recursive: true });
  mkdirSync(join(dir, 'web'), { recursive: true });
  mkdirSync(join(dir, 'rustlib', 'src'), { recursive: true });
  mkdirSync(join(dir, 'java', 'src', 'main', 'java', 'com', 'otto'), { recursive: true });
  mkdirSync(join(dir, 'java', 'src', 'test', 'java', 'com', 'otto'), { recursive: true });

  // 1. Go
  writeFileSync(join(dir, 'go.mod'), 'module example.com/ottoide\n\ngo 1.21\n');
  writeFileSync(
    join(dir, 'calc', 'calc.go'),
    [
      'package calc',
      '',
      'func Add(a, b int) int {',
      '\tsum := a + b',
      '\treturn sum',
      '}',
      '',
      'func Multiply(a, b int) int {',
      '\tprod := a * b',
      '\treturn prod',
      '}',
      '',
    ].join('\n'),
  );
  writeFileSync(
    join(dir, 'calc', 'calc_test.go'),
    [
      'package calc',
      '',
      'import "testing"',
      '',
      'func TestAdd(t *testing.T) {',
      '\tx := 20',
      '\ty := 22',
      '\tgot := Add(x, y)',
      '\tif got != 42 {',
      '\t\tt.Fatalf("got %d, want 42", got)',
      '\t}',
      '}',
      '',
    ].join('\n'),
  );
  writeFileSync(
    join(dir, 'cmd', 'demo', 'main.go'),
    [
      'package main',
      '',
      'import (',
      '\t"fmt"',
      '\t"example.com/ottoide/calc"',
      ')',
      '',
      'func main() {',
      '\tfmt.Println("Total:", calc.Add(20, 22))',
      '}',
      '',
    ].join('\n'),
  );

  // 2. Python
  writeFileSync(join(dir, 'pyapp', '__init__.py'), '');
  writeFileSync(
    join(dir, 'pyapp', 'calc.py'),
    [
      'def compute_total(items: list[int], bonus: int = 2) -> int:',
      '    subtotal = sum(items)',
      '    total = subtotal + bonus',
      '    return total',
      '',
      '',
      'if __name__ == "__main__":',
      '    print(compute_total([20, 20], bonus=2))',
      '',
    ].join('\n'),
  );
  writeFileSync(
    join(dir, 'pyapp', 'test_calc.py'),
    [
      'import unittest',
      'from pyapp.calc import compute_total',
      '',
      '',
      'def test_calc():',
      '    assert compute_total([20, 20], bonus=2) == 42',
      '',
      '',
      'class TestCalcSuite(unittest.TestCase):',
      '    def test_suite_sum(self):',
      '        self.assertEqual(compute_total([19, 21], bonus=2), 42)',
      '',
      '',
      'if __name__ == "__main__":',
      '    unittest.main()',
      '',
    ].join('\n'),
  );

  // 3. TypeScript & JavaScript (Node test runner)
  writeFileSync(
    join(dir, 'package.json'),
    JSON.stringify(
      {
        name: 'otto-ide-polyglot',
        version: '1.0.0',
        type: 'module',
        scripts: {
          test: 'node --test web/math.test.mjs',
        },
      },
      null,
      2,
    ) + '\n',
  );
  writeFileSync(
    join(dir, 'web', 'math.ts'),
    [
      'export interface Vector2D {',
      '  x: number;',
      '  y: number;',
      '}',
      '',
      'export function dotProduct(a: Vector2D, b: Vector2D): number {',
      '  return a.x * b.x + a.y * b.y;',
      '}',
      '',
    ].join('\n'),
  );
  writeFileSync(
    join(dir, 'web', 'math.test.mjs'),
    [
      "import test from 'node:test';",
      "import assert from 'node:assert/strict';",
      '',
      "test('computes arithmetic sum cleanly', () => {",
      '  const value = 20 + 22;',
      '  assert.equal(value, 42);',
      '});',
      '',
    ].join('\n'),
  );

  // 4. Rust
  writeFileSync(
    join(dir, 'rustlib', 'Cargo.toml'),
    ['[package]', 'name = "rustlib"', 'version = "0.1.0"', 'edition = "2021"', ''].join('\n'),
  );
  writeFileSync(
    join(dir, 'rustlib', 'src', 'lib.rs'),
    [
      'pub fn fibonacci(n: u32) -> u32 {',
      '    match n {',
      '        0 => 0,',
      '        1 => 1,',
      '        _ => fibonacci(n - 1) + fibonacci(n - 2),',
      '    }',
      '}',
      '',
      '#[cfg(test)]',
      'mod tests {',
      '    use super::*;',
      '',
      '    #[test]',
      '    fn test_fibonacci() {',
      '        assert_eq!(fibonacci(8), 21);',
      '    }',
      '}',
      '',
    ].join('\n'),
  );

  // 5. Java
  writeFileSync(
    join(dir, 'java', 'src', 'main', 'java', 'com', 'otto', 'App.java'),
    [
      'package com.otto;',
      '',
      'public class App {',
      '    public static int greetCount(String name) {',
      '        return name.length();',
      '    }',
      '',
      '    public static void main(String[] args) {',
      '        System.out.println("Len=" + greetCount("Otto"));',
      '    }',
      '}',
      '',
    ].join('\n'),
  );
  writeFileSync(
    join(dir, 'java', 'src', 'test', 'java', 'com', 'otto', 'AppTest.java'),
    [
      'package com.otto;',
      '',
      'import org.junit.jupiter.api.Test;',
      '',
      'public class AppTest {',
      '    @Test',
      '    void verifiesGreetingLength() {',
      '        assert App.greetCount("Otto") == 4;',
      '    }',
      '}',
      '',
    ].join('\n'),
  );

  // Multi-line editing fixture & dotenv fixture
  writeFileSync(join(dir, 'multiline_target.txt'), ['alpha', 'beta', 'gamma', 'delta', ''].join('\n'));
  writeFileSync(
    join(dir, '.env.example'),
    ['API_URL=http://127.0.0.1:8080', 'FEATURE_FLAG=enabled', ''].join('\n'),
  );

  // Initialize git with dirty + untracked states so tree Git badges render
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'e2e@otto.local');
  git('config', 'user.name', 'Otto E2E');
  git('config', 'commit.gpgsign', 'false');
  git('add', '-A');
  git('commit', '-q', '-m', 'seed polyglot workspace');

  // Modify one tracked file and add one untracked file
  writeFileSync(
    join(dir, 'cmd', 'demo', 'main.go'),
    [
      'package main',
      '',
      'import (',
      '\t"fmt"',
      '\t"example.com/ottoide/calc"',
      ')',
      '',
      'func main() {',
      '\tfmt.Println("Total:", calc.Add(20, 22), calc.Multiply(6, 7))',
      '}',
      '',
    ].join('\n'),
  );
  writeFileSync(join(dir, 'scratch_notes.md'), '# Scratch Notes\n\nUntracked workspace note.\n');
}

async function ensureSeeded(): Promise<void> {
  if (workspaceId) return;
  mkdirSync(SCREENSHOT_DIR, { recursive: true });
  const c = await apiCtx();
  base = c.base;
  token = c.token;

  // Reuse an already-seeded 'Otto Polyglot IDE' workspace across Playwright projects
  // so `ws.workspaces[0]` and `localTarget()` always share one canonical root.
  const listResp = await c.ctx.get(`${base}/api/v1/workspaces`);
  if (listResp.ok()) {
    const existingList = (await listResp.json()) as Array<{
      id: string;
      name: string;
      root_path?: string | null;
    }>;
    const found = existingList.find(
      (w) => w.name === 'Otto Polyglot IDE' && w.root_path && existsSync(w.root_path),
    );
    if (found && found.root_path) {
      workspaceId = found.id;
      workspaceRoot = realpathSync(found.root_path);
      const connsResp = await c.ctx.get(`${base}/api/v1/workspaces/${workspaceId}/connections`);
      if (connsResp.ok()) {
        const conns = (await connsResp.json()) as Array<{ id: string; name: string; kind: string }>;
        sshConnId = conns.find((cn) => cn.name === 'staging-linux-arm64')?.id ?? '';
      }
      if (!sshConnId) {
        const sshCreate = await c.ctx.post(`${base}/api/v1/workspaces/${workspaceId}/connections`, {
          data: {
            name: 'staging-linux-arm64',
            kind: 'ssh',
            params: { host: '10.0.14.22', port: 22, user: 'deploy' },
            secret: null,
            environment: 'dev',
            read_only: false,
          },
        });
        if (sshCreate.ok()) sshConnId = ((await sshCreate.json()) as { id: string }).id;
      }
      await c.ctx.dispose();
      return;
    }
  }

  workspaceRoot = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-polyglot-')));
  seedPolyglotDirectory(workspaceRoot);

  const wsResp = await c.ctx.post(`${base}/api/v1/workspaces`, {
    data: { name: 'Otto Polyglot IDE', root_path: workspaceRoot },
  });
  if (!wsResp.ok()) {
    throw new Error(`Failed to seed IDE workspace: ${wsResp.status()} ${await wsResp.text()}`);
  }
  workspaceId = ((await wsResp.json()) as { id: string }).id;

  const sshResp = await c.ctx.post(`${base}/api/v1/workspaces/${workspaceId}/connections`, {
    data: {
      name: 'staging-linux-arm64',
      kind: 'ssh',
      params: { host: '10.0.14.22', port: 22, user: 'deploy' },
      secret: null,
      environment: 'dev',
      read_only: false,
    },
  });
  if (sshResp.ok()) {
    sshConnId = ((await sshResp.json()) as { id: string }).id;
  }

  // Register workspaces against the IDE git repo as well so Git integration knows it
  await c.ctx
    .post(`${base}/api/v1/workspaces/${workspaceId}/repos`, {
      data: { path: workspaceRoot, name: 'otto-ide-polyglot' },
    })
    .catch(() => {});

  await c.ctx.dispose();
}

function localTarget(): IdeTargetSpec {
  return {
    kind: 'local',
    workspace_id: workspaceId,
    root: workspaceRoot,
    connection_id: null,
  };
}

async function daemon<T>(
  ctx: APIRequestContext,
  method: 'GET' | 'POST' | 'PUT' | 'DELETE',
  path: string,
  data?: unknown,
): Promise<T> {
  const r = await ctx.fetch(`${base}/api/v1${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    data: data === undefined ? undefined : JSON.stringify(data),
  });
  const txt = await r.text();
  expect(r.ok(), `${method} ${path} -> ${r.status()} ${txt}`).toBeTruthy();
  return (txt ? JSON.parse(txt) : undefined) as T;
}

async function saveShot(page: Page, name: string): Promise<void> {
  mkdirSync(SCREENSHOT_DIR, { recursive: true });
  await page.screenshot({ path: join(SCREENSHOT_DIR, name), fullPage: false });
}

async function bootIdePage(
  page: Page,
  opts: { scheme?: 'dark' | 'light'; direction?: 'ltr' | 'rtl'; startHash?: string } = {},
): Promise<void> {
  const scheme = opts.scheme ?? 'dark';
  const direction = opts.direction ?? 'ltr';
  await page.addInitScript(
    ({ wsId, sc, dir }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      if (!sessionStorage.getItem('__otto_ide_e2e_init')) {
        sessionStorage.setItem('__otto_ide_e2e_init', '1');
        localStorage.setItem('otto_scheme', sc);
        localStorage.setItem('otto_direction', dir);
      }
    },
    { wsId: workspaceId, sc: scheme, dir: direction },
  );
  await page.goto(`/#/${opts.startHash ?? 'ide'}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
}

/** Click an action in the IDE PageHeader, seamlessly handling `⋯` overflow collapse. */
async function clickHeaderAction(page: Page, labelPattern: RegExp): Promise<void> {
  const header = page.getByTestId('page-header');
  await expect(header).toBeVisible();

  const directBtn = header
    .locator('button:not([data-ph-hidden]):not([data-testid="ide-more-btn"])')
    .filter({ hasText: labelPattern })
    .or(
      header.locator(
        `button:not([data-ph-hidden]):not([data-testid="ide-more-btn"])[aria-label], button:not([data-ph-hidden]):not([data-testid="ide-more-btn"])[title]`,
      ),
    );

  const count = await directBtn.count();
  for (let i = 0; i < count; i++) {
    const candidate = directBtn.nth(i);
    const aria = (await candidate.getAttribute('aria-label')) ?? '';
    const title = (await candidate.getAttribute('title')) ?? '';
    const text = ((await candidate.textContent()) ?? '').trim();
    if (
      (labelPattern.test(aria) || labelPattern.test(title) || labelPattern.test(text)) &&
      (await candidate.isVisible())
    ) {
      const box = await candidate.boundingBox();
      if (box && box.width > 0 && box.height > 0) {
        await candidate.click();
        return;
      }
    }
  }

  // Otherwise open IDE's `⋯` overflow menu (`data-testid="ide-more-btn"` or `More actions`)
  const moreBtn = page
    .getByTestId('ide-more-btn')
    .or(header.getByRole('button', { name: 'More actions' }))
    .first();
  await expect(moreBtn).toBeVisible();
  await moreBtn.click();
  const menu = page.locator('.ctx-menu');
  await expectFullyInViewport(page, menu, 'IDE header overflow menu');
  await menu.getByRole('menuitem', { name: labelPattern }).first().click();
}

/** Expand directory segments in the IDE project tree and open a file tab. */
async function openFileFromTree(page: Page, ...segments: string[]): Promise<void> {
  const tree = page.getByTestId('ide-project-tree');
  await expect(tree).toBeVisible({ timeout: 15_000 });

  const filesTab = tree.getByRole('tab', { name: /^Files$/i });
  if ((await filesTab.count()) > 0 && (await filesTab.first().isVisible())) {
    if ((await filesTab.first().getAttribute('aria-selected')) !== 'true') {
      await filesTab.first().click();
    }
  }

  for (let i = 0; i < segments.length; i++) {
    const relPath = segments.slice(0, i + 1).join('/');
    const isLast = i === segments.length - 1;
    const item = tree
      .locator(`button.tree-item[title="${relPath}"], button.tree-item[title$="/${relPath}"]`)
      .first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    if (isLast) {
      await item.dblclick();
    } else {
      const expanded = await item.getAttribute('aria-expanded');
      if (expanded !== 'true') {
        await item.click();
      }
    }
  }
}

function activeEditorContent(page: Page): Locator {
  return page.locator('.cm-editor .cm-content').first();
}

// ─────────────────────────────────────────────────────────────────────────────
// SUITE 1: Desktop Full IDE End-to-End Verification (Chromium 1440×900 Retina)
// ─────────────────────────────────────────────────────────────────────────────

test.describe('Desktop IDE — Full End-to-End Workflows', () => {
  test.use({
    viewport: { width: 1440, height: 900 },
    isMobile: false,
    hasTouch: false,
    deviceScaleFactor: 2,
    userAgent:
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
  });

  test.describe.configure({ mode: 'serial' });

  test.beforeAll(async () => {
    await ensureSeeded();
  });

  // Run the heavy desktop workflow suite once on Chromium (`android-phone`
  // overridden to Desktop Chrome above, or `desktop-browser` if invoked directly).
  test.beforeEach(async ({}, info) => {
    test.skip(
      !['android-phone', 'desktop-browser'].includes(info.project.name),
      'Desktop IDE workflow suite runs once on Chromium desktop viewport',
    );
  });

  test('1. sidebar navigation to #/ide, polyglot tree, create/edit/⌘S save, 409 conflict guard, Structure jump & ⇧⇧ Search Everywhere + replace preview', async ({
    page,
  }) => {
    test.setTimeout(90_000);
    await bootIdePage(page, { startHash: 'home' });

    // Navigate via Sidebar Rail to `#/ide`
    const railIde = page.getByTestId('rail-ide').or(page.locator('[data-nav-id="ide"]')).first();
    await expect(railIde).toBeVisible({ timeout: 15_000 });
    await railIde.click();
    await expect(page).toHaveURL(/#\/ide/);

    // Single unified 46px PageHeader titled IDE
    const header = page.getByTestId('page-header');
    await expect(header).toBeVisible();
    await expect(header.locator('.ph-title')).toContainText('IDE');
    await expectNoHorizontalOverflow(page);
    await expectContentHasHeight(page, 400);

    // Seeded polyglot workspace folders auto-load in the tree
    for (const entry of ['calc', 'pyapp', 'web', 'rustlib', 'java']) {
      await expect(page.getByText(entry, { exact: true }).first()).toBeVisible({ timeout: 15_000 });
    }

    // Open Go source file `calc/calc.go`
    await openFileFromTree(page, 'calc', 'calc.go');
    const editor = activeEditorContent(page);
    await expect(editor).toBeVisible();
    await expect(editor).toContainText('func Add(a, b int) int');
    await saveShot(page, '01-ide-workbench-dark.png');

    // Create a new file `calc/subtract.go` via the Project Explorer `+` ("New file") UI, edit in CodeMirror, and save with ⌘S
    const newFileSub = [
      'package calc',
      '',
      'func Subtract(a, b int) int {',
      '\treturn a - b',
      '}',
      '',
    ].join('\n');

    await page.getByTestId('ide-project-tree').getByRole('button', { name: 'New file' }).click();
    const promptDialog = page.locator('.sheet[role="dialog"]').last();
    await expectFullyInViewport(page, promptDialog, 'New file prompt dialog');
    await promptDialog.locator('input').first().fill('calc/subtract.go');
    await promptDialog.getByRole('button', { name: /^Create$/i }).click();
    await expect(promptDialog).toBeHidden();

    await openFileFromTree(page, 'calc', 'subtract.go');
    await expect(editor).toBeVisible();
    await editor.click();
    const selectAll = await editorSelectAll(page);
    await page.keyboard.press(selectAll);
    await page.keyboard.insertText(newFileSub);

    const saveResp = page.waitForResponse(
      (r) => r.request().method() === 'PUT' && r.url().includes('/api/v1/ide/fs/file'),
      { timeout: 15_000 },
    );
    await page.keyboard.press('Meta+s');
    const saved = await saveResp;
    expect(saved.ok()).toBeTruthy();
    expect(readFileSync(join(workspaceRoot, 'calc', 'subtract.go'), 'utf8')).toBe(newFileSub);

    // Verify optimistic `expected_sha256` conflict guard returns 409 Conflict on stale hash
    const conflictCheck = await page.request.fetch(`${base}/api/v1/ide/fs/file`, {
      method: 'PUT',
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      data: JSON.stringify({
        target: localTarget(),
        path: join(workspaceRoot, 'calc', 'subtract.go'),
        content: 'package calc\n// stale overwrite\n',
        expected_sha256: '0000000000000000000000000000000000000000000000000000000000000000',
      }),
    });
    expect(conflictCheck.status()).toBe(409);

    // Structure outline jump on `calc/calc.go`
    await openFileFromTree(page, 'calc', 'calc.go');
    await expect(editor).toContainText('func Multiply');
    const structureTab = page.getByRole('tab', { name: /Structure/i }).first();
    await expect(structureTab).toBeVisible();
    await structureTab.click();
    const multiplySym = page.getByText('Multiply').first();
    await expect(multiplySym).toBeVisible({ timeout: 10_000 });
    await multiplySym.click();

    // Switch back to Files tab
    await page.getByRole('tab', { name: /^Files$/i }).first().click();

    // `⇧⇧` Double-Shift opens Search Everywhere modal directly
    await page.keyboard.press('Shift');
    await page.keyboard.press('Shift');
    const searchDialog = page.locator('.sheet[role="dialog"]').last();
    await expect(searchDialog).toBeVisible({ timeout: 5_000 });
    await expectFullyInViewport(page, searchDialog, 'Search Everywhere modal');

    // Search files (`calc_test.go`), symbols (`compute_total`), and text (`fibonacci`)
    const searchInput = searchDialog.locator('input[type="text"], input[type="search"], input:not([type="checkbox"])').first();
    await expect(searchInput).toBeVisible();

    await searchInput.fill('calc_test.go');
    await expect(searchDialog).toContainText('calc_test.go', { timeout: 10_000 });

    const symbolsTab = searchDialog.getByRole('tab', { name: /Symbols|Classes/i }).first();
    if ((await symbolsTab.count()) > 0 && (await symbolsTab.isVisible())) {
      await symbolsTab.click();
    }
    await searchInput.fill('compute_total');
    await expect(searchDialog).toContainText('compute_total', { timeout: 10_000 });

    const textTab = searchDialog.getByRole('tab', { name: /Find in path|Text/i }).first();
    if ((await textTab.count()) > 0 && (await textTab.isVisible())) {
      await textTab.click();
    }
    await searchInput.fill('FEATURE_FLAG=enabled');
    await expect(searchDialog).toContainText('.env.example', { timeout: 10_000 });

    // Exercise Replace preview (via UI toggle in Search Everywhere + dry-run contract)
    const replaceToggle = searchDialog.getByTestId('ide-se-replace-toggle');
    await expect(replaceToggle).toBeVisible();
    await replaceToggle.click();
    const replaceInput = searchDialog.getByTestId('ide-replace-input');
    await expect(replaceInput).toBeVisible();
    await replaceInput.fill('FEATURE_FLAG=turbo');
    const previewBtn = searchDialog.getByRole('button', { name: /^Preview$/i }).first();
    await expect(previewBtn).toBeVisible();
    await previewBtn.click();
    await expect(searchDialog).toContainText('FEATURE_FLAG=turbo', { timeout: 10_000 });
    // Also verify backend dry-run replace preview leaves file untouched until applied
    const replacePreview = await daemon<{
      files_changed: number;
      replacements: number;
      previews: Array<{ path: string; sample_after: string }>;
    }>(page.request, 'POST', '/ide/replace', {
      target: localTarget(),
      query: 'FEATURE_FLAG=enabled',
      replacement: 'FEATURE_FLAG=turbo',
      dry_run: true,
    });
    expect(replacePreview.replacements).toBeGreaterThanOrEqual(1);
    expect(readFileSync(join(workspaceRoot, '.env.example'), 'utf8')).toContain(
      'FEATURE_FLAG=enabled',
    );

    await saveShot(page, '02-ide-search-everywhere.png');
    await page.keyboard.press('Escape');
    await expect(searchDialog).toBeHidden();
  });

  test('2. multi-line selection & simultaneous before/after (prefix + suffix) line editing', async ({
    page,
  }) => {
    test.setTimeout(60_000);
    writeFileSync(
      join(workspaceRoot, 'multiline_target.txt'),
      ['alpha', 'beta', 'gamma', 'delta'].join('\n'),
    );

    await bootIdePage(page);
    await openFileFromTree(page, 'multiline_target.txt');
    const editor = activeEditorContent(page);
    await expect(editor).toBeVisible();
    await expect(editor).toContainText('alpha');

    // Part A1: Multi-cursor selection across lines 1..4 (`⌥⇧I`) + simultaneous before & after typing
    await editor.click();
    const selectAll = await editorSelectAll(page);
    await page.keyboard.press(selectAll);
    // Split multi-line selection into one cursor per line (`⌥⇧I`)
    await page.keyboard.press('Alt+Shift+i');
    await page.keyboard.press('ArrowLeft');
    await page.keyboard.insertText('item_');
    await page.keyboard.press('End');
    await page.keyboard.insertText(';');

    await expect
      .poll(async () => ((await editor.textContent()) ?? '').replace(/\u200b/g, ''))
      .toContain('item_alpha;');
    await expect(editor).toContainText('item_beta;');
    await expect(editor).toContainText('item_gamma;');
    await expect(editor).toContainText('item_delta;');

    // Part A2: Option (`Alt`) + vertical mouse drag rectangular selection across lines 1..4
    await editor.click();
    await page.keyboard.press('Escape');
    await page.keyboard.press(selectAll);
    await page.keyboard.insertText('alpha\nbeta\ngamma\ndelta');
    await page.keyboard.press('Escape');

    const cmLines = page.locator('.cm-editor .cm-content .cm-line');
    await expect(cmLines).toHaveCount(4);
    const line1Box = await cmLines.nth(0).boundingBox();
    const line4Box = await cmLines.nth(3).boundingBox();
    expect(line1Box && line4Box).toBeTruthy();

    const dragX = line1Box!.x + 2;
    const startY = line1Box!.y + line1Box!.height / 2;
    const endY = line4Box!.y + line4Box!.height / 2;
    // Place initial caret at line 1 col 1, then Option (`Alt`) + vertical drag across lines 1..4
    await page.mouse.click(dragX, startY);
    await page.keyboard.down('Alt');
    await page.mouse.down();
    await page.mouse.move(dragX, endY, { steps: 10 });
    await page.mouse.up();
    await page.keyboard.up('Alt');

    // Verify rectangular drag created 4 live cursors and types before & after every line simultaneously!
    await expect(page.getByText('4 cursors')).toBeVisible({ timeout: 5_000 });
    await page.keyboard.insertText('col_');
    await page.keyboard.press('End');
    await page.keyboard.insertText('!');

    await expect
      .poll(async () => ((await editor.textContent()) ?? '').replace(/\u200b/g, ''))
      .toContain('col_alpha!');
    await expect(editor).toContainText('col_beta!');
    await expect(editor).toContainText('col_gamma!');
    await expect(editor).toContainText('col_delta!');

    // Part B: Explicit Before / After (Prefix + Suffix) multi-line bar on selected lines
    await editor.click();
    await page.keyboard.press('Escape');
    await page.keyboard.press(selectAll);
    await page.keyboard.insertText('one\ntwo\nthree');
    await page.keyboard.press(selectAll);

    // Reveal the Multi-Line Prefix / Suffix bar if behind a toolbar toggle
    const prefixInput = page.getByTestId('ide-prefix-input');
    if (!(await prefixInput.isVisible())) {
      const multiBtn = page
        .getByTestId('ide-prefix-suffix-toggle')
        .or(page.getByRole('button', { name: /Prefix \/ suffix|Prefix|Multi-Line/i }))
        .first();
      await expect(multiBtn).toBeVisible();
      await multiBtn.click();
    }
    const suffixInput = page.getByTestId('ide-suffix-input');

    await expect(prefixInput).toBeVisible();
    await expect(suffixInput).toBeVisible();
    await prefixInput.fill('  - [');
    await suffixInput.fill('] // ok');

    const applyBtn = page.getByTestId('ide-apply-prefix-suffix');
    await applyBtn.click();

    await saveShot(page, '03-ide-multiline-prefix-suffix.png');

    // Save with ⌘S and assert exact resulting source text on disk!
    const saved = page.waitForResponse(
      (r) => r.request().method() === 'PUT' && r.url().includes('/api/v1/ide/fs/file'),
      { timeout: 15_000 },
    );
    await editor.click();
    await page.keyboard.press('Meta+s');
    await saved;

    const expectedMultiText = ['  - [one] // ok', '  - [two] // ok', '  - [three] // ok'].join(
      '\n',
    );
    expect(readFileSync(join(workspaceRoot, 'multiline_target.txt'), 'utf8')).toBe(
      expectedMultiText,
    );
  });

  test('3. gutter Run & Debug icons across all 5 languages + structured test pass result in Run panel', async ({
    page,
  }) => {
    test.setTimeout(90_000);
    await bootIdePage(page);

    const languageCases: Array<{
      segments: string[];
      relPath: string;
      expectedLabel: RegExp;
    }> = [
      { segments: ['calc', 'calc_test.go'], relPath: 'calc/calc_test.go', expectedLabel: /TestAdd/ },
      { segments: ['pyapp', 'test_calc.py'], relPath: 'pyapp/test_calc.py', expectedLabel: /test_calc/ },
      {
        segments: ['web', 'math.test.mjs'],
        relPath: 'web/math.test.mjs',
        expectedLabel: /computes arithmetic sum cleanly|test/i,
      },
      {
        segments: ['rustlib', 'src', 'lib.rs'],
        relPath: 'rustlib/src/lib.rs',
        expectedLabel: /test_fibonacci/,
      },
      {
        segments: ['java', 'src', 'test', 'java', 'com', 'otto', 'AppTest.java'],
        relPath: 'java/src/test/java/com/otto/AppTest.java',
        expectedLabel: /verifiesGreetingLength|AppTest/,
      },
    ];

    for (const tc of languageCases) {
      const fullPath = join(workspaceRoot, tc.relPath);
      const gutterResp = await daemon<IdeGutterResp>(page.request, 'POST', '/ide/gutter', {
        target: localTarget(),
        path: fullPath,
        content: readFileSync(fullPath, 'utf8'),
      });
      expect(
        gutterResp.items.some((it) => tc.expectedLabel.test(it.label)),
        `Expected gutter detector hit matching ${tc.expectedLabel} in ${tc.relPath}, got ${JSON.stringify(gutterResp.items)}`,
      ).toBeTruthy();

      await openFileFromTree(page, ...tc.segments);
      // Verify gutter run marker renders inside CodeMirror gutters
      const gutterMarker = page
        .locator(
          '.cm-gutters [data-gutter-run], .cm-gutters .cm-ide-run-btn, .cm-gutters button, .cm-gutter-run .cm-gutterElement',
        )
        .filter({ hasText: /▶|Run/i })
        .or(page.locator('.cm-gutters [title*="Run" i], .cm-gutters [aria-label*="Run" i]'));
      await expect(gutterMarker.first()).toBeVisible({ timeout: 15_000 });
    }

    // Open Go `calc/calc_test.go` and run `TestAdd` directly from the gutter!
    await openFileFromTree(page, 'calc', 'calc_test.go');
    const goRunGutter = page
      .locator('.cm-gutters [title*="Run" i], .cm-gutters [aria-label*="Run" i], .cm-gutters .cm-ide-run-btn')
      .first();
    await expect(goRunGutter).toBeVisible();
    await goRunGutter.click();

    // If clicking the gutter icon opens a `Run | Debug` context menu, pick `Run`
    const menu = page.locator('.ctx-menu');
    if (
      await menu
        .waitFor({ state: 'visible', timeout: 1_500 })
        .then(() => true, () => false)
    ) {
      await expectFullyInViewport(page, menu, 'gutter Run/Debug menu');
      await menu.getByRole('menuitem', { name: /^Run/i }).first().click();
    }

    // Verify Bottom Dock shows Run & Tests with structured passed test result + output!
    const runTab = page.getByRole('tab', { name: /Run/i }).first();
    await expect(runTab).toBeVisible({ timeout: 15_000 });
    if ((await runTab.getAttribute('aria-selected')) !== 'true') {
      await runTab.click();
    }

    await expect(page.getByText(/TestAdd/).first()).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText(/passed|PASS/i).first()).toBeVisible({ timeout: 30_000 });
    await saveShot(page, '04-ide-gutter-and-run-panel.png');
  });

  test('4. live DAP debugging: breakpoints, conditional breakpoint, paused line, Call Stack, Variables & Evaluate REPL', async ({
    page,
  }) => {
    test.setTimeout(90_000);
    // Ensure `/Users/itziklavon/go/bin` & `/opt/homebrew/bin` are in the project environment extra_path
    const goBin = join(homedir(), 'go', 'bin');
    const extraPath = [goBin, '/opt/homebrew/bin', '/usr/local/bin'].filter(existsSync).join(':');
    await daemon(page.request, 'PUT', '/ide/environments', {
      target: localTarget(),
      extra_path: extraPath,
      env_vars: [],
    });

    const useGoDlv = existsSync(join(goBin, 'dlv')) || existsSync('/opt/homebrew/bin/dlv');

    await bootIdePage(page);
    if (useGoDlv) {
      await openFileFromTree(page, 'calc', 'calc_test.go');
      await expect(activeEditorContent(page)).toContainText('got := Add(x, y)');
    } else {
      await openFileFromTree(page, 'web', 'math.test.mjs');
      await expect(activeEditorContent(page)).toContainText('const value = 20 + 22');
    }

    // Set breakpoint + conditional breakpoint via the real CodeMirror `.cm-ide-bp-gutter`
    const targetLine1 = useGoDlv ? 8 : 5;
    const targetLine2 = useGoDlv ? 9 : 6;
    const bpGutter = page.locator('.cm-ide-bp-gutter').first();
    await expect(bpGutter).toBeVisible();
    const lineNum1 = page
      .locator('.cm-lineNumbers .cm-gutterElement')
      .filter({ hasText: new RegExp(`^${targetLine1}$`) })
      .first();
    const lineNum2 = page
      .locator('.cm-lineNumbers .cm-gutterElement')
      .filter({ hasText: new RegExp(`^${targetLine2}$`) })
      .first();
    const bpBox = await bpGutter.boundingBox();
    const ln1Box = await lineNum1.boundingBox();
    const ln2Box = await lineNum2.boundingBox();
    expect(bpBox && ln1Box && ln2Box).toBeTruthy();

    // Click line 8 & line 9 in breakpoint gutter
    await page.mouse.click(bpBox!.x + bpBox!.width / 2, ln1Box!.y + ln1Box!.height / 2);
    await page.mouse.click(bpBox!.x + bpBox!.width / 2, ln2Box!.y + ln2Box!.height / 2);
    await expect(page.locator('.cm-ide-bp-marker').first()).toBeVisible();

    if (useGoDlv) {
      // Right-click breakpoint on line 8 -> opens GoLand-style inline breakpoint popover -> set `x == 20`
      await page.mouse.click(bpBox!.x + bpBox!.width / 2, ln1Box!.y + ln1Box!.height / 2, {
        button: 'right',
      });
      const bpPop = page.getByTestId('ide-breakpoint-popover');
      await expectFullyInViewport(page, bpPop, 'Breakpoint condition popover');
      await bpPop.getByTestId('ide-bp-condition-input').fill('x == 20');
      await bpPop.getByTestId('ide-bp-done-btn').click();
      await expect(bpPop).toBeHidden();
    }

    // Start live debug session from the CodeMirror gutter Run/Debug play button!
    const gutterBtn = page
      .locator('.cm-gutters .cm-ide-gutter-run-btn, .cm-gutters [title*="Run or debug" i]')
      .first();
    await expect(gutterBtn).toBeVisible({ timeout: 15_000 });
    const startDebugWait = page.waitForResponse(
      (r) => r.request().method() === 'POST' && r.url().includes('/api/v1/ide/debug/sessions'),
      { timeout: 20_000 },
    );
    await gutterBtn.click();
    const dbgMenu = page.locator('.ctx-menu');
    await expectFullyInViewport(page, dbgMenu, 'gutter Run/Debug menu');
    await dbgMenu.getByRole('menuitem', { name: /^Debug/i }).first().click();

    const startedJson = (await (await startDebugWait).json()) as { session: IdeDebugSnapshot };
    const debugId = startedJson.session.id;
    expect(debugId).toBeTruthy();

    try {
      const dbgPanel = page.getByTestId('ide-debugger-panel');
      await expect(dbgPanel).toBeVisible({ timeout: 15_000 });

      // Wait until real `dlv dap` / Node inspector compiles & hits breakpoint (`Paused`)
      await expect(dbgPanel).toContainText(/Paused/i, { timeout: 45_000 });
      await expect(page.locator('.cm-ide-paused-line').first()).toBeVisible({ timeout: 15_000 });

      // Verify Call Stack & Variables via snapshot + UI rendering
      const pausedSnap = (
        await daemon<{ session: IdeDebugSnapshot }>(
          page.request,
          'GET',
          `/ide/debug/sessions/${debugId}`,
        )
      ).session;
      expect((pausedSnap.stack_frames ?? []).length).toBeGreaterThan(0);
      expect((pausedSnap.scopes ?? []).length).toBeGreaterThan(0);

      // Expand top scope variables via `/ide/debug/sessions/{id}/variables`
      const firstScopeRef = pausedSnap.scopes![0].variables_reference;
      if (firstScopeRef > 0) {
        const varsResp = await daemon<{ variables: Array<{ name: string; value: string }> }>(
          page.request,
          'POST',
          `/ide/debug/sessions/${debugId}/variables`,
          { variables_reference: firstScopeRef },
        );
        expect(varsResp.variables.length).toBeGreaterThan(0);
      }

      // Click `Step over` in the UI Debugger toolbar (advances line 8 -> line 9 where `got == 42`)
      const stepOverBtn = dbgPanel.getByRole('button', { name: 'Step over' });
      await expect(stepOverBtn).toBeEnabled({ timeout: 10_000 });
      await stepOverBtn.click();
      await expect
        .poll(
          async () =>
            (
              await daemon<{ session: IdeDebugSnapshot }>(
                page.request,
                'GET',
                `/ide/debug/sessions/${debugId}`,
              )
            ).session.state,
          { timeout: 20_000 },
        )
        .toBe('paused');
      await expect(dbgPanel).toContainText(/Paused/i, { timeout: 10_000 });

      // Evaluate expression in the UI Debug Evaluate REPL!
      const evalExpr = useGoDlv ? 'got' : '20 + 22';
      const replInput = dbgPanel
        .locator('input[placeholder*="Evaluate" i], input[aria-label*="Evaluate" i], form input[type="text"]')
        .last();
      await expect(replInput).toBeVisible();
      await replInput.fill(evalExpr);
      await replInput.press('Enter');
      await expect(dbgPanel).toContainText('42', { timeout: 10_000 });

      await saveShot(page, '05-ide-live-debugger.png');

      // Stop session via UI toolbar button
      const stopBtn = dbgPanel.getByRole('button', { name: 'Stop debug session' });
      if ((await stopBtn.count()) > 0 && (await stopBtn.isEnabled())) {
        await stopBtn.click();
      }
    } finally {
      await daemon(page.request, 'POST', `/ide/debug/sessions/${debugId}/command`, {
        action: 'stop',
      }).catch(() => {});
    }
  });

  test('5. Toolchains & SDK Downloader modal: host OS/arch detection + all 5 ecosystems', async ({
    page,
  }) => {
    test.setTimeout(60_000);
    await bootIdePage(page);
    await clickHeaderAction(page, /Toolchain|SDK/i);

    const modal = page.locator('.sheet[role="dialog"]').last();
    await expectFullyInViewport(page, modal, 'Toolchains & SDK Downloader modal');

    // Machine OS & CPU architecture badge matches host
    const expectedOsRe = platform() === 'darwin' ? /macOS|darwin/i : /Linux/i;
    const expectedArchRe = arch() === 'arm64' ? /arm64|aarch64/i : /x86_64|x64|amd64/i;
    await expect(modal).toContainText(expectedOsRe);
    await expect(modal).toContainText(expectedArchRe);

    // Verify all 5 ecosystems render cleanly (either via ecosystem tabs or cards)
    for (const eco of [
      { label: /Java/i, dlRe: /21|17|Temurin/i },
      { label: /^Go$|Go\b/i, dlRe: /1\.2\d|go/i },
      { label: /Rust/i, dlRe: /stable|nightly|rust/i },
      { label: /Node|JavaScript|TypeScript/i, dlRe: /22|20|24|LTS/i },
      { label: /Python/i, dlRe: /3\.1\d|uv|python/i },
    ]) {
      const ecoTab = modal.getByRole('tab', { name: eco.label }).first();
      if ((await ecoTab.count()) > 0 && (await ecoTab.isVisible())) {
        await ecoTab.click();
      }
      await expect(modal).toContainText(eco.dlRe);
    }

    await saveShot(page, '06-ide-toolchains-modal.png');
    await page.keyboard.press('Escape');
    await expect(modal).toBeHidden();
  });

  test('6. Environments & Run Configurations modal: secret env vars + saved Run Config persistence', async ({
    page,
  }) => {
    test.setTimeout(60_000);
    // Save project environment with both plain and secret env vars, and verify secret masking
    const goBin = join(homedir(), 'go', 'bin');
    const extraPath = [goBin, '/opt/homebrew/bin', '/usr/local/bin'].filter(existsSync).join(':');
    const putEnv = await daemon<{
      env: { env_vars: Array<{ key: string; value?: string | null; secret: boolean; has_secret?: boolean }> };
    }>(page.request, 'PUT', '/ide/environments', {
      target: localTarget(),
      dotenv_path: '.env.example',
      extra_path: extraPath,
      env_vars: [
        { key: 'APP_ENV', value: 'staging', secret: false },
        { key: 'STRIPE_API_SECRET', value: 'sk_test_super_secret_value', secret: true },
      ],
    });
    const secretRow = putEnv.env.env_vars.find((v) => v.key === 'STRIPE_API_SECRET');
    expect(secretRow).toBeTruthy();
    expect(secretRow?.has_secret).toBe(true);
    expect(secretRow?.value ?? '').not.toContain('sk_test_super_secret_value');

    await bootIdePage(page);
    await clickHeaderAction(page, /Environments & run configs|Environment|Run Config/i);

    const modal = page.locator('.sheet[role="dialog"]').last();
    await expectFullyInViewport(page, modal, 'Environments & Run Configurations modal');

    // Verify masked Keychain secret badge renders cleanly on Project environment tab without exposing plaintext
    const envTab = modal.getByRole('tab', { name: /Project environment/i }).first();
    if ((await envTab.count()) > 0 && (await envTab.getAttribute('aria-selected')) !== 'true') {
      await envTab.click();
    }
    await expect(modal).toContainText('Stored in Keychain');
    await expect(modal).not.toContainText('sk_test_super_secret_value');

    // Switch to Run / Debug Configurations tab and create a saved Run Config via the UI form!
    const configsTab = modal.getByRole('tab', { name: /Run \/ debug configurations|Run|Config/i }).first();
    await expect(configsTab).toBeVisible();
    await configsTab.click();
    const newCfgBtn = modal.getByRole('button', { name: /^New$/i }).first();
    if ((await newCfgBtn.count()) > 0 && (await newCfgBtn.isVisible())) {
      await newCfgBtn.click();
    }

    await modal.locator('#cfg-name').fill('Go Calc Unit Suite');
    await modal.locator('#cfg-runner').selectOption('go_test');
    await modal.locator('#cfg-args').fill('./calc/...');
    await modal.locator('#cfg-cwd').fill(workspaceRoot);
    await modal.getByRole('button', { name: /^Save configuration$/i }).click();

    await expect(modal.locator('.env-cfg-rail')).toContainText('Go Calc Unit Suite');
    await saveShot(page, '07-ide-environments-modal.png');
    await page.keyboard.press('Escape');
    await expect(modal).toBeHidden();

    // Reload `#/ide` and verify `Go Calc Unit Suite` persists in the IDE header & backend!
    await page.reload();
    await expect(page.locator('.shell')).toBeVisible();
    const envGet = await daemon<{ configs: Array<{ name: string }> }>(
      page.request,
      'GET',
      `/ide/environments?kind=local&root=${encodeURIComponent(workspaceRoot)}`,
    );
    expect(envGet.configs.some((c) => c.name === 'Go Calc Unit Suite')).toBe(true);
    await expect(page.getByTestId('ide-run-config-select')).toContainText('Go Calc Unit Suite');
  });

  test('7. Remote SSH Target modal: saved SSH profiles & switching UI', async ({ page }) => {
    test.setTimeout(60_000);
    expect(sshConnId).toBeTruthy();

    await bootIdePage(page);
    await clickHeaderAction(page, /Connect remote SSH/i);

    const modal = page.locator('.sheet[role="dialog"]').last();
    await expectFullyInViewport(page, modal, 'Remote SSH Target modal');

    // Verify both Local Workspace and our seeded SSH connection `staging-linux-arm64` appear
    await expect(modal).toContainText(/Local/i);
    await expect(modal).toContainText('staging-linux-arm64');
    await expect(modal).toContainText(/10\.0\.14\.22|deploy/);

    await saveShot(page, '08-ide-ssh-target-modal.png');
    await page.keyboard.press('Escape');
    await expect(modal).toBeHidden();
  });

  test('8. Share with Friends (2 Browser Contexts): live peer presence, multi-cursor sync, Viewer -> Editor toggle, Follow mode & clean revoke', async ({
    page,
    browser,
  }) => {
    test.setTimeout(90_000);
    // Ensure calc.go starts cleanly
    const initialCalcGo = [
      'package calc',
      '',
      'func Add(a, b int) int {',
      '\tsum := a + b',
      '\treturn sum',
      '}',
      '',
      'func Multiply(a, b int) int {',
      '\tprod := a * b',
      '\treturn prod',
      '}',
      '',
    ].join('\n');
    writeFileSync(join(workspaceRoot, 'calc', 'calc.go'), initialCalcGo);

    await bootIdePage(page);
    await openFileFromTree(page, 'calc', 'calc.go');
    await expect(activeEditorContent(page)).toContainText('func Add(a, b int) int');

    // Host opens Share modal and starts a room with default_role = 'viewer'
    await clickHeaderAction(page, /^Share|Collaborate/i);
    const shareModal = page.locator('.sheet[role="dialog"]').last();
    await expectFullyInViewport(page, shareModal, 'Share with Friends modal');

    const startRoomBtn = shareModal
      .getByRole('button', { name: /Start sharing|Create room|Start session|Start/i })
      .first();
    await expect(startRoomBtn).toBeVisible();

    const createWait = page.waitForResponse(
      (r) => r.request().method() === 'POST' && r.url().includes('/api/v1/ide/collab/rooms'),
      { timeout: 15_000 },
    );
    await startRoomBtn.click();
    const created = (await (await createWait).json()) as {
      room: { id: string };
      invite_token: string;
      share_hash: string;
      host_peer_id: string;
      host_peer_secret: string;
    };
    expect(created.host_peer_id).toBeTruthy();
    expect(created.host_peer_secret).toBeTruthy();

    const roomId = created.room.id;
    const inviteToken = created.invite_token;
    const shareHash = created.share_hash;
    expect(roomId).toBeTruthy();
    expect(inviteToken.length).toBeGreaterThanOrEqual(32);

    // Wait for invite URL box to appear & assert zero error toasts on Host!
    await expect(shareModal.locator('#sh-invite-url')).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('.toast')).not.toContainText(/Couldn’t|can't reach/i);
    await saveShot(page, '09-ide-share-modal.png');

    // Context 2: Friend joins using ONLY the capability share link (zero owner token!)
    const uiOrigin = new URL(page.url()).origin;
    const friendContext = await browser.newContext({
      viewport: { width: 1280, height: 800 },
      storageState: {
        cookies: [],
        origins: [
          {
            origin: uiOrigin,
            localStorage: [{ name: 'otto_base', value: base }],
          },
        ],
      },
    });

    try {
      const friendPage = await friendContext.newPage();
      const guestShareRef = shareHash.replace(/^\//, '');
      const sep = guestShareRef.includes('?') ? '&' : '?';
      await friendPage.goto(
        `${uiOrigin}/${guestShareRef}${sep}name=${encodeURIComponent('Maya (Friend)')}`,
      );

      // Friend browser context (zero owner token!) must render the collaborative IDE view, NOT get trapped on Login!
      await expect(friendPage.getByTestId('page-header')).toBeVisible({ timeout: 15_000 });
      const friendEditor = friendPage.locator('.cm-editor .cm-content').first();
      await expect(friendEditor).toContainText('func Add(a, b int) int', { timeout: 15_000 });

      // Default guest role is Viewer -> typing in Friend's CodeMirror is blocked by `readOnly`!
      await friendEditor.click();
      await friendPage.keyboard.insertText('// blocked viewer edit\n');
      await expect(friendEditor).not.toContainText('// blocked viewer edit');

      // Host's Share modal updates live with Maya (Friend) in Connected peers!
      await expect(shareModal.locator('.sh-peer-list')).toContainText('Maya (Friend)', {
        timeout: 15_000,
      });

      // Host toggles Follow mode on Maya (Friend) AND upgrades Maya from Viewer -> Editor via the UI!
      const followBtn = shareModal.getByRole('button', { name: /^Follow$/i }).first();
      await expect(followBtn).toBeVisible();
      await followBtn.click();
      await expect(shareModal.getByRole('button', { name: /^Following$/i }).first()).toBeVisible();

      const roleSelect = shareModal.getByLabel('Access role for Maya (Friend)');
      await expect(roleSelect).toBeVisible();
      await roleSelect.selectOption('editor');

      // Close Share modal on Host temporarily so Host's CodeMirror shows live peer cursor + live code edit!
      await page.keyboard.press('Escape');
      await expect(shareModal).toBeHidden();

      // Wait until Friend's `readOnly` compartment unlocks via SSE and edit `calc/calc.go`!
      await expect
        .poll(
          async () => {
            await friendEditor.click();
            await friendPage.keyboard.insertText('// editable_ok ');
            const txt = (await friendEditor.textContent()) ?? '';
            return txt.includes('editable_ok');
          },
          { timeout: 10_000 },
        )
        .toBe(true);

      const friendSelectAll = await editorSelectAll(friendPage);
      await friendPage.keyboard.press(friendSelectAll);
      const collabEditedGo = [
        'package calc',
        '',
        '// Collaborative edit by Maya (Friend)',
        'func Add(a, b int) int {',
        '\tsum := a + b',
        '\treturn sum',
        '}',
        '',
        'func Multiply(a, b int) int {',
        '\tprod := a * b',
        '\treturn prod',
        '}',
        '',
      ].join('\n');
      await friendPage.keyboard.insertText(collabEditedGo);
      // Select `Add(a, b int)` in Friend's editor so Host renders Maya's peer selection + caret pill!
      await friendPage.keyboard.press('ArrowUp');
      await friendPage.keyboard.press('ArrowUp');
      // Save from Friend (`⌘S` -> `/ide/collab/guest/{roomId}/edit` with `save: true`)
      await friendPage.keyboard.press('Meta+s');

      // Host's CodeMirror updates live with Maya's collaborative edit AND renders Maya's zero-width peer caret + floating pill!
      await expect(activeEditorContent(page)).toContainText(
        '// Collaborative edit by Maya (Friend)',
        { timeout: 15_000 },
      );
      await expect(
        page.locator('.cm-ide-peer-caret, .cm-ide-peer-eol').first(),
      ).toBeAttached({ timeout: 15_000 });
      await expect(page.locator('.cm-ide-peer-pill').first()).toBeVisible({
        timeout: 15_000,
      });

      await saveShot(page, '10-ide-collab-two-peers.png');

      // Re-open Share modal on Host and click `Stop sharing` to revoke access immediately!
      await clickHeaderAction(page, /^Share|Collaborate/i);
      const reopenModal = page.locator('.sheet[role="dialog"]').last();
      await expect(reopenModal).toBeVisible();
      const stopShareBtn = reopenModal
        .getByRole('button', { name: /Stop sharing|Revoke/i })
        .first();
      await expect(stopShareBtn).toBeVisible();
      await stopShareBtn.click();

      // Confirm destructive dialog (`Stop sharing workspace?`)
      const confirmBtn = page
        .locator('.sheet[role="dialog"]')
        .last()
        .getByRole('button', { name: /^Stop sharing$/i });
      if (
        await confirmBtn
          .waitFor({ state: 'visible', timeout: 3_000 })
          .then(() => true, () => false)
      ) {
        await confirmBtn.click();
      }

      // Verify guest capability token is immediately revoked on backend (404/401)
      await expect
        .poll(
          async () =>
            (
              await friendPage.request.get(
                `${base}/api/v1/ide/collab/guest/${roomId}/state?token=${inviteToken}`,
              )
            ).ok(),
          { timeout: 10_000 },
        )
        .toBe(false);
    } finally {
      await friendContext.close();
      if (await page.locator('.sheet[role="dialog"]').isVisible()) {
        await page.keyboard.press('Escape');
      }
      writeFileSync(join(workspaceRoot, 'calc', 'calc.go'), initialCalcGo);
    }
  });

  test('9. Runtime IDE Plugins modal: zero-rebuild install of kotlin-ide, immediate .kt gutter activation & enable/disable toggle', async ({
    page,
  }) => {
    test.setTimeout(60_000);
    expect(existsSync(KOTLIN_PLUGIN_DIR), `Missing reference plugin at ${KOTLIN_PLUGIN_DIR}`).toBe(
      true,
    );

    // Ensure clean starting state if re-running
    await page.request
      .fetch(`${base}/api/v1/ide/plugins/kotlin-ide`, {
        method: 'DELETE',
        headers: { Authorization: `Bearer ${token}` },
      })
      .catch(() => {});

    // Seed a Kotlin test file matching `kotlin-ide`'s `fun test...()` gutter detector
    const ktPath = join(workspaceRoot, 'SampleKotlinTest.kt');
    const ktContent = [
      'package com.otto.demo',
      '',
      'import kotlin.test.Test',
      '',
      'class SampleKotlinTest {',
      '    @Test',
      '    fun testKotlinPluginGutter() {',
      '        val sum = 20 + 22',
      '        check(sum == 42)',
      '    }',
      '}',
      '',
    ].join('\n');
    writeFileSync(ktPath, ktContent);

    await bootIdePage(page);
    await clickHeaderAction(page, /Plugin|Extension/i);

    const modal = page.locator('.sheet[role="dialog"]').last();
    await expectFullyInViewport(page, modal, 'Runtime IDE Plugins modal');

    // Install `examples/ide-plugins/kotlin-ide` via the real UI `Install from path / Git` tab!
    const installTab = modal.getByRole('tab', { name: /Path|Git|Source/i }).first();
    await expect(installTab).toBeVisible();
    await installTab.click();

    const sourceInput = modal.locator('#pl-source-input');
    await expect(sourceInput).toBeVisible();
    await sourceInput.fill(KOTLIN_PLUGIN_DIR);
    await modal.getByRole('button', { name: /^Install$/i }).click();

    // Wait for `IdePluginsModal` to switch to Installed tab and render the Kotlin IDE Pack card!
    await expect(modal.locator('.pl-item').first()).toContainText(/Kotlin/i, { timeout: 15_000 });
    await saveShot(page, '11-ide-plugins-modal.png');

    // Immediately verify `.kt` gutter detector is active with zero rebuild!
    const ktGutterActive = await daemon<IdeGutterResp>(page.request, 'POST', '/ide/gutter', {
      target: localTarget(),
      path: ktPath,
      content: ktContent,
    });
    expect(ktGutterActive.items.length).toBeGreaterThan(0);

    // Disable `kotlin-ide` -> gutter detector deactivates immediately
    await daemon(page.request, 'POST', '/ide/plugins/kotlin-ide/toggle', { enabled: false });
    const ktGutterDisabled = await daemon<IdeGutterResp>(page.request, 'POST', '/ide/gutter', {
      target: localTarget(),
      path: ktPath,
      content: ktContent,
    });
    expect(ktGutterDisabled.items.length).toBe(0);

    // Re-enable `kotlin-ide` -> gutter detector re-activates immediately
    await daemon(page.request, 'POST', '/ide/plugins/kotlin-ide/toggle', { enabled: true });
    const ktGutterReenabled = await daemon<IdeGutterResp>(page.request, 'POST', '/ide/gutter', {
      target: localTarget(),
      path: ktPath,
      content: ktContent,
    });
    expect(ktGutterReenabled.items.length).toBeGreaterThan(0);

    await page.keyboard.press('Escape');
    await expect(modal).toBeHidden();

    // Also open `SampleKotlinTest.kt` in the real editor and verify the Kotlin gutter Run icon appears live!
    await page.reload();
    await expect(page.locator('.shell')).toBeVisible();
    await openFileFromTree(page, 'SampleKotlinTest.kt');
    await expect(
      page.locator('.cm-gutters .cm-ide-gutter-run-btn, .cm-gutters [title*="Run" i]').first(),
    ).toBeVisible({ timeout: 15_000 });
  });

  test('10. Visual, Theme & Accessibility sweep: Desktop Light, Desktop Dark & RTL (dir="rtl")', async ({
    page,
  }) => {
    test.setTimeout(90_000);

    // A. Desktop Light Mode
    await bootIdePage(page, { scheme: 'light', direction: 'ltr' });
    await openFileFromTree(page, 'calc', 'calc.go');
    await expectNoHorizontalOverflow(page);
    await expectContentHasHeight(page, 400);
    await expectAccessible(page, { key: 'ide' });
    await saveShot(page, '12-ide-desktop-light.png');

    // B. Desktop Dark Mode
    await page.evaluate(() => localStorage.setItem('otto_scheme', 'dark'));
    await page.reload();
    await expect(page.locator('.shell')).toBeVisible();
    await openFileFromTree(page, 'calc', 'calc.go');
    await expectNoHorizontalOverflow(page);
    await expectContentHasHeight(page, 400);
    await expectAccessible(page, { key: 'ide' });
    await saveShot(page, '13-ide-desktop-dark.png');

    // C. Desktop RTL (`dir="rtl"`) — also verifies initial entry file auto-opens on load
    await page.evaluate(() => localStorage.setItem('otto_direction', 'rtl'));
    await page.reload();
    await expect(page.locator('.shell')).toBeVisible();
    await expect.poll(() => page.evaluate(() => document.documentElement.dir)).toBe('rtl');
    await expect(page.locator('.cm-editor').first()).toBeVisible({ timeout: 15_000 });
    await expectNoHorizontalOverflow(page);
    await expectContentHasHeight(page, 400);
    await expectAccessible(page, { key: 'ide' });
    await saveShot(page, '14-ide-desktop-rtl.png');
  });

  test('11. GoLand workflow on go_admission_demo + attached go_shared_kit: multi-root tree, New Go file, 3-tab persistence + Split Right, Shell/.env/Config env in Run & dlv Debug with inline values, $GOROOT Dive-In & 0 leaked dlv', async ({
    page,
  }) => {
    test.setTimeout(180_000);
    const goBin = join(homedir(), 'go', 'bin');
    const extraPath = [goBin, '/opt/homebrew/bin', '/usr/local/bin'].filter(existsSync).join(':');
    const hasDlv = existsSync(join(goBin, 'dlv')) || existsSync('/opt/homebrew/bin/dlv');

    const admParent = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-admission-suite-')));
    const admRoot = join(admParent, 'go_admission_demo');
    const sharedKitRoot = join(admParent, 'go_shared_kit');
    const entRoot1 = join(admParent, 'go_admission_enterprise_multi_region_gateway_service');
    const entRoot2 = join(admParent, 'bo_common_ui_player_loyalty_orchestrator_portal');
    mkdirSync(join(admRoot, 'cmd', 'server'), { recursive: true });
    mkdirSync(join(admRoot, 'internal', 'admission'), { recursive: true });
    mkdirSync(sharedKitRoot, { recursive: true });
    mkdirSync(entRoot1, { recursive: true });
    mkdirSync(entRoot2, { recursive: true });

    writeFileSync(join(admRoot, 'go.mod'), 'module example.com/go_admission_demo\n\ngo 1.22\n');
    writeFileSync(join(admRoot, '.env'), 'DOTENV_FILE_VAR=dotenv-admission-ok\n');
    writeFileSync(
      join(admRoot, 'internal', 'admission', 'check.go'),
      [
        'package admission',
        '',
        'type Request struct {',
        '\tTenantID string',
        '\tQuotaRPS int',
        '}',
        '',
        'type Decision struct {',
        '\tAllowed   bool',
        '\tShellEnv  string',
        '\tDotenvVar string',
        '\tCustomCfg string',
        '}',
        '',
        'func EvaluateAdmission(req Request, shellEnv, dotenvVar, customCfg string) Decision {',
        '\tallowed := req.QuotaRPS > 0 && len(req.TenantID) > 0',
        '\tdec := Decision{',
        '\t\tAllowed:   allowed,',
        '\t\tShellEnv:  shellEnv,',
        '\t\tDotenvVar: dotenvVar,',
        '\t\tCustomCfg: customCfg,',
        '\t}',
        '\treturn dec',
        '}',
        '',
      ].join('\n'),
    );
    writeFileSync(
      join(admRoot, 'internal', 'admission', 'check_test.go'),
      [
        'package admission',
        '',
        'import "testing"',
        '',
        'func TestEvaluateAdmission(t *testing.T) {',
        '\td := EvaluateAdmission(Request{TenantID: "t1", QuotaRPS: 100}, "sh", "env", "cfg")',
        '\tif !d.Allowed {',
        '\t\tt.Fatal("expected allowed")',
        '\t}',
        '}',
        '',
      ].join('\n'),
    );
    const shellVarKey = process.env.GO_DEBUGGING_PARAMS ? 'GO_DEBUGGING_PARAMS' : 'HOME';
    const expectedShellVal = process.env[shellVarKey] ?? '';
    expect(expectedShellVal.length).toBeGreaterThan(0);

    writeFileSync(
      join(admRoot, 'cmd', 'server', 'main.go'),
      [
        'package main',
        '',
        'import (',
        '\t"fmt"',
        '\t"os"',
        '',
        '\t"example.com/go_admission_demo/internal/admission"',
        ')',
        '',
        'func main() {',
        `\tshellEnv := os.Getenv(${JSON.stringify(shellVarKey)})`,
        '\tdotenvVar := os.Getenv("DOTENV_FILE_VAR")',
        '\tcustomCfg := os.Getenv("CUSTOM_CFG_VAR")',
        '\treq := admission.Request{TenantID: "casino-eu-1", QuotaRPS: 250}',
        '\tdec := admission.EvaluateAdmission(req, shellEnv, dotenvVar, customCfg)',
        '\tfmt.Printf("ADMISSION_OK=%v SHELL=%s DOTENV=%s CUSTOM=%s\\n", dec.Allowed, dec.ShellEnv, dec.DotenvVar, dec.CustomCfg)',
        '}',
        '',
      ].join('\n'),
    );
    writeFileSync(join(sharedKitRoot, 'go.mod'), 'module example.com/go_shared_kit\n\ngo 1.22\n');
    writeFileSync(
      join(sharedKitRoot, 'kit.go'),
      [
        'package kit',
        '',
        'import "strings"',
        '',
        'func NormalizeTenant(id string) string {',
        '\treturn strings.TrimSpace(strings.ToLower(id))',
        '}',
        '',
      ].join('\n'),
    );
    writeFileSync(join(entRoot1, 'go.mod'), 'module example.com/ent1\n\ngo 1.22\n');
    writeFileSync(join(entRoot2, 'package.json'), '{"name":"bo-common-ui-portal","version":"1.0.0"}\n');

    const setLiveScheme = async (scheme: 'dark' | 'light'): Promise<void> => {
      await page.evaluate(() => {
        localStorage.setItem('otto_scheme', 'auto');
      });
      await page.emulateMedia({ colorScheme: scheme });
      await expect
        .poll(() => page.evaluate(() => document.documentElement.dataset.scheme))
        .toBe(scheme);
      // Wait for active CSS transitions (`--dur-fast` / `--dur-enter`) to finish settling cleanly.
      await expect
        .poll(() =>
          page.evaluate(() =>
            document.getAnimations().every((a) => a.playState !== 'running'),
          ),
        )
        .toBe(true);
    };

    const createdWsIds: string[] = [];
    try {
      const admWs = await daemon<{ id: string }>(page.request, 'POST', '/workspaces', {
        name: 'go_admission_demo',
        root_path: admRoot,
      });
      createdWsIds.push(admWs.id);

      for (const [nm, rp] of [
        ['go_admission_enterprise_multi_region_gateway_service', entRoot1],
        ['bo_common_ui_player_loyalty_orchestrator_portal', entRoot2],
      ] as const) {
        const w = await daemon<{ id: string }>(page.request, 'POST', '/workspaces', { name: nm, root_path: rp });
        createdWsIds.push(w.id);
      }

      const admTarget: IdeTargetSpec = {
        kind: 'local',
        workspace_id: admWs.id,
        root: admRoot,
        connection_id: null,
      };

      // 1. Measure instant runner discovery (< 100ms) + verify cached shell env snapshot!
      const tDisc0 = performance.now();
      const admEnvInit = await daemon<{
        env: { inherit_shell_env?: boolean; additional_roots?: string[] };
        configs: Array<{ id: string; name: string; runner: string }>;
        shell_env?: { vars?: Record<string, string> };
      }>(
        page.request,
        'GET',
        `/ide/environments?kind=local&root=${encodeURIComponent(admRoot)}&workspace_id=${encodeURIComponent(admWs.id)}`,
      );
      const discoveryMs = performance.now() - tDisc0;
      expect(discoveryMs, `Runner discovery latency (${discoveryMs.toFixed(1)}ms) must be < 100ms`).toBeLessThan(100);
      expect(admEnvInit.configs.some((c) => c.runner === 'go_run' || c.runner === 'go_test')).toBe(true);

      const expectedShellDbgParam = process.env.GO_DEBUGGING_PARAMS || '';
      const shellSnap = await daemon<{ shell: string; vars: Record<string, string> }>(
        page.request,
        'GET',
        '/ide/shell-env?kind=local',
      );
      expect(shellSnap.vars.HOME).toBeTruthy();
      if (expectedShellDbgParam) {
        expect(shellSnap.vars.GO_DEBUGGING_PARAMS).toBe(expectedShellDbgParam);
      }

      const shellReloaded = await daemon<{ shell: string; vars: Record<string, string> }>(
        page.request,
        'POST',
        '/ide/shell-env/reload',
        { target: admTarget },
      );
      expect(shellReloaded.vars.HOME).toBe(shellSnap.vars.HOME);
      if (expectedShellDbgParam) {
        expect(shellReloaded.vars.GO_DEBUGGING_PARAMS).toBe(expectedShellDbgParam);
      }

      // Configure extra_path + .env on `go_admission_demo` AND attach `go_shared_kit` via `/ide/projects/folders`!
      await daemon(page.request, 'PUT', '/ide/environments', {
        target: admTarget,
        inherit_shell_env: true,
        dotenv_path: '.env',
        extra_path: extraPath,
        env_vars: expectedShellDbgParam
          ? []
          : [{ key: 'GO_DEBUGGING_PARAMS', value: 'admission-e2e-shell-ok', secret: false }],
      });
      const attachResp = await daemon<{ env: { additional_roots: string[] } }>(
        page.request,
        'POST',
        '/ide/projects/folders',
        { target: admTarget, folder_path: sharedKitRoot },
      );
      expect(attachResp.env.additional_roots).toContain(sharedKitRoot);

      // Boot IDE directly on `go_admission_demo`
      await page.addInitScript((wsId: string) => {
        localStorage.setItem('otto_workspace', wsId);
        localStorage.setItem('otto_rail_expanded', '0');
        localStorage.setItem('otto_scheme', 'auto');
        localStorage.setItem('otto_direction', 'ltr');
      }, admWs.id);
      await page.emulateMedia({ colorScheme: 'dark' });
      await page.goto('/#/ide');
      await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
      const tree = page.getByTestId('ide-project-tree');
      await expect(tree).toBeVisible({ timeout: 15_000 });

      // 2. Verify IntelliJ Project Tree root node (`go_admission_demo`) + attached root (`go_shared_kit`)
      await expect(tree.getByTestId('ide-tree-root-node')).toContainText('go_admission_demo');
      await expect(tree.getByTestId('ide-tree-attached-root')).toContainText('go_shared_kit', { timeout: 15_000 });

      // Expand `go_shared_kit` if needed and open `kit.go` from the attached folder!
      const kitRootBtn = tree.getByTestId('ide-tree-attached-root').locator('button.tree-item').first();
      if ((await kitRootBtn.getAttribute('aria-expanded')) !== 'true') {
        await kitRootBtn.click();
      }
      const kitFileBtn = tree.locator(`button.tree-item[title$="kit.go"]`).first();
      await expect(kitFileBtn).toBeVisible({ timeout: 10_000 });
      await kitFileBtn.click();
      await expect(activeEditorContent(page)).toContainText('func NormalizeTenant');

      // Expand `internal` -> `admission` and create `New Go file…` (`policy` -> `policy.go` with `package admission` auto-header!)
      for (const seg of ['internal', 'internal/admission']) {
        const dirBtn = tree.locator(`button.tree-item[title="${seg}"]`).first();
        await expect(dirBtn).toBeVisible({ timeout: 10_000 });
        if ((await dirBtn.getAttribute('aria-expanded')) !== 'true') await dirBtn.click();
      }
      await tree.locator('button.tree-item[title="internal/admission"]').first().click({ button: 'right' });
      const ctx = page.locator('.ctx-menu');
      await expectFullyInViewport(page, ctx, 'Project tree folder context menu');
      await ctx.getByRole('menuitem', { name: /New Go file/i }).click();
      const goFileDialog = page.locator('.sheet[role="dialog"]').last();
      await expectFullyInViewport(page, goFileDialog, 'New Go file dialog');
      await goFileDialog.locator('input').first().fill('policy');
      await goFileDialog.getByRole('button', { name: /^Create$/i }).click();
      await expect(goFileDialog).toBeHidden();
      await expect(activeEditorContent(page)).toContainText('package admission');
      expect(readFileSync(join(admRoot, 'internal', 'admission', 'policy.go'), 'utf8')).toContain('package admission');

      // 3. Multi-Tab Opening (zero tab overwriting!) + edit/undo preservation + Split Right
      for (const seg of ['cmd', 'cmd/server']) {
        const dirBtn = tree.locator(`button.tree-item[title="${seg}"]`).first();
        await expect(dirBtn).toBeVisible({ timeout: 10_000 });
        if ((await dirBtn.getAttribute('aria-expanded')) !== 'true') await dirBtn.click();
      }
      await tree.locator('button.tree-item[title="cmd/server/main.go"]').first().click();
      await tree.locator('button.tree-item[title="internal/admission/check.go"]').first().click();
      await tree.locator('button.tree-item[title="internal/admission/check_test.go"]').first().click();

      const tabStrip = page.getByTestId('ide-tabs-strip');
      for (const tabName of ['main.go', 'check.go', 'check_test.go']) {
        await expect(tabStrip.locator('.ide-tab-btn', { hasText: tabName }).first()).toBeVisible();
      }

      // Edit `check.go`, switch to `main.go`, switch back to `check.go` -> unsaved edit & Undo (`⌘Z`) preserved!
      await tabStrip.locator('.ide-tab-btn', { hasText: 'check.go' }).first().click();
      await expect(activeEditorContent(page)).toContainText('func EvaluateAdmission');
      await activeEditorContent(page).click();
      await page.keyboard.press('Meta+ArrowDown');
      await page.keyboard.insertText('\n// transient multi-tab edit\n');
      await tabStrip.locator('.ide-tab-btn', { hasText: 'main.go' }).first().click();
      await expect(activeEditorContent(page)).toContainText('func main()');
      await tabStrip.locator('.ide-tab-btn', { hasText: 'check.go' }).first().click();
      await expect(activeEditorContent(page)).toContainText('// transient multi-tab edit');
      await activeEditorContent(page).click();
      for (let u = 0; u < 30; u++) {
        const txt = (await activeEditorContent(page).textContent()) ?? '';
        if (!txt.includes('transient multi-tab edit')) break;
        await page.keyboard.press('Meta+z');
      }
      await expect(activeEditorContent(page)).not.toContainText('transient multi-tab edit');
      // Save clean `check.go` via ⌘S so line numbers match disk for `dlv dap`
      await activeEditorContent(page).click();
      await page.keyboard.press('Meta+s');
      await tabStrip.locator('.ide-tab-btn', { hasText: 'check_test.go' }).first().click();

      // Test Split Right (`ide-split-right-btn` -> `ide-split-pane`)
      await page.getByTestId('ide-split-right-btn').click();
      await expect(page.getByTestId('ide-split-pane')).toBeVisible({ timeout: 10_000 });
      await page.getByTestId('ide-split-pane').getByRole('button', { name: 'Close split pane' }).click();
      await expect(page.getByTestId('ide-split-pane')).toBeHidden();

      // 4. Workspace Switcher at 2560x1440 AND 1440x900 with long enterprise names
      for (const vp of [
        { width: 2560, height: 1440 },
        { width: 1440, height: 900 },
      ]) {
        await page.setViewportSize(vp);
        const switcher = page.getByTestId('ide-target-switcher');
        await expect(switcher).toBeVisible();
        const swBox = await switcher.boundingBox();
        expect(swBox && swBox.width > 160).toBeTruthy();
        await switcher.click();
        const wsPop = page.getByTestId('ide-workspace-popover');
        await expectFullyInViewport(page, wsPop, `Workspace popover at ${vp.width}x${vp.height}`);
        await expect(wsPop).toContainText('Folders in current project (2)');
        await expect(wsPop).toContainText('go_admission_enterprise_multi_region_gateway_service');
        await expect(wsPop).toContainText('bo_common_ui_player_loyalty_orchestrator_portal');
        await page.keyboard.press('Escape');
        await expect(wsPop).toBeHidden();
      }

      // 5. Inspect Login Shell Env in UI + create `Admission Server` Run Config + verify Shell/.env/Config env in Run!
      await page.getByTestId('ide-edit-configs-btn').click();
      const envModal = page.locator('.sheet[role="dialog"]').last();
      await expectFullyInViewport(page, envModal, 'Environments & Run Configurations modal');

      // Switch to Project environment tab first to inspect `IdeShellEnvCard` (`GO_DEBUGGING_PARAMS`)
      await envModal.getByRole('tab', { name: /Project environment/i }).click();
      await expect(page.getByTestId('ide-shell-env-card')).toBeVisible();
      await page.getByTestId('ide-shell-env-preview-btn').click();
      await expect(page.getByTestId('ide-shell-env-preview')).toBeVisible();
      await page.getByTestId('ide-shell-env-search').fill(shellVarKey);
      await expect(page.getByTestId('ide-shell-env-preview')).toContainText(expectedShellVal);
      if (shellVarKey === 'GO_DEBUGGING_PARAMS') {
        await expect(page.getByTestId('ide-shell-env-preview')).toContainText('Go / Debug');
      }

      // Switch to Run / debug configurations tab & save `Admission Server`
      await envModal.getByRole('tab', { name: /Run \/ debug configurations/i }).click();
      await envModal.getByRole('button', { name: /^New$/i }).click();
      await envModal.locator('#cfg-name').fill('Admission Server');
      await envModal.locator('#cfg-runner').selectOption('go_run');
      await envModal.locator('#cfg-target').fill('./cmd/server');
      await envModal.locator('#cfg-cwd').fill(admRoot);
      await envModal.locator('#cfg-env-file').fill('.env');
      await envModal.getByRole('button', { name: /Add override/i }).click();
      await envModal.getByLabel('Override variable name').last().fill('CUSTOM_CFG_VAR');
      await envModal.getByLabel('Override value').last().fill('cfg-override-ok');
      await envModal.getByRole('button', { name: /^Save configuration$/i }).click();
      await expect(envModal.locator('.env-cfg-rail')).toContainText('Admission Server');

      // Open SDK Version Downloader directly via `SDKs…` inside `Environments & run configurations` (verifies clean modal stacking!)
      await envModal.getByTestId('ide-cfg-open-toolchains').click();
      const tcModal = page.locator('.sheet[role="dialog"]').last();
      await expectFullyInViewport(page, tcModal, 'SDK Version Downloader modal');
      await expect(tcModal.getByTestId('ide-sdk-version-select')).toBeVisible({ timeout: 10_000 });
      await expect(tcModal.getByTestId('ide-add-local-sdk-btn')).toBeVisible();
      const goEnvRoot = execFileSync('go', ['env', 'GOROOT'], { encoding: 'utf8' }).trim();
      if (goEnvRoot && existsSync(goEnvRoot)) {
        const regSdk = await daemon<{ sdk: { id: string; path: string; version: string; source: string } }>(
          page.request,
          'POST',
          '/ide/toolchains/local',
          { target: admTarget, ecosystem: 'go', path: goEnvRoot },
        );
        expect(regSdk.sdk.source).toBe('local');
        expect(regSdk.sdk.path).toBeTruthy();
      }
      await page.keyboard.press('Escape');
      await page.keyboard.press('Escape');
      await expect(page.locator('.sheet[role="dialog"]')).toHaveCount(0);

      // Select `Admission Server` in `ide-run-config-select` & click `Run` -> assert all 3 env layers print!
      await page.getByTestId('ide-run-config-select').click();
      const runPop = page.getByTestId('ide-run-config-popover');
      await expectFullyInViewport(page, runPop, 'Launch target popover');
      await runPop.locator('.run-item', { hasText: 'Admission Server' }).first().click();

      const runRespWait = page.waitForResponse(
        (r) => r.request().method() === 'POST' && r.url().includes('/api/v1/ide/runs'),
        { timeout: 20_000 },
      );
      await page.getByTestId('ide-run-btn').click();
      const startedRunId = ((await (await runRespWait).json()) as { run: { id: string } }).run.id;
      await expect
        .poll(
          async () => {
            const r = await daemon<{ run: { status: string; output: string[] } }>(
              page.request,
              'GET',
              `/ide/runs/${encodeURIComponent(startedRunId)}`,
            );
            return r.run.status === 'passed' ? r.run.output.join('\n') : '';
          },
          { timeout: 30_000 },
        )
        .toContain(
          `ADMISSION_OK=true SHELL=${expectedShellVal} DOTENV=dotenv-admission-ok CUSTOM=cfg-override-ok`,
        );

      // 6. Real Gutter Breakpoint + `dlv dap` Debugging on `check.go` + Inline Values + Audit (b) PNGs + 0 Leaked `dlv`!
      if (hasDlv) {
        await tabStrip.locator('.ide-tab-btn', { hasText: 'check.go' }).first().click();
        await expect(activeEditorContent(page)).toContainText('return dec');

        // Click gutter on line 23 (`return dec`) in `check.go`
        const bpGutter = page.locator('.cm-ide-bp-gutter').first();
        const ln23 = page
          .locator('.cm-lineNumbers .cm-gutterElement')
          .filter({ hasText: /^23$/ })
          .first();
        const bpBox = await bpGutter.boundingBox();
        const ln23Box = await ln23.boundingBox();
        expect(bpBox && ln23Box).toBeTruthy();
        await page.mouse.click(bpBox!.x + bpBox!.width / 2, ln23Box!.y + ln23Box!.height / 2);
        await expect(page.locator('.cm-ide-bp-marker.cm-ide-bp-enabled').first()).toBeVisible();

        const startDbgWait = page.waitForResponse(
          (r) => r.request().method() === 'POST' && r.url().includes('/api/v1/ide/debug/sessions'),
          { timeout: 25_000 },
        );
        await page.getByTestId('ide-debug-btn').click();
        const dbgSessionId = ((await (await startDbgWait).json()) as { session: IdeDebugSnapshot }).session.id;

        try {
          const dbgPanel = page.getByTestId('ide-debugger-panel');
          await expect(dbgPanel).toBeVisible({ timeout: 20_000 });
          await expect(dbgPanel).toContainText(/Paused/i, { timeout: 45_000 });
          await expect(page.locator('.cm-ide-paused-line').first()).toBeVisible({ timeout: 15_000 });

          // Assert GoLand-style inline variable values render directly inside CodeMirror!
          await expect(page.locator('.cm-ide-inline-values').first()).toBeVisible({ timeout: 15_000 });
          const inlineText = (await page.locator('.cm-ide-inline-values').allTextContents()).join(' ');
          expect(inlineText).toMatch(/allowed|shellEnv|customCfg|dec/);

          // Expand Go struct `req` in the Variables tree -> assert `TenantID` & `"casino-eu-1"`!
          const expandReqBtn = dbgPanel.getByRole('button', { name: /Expand req/i }).first();
          await expect(expandReqBtn).toBeVisible({ timeout: 15_000 });
          await expandReqBtn.click();
          await expect(dbgPanel).toContainText('TenantID', { timeout: 10_000 });
          await expect(dbgPanel).toContainText('casino-eu-1', { timeout: 10_000 });

          // Evaluate both `shellEnv` (from login shell) and `customCfg` (from Run Config) in Debug REPL!
          const replIn = dbgPanel
            .locator('input[placeholder*="Evaluate" i], input[aria-label*="Evaluate" i], form input[type="text"]')
            .last();
          await replIn.fill('shellEnv');
          await replIn.press('Enter');
          await expect(dbgPanel).toContainText(expectedShellVal, { timeout: 10_000 });
          await replIn.fill('customCfg');
          await replIn.press('Enter');
          await expect(dbgPanel).toContainText('cfg-override-ok', { timeout: 10_000 });

          // Capture Mandatory Visual Audit (b) PNGs at 2560x1440 & 1440x900 in BOTH Dark and Light mode!
          for (const vp of [
            { w: 2560, h: 1440 },
            { w: 1440, h: 900 },
          ]) {
            await page.setViewportSize({ width: vp.w, height: vp.h });
            for (const sc of ['dark', 'light'] as const) {
              await setLiveScheme(sc);
              await expectNoHorizontalOverflow(page);
              await expect(page.locator('.cm-ide-paused-line').first()).toBeVisible();
              await expect(page.locator('.cm-ide-inline-values').first()).toBeVisible();
              await saveShot(page, `audit-b-paused-go-debug-inline-${vp.w}x${vp.h}-${sc}.png`);
            }
          }
          await setLiveScheme('dark');
          await page.setViewportSize({ width: 1440, height: 900 });

          // Click caller frame (`main.main`) -> editor navigates to `main.go`, then click `EvaluateAdmission` -> back to `check.go`!
          const callerFrame = dbgPanel.locator('.dbg-frame', { hasText: /main\.main/ }).first();
          await expect(callerFrame).toBeVisible();
          await callerFrame.click();
          await expect(activeEditorContent(page)).toContainText(`os.Getenv(${JSON.stringify(shellVarKey)})`, {
            timeout: 10_000,
          });
          await dbgPanel.locator('.dbg-frame', { hasText: /EvaluateAdmission/ }).first().click();
          await expect(activeEditorContent(page)).toContainText('return dec', { timeout: 10_000 });

          // Step Over (`check.go:23` -> `main.go`) & Stop Debug Session
          await dbgPanel.getByRole('button', { name: 'Step over' }).click();
          await expect
            .poll(
              async () =>
                (
                  await daemon<{ session: IdeDebugSnapshot }>(
                    page.request,
                    'GET',
                    `/ide/debug/sessions/${dbgSessionId}`,
                  )
                ).session.state,
              { timeout: 20_000 },
            )
            .toBe('paused');
          await dbgPanel.getByRole('button', { name: 'Stop debug session' }).click();
          await expect
            .poll(
              async () =>
                (
                  await daemon<{ session: IdeDebugSnapshot }>(
                    page.request,
                    'GET',
                    `/ide/debug/sessions/${dbgSessionId}`,
                  )
                ).session.state,
              { timeout: 15_000 },
            )
            .toBe('terminated');
        } finally {
          await daemon(page.request, 'POST', `/ide/debug/sessions/${dbgSessionId}/command`, {
            action: 'stop',
          }).catch(() => {});
        }

        // Assert ZERO leaked `dlv` processes referencing `admRoot`!
        await expect
          .poll(() => {
            const psOut = execFileSync('ps', ['-axo', 'command='], { encoding: 'utf8' });
            return psOut
              .split('\n')
              .filter((ln) => ln.includes(admRoot) && /\bdlv\b|__debug_bin/.test(ln)).length;
          })
          .toBe(0);
      }

      // 7. Real "Dive In" Navigation: `main.go` -> `check.go` AND `fmt.Printf` -> `$GOROOT/src/fmt/print.go` (Go SDK + Read-only) + `Cmd+[` Back + Find Usages!
      await tabStrip.locator('.ide-tab-btn', { hasText: 'main.go' }).first().click();
      const mainContent = readFileSync(join(admRoot, 'cmd', 'server', 'main.go'), 'utf8');

      // Dive 1: Place caret on `EvaluateAdmission` in `main.go` (line 15) and press `F12`!
      const evalLineEl = page
        .locator('.cm-editor .cm-content .cm-line')
        .filter({ hasText: 'EvaluateAdmission' })
        .first();
      await expect(evalLineEl).toBeVisible();
      const evalBox = await evalLineEl.boundingBox();
      expect(evalBox).toBeTruthy();
      // Click on `EvaluateAdmission` line (auto-folded `import (…)` collapses lines 3-7) and press F12
      await page.mouse.click(evalBox!.x + 8, evalBox!.y + evalBox!.height / 2);
      await page.keyboard.press('Home');
      for (let right = 0; right < 22; right++) await page.keyboard.press('ArrowRight');
      await page.keyboard.press('F12');
      await expect(activeEditorContent(page)).toContainText('func EvaluateAdmission', { timeout: 15_000 });

      // Navigate Back via `ide-nav-back` (`Cmd+[`) -> returns to `main.go`!
      await expect(page.getByTestId('ide-nav-back')).toBeEnabled();
      await page.getByTestId('ide-nav-back').click();
      await expect(activeEditorContent(page)).toContainText(`os.Getenv("${shellVarKey}")`, {
        timeout: 10_000,
      });

      // Dive 2: Jump into Go stdlib `$GOROOT/src/fmt/print.go` (`fmt.Printf` on line 16) -> verify `Go SDK` + `Read-only` badges!
      const stdlibDef = await daemon<{
        symbol: string;
        locations: Array<{ path: string; line: number; column: number }>;
      }>(page.request, 'POST', '/ide/lsp/definition', {
        target: admTarget,
        path: join(admRoot, 'cmd', 'server', 'main.go'),
        line: 16,
        column: 8,
        symbol: 'Printf',
        content: mainContent,
      });
      expect(stdlibDef.locations.length).toBeGreaterThan(0);
      expect(stdlibDef.locations[0].path).toMatch(/\/src\/fmt\/print\.go$/);

      // Trigger real F12 jump on `Printf` in CodeMirror!
      const printfLineEl = page
        .locator('.cm-editor .cm-content .cm-line')
        .filter({ hasText: 'fmt.Printf' })
        .first();
      await expect(printfLineEl).toBeVisible();
      const printfBox = await printfLineEl.boundingBox();
      expect(printfBox).toBeTruthy();
      await page.mouse.click(printfBox!.x + 8, printfBox!.y + printfBox!.height / 2);
      await page.keyboard.press('Home');
      for (let right = 0; right < 7; right++) await page.keyboard.press('ArrowRight');
      await page.keyboard.press('F12');

      const subbar = page.locator('.ide-subbar');
      await expect(subbar).toContainText('Go SDK', { timeout: 15_000 });
      await expect(subbar).toContainText('Read-only', { timeout: 15_000 });
      await expect(activeEditorContent(page)).toContainText('func Printf(', { timeout: 15_000 });

      // Navigate Back (`Cmd+[`) from `$GOROOT/src/fmt/print.go` -> returns directly to `main.go`!
      await page.getByTestId('ide-nav-back').click();
      await expect(activeEditorContent(page)).toContainText(`os.Getenv("${shellVarKey}")`, {
        timeout: 10_000,
      });

      // Find Usages (`Alt+F7`) on `EvaluateAdmission` -> shows usages across `check.go`, `check_test.go`, `main.go`!
      const refsResp = await daemon<{ symbol: string; locations: Array<{ path: string; line: number }> }>(
        page.request,
        'POST',
        '/ide/lsp/references',
        {
          target: admTarget,
          path: join(admRoot, 'internal', 'admission', 'check.go'),
          line: 15,
          column: 7,
          symbol: 'EvaluateAdmission',
        },
      );
      expect(refsResp.locations.length).toBeGreaterThanOrEqual(2);
    } finally {
      for (const id of createdWsIds) {
        await page.request
          .fetch(`${base}/api/v1/workspaces/${id}`, {
            method: 'DELETE',
            headers: { Authorization: `Bearer ${token}` },
          })
          .catch(() => {});
      }
      rmSync(admParent, { recursive: true, force: true });
    }
  });

  test('12. 8-language deep support: Shell source dive & bash DAP debugger, JSON outline/JSONPath/transforms/$ref, CSV virtualized Table Grid + Rainbow columns, and node_modules/Python stdlib drill-down', async ({
    page,
  }) => {
    test.setTimeout(120_000);
    await ensureSeeded();

    // Seed Shell, JSON ($ref), CSV (50 rows), and node_modules `.d.ts` fixtures in `workspaceRoot`
    mkdirSync(join(workspaceRoot, 'scripts'), { recursive: true });
    mkdirSync(join(workspaceRoot, 'schemas'), { recursive: true });
    mkdirSync(join(workspaceRoot, 'data'), { recursive: true });
    mkdirSync(join(workspaceRoot, 'node_modules', 'mini-pkg'), { recursive: true });

    writeFileSync(
      join(workspaceRoot, 'scripts', 'lib.sh'),
      ['#!/usr/bin/env bash', 'validate_env() {', '  local tag="$1"', '  echo "ok:$tag"', '}', ''].join('\n'),
    );
    const deployShPath = join(workspaceRoot, 'scripts', 'deploy.sh');
    writeFileSync(
      deployShPath,
      [
        '#!/usr/bin/env bash',
        'source ./lib.sh',
        'count=41',
        'count=$((count + 1))',
        'validate_env "$count"',
        '',
      ].join('\n'),
    );

    writeFileSync(
      join(workspaceRoot, 'schemas', 'base.json'),
      JSON.stringify({ $id: 'base', type: 'object', required: ['tenant'] }, null, 2) + '\n',
    );
    writeFileSync(
      join(workspaceRoot, 'schemas', 'admission.json'),
      JSON.stringify(
        {
          $ref: './base.json',
          zed: 99,
          alpha: { nested: [10, 20], enabled: true },
        },
        null,
        2,
      ) + '\n',
    );

    const csvLines = ['tenant_id,region,quota_rps,tier'];
    for (let i = 1; i <= 50; i++) {
      csvLines.push(`tenant_${String(i).padStart(2, '0')},${i % 2 === 0 ? 'eu-west' : 'us-east'},${i * 10},${i % 3 === 0 ? 'vip' : 'std'}`);
    }
    writeFileSync(join(workspaceRoot, 'data', 'tenants.csv'), csvLines.join('\n') + '\n');

    writeFileSync(
      join(workspaceRoot, 'node_modules', 'mini-pkg', 'package.json'),
      JSON.stringify({ name: 'mini-pkg', version: '1.0.0', types: 'index.d.ts' }, null, 2) + '\n',
    );
    writeFileSync(
      join(workspaceRoot, 'node_modules', 'mini-pkg', 'index.d.ts'),
      'export declare function parseToken(raw: string): boolean;\n',
    );
    const authTsPath = join(workspaceRoot, 'web', 'auth.ts');
    writeFileSync(
      authTsPath,
      ["import { parseToken } from 'mini-pkg';", '', "export const valid = parseToken('tok_123');", ''].join('\n'),
    );

    await bootIdePage(page);

    // A. Shell (`.sh`): `Cmd+F12` Move-to-Function, `source ./lib.sh` Dive-In, Gutter Run & Real Bash DAP Debug!
    await openFileFromTree(page, 'scripts', 'lib.sh');
    await page.getByTestId('ide-structure-btn').click();
    const structModal = page.getByTestId('ide-file-structure-modal');
    await expect(structModal).toBeVisible();
    await expect(structModal).toContainText('validate_env');
    await page.keyboard.press('Escape');

    await openFileFromTree(page, 'scripts', 'deploy.sh');
    await expect(activeEditorContent(page)).toContainText('source ./lib.sh');
    // Dive from `source ./lib.sh` (line 2) into `scripts/lib.sh` via F12!
    await activeEditorContent(page).click();
    await page.keyboard.press('Meta+ArrowUp');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('End');
    await page.keyboard.press('F12');
    await expect(activeEditorContent(page)).toContainText('validate_env()', { timeout: 10_000 });
    await page.getByTestId('ide-nav-back').click();
    await expect(activeEditorContent(page)).toContainText('source ./lib.sh', { timeout: 10_000 });

    // Real Bash DAP debugger (`otto-sh-dap` / `shell_bridge`): pause on line 5 (`validate_env "$count"`), inspect `count` == 42!
    const shDbg = await daemon<{ session: IdeDebugSnapshot }>(page.request, 'POST', '/ide/debug/sessions', {
      target: localTarget(),
      language: 'shell',
      launch: { program: deployShPath, cwd: workspaceRoot },
      breakpoints: [{ path: deployShPath, line: 5, enabled: true }],
    });
    const shDbgId = shDbg.session.id;
    try {
      await expect
        .poll(
          async () =>
            (await daemon<{ session: IdeDebugSnapshot }>(page.request, 'GET', `/ide/debug/sessions/${shDbgId}`))
              .session.state,
          { timeout: 15_000 },
        )
        .toBe('paused');
      const shSnap = (
        await daemon<{ session: IdeDebugSnapshot }>(page.request, 'GET', `/ide/debug/sessions/${shDbgId}`)
      ).session;
      expect(shSnap.paused_line).toBe(5);
      const localsScope = shSnap.scopes?.[0];
      expect(localsScope).toBeTruthy();
      const shVars = await daemon<{ variables: Array<{ name: string; value: string }> }>(
        page.request,
        'POST',
        `/ide/debug/sessions/${shDbgId}/variables`,
        { variables_reference: localsScope!.variables_reference },
      );
      expect(shVars.variables.some((v) => v.name === 'count' && /42/.test(v.value))).toBe(true);
    } finally {
      await daemon(page.request, 'POST', `/ide/debug/sessions/${shDbgId}/command`, { action: 'stop' }).catch(
        () => {},
      );
    }
    await expect
      .poll(() => {
        const psOut = execFileSync('ps', ['-axo', 'command='], { encoding: 'utf8' });
        return psOut.split('\n').filter((ln) => ln.includes('otto-sh-dap') && ln.includes(deployShPath)).length;
      })
      .toBe(0);

    // B. JSON (`.json`): `Cmd+F12` key outline, live JSONPath breadcrumb, Pretty/Minify/Sort keys & `$ref` Dive-In!
    await openFileFromTree(page, 'schemas', 'admission.json');
    await expect(activeEditorContent(page)).toContainText('"nested"');
    await page.getByTestId('ide-structure-btn').click();
    await expect(page.getByTestId('ide-file-structure-modal')).toContainText('alpha');
    await expect(page.getByTestId('ide-file-structure-modal')).toContainText('nested');
    await page.getByTestId('ide-file-structure-modal').getByRole('option', { name: /nested/i }).first().click();
    await expect(page.getByTestId('ide-jsonpath-crumb')).toContainText('$.alpha');
    await page.keyboard.press('ArrowDown');
    await expect(page.getByTestId('ide-jsonpath-crumb')).toContainText('$.alpha.nested');

    // Sort keys -> `"alpha"` appears before `"zed"`; Minify -> single line; Pretty -> multi-line
    await page.locator('.ide-subbar').getByRole('button', { name: 'Sort keys' }).click();
    await expect
      .poll(async () => {
        const txt = (await activeEditorContent(page).textContent()) ?? '';
        return txt.indexOf('"alpha"') < txt.indexOf('"zed"');
      })
      .toBe(true);
    await page.locator('.ide-subbar').getByRole('button', { name: 'Minify' }).click();
    await expect(page.locator('.cm-editor .cm-content .cm-line')).toHaveCount(1);
    await page.locator('.ide-subbar').getByRole('button', { name: 'Pretty' }).click();
    await expect
      .poll(async () => page.locator('.cm-editor .cm-content .cm-line').count())
      .toBeGreaterThan(4);

    // `$ref` jump (`"$ref": "./base.json"` on line 2) -> opens `schemas/base.json`!
    await activeEditorContent(page).click();
    await page.keyboard.press('Meta+ArrowUp');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('End');
    await page.keyboard.press('ArrowLeft');
    await page.keyboard.press('ArrowLeft');
    await page.keyboard.press('F12');
    await expect(activeEditorContent(page)).toContainText('"$id": "base"', { timeout: 10_000 });

    // C. CSV (`.csv`): DataGrip virtualized Table Grid, stats, filter, sort, cell edit + Undo, +Row/+Col & Text Rainbow Columns!
    await openFileFromTree(page, 'data', 'tenants.csv');
    const csvGrid = page.getByTestId('ide-csv-view');
    await expect(csvGrid).toBeVisible({ timeout: 15_000 });
    await expect(csvGrid).toContainText('50 rows · 4 cols');

    // Click `quota_rps` header to sort & show numeric aggregate stats (`min=10`, `max=500`, `sum=12750`, `avg=255`)
    await csvGrid.getByRole('button', { name: 'quota_rps', exact: true }).click();
    const statsBar = page.getByTestId('ide-csv-stats');
    await expect(statsBar).toContainText('quota_rps:');
    await expect(statsBar).toContainText('min=10');
    await expect(statsBar).toContainText('max=500');

    // Filter rows (`tenant_42`), then clear
    await page.getByTestId('ide-csv-filter-input').fill('tenant_42');
    await expect(csvGrid).toContainText('1 row · 4 cols');
    await page.getByTestId('ide-csv-filter-input').fill('');
    await expect(csvGrid).toContainText('50 rows · 4 cols');

    // Inline cell edit + `Undo table edit` (`⌘Z`), then `+ Row` & `+ Column`
    const firstCell = csvGrid.locator('.csv-cell-btn').first();
    await firstCell.dblclick();
    const cellInput = csvGrid.locator('.csv-cell-input').first();
    await expect(cellInput).toBeVisible();
    await cellInput.fill('edited_tenant');
    await cellInput.press('Enter');
    await expect(csvGrid).toContainText('edited_tenant');
    await csvGrid.getByRole('button', { name: 'Undo table edit' }).click();
    await expect(csvGrid).not.toContainText('edited_tenant');

    await page.getByTestId('ide-csv-add-row').click();
    await expect(csvGrid).toContainText('51 rows · 4 cols');
    await page.getByTestId('ide-csv-add-col').click();
    await expect(csvGrid).toContainText('51 rows · 5 cols');

    // Switch to `Text` mode -> verify CodeMirror Rainbow Column Highlighting (`.cm-ide-csv-col-0..2`)!
    await page.getByTestId('ide-csv-mode-toggle').getByRole('button', { name: 'Text' }).click();
    await expect(page.locator('.cm-ide-csv-col-0').first()).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('.cm-ide-csv-col-1').first()).toBeVisible();
    await expect(page.locator('.cm-ide-csv-col-2').first()).toBeVisible();
    await page.getByTestId('ide-csv-mode-toggle').getByRole('button', { name: 'Table' }).click();
    await expect(csvGrid).toBeVisible();

    // D. JS/TS `node_modules` `.d.ts` Dive-In (`node_modules` + `Read-only` badges) & Python stdlib resolution!
    const tsDef = await daemon<{ locations: Array<{ path: string; line: number }> }>(
      page.request,
      'POST',
      '/ide/lsp/definition',
      {
        target: localTarget(),
        path: authTsPath,
        line: 3,
        column: 24,
        symbol: 'parseToken',
        content: readFileSync(authTsPath, 'utf8'),
      },
    );
    expect(tsDef.locations.some((l) => l.path.endsWith('node_modules/mini-pkg/index.d.ts'))).toBe(true);

    await openFileFromTree(page, 'web', 'auth.ts');
    await activeEditorContent(page).click();
    await page.keyboard.press('Meta+ArrowUp');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Home');
    for (let r = 0; r < 24; r++) await page.keyboard.press('ArrowRight');
    await page.keyboard.press('F12');
    await expect(page.locator('.ide-subbar')).toContainText('node_modules', { timeout: 10_000 });
    await expect(page.locator('.ide-subbar')).toContainText('Read-only');
    await expect(activeEditorContent(page)).toContainText('export declare function parseToken');
  });

  test('13. Mandatory 2560x1440 & 1440x900 Dark + Light Visual PNG Audit across multi-root main view, Run/Debug Configs, SDK Downloader, Unified Threads Agent Dock & CSV Grid', async ({
    page,
  }) => {
    test.setTimeout(150_000);
    await ensureSeeded();

    // Ensure an attached project folder + CSV fixture exist so main view shows multi-root tree + multiple open tabs!
    const extraAuditFolder = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-audit-kit-')));
    writeFileSync(join(extraAuditFolder, 'shared_audit.go'), 'package shared\n\nconst AuditReady = true\n');
    mkdirSync(join(workspaceRoot, 'data'), { recursive: true });
    if (!existsSync(join(workspaceRoot, 'data', 'tenants.csv'))) {
      writeFileSync(
        join(workspaceRoot, 'data', 'tenants.csv'),
        'tenant_id,region,quota_rps,tier\ntenant_01,eu-west,250,vip\ntenant_02,us-east,120,std\n',
      );
    }

    const setLiveScheme = async (scheme: 'dark' | 'light'): Promise<void> => {
      await page.evaluate(() => {
        localStorage.setItem('otto_scheme', 'auto');
      });
      await page.emulateMedia({ colorScheme: scheme });
      await expect
        .poll(() => page.evaluate(() => document.documentElement.dataset.scheme))
        .toBe(scheme);
      // Wait for active CSS transitions (`--dur-fast` / `--dur-enter`) to finish settling cleanly.
      await expect
        .poll(() =>
          page.evaluate(() =>
            document.getAnimations().every((a) => a.playState !== 'running'),
          ),
        )
        .toBe(true);
    };

    try {
      await daemon(page.request, 'POST', '/ide/projects/folders', {
        target: localTarget(),
        folder_path: extraAuditFolder,
      });

      await page.emulateMedia({ colorScheme: 'dark' });
      await bootIdePage(page, { scheme: 'auto' as 'dark', direction: 'ltr' });
      await openFileFromTree(page, 'cmd', 'demo', 'main.go');
      await openFileFromTree(page, 'calc', 'calc.go');
      await openFileFromTree(page, 'calc', 'calc_test.go');

      const viewports = [
        { w: 2560, h: 1440 },
        { w: 1440, h: 900 },
      ] as const;

      // (a) IDE main view with multi-root Project Tree + multiple open tabs + workspace dropdown open
      for (const vp of viewports) {
        await page.setViewportSize({ width: vp.w, height: vp.h });
        for (const sc of ['dark', 'light'] as const) {
          await setLiveScheme(sc);
          await page.getByTestId('ide-target-switcher').click();
          const wsPop = page.getByTestId('ide-workspace-popover');
          await expectFullyInViewport(page, wsPop, `Audit (a) workspace popover ${vp.w}x${vp.h} ${sc}`);
          await expectNoHorizontalOverflow(page);
          await saveShot(page, `audit-a-main-multiroot-switcher-${vp.w}x${vp.h}-${sc}.png`);
          await page.keyboard.press('Escape');
          await expect(wsPop).toBeHidden();
        }
      }

      // (c) Edit Run/Debug Configurations modal showing Env Vars & Shell Env + SDK selector
      await page.getByTestId('ide-edit-configs-btn').click();
      const envModal = page.locator('.sheet[role="dialog"]').last();
      await expect(envModal).toBeVisible();
      for (const vp of viewports) {
        await page.setViewportSize({ width: vp.w, height: vp.h });
        for (const sc of ['dark', 'light'] as const) {
          await setLiveScheme(sc);
          await expectFullyInViewport(page, envModal, `Audit (c) run configs modal ${vp.w}x${vp.h} ${sc}`);
          await saveShot(page, `audit-c-run-debug-configs-shellenv-${vp.w}x${vp.h}-${sc}.png`);
        }
      }

      // (d) SDK Version Downloader modal opened directly from `SDKs…` inside `Environments & run configurations`
      await envModal.getByTestId('ide-cfg-open-toolchains').click();
      const tcModal = page.locator('.sheet[role="dialog"]').last();
      await expect(tcModal.getByTestId('ide-sdk-version-select')).toBeVisible({ timeout: 10_000 });
      for (const vp of viewports) {
        await page.setViewportSize({ width: vp.w, height: vp.h });
        for (const sc of ['dark', 'light'] as const) {
          await setLiveScheme(sc);
          await expectFullyInViewport(page, tcModal, `Audit (d) SDK downloader modal ${vp.w}x${vp.h} ${sc}`);
          await saveShot(page, `audit-d-sdk-version-downloader-${vp.w}x${vp.h}-${sc}.png`);
        }
      }
      await page.keyboard.press('Escape');
      await page.keyboard.press('Escape');
      await expect(page.locator('.sheet[role="dialog"]')).toHaveCount(0);

      // (e) Unified Threads AI Agent Dock with provider/model/thread pickers
      await daemon(page.request, 'POST', `/workspaces/${workspaceId}/sessions`, {
        kind: 'agent',
        provider: 'agy',
        title: 'Admission Auth Service Refactor',
        cwd: workspaceRoot,
        meta: { surface: 'threads', origin: 'e2e' },
      });
      await daemon(page.request, 'POST', `/workspaces/${workspaceId}/sessions`, {
        kind: 'agent',
        provider: 'claude',
        title: 'Debug Delve Env Inheritance',
        cwd: workspaceRoot,
        meta: { surface: 'threads', origin: 'e2e' },
      });
      await page.getByTestId('ide-toggle-agent').click();
      const agentPanel = page.getByTestId('ide-agent-panel');
      await expect(agentPanel).toBeVisible({ timeout: 15_000 });
      await page.getByTestId('ide-agent-new').click();
      await agentPanel.getByRole('button', { name: /Include active file|Attach active file/i }).first().click();
      for (const vp of viewports) {
        await page.setViewportSize({ width: vp.w, height: vp.h });
        for (const sc of ['dark', 'light'] as const) {
          await setLiveScheme(sc);
          await page.getByTestId('ide-agent-provider').click();
          const mp = page.getByTestId('model-picker');
          await expectFullyInViewport(page, mp, `Audit (e) model picker ${vp.w}x${vp.h} ${sc}`);
          await saveShot(page, `audit-e-unified-threads-agent-dock-${vp.w}x${vp.h}-${sc}.png`);
          await page.keyboard.press('Escape');
          await expect(mp).toBeHidden();
        }
      }
      await page.getByTestId('ide-agent-close').click();

      // (f) CSV virtualized Table Grid at 2560x1440 & 1440x900 Dark + Light
      await openFileFromTree(page, 'data', 'tenants.csv');
      await expect(page.getByTestId('ide-csv-view')).toBeVisible({ timeout: 15_000 });
      for (const vp of viewports) {
        await page.setViewportSize({ width: vp.w, height: vp.h });
        for (const sc of ['dark', 'light'] as const) {
          await setLiveScheme(sc);
          await expectNoHorizontalOverflow(page);
          await saveShot(page, `audit-f-csv-table-grid-${vp.w}x${vp.h}-${sc}.png`);
        }
      }
    } finally {
      await daemon(page.request, 'DELETE', `/ide/projects/folders?kind=local&root=${encodeURIComponent(workspaceRoot)}&folder_path=${encodeURIComponent(extraAuditFolder)}`).catch(() => {});
      rmSync(extraAuditFolder, { recursive: true, force: true });
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// SUITE 2: Responsive iPad & iPhone Verification (Zero Overflow + Axe + Shots)
// ─────────────────────────────────────────────────────────────────────────────

test.describe('Responsive Mobile & Tablet IDE Verification', () => {
  test.describe.configure({ mode: 'serial' });

  test.beforeAll(async () => {
    await ensureSeeded();
  });

  test('responsive layout, zero horizontal overflow & axe clean on iPad and iPhone', async ({
    page,
  }, info) => {
    test.skip(
      !['ipad-portrait', 'ipad-landscape', 'iphone-portrait'].includes(info.project.name),
      'Scoped to representative iPad portrait/landscape and iPhone portrait profiles',
    );
    test.setTimeout(60_000);

    await bootIdePage(page, { scheme: 'dark', direction: 'ltr' });
    // Verify IDE auto-opens an initial workspace entry file rather than an empty pane
    await expect(page.locator('.cm-editor').first()).toBeVisible({ timeout: 15_000 });
    await expectNoHorizontalOverflow(page);
    await expectContentHasHeight(page, 200);
    await expectAccessible(page, { key: 'ide' });
    await saveShot(page, `15-ide-${info.project.name}.png`);
  });
});
