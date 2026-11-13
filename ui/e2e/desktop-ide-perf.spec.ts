import { test, expect, type APIRequestContext, type Page, type TestInfo } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import {
  budgetMs,
  dist,
  frameDeltas,
  heapAfterGC,
  isDesktopProject,
  longTasks,
  requestLog,
  watchFatalUiErrors,
  watchLongTasks,
} from './perf';
import { processResources } from './process-resources';

// Otto IDE — Desktop Performance, Virtualization, Retention & Parallel Load Gate
// Runs on BOTH `desktop-webkit` (WKWebView proxy) and `desktop-browser` (Chromium):
// 1) 4.0s idle zero-polling + CPU/RSS gate; 2) 2,500-file tree virtualization + 5,000-line
// keystroke/Prefix-Suffix latency; 3) 20-tab JS heap + RSS retention; 4) N=0,1,4,8 parallel load.

test.use({ serviceWorkers: 'block' });
test.describe.configure({ mode: 'serial' });

const TREE_FILE_COUNT = 2_500;
const BIG_FILE_LINES = 5_000;

interface ProcMetric {
  cpuPct: number;
  rssMb: number;
  count: number;
}

interface ResourceSample {
  daemon: ProcMetric;
  browser: ProcMetric;
  agentChildren: ProcMetric;
  total: ProcMetric;
  runtimeAgentProcs: number;
}

interface RuntimeSessionDto {
  id: string;
  status: string;
  title?: string | null;
}

type RuntimeReplayDto =
  | { kind: 'snapshot'; snapshot: { turns: Array<{ id: string; status: string }> } }
  | {
      kind: 'events';
      events: Array<
        | { type: 'turn.terminal'; status: string }
        | { type: 'provider_turn.updated'; provider_turn: { status: string } }
        | { type: string }
      >;
    };

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));
const round2 = (n: number): number => Math.round(n * 100) / 100;
const perfBudget = (ms: number): number => budgetMs(ms);

function readDaemonPid(): number {
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const metaPath = join(process.cwd(), 'e2e', `.auth-${slot}`, 'daemon.json');
  const meta = JSON.parse(readFileSync(metaPath, 'utf8')) as { pid: number };
  if (!Number.isInteger(meta.pid) || meta.pid <= 1) {
    throw new Error(`Invalid daemon pid in ${metaPath}: ${String(meta.pid)}`);
  }
  return meta.pid;
}

function emitPerfMetric(info: TestInfo, metric: string, payload: Record<string, unknown>): void {
  const record = { project: info.project.name, metric, ...payload };
  const serialized = JSON.stringify(record);
  info.annotations.push({ type: `perf:${metric}`, description: serialized });
  console.log(`[perf-metric] ${serialized}`);
}

function countRuntimeAgentProcs(daemonPid: number): number {
  const out = execFileSync('ps', ['-axo', 'pid=,ppid=,comm='], { encoding: 'utf8', timeout: 3_000 });
  const rows = out.trim().split('\n').map((l) => /^\s*(\d+)\s+(\d+)\s+(.+)$/.exec(l)).filter(Boolean) as RegExpExecArray[];
  const ppidMap = new Map(rows.map((m) => [Number(m[1]), Number(m[2])]));
  const isDesc = (pid: number): boolean => {
    for (let d = 0, cur = pid; d < 32 && cur > 1; d++) { if (cur === daemonPid) return true; cur = ppidMap.get(cur) ?? 0; }
    return false;
  };
  return rows.filter((m) => isDesc(Number(m[1])) && /otto-fake-|fake-agent-bin\/(claude|codex|gemini)/.test(m[3])).length;
}

async function sampleResources(daemonPid: number, gapMs = 700): Promise<ResourceSample> {
  const first = processResources(daemonPid);
  await sleep(gapMs);
  const second = processResources(daemonPid);
  const pick = (rows: ReturnType<typeof processResources>, label: string) =>
    rows.find((r) => r.process === label) ?? { cpu_percent: 0, rss_mb: 0, pids: [] as number[] };
  const merge = (label: string): ProcMetric => {
    const a = pick(first, label);
    const b = pick(second, label);
    return { cpuPct: round2((a.cpu_percent + b.cpu_percent) / 2), rssMb: round2(b.rss_mb), count: b.pids.length };
  };
  return { daemon: merge('daemon'), browser: merge('browser'), agentChildren: merge('agent-children'), total: merge('total'), runtimeAgentProcs: countRuntimeAgentProcs(daemonPid) };
}

function buildBigGoSource(lineCount: number): string {
  const lines: string[] = ['package main', ''];
  for (let idx = 1; lines.length < lineCount; idx++) {
    const fnName = `ComputeMetric${String(idx).padStart(4, '0')}`;
    const block = [
      `// ${fnName} evaluates deterministic throughput bucket ${idx}.`,
      `func ${fnName}(input int) int {`, `\tacc := input + ${idx}`,
      `\tif acc%2 == 0 {\n\t\tacc = (acc * 3) + ${(idx % 17) + 1}\n\t}`, `\treturn acc`, `}`, '',
    ];
    for (const ln of block.join('\n').split('\n')) {
      if (lines.length >= lineCount) break;
      lines.push(ln);
    }
  }
  return lines.join('\n');
}

function seedPerfWorkspace(root: string): void {
  for (const dir of ['pkg', 'pyapp', 'web']) mkdirSync(join(root, dir), { recursive: true });
  writeFileSync(join(root, 'go.mod'), 'module example.com/ideperf\n\ngo 1.22\n');
  writeFileSync(join(root, 'main.go'), 'package main\n\nimport "fmt"\n\nfunc main() {\n\tfmt.Println("ide perf ready")\n}\n');
  writeFileSync(join(root, 'pyapp', 'calc.py'), 'def add(a: int, b: int) -> int:\n    return a + b\n');
  writeFileSync(join(root, 'web', 'index.ts'), 'export const version = "1.0.0";\n');
  for (let i = 1; i <= TREE_FILE_COUNT; i++) {
    const pad = String(i).padStart(4, '0');
    writeFileSync(join(root, 'pkg', `file_${pad}.go`), `package pkg\n\nfunc Leaf${pad}(v int) int {\n\treturn v + ${i}\n}\n`);
  }
  writeFileSync(join(root, 'big_service.go'), buildBigGoSource(BIG_FILE_LINES));

  const git = (...args: string[]) => execFileSync('git', ['-C', root, ...args], { stdio: 'ignore' });
  git('init', '-q');
  git('config', 'user.email', 'perf@otto.local');
  git('config', 'user.name', 'Otto Perf');
  git('config', 'commit.gpgsign', 'false');
  git('add', 'go.mod', 'main.go', 'pyapp/calc.py', 'web/index.ts');
  git('commit', '-q', '-m', 'seed ide perf workspace');
}

let daemonPid = 0;
let base = '';
let workspaceId = '';
let workspaceRoot = '';

function localTarget() {
  return { kind: 'local' as const, workspace_id: workspaceId, root: workspaceRoot, connection_id: null };
}

async function bootIde(page: Page): Promise<void> {
  await page.addInitScript((wsId: string) => {
    localStorage.setItem('otto_workspace', wsId);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.dockOpen', '0');
    localStorage.setItem('otto.ide.agentOpen', '0');
  }, workspaceId);
  await page.goto('/#/ide');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.ide-tab').first()).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.cm-editor .cm-content').first()).toBeVisible({ timeout: 20_000 });
}

async function expandTreeFolder(page: Page, folderName: string): Promise<void> {
  const tree = page.getByTestId('ide-project-tree');
  const item = tree
    .locator(`button.tree-item[title="${folderName}"], button.tree-item[title$="/${folderName}"]`)
    .first();
  await expect(item).toBeVisible({ timeout: 15_000 });
  if ((await item.getAttribute('aria-expanded')) !== 'true') {
    await item.click();
  }
}

async function openTreeFilePinned(page: Page, relPath: string): Promise<void> {
  const tree = page.getByTestId('ide-project-tree');
  const item = tree
    .locator(`button.tree-item[title="${relPath}"], button.tree-item[title$="/${relPath}"]`)
    .first();
  await expect(item).toBeVisible({ timeout: 15_000 });
  await item.dblclick();
}

async function apiJson<T>(
  ctx: APIRequestContext,
  method: 'GET' | 'POST' | 'PUT' | 'DELETE',
  path: string,
  data?: unknown,
): Promise<T> {
  const resp = await ctx.fetch(`${base}/api/v1${path}`, {
    method,
    data: data === undefined ? undefined : JSON.stringify(data),
  });
  const text = await resp.text();
  if (!resp.ok()) {
    throw new Error(`${method} ${path} failed (${resp.status()}): ${text}`);
  }
  return (text ? JSON.parse(text) : undefined) as T;
}

test.describe('Desktop IDE — Perf, Virtualization, Retention & Parallel Load', () => {
  test.beforeAll(async () => {
    test.setTimeout(120_000);
    daemonPid = readDaemonPid();
    workspaceRoot = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-perf-')));
    seedPerfWorkspace(workspaceRoot);

    const conn = await apiCtx();
    base = conn.base;
    try {
      const ws = await apiJson<{ id: string }>(conn.ctx, 'POST', '/workspaces', {
        name: `IDE Perf ${Date.now()}`,
        root_path: workspaceRoot,
      });
      workspaceId = ws.id;
      await apiJson(conn.ctx, 'POST', '/ide/run-configs', {
        target: localTarget(),
        id: 'burst-2000',
        name: 'Burst 2000',
        runner: 'custom',
        command: 'python3',
        args: ['-c', 'for i in range(2000): print(f"burst line {i:04d} ok")'],
        cwd: workspaceRoot,
        env_overrides: {},
      });
    } finally {
      await conn.ctx.dispose().catch(() => {});
    }
  });

  test.afterAll(async () => {
    if (workspaceRoot) {
      rmSync(workspaceRoot, { recursive: true, force: true });
    }
  });

  test.beforeEach(async ({}, info) => {
    test.skip(!isDesktopProject(info.project.name), 'Desktop perf projects only (desktop-webkit & desktop-browser)');
  });

  test('1. idle footprint & 4.0s zero-polling gate across Tree, Editor, Debug dock & Agent dock', async ({
    page,
    browserName,
  }, info) => {
    test.setTimeout(90_000);
    const fatal = watchFatalUiErrors(page);
    await bootIde(page);

    // Open Bottom Dock on Debug AND open the AI Agent dock.
    await page.getByTestId('ide-tab-debug').click();
    await expect(page.getByTestId('ide-debugger-panel')).toBeVisible({ timeout: 10_000 });

    await page.getByTestId('ide-toggle-agent').click();
    await expect(page.getByTestId('ide-agent-panel')).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('ide-agent-composer')).toBeVisible({ timeout: 15_000 });

    // Wait for initial toolchain/symbol/gutter/session loads + 800ms debounce settle.
    await page.waitForLoadState('networkidle').catch(() => {});
    await sleep(800);

    const apiReqs = requestLog(page, /https?:\/\/[^/]+\/api\//);
    if (browserName === 'chromium') {
      await watchLongTasks(page);
    }

    // 4.0-second quiet window (3.2s pure quiet + 0.8s 2-tick OS resource sample = 4.0s total).
    await sleep(3_200);
    const idleRes = await sampleResources(daemonPid, 800);
    const reqStats = apiReqs.stats();
    apiReqs.stop();

    const ltDurations = browserName === 'chromium' ? await longTasks(page) : null;

    emitPerfMetric(info, 'test1_idle_gate', {
      idleApiRequests: reqStats.count,
      idlePaths: reqStats.paths,
      longTasksCount: ltDurations ? ltDurations.length : null,
      daemonRssMb: idleRes.daemon.rssMb,
      daemonCpuPct: idleRes.daemon.cpuPct,
      browserRssMb: idleRes.browser.rssMb,
      browserCpuPct: idleRes.browser.cpuPct,
      totalRssMb: idleRes.total.rssMb,
      totalCpuPct: idleRes.total.cpuPct,
    });

    expect(fatal, 'zero fatal UI errors').toEqual([]);
    expect(reqStats.paths, '0 background /api/ requests during 4.0s idle window').toEqual([]);
    expect(reqStats.count).toBe(0);
    if (ltDurations !== null) {
      expect(ltDurations.length, '0 Chromium long tasks during 4.0s idle window').toBe(0);
    }
    expect(idleRes.daemon.rssMb).toBeLessThan(250);
    expect(idleRes.daemon.cpuPct).toBeLessThan(5);
  });

  test('2. 2,500-file tree virtualization (< 160 DOM rows) & 5,000-line keystroke / 40-line prefix-suffix latency', async ({
    page,
  }, info) => {
    test.setTimeout(90_000);
    const fatal = watchFatalUiErrors(page);
    await bootIde(page);

    // Expand `pkg/` containing 2,500 Go files and assert virtualized DOM count < 160.
    await expandTreeFolder(page, 'pkg');
    await expect(
      page.getByTestId('ide-project-tree').locator('button.tree-item[title$="pkg/file_0001.go"]').first(),
    ).toBeVisible({ timeout: 15_000 });

    const treeSelector = '[data-testid="ide-project-tree"] .tree-row, [data-testid="ide-project-tree"] .tree-item';
    const domCountTop = await page.locator(treeSelector).count();
    const rowCountTop = await page.locator('[data-testid="ide-project-tree"] .tree-row').count();

    // Also verify mid-scroll virtualization unmounts top rows while keeping total DOM < 160.
    await page.locator('[data-testid="ide-project-tree"] .tree-scroll').evaluate((el: HTMLElement) => {
      el.scrollTop = Math.floor(el.scrollHeight * 0.5);
      el.dispatchEvent(new Event('scroll'));
    });
    await expect(
      page.getByTestId('ide-project-tree').locator('button.tree-item[title$="pkg/file_0001.go"]'),
    ).toHaveCount(0, { timeout: 10_000 });
    await expect(
      page.getByTestId('ide-project-tree').locator('button.tree-item[title*="pkg/file_"]').first(),
    ).toBeVisible({ timeout: 10_000 });
    const domCountMid = await page.locator(treeSelector).count();

    // Reset tree scroll back to top and collapse `pkg/` so root-level `big_service.go` (below 2,500 dir children) is in view.
    await page.locator('[data-testid="ide-project-tree"] .tree-scroll').evaluate((el: HTMLElement) => {
      el.scrollTop = 0;
      el.dispatchEvent(new Event('scroll'));
    });
    await page
      .getByTestId('ide-project-tree')
      .locator('button.tree-item[title="pkg"], button.tree-item[title$="/pkg"]')
      .first()
      .click();

    // Open 5,000-line (~185 KB) `big_service.go`.
    await openTreeFilePinned(page, 'big_service.go');
    const editor = page.locator('.cm-editor .cm-content').first();
    await expect(editor).toContainText('ComputeMetric0001', { timeout: 15_000 });
    await sleep(450);

    // Measure 20 keystrokes in the 5,000-line buffer (1 keystroke per frame cadence).
    await editor.click();
    await page.keyboard.press('Meta+ArrowUp');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('End');
    await page.keyboard.insertText('\n// ');

    // Per-keystroke frame work (`beforeinput` script + `rAF`->post-paint `MessageChannel`, excluding idle vsync wait).
    await page.evaluate(() => {
      const w = window as unknown as { __ideKeyFrames: number[] };
      w.__ideKeyFrames = [];
      let armed = false;
      let t0 = 0;
      let script = 0;
      window.addEventListener('beforeinput', () => {
        if (armed) return;
        armed = true;
        t0 = performance.now();
        script = 0;
        requestAnimationFrame(() => {
          const r0 = performance.now();
          const ch = new MessageChannel();
          ch.port1.onmessage = () => {
            w.__ideKeyFrames.push(script + (performance.now() - r0));
            armed = false;
            ch.port1.close();
          };
          ch.port2.postMessage(null);
        });
      }, true);
      window.addEventListener('input', () => {
        if (armed && script === 0) queueMicrotask(() => { script = performance.now() - t0; });
      });
    });
    const rafDist = await frameDeltas(page, async () => {
      for (const ch of 'abcdefghijklmnopqrst') {
        await page.keyboard.type(ch);
        await sleep(45);
      }
    });
    const keyCosts = dist(
      await page.evaluate(() => (window as unknown as { __ideKeyFrames?: number[] }).__ideKeyFrames ?? []),
    );

    // Select 40 lines first (revealing the selection bar), fill Prefix / suffix (`// ` + ` /*checked*/`), and time apply.
    await editor.click();
    await page.keyboard.press('Meta+ArrowUp');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Home');
    for (let i = 0; i < 39; i++) {
      await page.keyboard.press('Shift+ArrowDown');
    }
    await page.keyboard.press('Shift+End');

    const prefixInput = page.getByTestId('ide-prefix-input');
    if (!(await prefixInput.isVisible())) {
      await expect(page.getByTestId('ide-prefix-suffix-toggle')).toBeVisible({ timeout: 5_000 });
      await page.getByTestId('ide-prefix-suffix-toggle').click();
    }
    await expect(prefixInput).toBeVisible();
    await prefixInput.fill('// ');
    await page.getByTestId('ide-suffix-input').fill(' /*checked*/');

    const applyBtn = page.getByTestId('ide-apply-prefix-suffix');
    await expect(applyBtn).toBeVisible();
    const prefixSuffixMs = await page.evaluate(async () => {
      const btn = document.querySelector<HTMLButtonElement>('[data-testid="ide-apply-prefix-suffix"]');
      if (!btn) throw new Error('Missing ide-apply-prefix-suffix button');
      const t0 = performance.now();
      btn.click();
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          const ch = new MessageChannel();
          ch.port1.onmessage = () => {
            ch.port1.close();
            resolve();
          };
          ch.port2.postMessage(null);
        });
      });
      return performance.now() - t0;
    });

    const checkedLineCount = await page.evaluate(() => {
      const el = document.querySelector('.cm-editor .cm-content') as HTMLElement & {
        cmTile?: { root?: { view?: { state: { doc: { toString(): string } } } } };
      };
      const docText = el?.cmTile?.root?.view?.state.doc.toString() ?? '';
      return docText.split('\n').filter((ln) => ln.startsWith('// ') && ln.endsWith(' /*checked*/')).length;
    });

    emitPerfMetric(info, 'test2_tree_and_editor', {
      treeFilesTotal: TREE_FILE_COUNT,
      treeDomElementsTop: domCountTop,
      treeRowsTop: rowCountTop,
      treeDomElementsMidScroll: domCountMid,
      bigFileLines: BIG_FILE_LINES,
      keystrokeSamples: keyCosts.n,
      keystrokeP50Ms: round2(keyCosts.p50),
      keystrokeP95Ms: round2(keyCosts.p95),
      keystrokeMaxMs: round2(keyCosts.max),
      rafP95Ms: round2(rafDist.p95),
      rafMaxMs: round2(rafDist.max),
      prefixSuffix40LinesMs: round2(prefixSuffixMs),
      checkedLineCount,
    });

    expect(fatal).toEqual([]);
    expect(domCountTop).toBeGreaterThan(20);
    expect(domCountTop, '2,500-file expanded tree stays virtualized (< 160 DOM nodes)').toBeLessThan(160);
    expect(domCountMid, 'mid-scroll 2,500-file tree stays virtualized (< 160 DOM nodes)').toBeLessThan(160);
    expect(keyCosts.n).toBeGreaterThanOrEqual(20);
    expect(keyCosts.p95).toBeLessThan(perfBudget(45));
    expect(rafDist.max, 'zero UI freezes while typing in 5,000-line file').toBeLessThan(perfBudget(250));
    expect(checkedLineCount).toBe(40);
    expect(prefixSuffixMs).toBeLessThan(perfBudget(250));
  });

  test('3. 20-tab open / switch / edit / save / close JS heap & daemon RSS retention gate', async ({
    page,
    browserName,
  }, info) => {
    test.setTimeout(120_000);
    const fatal = watchFatalUiErrors(page);
    await bootIde(page);
    await expandTreeFolder(page, 'pkg');

    // Warm 1 Go tab (`pkg/file_0001.go`) and close any auto-opened initial tab so baseline = 1 Go tab.
    await openTreeFilePinned(page, 'pkg/file_0001.go');
    await expect(page.locator('.cm-editor .cm-content').first()).toContainText('Leaf0001');
    while ((await page.locator('.ide-tab').count()) > 1) {
      await page.locator('.ide-tab:not(:has-text("file_0001.go")) .ide-tab-close').first().click();
    }
    await expect(page.locator('.ide-tab')).toHaveCount(1);
    await sleep(450);

    const heapBefore = browserName === 'chromium' ? await heapAfterGC(page) : null;
    const resBefore = await sampleResources(daemonPid, 400);

    // Open 19 more distinct Go files (`file_0002.go` .. `file_0020.go`) => 20 open tabs.
    for (let i = 2; i <= 20; i++) {
      const name = `pkg/file_${String(i).padStart(4, '0')}.go`;
      await openTreeFilePinned(page, name);
    }
    await expect(page.locator('.ide-tab')).toHaveCount(20);

    // Switch through & edit 5 tabs (`file_0016` .. `file_0020`), saving each via `⌘S`.
    const editor = page.locator('.cm-editor .cm-content').first();
    for (let i = 16; i <= 20; i++) {
      const baseName = `file_${String(i).padStart(4, '0')}.go`;
      await page.locator('.ide-tab-btn', { hasText: baseName }).first().click();
      await expect(editor).toContainText(`Leaf${String(i).padStart(4, '0')}`);
      await editor.click();
      await page.keyboard.press('Meta+ArrowDown');
      await page.keyboard.insertText(`\n// tab lifecycle edit ${i}\n`);
      const saveWait = page.waitForResponse(
        (r) => r.request().method() === 'PUT' && r.url().includes('/api/v1/ide/fs/file'),
        { timeout: 15_000 },
      );
      await page.keyboard.press('Meta+s');
      const savedResp = await saveWait;
      expect(savedResp.ok()).toBeTruthy();
    }

    // Close 19 tabs (`file_0002` .. `file_0020`), leaving `file_0001.go` as the single open tab.
    for (let closed = 0; closed < 19; closed++) {
      await page.locator('.ide-tab-close').nth(1).click();
    }
    await expect(page.locator('.ide-tab')).toHaveCount(1);
    await sleep(500);

    const heapAfter = browserName === 'chromium' ? await heapAfterGC(page) : null;
    const resAfter = await sampleResources(daemonPid, 400);

    const heapDeltaBytes = heapBefore !== null && heapAfter !== null ? heapAfter - heapBefore : null;
    const heapDeltaMb = heapDeltaBytes !== null ? round2(heapDeltaBytes / (1024 * 1024)) : null;
    const daemonRssDeltaMb = round2(resAfter.daemon.rssMb - resBefore.daemon.rssMb);

    emitPerfMetric(info, 'test3_tab_retention', {
      heapBeforeMb: heapBefore !== null ? round2(heapBefore / (1024 * 1024)) : null,
      heapAfterMb: heapAfter !== null ? round2(heapAfter / (1024 * 1024)) : null,
      heapDeltaMb,
      daemonRssBeforeMb: resBefore.daemon.rssMb,
      daemonRssAfterMb: resAfter.daemon.rssMb,
      daemonRssDeltaMb,
      browserRssBeforeMb: resBefore.browser.rssMb,
      browserRssAfterMb: resAfter.browser.rssMb,
    });

    expect(fatal).toEqual([]);
    if (heapDeltaBytes !== null) {
      expect(heapDeltaBytes, 'retained JS heap delta across 20 tabs < 16 MB').toBeLessThan(16 * 1024 * 1024);
    }
    expect(daemonRssDeltaMb, 'daemon RSS delta across 20 tabs < 30 MB').toBeLessThan(30);
  });

  test('4. parallel load N = 0, 1, 4, 8 concurrent streaming agents + 2,000-line run burst + post-settle cleanup', async ({
    page,
  }, info) => {
    test.setTimeout(180_000);
    const fatal = watchFatalUiErrors(page);
    const httpErrors: string[] = [];
    page.on('response', (resp) => {
      if (/https?:\/\/[^/]+\/api\//.test(resp.url()) && resp.status() >= 500) {
        httpErrors.push(`${resp.status()} ${resp.request().method()} ${resp.url()}`);
      }
    });

    await bootIde(page);
    await expandTreeFolder(page, 'pkg');
    await openTreeFilePinned(page, 'pkg/file_0001.go');

    // Open Bottom Dock on Run before N=0 baseline; Agent dock mounts at N=1 so it picks up `s1` cleanly.
    await page.getByTestId('ide-tab-run').click();
    await expect(page.getByTestId('ide-run-panel')).toBeVisible({ timeout: 10_000 });
    await sleep(600);

    const conn = await apiCtx();
    const sessionIds: string[] = [];
    let failedRequests = 0;

    const createSession = async (label: string): Promise<string> => {
      try {
        const s = await apiJson<RuntimeSessionDto>(
          conn.ctx,
          'POST',
          `/workspaces/${workspaceId}/runtime-sessions`,
          { provider: 'claude', cwd: workspaceRoot, title: label },
        );
        sessionIds.push(s.id);
        return s.id;
      } catch (err) {
        failedRequests += 1;
        throw err;
      }
    };

    const startSlowTurn = async (id: string, secs = 4): Promise<void> => {
      try {
        await apiJson(conn.ctx, 'POST', `/runtime-sessions/${id}/prompt`, { text: `slow ${secs}` });
      } catch (err) {
        failedRequests += 1;
        throw err;
      }
    };

    const waitTurnsIdle = async (ids: readonly string[]): Promise<number> => {
      const deadline = Date.now() + 25_000;
      while (Date.now() < deadline) {
        const states = await Promise.all(
          ids.map((id) => apiJson<RuntimeSessionDto>(conn.ctx, 'GET', `/runtime-sessions/${id}`)),
        );
        if (states.every((s) => s.status === 'idle')) {
          const replays = await Promise.all(
            ids.map((id) => apiJson<RuntimeReplayDto>(conn.ctx, 'GET', `/runtime-sessions/${id}/events`)),
          );
          return replays.filter((r) =>
            r.kind === 'snapshot'
              ? r.snapshot.turns.some((t) => t.status === 'completed')
              : r.events.some(
                  (e) =>
                    (e.type === 'turn.terminal' && 'status' in e && e.status === 'completed') ||
                    (e.type === 'provider_turn.updated' && 'provider_turn' in e && e.provider_turn.status === 'completed'),
                ),
          ).length;
        }
        await sleep(180);
      }
      throw new Error(`Timed out waiting for ${ids.length} runtime sessions to complete turns`);
    };

    try {
      // ── Stage N = 0 (Baseline) ───────────────────────────────────────────
      const n0 = await sampleResources(daemonPid, 700);
      emitPerfMetric(info, 'test4_load_n0', { stage: 'N=0', ...n0 });

      // ── Stage N = 1 (Mounted in live IdeAgentPanel) ──────────────────────
      const s1 = await createSession('IDE Load S1');
      await page.getByTestId('ide-toggle-agent').click();
      await expect(page.getByTestId('ide-agent-panel')).toBeVisible({ timeout: 15_000 });
      await expect
        .poll(
          async () => {
            await page.getByTestId('ide-agent-sessions').click();
            const menu = page.locator('.ctx-menu.ctx-ready');
            await menu.waitFor({ state: 'visible', timeout: 1_500 }).catch(() => {});
            const item = menu.getByRole('menuitemcheckbox', { name: /IDE Load S1/ });
            if ((await item.count()) > 0) {
              await item.first().click();
              return true;
            }
            await page.keyboard.press('Escape');
            return false;
          },
          { timeout: 10_000 },
        )
        .toBe(true);

      await startSlowTurn(s1, 4);
      await expect(page.getByTestId('ide-agent-stop')).toBeVisible({ timeout: 8_000 });
      const n1 = await sampleResources(daemonPid, 700);
      emitPerfMetric(info, 'test4_load_n1', { stage: 'N=1', ...n1 });
      expect(await waitTurnsIdle([s1])).toBe(1);

      // ── Stage N = 4 (+ Simultaneous 2,000-Line `/ide/runs` Output Burst) ─
      const s234 = await Promise.all(['IDE Load S2', 'IDE Load S3', 'IDE Load S4'].map(createSession));
      const active4 = [s1, ...s234];
      const runPromise = apiJson<{ run: { id: string; status: string } }>(conn.ctx, 'POST', '/ide/runs', {
        target: localTarget(),
        config_id: 'burst-2000',
      });
      await Promise.all(active4.map((id) => startSlowTurn(id, 4)));
      const startedRun = await runPromise;

      const n4 = await sampleResources(daemonPid, 700);
      emitPerfMetric(info, 'test4_load_n4', { stage: 'N=4+run2000', ...n4 });

      await expect
        .poll(
          async () => {
            const r = await apiJson<{ run: { status: string; output: string[] } }>(
              conn.ctx,
              'GET',
              `/ide/runs/${encodeURIComponent(startedRun.run.id)}`,
            );
            return r.run.status === 'passed' ? r.run.output.length : -1;
          },
          { timeout: 20_000 },
        )
        .toBeGreaterThanOrEqual(2_000);
      expect(await waitTurnsIdle(active4)).toBe(4);

      // ── Stage N = 8 (+ Simultaneous Active Editor Typing) ────────────────
      const s5678 = await Promise.all(
        ['IDE Load S5', 'IDE Load S6', 'IDE Load S7', 'IDE Load S8'].map(createSession),
      );
      const active8 = [...active4, ...s5678];
      await Promise.all(active8.map((id) => startSlowTurn(id, 4)));

      const editor = page.locator('.cm-editor .cm-content').first();
      await editor.click();
      await page.keyboard.press('Meta+ArrowDown');
      const typingTask = (async () => {
        await page.keyboard.type('\n// live typing under 8 streaming claude sessions\n', { delay: 14 });
      })();
      const n8 = await sampleResources(daemonPid, 700);
      await typingTask;
      emitPerfMetric(info, 'test4_load_n8', { stage: 'N=8+typing', ...n8 });

      const completed8 = await waitTurnsIdle(active8);
      expect(completed8, 'all 8 concurrent streaming turns completed cleanly').toBe(8);

      // ── Close / DELETE all 8 sessions & measure +5s Post-Settle ──────────
      for (const id of [...sessionIds]) {
        await apiJson(conn.ctx, 'POST', `/runtime-sessions/${id}/close`).catch(() => {});
        await apiJson(conn.ctx, 'DELETE', `/runtime-sessions/${id}`);
      }
      sessionIds.length = 0;

      await sleep(5_000);
      const postSettle = await sampleResources(daemonPid, 700);
      emitPerfMetric(info, 'test4_post_settle', {
        stage: 'post-settle(+5s)',
        completedTurns: completed8,
        failedRequests: failedRequests + httpErrors.length,
        ...postSettle,
      });

      expect(fatal).toEqual([]);
      expect(httpErrors, '0 HTTP 5xx responses under parallel load').toEqual([]);
      expect(failedRequests, '0 failed runtime-session API requests').toBe(0);
      expect(n8.runtimeAgentProcs, '8 active runtime agent child processes during N=8').toBeGreaterThanOrEqual(8);
      expect(postSettle.runtimeAgentProcs, '0 orphan runtime agent child processes after DELETE + settle').toBe(0);
      expect(postSettle.agentChildren.count).toBeLessThanOrEqual(n0.agentChildren.count);
      expect(postSettle.daemon.cpuPct, 'daemon CPU returns < 8% after settle').toBeLessThan(8);
      expect(postSettle.daemon.rssMb, 'daemon RSS stays bounded (< baseline + 60 MB)').toBeLessThan(
        n0.daemon.rssMb + 60,
      );
    } finally {
      for (const id of sessionIds) {
        await conn.ctx.fetch(`${base}/api/v1/runtime-sessions/${id}`, { method: 'DELETE' }).catch(() => {});
      }
      await conn.ctx.dispose().catch(() => {});
    }
  });
});
