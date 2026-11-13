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

// Otto IDE — Desktop Performance, Virtualization, Retention, Cold-Load & Zero-Leak Gate
// Runs on BOTH `desktop-webkit` (WKWebView proxy) and `desktop-browser` (Chromium):
// 1) 4.0s idle zero-polling + CPU/RSS gate; 2) 2,500-file tree virtualization + 5,000-line
// keystroke/Prefix-Suffix latency; 3) 20-tab JS heap + RSS retention; 4) N=0,1,4,8 parallel load;
// 5) 5-file workspace cold load (<150ms) + <10ms tree / <25ms runner discovery + zero stale FS cache;
// 6) <100MB IDE overhead / <1GB Otto ceiling & 25-cycle 8-language zero-leak torture gate.

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
  clickhouse: ProcMetric;
  collector: ProcMetric;
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
  return {
    daemon: merge('daemon'),
    browser: merge('browser'),
    agentChildren: merge('agent-children'),
    clickhouse: merge('clickhouse'),
    collector: merge('collector'),
    total: merge('total'),
    runtimeAgentProcs: countRuntimeAgentProcs(daemonPid),
  };
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

  test('5. 5-file workspace cold load (< 150ms), sub-10ms tree, sub-25ms discovery & zero stale FS/tree cache', async ({
    page,
  }, info) => {
    test.setTimeout(90_000);
    const fatal = watchFatalUiErrors(page);
    const coldRoot = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-cold5-')));
    writeFileSync(join(coldRoot, 'go.mod'), 'module example.com/cold5\n\ngo 1.22\n');
    writeFileSync(
      join(coldRoot, 'main.go'),
      'package main\n\nimport "fmt"\n\nfunc main() {\n\tfmt.Println("ide cold five-file ready")\n}\n',
    );
    writeFileSync(join(coldRoot, 'calc.go'), 'package main\n\nfunc AddCold(a, b int) int { return a + b }\n');
    writeFileSync(
      join(coldRoot, 'calc_test.go'),
      'package main\n\nimport "testing"\n\nfunc TestAddCold(t *testing.T) { if AddCold(2, 3) != 5 { t.Fatal() } }\n',
    );
    writeFileSync(join(coldRoot, 'config.json'), '{\n  "service": "cold5",\n  "port": 8080\n}\n');

    const conn = await apiCtx();
    let coldWsId = '';
    const coldWsName = `IDE Cold5 ${Date.now()}`;
    try {
      const wsCreated = await apiJson<{ id: string }>(conn.ctx, 'POST', '/workspaces', {
        name: coldWsName,
        root_path: coldRoot,
      });
      coldWsId = wsCreated.id;

      // 1. Measure cold backend tree listing (`< 10ms`) & runner discovery (`< 25ms`) on never-visited root.
      const tTree0 = performance.now();
      const treeResp = await apiJson<{ entries: Array<{ name: string }> }>(
        conn.ctx,
        'GET',
        `/ide/fs/tree?kind=local&root=${encodeURIComponent(coldRoot)}`,
      );
      const coldTreeMs = performance.now() - tTree0;

      const tEnv0 = performance.now();
      const envResp = await apiJson<{ configs: Array<{ runner: string }> }>(
        conn.ctx,
        'GET',
        `/ide/environments?kind=local&root=${encodeURIComponent(coldRoot)}&workspace_id=${encodeURIComponent(coldWsId)}`,
      );
      const coldEnvMs = performance.now() - tEnv0;

      expect(treeResp.entries.map((e) => e.name)).toEqual(
        expect.arrayContaining(['go.mod', 'main.go', 'calc.go', 'calc_test.go', 'config.json']),
      );
      expect(envResp.configs.length).toBeGreaterThan(0);

      // 2. Measure end-to-end UI workspace cold open (click workspace -> tree + first file painted in CodeMirror).
      await bootIde(page);
      await page.getByTestId('ide-target-switcher').click();
      const wsPopover = page.getByTestId('ide-workspace-popover');
      await expect(wsPopover).toBeVisible({ timeout: 10_000 });
      await expect(wsPopover.locator('[data-testid="ide-workspace-option"]', { hasText: coldWsName })).toBeVisible({
        timeout: 10_000,
      });

      const coldUiOpenMs = await page.evaluate(async (targetName: string) => {
        const opts = Array.from(document.querySelectorAll<HTMLButtonElement>('[data-testid="ide-workspace-option"]'));
        const btn = opts.find((el) => (el.textContent ?? '').includes(targetName));
        if (!btn) throw new Error(`Workspace option not found: ${targetName}`);
        const t0 = performance.now();
        btn.click();
        await new Promise<void>((resolve, reject) => {
          const deadline = performance.now() + 5_000;
          const poll = (): void => {
            const cm = document.querySelector('.cm-editor .cm-content');
            if ((cm?.textContent ?? '').includes('ide cold five-file ready')) {
              requestAnimationFrame(() => {
                const ch = new MessageChannel();
                ch.port1.onmessage = () => {
                  ch.port1.close();
                  resolve();
                };
                ch.port2.postMessage(null);
              });
              return;
            }
            if (performance.now() > deadline) {
              reject(new Error('Timed out waiting for cold5 main.go in CodeMirror'));
              return;
            }
            requestAnimationFrame(poll);
          };
          requestAnimationFrame(poll);
        });
        return performance.now() - t0;
      }, coldWsName);

      // 3. Zero stale FS/tree cache ("no devil caches"): external create, modify, delete reflect immediately.
      const extPath = join(coldRoot, 'external_live.go');
      writeFileSync(extPath, 'package main\n\nconst ExternalLive = 1\n');
      const treeAfterCreate = await apiJson<{ entries: Array<{ name: string }> }>(
        conn.ctx,
        'GET',
        `/ide/fs/tree?kind=local&root=${encodeURIComponent(coldRoot)}`,
      );
      const createdVisibleImmediately = treeAfterCreate.entries.some((e) => e.name === 'external_live.go');

      const fileBeforeMod = await apiJson<{ content: string; sha256: string }>(
        conn.ctx,
        'GET',
        `/ide/fs/file?kind=local&root=${encodeURIComponent(coldRoot)}&path=${encodeURIComponent(extPath)}`,
      );
      writeFileSync(extPath, 'package main\n\nconst ExternalLive = 2 // external disk edit\n');
      const fileAfterMod = await apiJson<{ content: string; sha256: string }>(
        conn.ctx,
        'GET',
        `/ide/fs/file?kind=local&root=${encodeURIComponent(coldRoot)}&path=${encodeURIComponent(extPath)}`,
      );

      rmSync(extPath, { force: true });
      const treeAfterDelete = await apiJson<{ entries: Array<{ name: string }> }>(
        conn.ctx,
        'GET',
        `/ide/fs/tree?kind=local&root=${encodeURIComponent(coldRoot)}`,
      );
      const deletedGoneImmediately = !treeAfterDelete.entries.some((e) => e.name === 'external_live.go');

      emitPerfMetric(info, 'test5_cold_open_and_no_stale_cache', {
        coldTreeMs: round2(coldTreeMs),
        coldEnvMs: round2(coldEnvMs),
        coldUiOpenMs: round2(coldUiOpenMs),
        createdVisibleImmediately,
        shaChangedImmediately: fileAfterMod.sha256 !== fileBeforeMod.sha256,
        deletedGoneImmediately,
      });

      expect(fatal).toEqual([]);
      expect(coldTreeMs, '5-file GET /ide/fs/tree < 10ms').toBeLessThan(perfBudget(10));
      expect(coldEnvMs, '5-file GET /ide/environments < 25ms').toBeLessThan(perfBudget(25));
      expect(coldUiOpenMs, '5-file end-to-end cold workspace load < 150ms').toBeLessThan(perfBudget(150));
      expect(createdVisibleImmediately, 'external file creation visible immediately without stale cache').toBe(true);
      expect(fileAfterMod.content).toContain('ExternalLive = 2 // external disk edit');
      expect(fileAfterMod.sha256).not.toBe(fileBeforeMod.sha256);
      expect(deletedGoneImmediately, 'external file deletion visible immediately without stale cache').toBe(true);
    } finally {
      if (coldWsId) {
        await conn.ctx.fetch(`${base}/api/v1/workspaces/${coldWsId}`, { method: 'DELETE' }).catch(() => {});
      }
      await conn.ctx.dispose().catch(() => {});
      rmSync(coldRoot, { recursive: true, force: true });
    }
  });

  test('6. < 100 MB IDE memory budget, < 1 GB Otto ceiling & 25-cycle 8-language zero-leak torture gate', async ({
    page,
    browserName,
  }, info) => {
    test.setTimeout(240_000);
    const fatal = watchFatalUiErrors(page);

    const poly8Root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-poly8-')));
    const altRoot = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-alt-')));

    // Seed all 8 mandatory languages in `poly8Root` + minimal `altRoot` for workspace-switch eviction.
    writeFileSync(join(poly8Root, 'go.mod'), 'module example.com/poly8\n\ngo 1.22\n');
    writeFileSync(
      join(poly8Root, 'service.go'),
      'package main\n\nfunc HandleAdmission(code int) int {\n\treturn code + 1\n}\n',
    );
    writeFileSync(join(poly8Root, 'package.json'), '{"name":"poly8","version":"1.0.0","type":"module"}\n');
    writeFileSync(
      join(poly8Root, 'index.ts'),
      'export function computeScore(v: number): number {\n  return v * 2;\n}\n',
    );
    writeFileSync(
      join(poly8Root, 'App.java'),
      'public class App {\n  public static int run(int n) {\n    return n + 1;\n  }\n}\n',
    );
    writeFileSync(
      join(poly8Root, 'Cargo.toml'),
      '[package]\nname = "poly8"\nversion = "0.1.0"\nedition = "2021"\n',
    );
    writeFileSync(join(poly8Root, 'lib.rs'), 'pub fn check_quota(n: u32) -> bool {\n    n < 100\n}\n');
    writeFileSync(
      join(poly8Root, 'worker.py'),
      'def process_batch(items: list[int]) -> int:\n    return sum(items)\n',
    );
    writeFileSync(
      join(poly8Root, 'deploy.sh'),
      '#!/usr/bin/env bash\nrun_step() {\n  echo "step:$1"\n}\nrun_step "ok"\n',
    );
    writeFileSync(
      join(poly8Root, 'manifest.json'),
      JSON.stringify({ service: 'admission', limits: { rps: 500, burst: 1000 } }, null, 2) + '\n',
    );
    const csvRows = ['ts,route,latency_ms,status'];
    for (let r = 1; r <= 60; r++) {
      csvRows.push(`2026-11-13T12:00:${String(r % 60).padStart(2, '0')}Z,/v1/check,${10 + (r % 25)},200`);
    }
    writeFileSync(join(poly8Root, 'metrics.csv'), csvRows.join('\n') + '\n');

    writeFileSync(join(altRoot, 'go.mod'), 'module example.com/altws\n\ngo 1.22\n');
    writeFileSync(join(altRoot, 'alt.go'), 'package altws\n\nfunc Ping() string { return "pong" }\n');

    const countOrphanIdeTools = (): number => {
      const out = execFileSync('ps', ['-axo', 'pid=,ppid=,command='], { encoding: 'utf8', timeout: 3_000 });
      const rows = out
        .trim()
        .split('\n')
        .map((l) => /^\s*(\d+)\s+(\d+)\s+(.+)$/.exec(l))
        .filter(Boolean) as RegExpExecArray[];
      const ppidMap = new Map(rows.map((m) => [Number(m[1]), Number(m[2])]));
      const isDaemonDescendant = (pid: number): boolean => {
        for (let d = 0, cur = pid; d < 32 && cur > 1; d++) {
          if (cur === daemonPid) return true;
          cur = ppidMap.get(cur) ?? 0;
        }
        return false;
      };
      return rows.filter((m) => {
        const pid = Number(m[1]);
        const cmd = m[3];
        if (!/(?:\bdlv\b|\bgopls\b|\bjdtls\b|\brust-analyzer\b|\bpyright\b|typescript-language-server|otto-sh-dap|\bdebugpy\b)/.test(cmd)) {
          return false;
        }
        return isDaemonDescendant(pid) || cmd.includes(poly8Root) || cmd.includes(altRoot);
      }).length;
    };

    const sampleCdpMetrics = async (): Promise<{ jsHeapBytes: number; nodes: number; listeners: number } | null> => {
      if (browserName !== 'chromium') return null;
      const cdp = await page.context().newCDPSession(page);
      try {
        await cdp.send('Performance.enable');
        await cdp.send('HeapProfiler.collectGarbage');
        await cdp.send('HeapProfiler.collectGarbage');
        const [{ usedSize }, { metrics }] = await Promise.all([
          cdp.send('Runtime.getHeapUsage'),
          cdp.send('Performance.getMetrics'),
        ]);
        const pickMetric = (name: string): number => metrics.find((m) => m.name === name)?.value ?? 0;
        return {
          jsHeapBytes: usedSize,
          nodes: pickMetric('Nodes'),
          listeners: pickMetric('JSEventListeners'),
        };
      } finally {
        await cdp.detach().catch(() => {});
      }
    };

    const conn = await apiCtx();
    let poly8WsId = '';
    let altWsId = '';
    const poly8Name = `IDE Poly8 ${Date.now()}`;
    const altName = `IDE AltSwitch ${Date.now()}`;

    try {
      poly8WsId = (
        await apiJson<{ id: string }>(conn.ctx, 'POST', '/workspaces', {
          name: poly8Name,
          root_path: poly8Root,
        })
      ).id;
      altWsId = (
        await apiJson<{ id: string }>(conn.ctx, 'POST', '/workspaces', {
          name: altName,
          root_path: altRoot,
        })
      ).id;

      // Pre-IDE baseline on `#/home` to measure exact IDE memory delta.
      await page.addInitScript((wsId: string) => {
        localStorage.setItem('otto_workspace', wsId);
        localStorage.setItem('otto_rail_expanded', '0');
        localStorage.setItem('otto.ide.dockOpen', '0');
        localStorage.setItem('otto.ide.agentOpen', '0');
      }, poly8WsId);
      await page.goto('/#/home');
      await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
      await sleep(300);
      const preIdeRes = await sampleResources(daemonPid, 400);
      const preIdeCdp = await sampleCdpMetrics();

      // Navigate into `#/ide` on `poly8WsId`.
      await page.goto('/#/ide');
      await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 20_000 });
      await expect(page.locator('.cm-editor .cm-content').first()).toBeVisible({ timeout: 20_000 });

      const eightFiles = [
        'service.go',
        'index.ts',
        'App.java',
        'lib.rs',
        'worker.py',
        'deploy.sh',
        'manifest.json',
        'metrics.csv',
      ];

      const switchWorkspaceTo = async (wsLabel: string): Promise<void> => {
        await page.getByTestId('ide-target-switcher').click();
        const pop = page.getByTestId('ide-workspace-popover');
        await expect(pop).toBeVisible({ timeout: 10_000 });
        await pop.locator('[data-testid="ide-workspace-option"]', { hasText: wsLabel }).first().click();
        await expect(pop).toBeHidden({ timeout: 10_000 });
      };

      let activePeakRes: Awaited<ReturnType<typeof sampleResources>> | null = null;

      const runOneTortureCycle = async (cycleIdx: number): Promise<void> => {
        const tree = page.getByTestId('ide-project-tree');
        for (const fname of eightFiles) {
          await tree.locator(`button.tree-item[title="${fname}"]`).first().click();
        }
        await expect(page.getByTestId('ide-csv-view')).toBeVisible({ timeout: 10_000 });

        // Switch back to Go file, edit buffer, and save cleanly via Cmd+S.
        await page.locator('.ide-tab-btn', { hasText: 'service.go' }).first().click();
        const editor = page.locator('.cm-editor .cm-content').first();
        await expect(editor).toContainText('HandleAdmission', { timeout: 10_000 });
        await editor.click();
        await page.keyboard.press('Meta+ArrowDown');
        await page.keyboard.insertText(`\n// cycle ${cycleIdx}`);
        await page.keyboard.press('Meta+s');
        await expect(page.locator('.ide-dirty-dot')).toHaveCount(0, { timeout: 5_000 });

        // Cmd+F12 Move-to-Function / File Structure modal open + close.
        await page.getByTestId('ide-structure-btn').click();
        await expect(page.getByTestId('ide-file-structure-modal')).toBeVisible({ timeout: 8_000 });
        await page.keyboard.press('Escape');
        await expect(page.getByTestId('ide-file-structure-modal')).toBeHidden({ timeout: 8_000 });

        // Find in Path / Search Everywhere modal open + close.
        await page.keyboard.press('Meta+Shift+f');
        const seModal = page.locator('.sheet[role="dialog"]').last();
        await expect(seModal).toBeVisible({ timeout: 8_000 });
        await page.keyboard.press('Escape');
        await expect(seModal).toBeHidden({ timeout: 8_000 });

        // Run Configurations modal + stacked SDK Toolchains modal open + close.
        await page.getByTestId('ide-edit-configs-btn').click();
        const envModal = page.locator('.sheet[role="dialog"]').last();
        await expect(envModal).toBeVisible({ timeout: 8_000 });
        await envModal.getByTestId('ide-cfg-open-toolchains').click();
        const tcModal = page.locator('.sheet[role="dialog"]').last();
        await expect(tcModal).toContainText(/SDKs, toolchains/i, { timeout: 8_000 });
        await page.keyboard.press('Escape');
        await page.keyboard.press('Escape');
        await expect(page.locator('.sheet[role="dialog"]')).toHaveCount(0, { timeout: 8_000 });

        if (cycleIdx === 25) {
          activePeakRes = await sampleResources(daemonPid, 250);
        }

        // Close all open tabs via tab-bar overflow menu.
        await page.getByTestId('ide-tabs-overflow-btn').click();
        const menu = page.locator('.ctx-menu');
        await expect(menu).toBeVisible({ timeout: 5_000 });
        await menu.getByRole('menuitem', { name: /Close all tabs/i }).click();
        await expect(page.locator('.ide-tab')).toHaveCount(0, { timeout: 8_000 });

        // Switch workspace (`poly8` -> `alt` -> `poly8`) to exercise target & LSP eviction.
        await switchWorkspaceTo(altName);
        await expect(tree.locator('button.tree-item[title="alt.go"]').first()).toBeVisible({ timeout: 10_000 });
        await switchWorkspaceTo(poly8Name);
        await expect(tree.locator('button.tree-item[title="service.go"]').first()).toBeVisible({ timeout: 10_000 });
      };

      // Warm 2 cycles so lazy chunks (`IdeCsvView`, modals, CodeMirror lang packs) settle.
      await runOneTortureCycle(0);
      await runOneTortureCycle(1);
      // Close the auto-opened initial tab after warm-up so cycle baseline and cycle-25 end in identical state (0 tabs).
      if ((await page.locator('.ide-tab').count()) > 0) {
        await page.getByTestId('ide-tabs-overflow-btn').click();
        await page.locator('.ctx-menu').getByRole('menuitem', { name: /Close all tabs/i }).click();
        await expect(page.locator('.ide-tab')).toHaveCount(0, { timeout: 8_000 });
      }
      await sleep(350);

      const cycle0Res = await sampleResources(daemonPid, 400);
      const cycle0Cdp = await sampleCdpMetrics();
      const liveDomElementsCycle0 = await page.evaluate(() => document.getElementsByTagName('*').length);

      const ideDaemonDeltaMb = round2(Math.max(0, cycle0Res.daemon.rssMb - preIdeRes.daemon.rssMb));
      const ideHeapDeltaMb =
        cycle0Cdp && preIdeCdp
          ? round2(Math.max(0, (cycle0Cdp.jsHeapBytes - preIdeCdp.jsHeapBytes) / (1024 * 1024)))
          : null;

      // Execute 25 full cycles across all 8 languages, modals, tab evictions, and workspace switches!
      for (let c = 1; c <= 25; c++) {
        await runOneTortureCycle(c);
      }
      if ((await page.locator('.ide-tab').count()) > 0) {
        await page.getByTestId('ide-tabs-overflow-btn').click();
        await page.locator('.ctx-menu').getByRole('menuitem', { name: /Close all tabs/i }).click();
        await expect(page.locator('.ide-tab')).toHaveCount(0, { timeout: 8_000 });
      }
      const liveDomElementsCycle25 = await page.evaluate(() => document.getElementsByTagName('*').length);
      const liveDomElementsDelta = liveDomElementsCycle25 - liveDomElementsCycle0;

      // Switch to `altName` at the end so any `poly8` LSP servers are evicted immediately on workspace switch.
      await switchWorkspaceTo(altName);
      if ((await page.locator('.ide-tab').count()) > 0) {
        await page.getByTestId('ide-tabs-overflow-btn').click();
        await page.locator('.ctx-menu').getByRole('menuitem', { name: /Close all tabs/i }).click();
      }
      await sleep(600);

      const cycle25Res = await sampleResources(daemonPid, 400);
      const cycle25Cdp = await sampleCdpMetrics();
      const orphanIdeTools = countOrphanIdeTools();

      const heapSlopeMb =
        cycle0Cdp && cycle25Cdp ? round2((cycle25Cdp.jsHeapBytes - cycle0Cdp.jsHeapBytes) / (1024 * 1024)) : null;
      const domNodesDelta = cycle0Cdp && cycle25Cdp ? cycle25Cdp.nodes - cycle0Cdp.nodes : null;
      const listenersDelta = cycle0Cdp && cycle25Cdp ? cycle25Cdp.listeners - cycle0Cdp.listeners : null;
      const daemonRssSlopeMb = round2(cycle25Res.daemon.rssMb - cycle0Res.daemon.rssMb);

      const daemonTreeRssMbCycle0 = round2(
        cycle0Res.daemon.rssMb +
          cycle0Res.agentChildren.rssMb +
          cycle0Res.clickhouse.rssMb +
          cycle0Res.collector.rssMb,
      );
      const daemonTreeRssMbCycle25 = round2(
        cycle25Res.daemon.rssMb +
          cycle25Res.agentChildren.rssMb +
          cycle25Res.clickhouse.rssMb +
          cycle25Res.collector.rssMb,
      );
      const peakRes = activePeakRes ?? cycle25Res;
      const activePeakOttoCoreRssMb = round2(
        peakRes.daemon.rssMb + peakRes.agentChildren.rssMb + peakRes.collector.rssMb,
      );
      const activePeakDaemonTreeRssMb = round2(activePeakOttoCoreRssMb + peakRes.clickhouse.rssMb);

      emitPerfMetric(info, 'test6_memory_budget_and_25cycle_torture', {
        browserEngine: browserName,
        preIdeDaemonRssMb: preIdeRes.daemon.rssMb,
        ideDaemonDeltaMb,
        ideHeapDeltaMb,
        activePeakOttoCoreRssMb,
        activePeakDaemonTreeRssMb,
        activePeakAgentChildrenRssMb: peakRes.agentChildren.rssMb,
        daemonTreeRssMbCycle0,
        daemonTreeRssMbCycle25,
        clickhouseRssMbCycle25: cycle25Res.clickhouse.rssMb,
        totalWithTestRunnerBrowserMbCycle25: cycle25Res.total.rssMb,
        cycle0HeapMb: cycle0Cdp ? round2(cycle0Cdp.jsHeapBytes / (1024 * 1024)) : null,
        cycle25HeapMb: cycle25Cdp ? round2(cycle25Cdp.jsHeapBytes / (1024 * 1024)) : null,
        heapSlopeMb,
        liveDomElementsCycle0,
        liveDomElementsCycle25,
        liveDomElementsDelta,
        domNodesCycle0: cycle0Cdp?.nodes ?? null,
        domNodesCycle25: cycle25Cdp?.nodes ?? null,
        domNodesDelta,
        listenersCycle0: cycle0Cdp?.listeners ?? null,
        listenersCycle25: cycle25Cdp?.listeners ?? null,
        listenersDelta,
        daemonRssSlopeMb,
        orphanIdeTools,
      });

      expect(fatal).toEqual([]);
      expect(ideDaemonDeltaMb, 'ottod IDE RSS delta < 80 MB').toBeLessThan(80);
      expect(activePeakOttoCoreRssMb, 'ottod + active LSP/DAP children RSS < 450 MB at peak 8-tab load').toBeLessThan(450);
      expect(activePeakDaemonTreeRssMb, 'whole ottod + active LSP/DAP + ClickHouse tree RSS < 1 GB at peak').toBeLessThan(1024);
      expect(daemonTreeRssMbCycle0, 'whole ottod daemon process tree RSS < 1 GB (cycle 0)').toBeLessThan(1024);
      expect(daemonTreeRssMbCycle25, 'whole ottod daemon process tree RSS < 1 GB (cycle 25)').toBeLessThan(1024);
      expect(liveDomElementsDelta, '25-cycle live DOM element delta <= 25 (zero leaked DOM subtrees)').toBeLessThanOrEqual(25);
      expect(daemonRssSlopeMb, 'ottod RSS stays flat across 25 8-language cycles').toBeLessThan(25);
      expect(orphanIdeTools, 'zero leaked LSP/DAP child processes after workspace switch & close').toBe(0);

      if (cycle25Cdp) {
        const heap25Mb = round2(cycle25Cdp.jsHeapBytes / (1024 * 1024));
        expect(activePeakDaemonTreeRssMb + heap25Mb, 'peak Otto daemon tree + UI JSHeap < 1 GB').toBeLessThan(1024);
        expect(daemonTreeRssMbCycle25 + heap25Mb, 'settled Otto daemon tree + UI JSHeap < 1 GB').toBeLessThan(1024);
      }
      if (ideHeapDeltaMb !== null) {
        expect(ideHeapDeltaMb, 'browser IDE JSHeapUsedSize delta < 80 MB').toBeLessThan(80);
        expect(
          ideDaemonDeltaMb + ideHeapDeltaMb,
          'combined IDE daemon + JSHeap overhead < 100 MB',
        ).toBeLessThan(100);
      }
      if (heapSlopeMb !== null && domNodesDelta !== null && listenersDelta !== null) {
        expect(heapSlopeMb, '25-cycle net JSHeapUsedSize slope < 5 MB').toBeLessThan(5);
        expect(domNodesDelta, '25-cycle net DOM Nodes delta <= 25').toBeLessThanOrEqual(25);
        expect(listenersDelta, '25-cycle net JSEventListeners delta <= 5').toBeLessThanOrEqual(5);
      }
    } finally {
      if (poly8WsId) {
        await conn.ctx.fetch(`${base}/api/v1/workspaces/${poly8WsId}`, { method: 'DELETE' }).catch(() => {});
      }
      if (altWsId) {
        await conn.ctx.fetch(`${base}/api/v1/workspaces/${altWsId}`, { method: 'DELETE' }).catch(() => {});
      }
      await conn.ctx.dispose().catch(() => {});
      rmSync(poly8Root, { recursive: true, force: true });
      rmSync(altRoot, { recursive: true, force: true });
    }
  });
});
