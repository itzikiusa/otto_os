import { test, expect, type Page, type Locator, type APIRequestContext } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import {
  expectAccessible,
  expectContentHasHeight,
  expectFullyInViewport,
  expectNoHorizontalOverflow,
} from './helpers';

// Otto IDE Round 2 — Adversarial Agent, Multi-Language Runtime & PTY E2E Suite
// Proves real ottod + otto-fake-claude + Python (unittest + bdb) + Node.js
// (node:test + V8 Inspector) + Go + live PTY shell end-to-end:
//   1. Side-by-side Agent (⌘L), 4 context sources (@selection, ⚠, ✗, ⏸), diff, Apply/⌘Z/⌘S
//   2. Inline Selection Edit (⌘I / IdeInlinePromptBar), diff preview, Accept & ⌘Z
//   3. Multi-language runs & live debugging (Python bdb + Node V8 Inspector)
//   4. Live Workspace Terminal PTY (IdeTerminalPanel): spawn, I/O, axe a11y & kill

const SCREENSHOT_DIR =
  '/Users/itziklavon/.gemini/antigravity-cli/brain/2fb92e16-6abc-4a33-985b-8e29db6f04eb/scratch/screenshots';

interface IdeTargetSpec {
  kind: 'local' | 'ssh';
  workspace_id?: string | null;
  root: string;
  connection_id?: string | null;
}

interface IdeDebugSnapshot {
  id: string;
  state: string;
  paused_file?: string | null;
  paused_line?: number | null;
  stack_frames?: Array<{ id: number; name: string; path?: string | null; line: number }>;
  scopes?: Array<{ name: string; variables_reference: number }>;
}

const INITIAL_CALC_GO =
  'package calc\n\nfunc Add(a, b int) int {\n\tsum := a + b\n\treturn sum\n}\n\nfunc Multiply(a, b int) int {\n\tprod := a * b\n\treturn prod\n}\n';

let base = '';
let token = '';
let workspaceId = '';
let workspaceRoot = '';
let cfgBrokenLabel = '';

function seedPolyglotRepo(dir: string): void {
  for (const sub of ['calc', 'pyapp', 'web']) mkdirSync(join(dir, sub), { recursive: true });

  writeFileSync(join(dir, 'go.mod'), 'module example.com/ottoider2\n\ngo 1.21\n');
  writeFileSync(join(dir, 'calc', 'calc.go'), INITIAL_CALC_GO);
  writeFileSync(
    join(dir, 'calc', 'calc_test.go'),
    [
      'package calc',
      '',
      'import "testing"',
      '',
      'func TestAdd(t *testing.T) {',
      '\tif got := Add(20, 22); got != 42 { t.Fatalf("Add(20, 22) = %d, want 42", got) }',
      '}',
      '',
      'func TestBroken(t *testing.T) {',
      '\tif got := Add(20, 20); got != 42 { t.Fatalf("TestBroken: got %d, want 42", got) }',
      '}',
      '',
    ].join('\n'),
  );

  writeFileSync(join(dir, 'pyapp', '__init__.py'), '');
  writeFileSync(join(dir, 'pyapp', 'calc.py'), 'def compute_answer(a: int, b: int) -> int:\n    return a + b\n');
  writeFileSync(
    join(dir, 'pyapp', 'test_calc.py'),
    [
      'import unittest',
      'from pyapp.calc import compute_answer',
      '',
      'class TestPythonCalc(unittest.TestCase):',
      '    def test_compute_answer(self) -> None:',
      '        self.assertEqual(compute_answer(20, 22), 42)',
      '',
      'if __name__ == "__main__":',
      '    unittest.main()',
      '',
    ].join('\n'),
  );
  writeFileSync(
    join(dir, 'pyapp', 'test_broken.py'),
    'import unittest\nfrom pyapp.calc import compute_answer\n\nclass TestBrokenPy(unittest.TestCase):\n    def test_broken_sum(self) -> None:\n        self.assertEqual(compute_answer(20, 20), 42)\n',
  );
  // Line 7 is `scaled = result * 2` (right after `result = compute_answer(20, 22)` on line 6).
  writeFileSync(
    join(dir, 'pyapp', 'calc_run.py'),
    [
      'def compute_answer(a: int, b: int) -> int:',
      '    return a + b',
      '',
      '',
      'def run_pipeline() -> int:',
      '    result = compute_answer(20, 22)',
      '    scaled = result * 2',
      '    return scaled',
      '',
      '',
      'if __name__ == "__main__":',
      '    print("pipeline:", run_pipeline())',
      '',
    ].join('\n'),
  );

  writeFileSync(join(dir, 'package.json'), '{"name":"otto-ide-r2-polyglot","version":"1.0.0","type":"module"}\n');
  writeFileSync(join(dir, 'web', 'calc.mjs'), 'export function combineTotals(left, right) {\n  return left + right;\n}\n');
  writeFileSync(
    join(dir, 'web', 'calc.test.mjs'),
    "import test from 'node:test';\nimport assert from 'node:assert/strict';\nimport { combineTotals } from './calc.mjs';\n\ntest('combineTotals sums values accurately', () => {\n  assert.equal(combineTotals(20, 22), 42);\n});\n",
  );
  // Line 5 is `const boosted = subtotal + 8;` (right after `const subtotal = combineTotals(20, 22);` on line 4).
  writeFileSync(
    join(dir, 'web', 'debug_demo.mjs'),
    [
      "import { combineTotals } from './calc.mjs';",
      '',
      'function runNodeDemo() {',
      '  const subtotal = combineTotals(20, 22);',
      '  const boosted = subtotal + 8;',
      '  return boosted;',
      '}',
      '',
      "console.log('node-demo:', runNodeDemo());",
      '',
    ].join('\n'),
  );

  try {
    for (const argv of [
      ['init', '-b', 'main'],
      ['config', 'user.email', 'ide-r2@otto.local'],
      ['config', 'user.name', 'Otto IDE R2'],
      ['add', '.'],
      ['commit', '-m', 'seed polyglot r2 repo'],
    ]) {
      execFileSync('git', argv, { cwd: dir, stdio: 'ignore' });
    }
  } catch { /* Git metadata is optional */ }
}

function localTarget(): IdeTargetSpec {
  return { kind: 'local', workspace_id: workspaceId, root: workspaceRoot, connection_id: null };
}

async function daemon<T>(ctx: APIRequestContext, method: 'GET' | 'POST' | 'PUT' | 'DELETE', path: string, data?: unknown): Promise<T> {
  const r = await ctx.fetch(`${base}/api/v1${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    data: data === undefined ? undefined : JSON.stringify(data),
  });
  const txt = await r.text();
  expect(r.ok(), `${method} ${path} -> ${r.status()} ${txt}`).toBeTruthy();
  return (txt ? JSON.parse(txt) : undefined) as T;
}

async function ensureSeeded(): Promise<void> {
  if (workspaceId && workspaceRoot && existsSync(workspaceRoot)) return;
  workspaceRoot = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-r2-')));
  seedPolyglotRepo(workspaceRoot);

  const c = await apiCtx();
  base = c.base;
  token = c.token;

  const wsResp = await c.ctx.post(`${base}/api/v1/workspaces`, {
    data: { name: 'IDE Agent Runtimes Suite', root_path: workspaceRoot },
  });
  expect(wsResp.ok(), `create workspace failed: ${await wsResp.text()}`).toBeTruthy();
  workspaceId = ((await wsResp.json()) as { id: string }).id;

  const extraPath = [join(homedir(), 'go', 'bin'), '/opt/homebrew/bin', '/usr/local/go/bin', '/usr/local/bin']
    .filter(existsSync)
    .join(':');
  await daemon(c.ctx, 'PUT', '/ide/environments', { target: localTarget(), extra_path: extraPath, env_vars: [] });

  const hasGo = ['/opt/homebrew/bin/go', '/usr/local/go/bin/go', '/usr/local/bin/go'].some(existsSync);
  const saveCfg = async (body: Record<string, unknown>): Promise<void> => {
    await daemon(c.ctx, 'POST', '/ide/run-configs', { target: localTarget(), ...body });
  };

  cfgBrokenLabel = hasGo ? 'Go Failing Check' : 'Python Failing Check';
  await saveCfg(
    hasGo
      ? { name: cfgBrokenLabel, runner: 'go_test', command: 'go', args: ['test', '-v', '-run', '^TestBroken$', './calc'], cwd: workspaceRoot, target_path: join(workspaceRoot, 'calc', 'calc_test.go') }
      : { name: cfgBrokenLabel, runner: 'pytest', command: 'python3', args: ['-m', 'unittest', '-v', 'pyapp.test_broken'], cwd: workspaceRoot, target_path: join(workspaceRoot, 'pyapp', 'test_broken.py') },
  );

  await saveCfg({
    name: 'Python Unit Suite',
    runner: 'pytest',
    command: 'python3',
    args: ['-m', 'unittest', '-v', 'pyapp.test_calc'],
    cwd: workspaceRoot,
    target_path: join(workspaceRoot, 'pyapp', 'test_calc.py'),
  });

  await saveCfg({
    name: 'Node Test Suite',
    runner: 'node',
    command: 'node',
    args: ['--test', 'web/calc.test.mjs'],
    cwd: workspaceRoot,
    target_path: join(workspaceRoot, 'web', 'calc.test.mjs'),
  });

  await saveCfg({
    name: 'Node Debug Demo',
    runner: 'node',
    command: 'node',
    args: [],
    cwd: workspaceRoot,
    target_path: join(workspaceRoot, 'web', 'debug_demo.mjs'),
  });

  await c.ctx.dispose();
}

async function saveShot(page: Page, name: string): Promise<void> {
  mkdirSync(SCREENSHOT_DIR, { recursive: true });
  await page.screenshot({ path: join(SCREENSHOT_DIR, name), fullPage: false });
}

async function bootIdePage(page: Page): Promise<void> {
  await page.addInitScript(
    ({ wsId }) => {
      (window as unknown as { __ottoTermProbe: unknown[] }).__ottoTermProbe = [];
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_scheme', 'dark');
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.agent.provider', 'claude');
      localStorage.removeItem('otto.ide.agentOpen');
    },
    { wsId: workspaceId },
  );
  await page.goto('/#/ide');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
}

async function openFileFromTree(page: Page, ...segments: string[]): Promise<void> {
  const tree = page.getByTestId('ide-project-tree');
  await expect(tree).toBeVisible({ timeout: 15_000 });
  const filesTab = tree.getByRole('tab', { name: /^Files$/i });
  if ((await filesTab.count()) > 0 && (await filesTab.first().getAttribute('aria-selected')) !== 'true') {
    await filesTab.first().click();
  }
  for (let i = 0; i < segments.length; i++) {
    const relPath = segments.slice(0, i + 1).join('/');
    const item = tree
      .locator(`button.tree-item[title="${relPath}"], button.tree-item[title$="/${relPath}"]`)
      .first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    if (i === segments.length - 1) {
      await item.dblclick();
    } else if ((await item.getAttribute('aria-expanded')) !== 'true') {
      await item.click();
    }
  }
}

function activeEditor(page: Page): Locator {
  return page.locator('.ide-editor-area .cm-editor .cm-content').first();
}

/** Select 1-based inclusive line range `[startLine, endLine]` in CodeMirror via keyboard. */
async function selectEditorLines(page: Page, startLine: number, endLine: number): Promise<void> {
  const editor = activeEditor(page);
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press('Meta+ArrowUp');
  for (let i = 1; i < startLine; i++) await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Meta+ArrowLeft');
  for (let i = startLine; i < endLine; i++) await page.keyboard.press('Shift+ArrowDown');
  await page.keyboard.press('Shift+Meta+ArrowRight');
}

/** Toggle a breakpoint on 1-based `line` by clicking `.cm-ide-bp-gutter`. */
async function toggleGutterBreakpoint(page: Page, line: number): Promise<void> {
  const bpGutter = page.locator('.cm-ide-bp-gutter').first();
  await expect(bpGutter).toBeVisible();
  const lineEl = page
    .locator('.cm-lineNumbers .cm-gutterElement')
    .filter({ hasText: new RegExp(`^${line}$`) })
    .first();
  await expect(lineEl).toBeVisible();
  const bpBox = await bpGutter.boundingBox();
  const lnBox = await lineEl.boundingBox();
  expect(bpBox && lnBox).toBeTruthy();
  await page.mouse.click(bpBox!.x + bpBox!.width / 2, lnBox!.y + lnBox!.height / 2);
  await expect(page.locator('.cm-ide-bp-marker').first()).toBeVisible();
}

test.describe('Desktop IDE Round 2 — Agent Dock, Inline Edit, Multi-Language Debug & Live PTY', () => {
  test.use({
    viewport: { width: 1440, height: 900 },
    isMobile: false,
    hasTouch: false,
    deviceScaleFactor: 2,
    userAgent:
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36',
  });

  test.beforeAll(async () => {
    await ensureSeeded();
  });

  test.beforeEach(async ({}, info) => {
    test.skip(
      !['desktop-browser', 'android-phone'].includes(info.project.name),
      'Desktop IDE agent & runtimes suite runs on Chromium desktop viewport',
    );
  });

  test('1. side-by-side AI Agent (⌘L), all 4 context chips (@selection, ⚠ problem, ✗ failed test, ⏸ debug frame), diff preview, Apply, ⌘Z Undo & ⌘S save', async ({
    page,
  }) => {
    test.setTimeout(90_000);
    const calcGoPath = join(workspaceRoot, 'calc', 'calc.go');
    writeFileSync(calcGoPath, INITIAL_CALC_GO);

    await bootIdePage(page);
    await expectNoHorizontalOverflow(page);
    await expectContentHasHeight(page, 400);

    // A) Attach `⏸` paused debug frame from a live Python debug session (`pyapp/calc_run.py`)
    await openFileFromTree(page, 'pyapp', 'calc_run.py');
    await expect(activeEditor(page)).toContainText('result = compute_answer(20, 22)');
    await toggleGutterBreakpoint(page, 7);

    const gutterBtn = page.locator('.cm-gutters .cm-ide-gutter-run-btn').first();
    await expect(gutterBtn).toBeVisible({ timeout: 15_000 });
    const startDbgWait = page.waitForResponse(
      (r) => r.request().method() === 'POST' && r.url().includes('/api/v1/ide/debug/sessions'),
      { timeout: 20_000 },
    );
    await gutterBtn.click();
    const dbgMenu = page.locator('.ctx-menu');
    await expectFullyInViewport(page, dbgMenu, 'Python gutter Run/Debug menu');
    await dbgMenu.getByRole('menuitem', { name: /^Debug/i }).first().click();
    await startDbgWait;

    const dbgPanel = page.getByTestId('ide-debugger-panel');
    await expect(dbgPanel).toBeVisible({ timeout: 15_000 });
    await expect(dbgPanel).toContainText(/Paused/i, { timeout: 20_000 });
    await expect(dbgPanel).toContainText('run_pipeline', { timeout: 15_000 });
    await expect(dbgPanel).toContainText('42', { timeout: 15_000 });

    const askAgentBtn = page.getByTestId('ide-debug-ask-agent');
    await expect(askAgentBtn).toBeEnabled();
    await askAgentBtn.click();

    const agentPanel = page.getByTestId('ide-agent-panel');
    await expect(agentPanel).toBeVisible();
    const tray = page.getByTestId('ide-agent-context-tray');
    await expect(tray).toContainText('⏸ run_pipeline @ calc_run.py:7');

    // Resume Python debug session so it finishes cleanly while keeping chip attached
    await dbgPanel.getByRole('button', { name: 'Resume program' }).click();
    await expect(dbgPanel).toContainText(/Terminated/i, { timeout: 15_000 });

    // B) Run failing test (`TestBroken` / `test_broken_sum`) and attach `✗` test-failure chip
    const cfgSelect = page.getByTestId('ide-run-config-select');
    await expect(cfgSelect).toBeVisible({ timeout: 15_000 });
    await cfgSelect.selectOption({ label: cfgBrokenLabel });
    await page.getByTestId('ide-run-btn').click();

    const runPanel = page.getByTestId('ide-run-panel');
    await expect(runPanel).toBeVisible({ timeout: 15_000 });
    const fixTestBtn = page.getByTestId('ide-test-fix-agent').first();
    await expect(fixTestBtn).toBeVisible({ timeout: 30_000 });
    await fixTestBtn.click();
    await expect(tray).toContainText(/✗ (TestBroken|test_broken_sum)/);

    // C) Open `calc/calc.go`, inject diagnostic into Problems panel, click `Fix with Agent` (`⚠`)
    await openFileFromTree(page, 'calc', 'calc.go');
    await expect(activeEditor(page)).toContainText('func Add(a, b int) int');
    await page.evaluate(async (filePath) => {
      // @ts-expect-error Vite runtime module import inside browser context
      const { ide } = await import('/src/modules/ide/ideStore.svelte.ts');
      ide.setDiagnostics(filePath, [
        {
          path: filePath,
          line: 4,
          column: 2,
          end_line: 4,
          end_column: 14,
          severity: 'error',
          message: 'undefined: offsetBonus (compiler)',
          source: 'gopls',
          code: 'UndeclaredName',
        },
      ]);
    }, calcGoPath);

    await page.getByTestId('ide-tab-problems').click();
    const problemsPanel = page.getByTestId('ide-problems-panel');
    await expect(problemsPanel).toContainText('undefined: offsetBonus');
    await page.getByTestId('ide-problem-fix-agent').first().click();
    await expect(tray).toContainText('⚠ calc/calc.go:4');

    // D) Select `Add` function lines (L3-L6) -> contextual selection bar -> `Add to Agent ⌘L`
    await selectEditorLines(page, 3, 6);
    const selBar = page.getByTestId('ide-selection-agent-bar');
    await expect(selBar).toBeVisible();
    await page.getByTestId('ide-selection-add-agent').click();

    await expect(tray.getByTestId('ide-context-chip')).toHaveCount(4);
    await expect(tray).toContainText('@calc/calc.go:L3-L6');
    await expectNoHorizontalOverflow(page);
    await saveShot(page, '16-ide-agent-dock-context-chips.png');

    // E) Send a real turn via `ottod` `/runtime-sessions` + `otto-fake-claude` with a fenced Go proposal
    const proposedCalcGo = [
      'package calc',
      '',
      'func Add(a, b int) int {',
      '\ttotal := a + b',
      '\treturn total',
      '}',
      '',
      'func Multiply(a, b int) int {',
      '\tprod := a * b',
      '\treturn prod',
      '}',
      '',
      'func ClampNonNegative(v int) int {',
      '\tif v < 0 {',
      '\t\treturn 0',
      '\t}',
      '\treturn v',
      '}',
    ].join('\n');

    const composer = page.getByTestId('ide-agent-composer');
    await composer.fill(
      `Refactor Add and add ClampNonNegative:\n\n\`\`\`go path=calc/calc.go\n${proposedCalcGo}\n\`\`\``,
    );
    await page.getByTestId('ide-agent-send').click();

    const proposalCard = page.getByTestId('ide-agent-proposal').first();
    await expect(proposalCard).toBeVisible({ timeout: 25_000 });
    await expect(proposalCard).toContainText('+9');
    await expect(proposalCard).toContainText('-2');
    const diffPreview = proposalCard.getByTestId('ide-agent-diff-preview');
    await expect(diffPreview).toBeVisible();
    await expect(diffPreview).toContainText('func ClampNonNegative(v int) int');
    await saveShot(page, '17-ide-agent-diff-preview-apply.png');

    // F) Click Apply -> verify CodeMirror updates immediately & Undo button appears
    await proposalCard.getByTestId('ide-agent-apply-proposal').click();
    await expect(activeEditor(page)).toContainText('func ClampNonNegative(v int) int');
    await expect(activeEditor(page)).toContainText('total := a + b');
    const undoBtn = proposalCard.getByTestId('ide-agent-undo-proposal');
    await expect(undoBtn).toBeVisible();

    // G) Press native `⌘Z` inside CodeMirror -> restores original buffer cleanly!
    await activeEditor(page).click();
    await page.keyboard.press('Meta+z');
    await expect(activeEditor(page)).toContainText('sum := a + b');
    await expect(activeEditor(page)).not.toContainText('ClampNonNegative');

    // Reset proposal state via Undo button, re-Apply, and press `⌘S` -> verify disk matches!
    await undoBtn.click();
    await expect(proposalCard.getByTestId('ide-agent-apply-proposal')).toBeVisible();
    await proposalCard.getByTestId('ide-agent-apply-proposal').click();
    await expect(activeEditor(page)).toContainText('func ClampNonNegative(v int) int');

    const saveWait = page.waitForResponse(
      (r) => r.request().method() === 'PUT' && r.url().includes('/api/v1/ide/fs/file'),
      { timeout: 15_000 },
    );
    await activeEditor(page).click();
    await page.keyboard.press('Meta+s');
    expect((await saveWait).ok()).toBeTruthy();
    expect(readFileSync(calcGoPath, 'utf8').trim()).toBe(proposedCalcGo.trim());
  });

  test('2. inline selection edit (⌘I / IdeInlinePromptBar): @path:Lstart-Lend badge, compact diff preview & Accept splice', async ({
    page,
  }) => {
    test.setTimeout(60_000);
    const calcGoPath = join(workspaceRoot, 'calc', 'calc.go');
    writeFileSync(calcGoPath, INITIAL_CALC_GO);

    await bootIdePage(page);
    await openFileFromTree(page, 'calc', 'calc.go');
    await expect(activeEditor(page)).toContainText('func Multiply(a, b int) int');

    // Select `Multiply` function (L8-L11) -> click `Edit ⌘I` in selection bar
    await selectEditorLines(page, 8, 11);
    await expect(page.getByTestId('ide-selection-agent-bar')).toBeVisible();
    await page.getByTestId('ide-selection-inline-edit').click();

    const inlineBar = page.getByTestId('ide-inline-prompt-bar');
    await expect(inlineBar).toBeVisible();
    await expect(inlineBar).toContainText('@calc/calc.go:L8-L11');

    // Verify Escape dismisses and `⌘I` (`Meta+i`) re-opens cleanly on the selection
    await page.keyboard.press('Escape');
    await expect(inlineBar).toBeHidden();
    await selectEditorLines(page, 8, 11);
    await page.keyboard.press('Meta+i');
    await expect(inlineBar).toBeVisible();
    await expect(inlineBar).toContainText('@calc/calc.go:L8-L11');

    const replacementMultiply = [
      'func Multiply(a, b int) int {',
      '\tproduct := a * b',
      '\treturn product',
      '}',
    ].join('\n');

    await page
      .getByTestId('ide-inline-prompt-input')
      .fill(`Rename prod to product:\n\`\`\`go\n${replacementMultiply}\n\`\`\``);
    await page.getByTestId('ide-inline-submit').click();

    const inlineDiff = page.getByTestId('ide-inline-diff-preview');
    await expect(inlineDiff).toBeVisible({ timeout: 25_000 });
    await expect(inlineBar).toContainText('+2');
    await expect(inlineBar).toContainText('-2');
    await expect(inlineDiff).toContainText('product := a * b');
    await page.getByTestId('ide-inline-prompt-input').evaluate((el) => {
      el.scrollTop = 0;
    });
    await expectNoHorizontalOverflow(page);
    await saveShot(page, '18-ide-inline-edit-cmd-i.png');

    // Accept inline edit -> splices cleanly into Multiply while preserving Add above
    await page.getByTestId('ide-inline-accept').click();
    await expect(inlineBar).toBeHidden();
    await expect(activeEditor(page)).toContainText('func Add(a, b int) int');
    await expect(activeEditor(page)).toContainText('product := a * b');

    // Verify `⌘Z` cleanly reverts the inline edit as a single undoable transaction
    await activeEditor(page).click();
    await page.keyboard.press('Meta+z');
    await expect(activeEditor(page)).toContainText('prod := a * b');
  });

  test('3. multi-language execution & live debugging: Python unittest + bdb bridge & Node.js node:test + V8 Inspector', async ({
    page,
  }) => {
    test.setTimeout(90_000);
    await bootIdePage(page);

    // ── Part A: Python `unittest` Run + Real Python `bdb` Live Debugger ──────
    const cfgSelect = page.getByTestId('ide-run-config-select');
    await expect(cfgSelect).toBeVisible({ timeout: 15_000 });
    await cfgSelect.selectOption({ label: 'Python Unit Suite' });
    await page.getByTestId('ide-run-btn').click();

    const runPanel = page.getByTestId('ide-run-panel');
    await expect(runPanel).toBeVisible({ timeout: 15_000 });
    await expect(runPanel).toContainText('test_compute_answer', { timeout: 20_000 });
    await expect(runPanel).toContainText(/passed/i, { timeout: 20_000 });

    await openFileFromTree(page, 'pyapp', 'calc_run.py');
    await expect(activeEditor(page)).toContainText('scaled = result * 2');
    await toggleGutterBreakpoint(page, 7);

    const pyGutterBtn = page.locator('.cm-gutters .cm-ide-gutter-run-btn').first();
    await expect(pyGutterBtn).toBeVisible({ timeout: 15_000 });
    const pyDbgStart = page.waitForResponse(
      (r) => r.request().method() === 'POST' && r.url().includes('/api/v1/ide/debug/sessions'),
      { timeout: 20_000 },
    );
    await pyGutterBtn.click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: /^Debug/i }).first().click();
    const pySessionId = ((await (await pyDbgStart).json()) as { session: IdeDebugSnapshot }).session.id;

    try {
      const dbgPanel = page.getByTestId('ide-debugger-panel');
      await expect(dbgPanel).toBeVisible({ timeout: 15_000 });
      await expect(dbgPanel).toContainText(/Paused/i, { timeout: 20_000 });
      await expect(page.locator('.cm-ide-paused-line').first()).toBeVisible({ timeout: 10_000 });
      await expect(dbgPanel).toContainText('run_pipeline');
      await expect(dbgPanel).toContainText('calc_run.py:7');
      await expect(dbgPanel).toContainText('result');
      await expect(dbgPanel).toContainText('42');

      // Evaluate Python expression `result * 2` -> `84` in Debug REPL
      const replInput = dbgPanel.getByLabel('Evaluate expression');
      await replInput.fill('result * 2');
      await dbgPanel.getByRole('button', { name: 'Evaluate', exact: true }).click();
      await expect(dbgPanel.locator('.dbg-repl-val')).toContainText('84', { timeout: 10_000 });

      await expectNoHorizontalOverflow(page);
      await saveShot(page, '19-ide-python-live-debugger.png');

      // Step over line 7 -> line 8 (`scaled = 84`) and verify `Ask Agent` attaches `⏸` chip
      await dbgPanel.getByRole('button', { name: 'Step over' }).click();
      await expect(dbgPanel).toContainText('calc_run.py:8', { timeout: 10_000 });
      await expect(dbgPanel).toContainText('scaled');
      await page.getByTestId('ide-debug-ask-agent').click();
      await expect(page.getByTestId('ide-agent-context-tray')).toContainText('⏸ run_pipeline @ calc_run.py:8');
      await page.getByTestId('ide-agent-close').click();

      await dbgPanel.getByRole('button', { name: 'Resume program' }).click();
      await expect(dbgPanel).toContainText(/Terminated/i, { timeout: 15_000 });
    } finally {
      await page.request
        .fetch(`${base}/api/v1/ide/debug/sessions/${pySessionId}`, {
          method: 'DELETE',
          headers: { Authorization: `Bearer ${token}` },
        })
        .catch(() => {});
    }

    // Remove Python breakpoint before Node run
    await toggleGutterBreakpoint(page, 7).catch(() => {});

    // ── Part B: Node.js `node --test` Run + Real V8 Inspector Debugger ───────
    await cfgSelect.selectOption({ label: 'Node Test Suite' });
    await page.getByTestId('ide-run-btn').click();
    await expect(runPanel).toBeVisible({ timeout: 15_000 });
    await expect(runPanel).toContainText('combineTotals sums values accurately', { timeout: 20_000 });
    await expect(runPanel).toContainText(/passed/i, { timeout: 20_000 });

    await openFileFromTree(page, 'web', 'debug_demo.mjs');
    await expect(activeEditor(page)).toContainText('const boosted = subtotal + 8;');
    await toggleGutterBreakpoint(page, 5);

    await cfgSelect.selectOption({ label: 'Node Debug Demo' });
    const nodeDbgStart = page.waitForResponse(
      (r) => r.request().method() === 'POST' && r.url().includes('/api/v1/ide/debug/sessions'),
      { timeout: 20_000 },
    );
    await page.getByTestId('ide-debug-btn').click();
    const nodeSessionId = ((await (await nodeDbgStart).json()) as { session: IdeDebugSnapshot }).session.id;

    try {
      const dbgPanel = page.getByTestId('ide-debugger-panel');
      await expect(dbgPanel).toBeVisible({ timeout: 15_000 });
      await expect(dbgPanel).toContainText(/Paused/i, { timeout: 20_000 });
      await expect(dbgPanel).toContainText('runNodeDemo', { timeout: 15_000 });
      await expect(dbgPanel).toContainText('subtotal', { timeout: 15_000 });
      await expect(dbgPanel).toContainText('42', { timeout: 15_000 });

      // Step over line 5 -> line 6 (`boosted = 50`) and evaluate `subtotal + boosted` -> `92`
      await dbgPanel.getByRole('button', { name: 'Step over' }).click();
      await expect(dbgPanel).toContainText('debug_demo.mjs:6', { timeout: 15_000 });
      await expect(dbgPanel).toContainText('50', { timeout: 15_000 });

      const replInput = dbgPanel.getByLabel('Evaluate expression');
      await replInput.fill('subtotal + boosted');
      await dbgPanel.getByRole('button', { name: 'Evaluate', exact: true }).click();
      await expect(dbgPanel.locator('.dbg-repl-val').last()).toContainText('92', { timeout: 10_000 });

      await dbgPanel.getByRole('button', { name: 'Resume program' }).click();
      await expect(dbgPanel).toContainText(/Terminated/i, { timeout: 15_000 });
    } finally {
      await page.request
        .fetch(`${base}/api/v1/ide/debug/sessions/${nodeSessionId}`, {
          method: 'DELETE',
          headers: { Authorization: `Bearer ${token}` },
        })
        .catch(() => {});
    }
  });

  test('4. live workspace Terminal PTY (IdeTerminalPanel): shell spawn, real command I/O, a11y & clean kill', async ({
    page,
  }) => {
    test.setTimeout(60_000);
    await bootIdePage(page);
    await openFileFromTree(page, 'calc', 'calc.go');

    const createSessionWait = page.waitForResponse(
      (r) =>
        r.request().method() === 'POST' &&
        /\/api\/v1\/(?:workspaces\/[^/]+\/)?sessions$/.test(new URL(r.url()).pathname),
      { timeout: 20_000 },
    );
    await page.getByTestId('ide-tab-terminal').click();

    const termPanel = page.getByTestId('ide-terminal-panel');
    await expect(termPanel).toBeVisible({ timeout: 15_000 });
    await expect(termPanel.getByRole('button', { name: 'Terminal 1' })).toBeVisible({ timeout: 15_000 });

    const createdSession = (await (await createSessionWait).json()) as { id: string };
    expect(createdSession.id).toBeTruthy();

    // Wait for xterm probe mount and send a real shell command into the PTY
    await expect
      .poll(
        () =>
          page.evaluate(
            () =>
              (window as unknown as { __ottoTermProbe: { disposed: boolean }[] }).__ottoTermProbe.some(
                (p) => !p.disposed,
              ),
          ),
        { timeout: 15_000 },
      )
      .toBeTruthy();
    await daemon(page.request, 'POST', `/sessions/${createdSession.id}/input`, {
      text: 'echo OTTO_IDE_PTY_OK\r',
    });

    await expect
      .poll(
        () =>
          page.evaluate(() =>
            (window as unknown as { __ottoTermProbe: { disposed: boolean; text(): string }[] }).__ottoTermProbe
              .filter((p) => !p.disposed)
              .map((p) => p.text())
              .join('\n'),
          ),
        { timeout: 15_000 },
      )
      .toContain('OTTO_IDE_PTY_OK');
    await expectNoHorizontalOverflow(page);
    await expectFullyInViewport(page, termPanel, 'IDE bottom terminal panel');
    await expectAccessible(page, { key: 'ide' });
    await saveShot(page, '20-ide-terminal-live-pty.png');

    // Kill terminal cleanly via UI button
    await termPanel.getByRole('button', { name: 'Kill active terminal' }).click();
    await expect(termPanel).toContainText('No open terminal', { timeout: 10_000 });
  });
});
