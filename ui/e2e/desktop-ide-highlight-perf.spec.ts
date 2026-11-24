import { test, expect, type Page } from '@playwright/test';
import { existsSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// Highlighting speed on real-sized files (owner, 2026-11-24: "the formatting of makefile is
// very slow … i scroll, and i see the colors slowly added"). For each language: open the
// file (time until the screen is coloured), then scroll top to bottom and require every
// line on screen to carry its final token classes in the first frame after it arrives, with
// no long animation frame. HIGHLIGHT_REPORT_ONLY=1 only prints the numbers (used to compare
// against an older build).

const REPORT_ONLY = !!process.env.HIGHLIGHT_REPORT_ONLY;
const mkSample = join(process.cwd(), 'unit', 'fixtures', 'Makefile.sample');
const MAKEFILE = existsSync(mkSample) ? readFileSync(mkSample, 'utf8') : 'APP := x\nbuild: ## Build\n\tgo build $(FLAGS) ./...\n';
const COMMENT = Array.from({ length: 12 }, () => '# ' + 'This target builds the acceptance suite with every flag the CI pipeline needs, see docs/ci.md. '.repeat(3)).join('\n');

export function repeatTo(lines: number, block: (i: number) => string): string {
  const out: string[] = [];
  for (let i = 0; out.length < lines; i++) out.push(...block(i).split('\n'));
  return out.slice(0, lines).join('\n') + '\n';
}

export const BIG_FILES: Record<string, { text: string; probe: string }> = {
  'Big.mk': {
    text: repeatTo(2000, (i) => `${MAKEFILE.replace(/^(\w[\w-]*):/gm, `$1_${i}:`)}\n${COMMENT}\nVERSION_${i} := $(shell git describe --tags --always --dirty 2>/dev/null | sed -e 's/^v//')`),
    probe: 'SHELL := /bin/bash',
  },
  'big.go': {
    text: 'package main\n\nimport (\n\t"fmt"\n\t"strings"\n)\n\n' + repeatTo(2993, (i) => `// Handler${i} formats a greeting for request ${i}.\nfunc Handler${i}(name string, n int) (string, error) {\n\tif n < 0 {\n\t\treturn "", fmt.Errorf("bad count %d for %s", n, name)\n\t}\n\tparts := make([]string, 0, n)\n\tfor j := 0; j < n; j++ {\n\t\tparts = append(parts, strings.Repeat(name, j))\n\t}\n\treturn strings.Join(parts, ","), nil\n}\n`),
    probe: 'func Handler0(',
  },
  'big.ts': {
    text: repeatTo(3000, (i) => `/** Loads page ${i}. */\nexport async function load${i}(id: string, retries = 3): Promise<Record<string, number>> {\n  const res = await fetch(\`/api/items/\${id}?page=${i}\`);\n  if (!res.ok) throw new Error(\`load ${i} failed: \${res.status}\`);\n  const body = (await res.json()) as { items: { k: string; v: number }[] };\n  return Object.fromEntries(body.items.map((x) => [x.k, x.v * retries]));\n}\n`),
    probe: 'export async function load0(',
  },
  'big.md': {
    text: repeatTo(3000, (i) => `## Section ${i}\n\nSome **bold** text, a [link](https://example.com/${i}) and \`code\`.\n\n- item one\n- item two\n\n\`\`\`go\nfunc x${i}() {}\n\`\`\`\n`),
    probe: '## Section 0',
  },
};

test.describe.configure({ mode: 'serial' }); // one file at a time: the timings are compared

let root: string | null = null;
let wsId = '';
async function seed(): Promise<void> {
  if (root) return;
  root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-hl-')));
  for (const [name, f] of Object.entries(BIG_FILES)) writeFileSync(join(root, name), f.text);
  const c = await apiCtx();
  try {
    wsId = (await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE Highlight ${Date.now()}`, root_path: root })).id;
  } finally {
    await c.ctx.dispose();
  }
}

async function boot(page: Page): Promise<void> {
  await seed();
  await page.addInitScript(({ wsId, root }) => {
    if (sessionStorage.getItem('__ide_hl_init')) return;
    sessionStorage.setItem('__ide_hl_init', '1');
    localStorage.setItem('otto_workspace', wsId);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_scheme', 'dark');
    localStorage.setItem('otto.ide.preferences.v1', JSON.stringify({ autoSave: false }));
    localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-hl', name: 'HL', folders: [{ path: root, name: 'hl' }], updatedAt: Date.now() }]));
  }, { wsId, root: root! });
  await page.goto('/#/ide');
  await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 30_000 });
}

/** Each on-screen line's span classes, read two frames after setting `top` (CodeMirror
 *  draws the new lines in the frame after the scroll). */
async function visibleMarkup(page: Page, top: number | null): Promise<Record<string, string>> {
  return page.evaluate((y) => new Promise<Record<string, string>>((resolve) => {
    const sc = document.querySelector('.ide-editor-area .cm-scroller')!;
    if (y != null) sc.scrollTop = y;
    requestAnimationFrame(() => requestAnimationFrame(() => {
      const box = sc.getBoundingClientRect();
      const out: Record<string, string> = {};
      for (const l of sc.querySelectorAll<HTMLElement>('.cm-line')) {
        const r = l.getBoundingClientRect();
        if (r.bottom < box.top || r.top > box.bottom || !l.textContent?.trim()) continue;
        // Syntax classes only (ͼ…): the IDE's semantic layer refines names later on purpose.
        out[l.textContent] = [...l.querySelectorAll('span')].flatMap((s) => [...s.classList].filter((c) => c.startsWith('ͼ'))).join(' ');
      }
      resolve(out);
    }));
  }), top);
}

for (const [name, f] of Object.entries(BIG_FILES)) {
  test(`${name}: coloured on open and the moment each line scrolls into view`, async ({ page }) => {
    test.setTimeout(240_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await boot(page);
    // Long animation frames (Chromium): every frame over 50 ms while the file opens and scrolls.
    await page.evaluate(() => {
      const w = window as unknown as { __loaf: number[] };
      w.__loaf = [];
      new PerformanceObserver((l) => { for (const e of l.getEntries()) w.__loaf.push(Math.round(e.duration)); })
        .observe({ type: 'long-animation-frame', buffered: false });
    });
    const item = page.getByTestId('ide-project-tree').locator(`button.tree-item[title="${name}"], button.tree-item[title$="/${name}"]`).first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    const t0 = await page.evaluate(() => performance.now());
    await item.dblclick();
    // Time to coloured: the first frame in which the probe line's tokens carry classes.
    const tColoured = await page.waitForFunction((probe) => {
      const l = [...document.querySelectorAll<HTMLElement>('.ide-editor-area .cm-line')].find((e) => e.textContent?.startsWith(probe));
      return l && l.querySelector('span[class]') ? performance.now() : null;
    }, f.probe, { polling: 'raf', timeout: 30_000 }).then((h) => h.jsonValue() as Promise<number>, (e: unknown) => {
      if (REPORT_ONLY) return null; // an older build with no language for this file
      throw e;
    });
    const openMs = tColoured == null ? null : Math.round(tColoured - t0);
    await page.waitForTimeout(800);
    await page.evaluate(() => { (window as unknown as { __loaf: number[] }).__loaf = []; });
    const height = await page.evaluate(() => document.querySelector('.ide-editor-area .cm-scroller')!.scrollHeight);
    const late: string[] = [];
    let steps = 0;
    for (let y = 0; y < height; y += 700) {
      const first = await visibleMarkup(page, y);
      await page.waitForTimeout(300);
      const settled = await visibleMarkup(page, null);
      steps++;
      for (const [text, cls] of Object.entries(settled)) if (text in first && first[text] !== cls) late.push(`@${y}: ${text.slice(0, 60)} | ${first[text]} -> ${cls}`);
    }
    const loaf = await page.evaluate(() => (window as unknown as { __loaf: number[] }).__loaf);
    if (late.length) console.log(late.slice(0, 4).join('\n'));
    console.log(`HL ${JSON.stringify({ file: name, openMs, steps, lateLines: late.length, longFrames: loaf.length, worstFrameMs: Math.max(0, ...loaf) })}`);
    if (REPORT_ONLY) return;
    expect(late, 'lines coloured after they arrived').toEqual([]);
    expect(loaf.filter((d) => d > 100), 'frames over 100 ms while scrolling').toEqual([]);
  });
}
