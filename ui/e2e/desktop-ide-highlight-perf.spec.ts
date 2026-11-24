import { test, expect, type Page } from '@playwright/test';
import { existsSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall } from './ide-polyglot-fixtures';
import { budgetMs, frameDeltas } from './perf';
import { apiCtx } from './seed';

// Highlighting speed on real-sized files (owner, 2026-11-24: "the formatting of makefile is
// very slow … i scroll, and i see the colors slowly added"). For each language: open the
// file (time until the screen is coloured), then scroll top to bottom and require every
// line on screen to carry its final token classes in the first frame after it arrives, and
// no scroll-driven frame over 50 ms (Chromium: long animation frames; WebKit, which has
// none: rAF-to-rAF gaps; both scaled by OTTO_PERF_BUDGET_SCALE). HIGHLIGHT_REPORT_ONLY=1
// only prints the numbers (used to compare against an older build).

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
  'application.properties': {
    text: repeatTo(2000, (i) => 'spring.datasource' + i + '.url=jdbc:mysql://' + '$' + '{DB_HOST:localhost}:3306/games\n# comment ' + i + '\nfeature' + i + '.flags={"enabled": true, "rollout": [1, 2, 3], "nested": {"a": [{"b": null}]}}\nlong' + i + '=' + 'value with words '.repeat(30)),
    probe: 'spring.datasource0.url=',
  },
  'long.properties': {
    // 5,000 lines of 4 KB (the owner's file: long values, JSON inside them).
    text: repeatTo(5000, (i) => 'kafka.topic' + i + '.config={' + Array.from({ length: 110 }, (_, k) => '"k' + k + '": ' + (k % 3 ? '"v' + k + '"' : k)).join(', ') + '}'),
    probe: 'kafka.topic0.config=',
  },
  'deploy.yaml': {
    text: repeatTo(2000, (i) => 'service' + i + ':\n  image: "nginx:1.' + i + '"\n  ports: [80, 443]\n  env: &env' + i + '\n    A: 1\n  other: *env' + i + '\n  script: |\n    echo ' + i + '\n# comment\n---'),
    probe: 'service0:',
  },
  'Cargo.toml': {
    text: repeatTo(2000, (i) => '[package' + i + ']\nname = "otto-' + i + '"\nversion = "0.' + i + '.0"\n# comment\nserde = { version = "1", features = ["derive"] }'),
    probe: '[package0]',
  },
  'setup.cfg': { text: repeatTo(2000, (i) => '[section' + i + ']\nhost = db' + i + '.internal ; comment\nport=5432\nenabled = yes'), probe: '[section0]' },
  'requirements.txt': { text: repeatTo(2000, (i) => 'requests' + i + '>=2.31,<3 ; python_version < "3.12"\n# pinned\ndjango==5.0.' + i), probe: 'requests0>=' },
  'Dockerfile': { text: repeatTo(2000, (i) => 'FROM golang:1.23 AS build' + i + '\nENV CGO_ENABLED=0 GOOS=linux\nRUN go build -o /out/app ./cmd/... && \\\n    echo "built $VERSION"\n# comment'), probe: 'FROM golang:1.23 AS build0' },
  'nginx.conf': { text: repeatTo(2000, (i) => 'server {\n  listen 44' + i + ' ssl;\n  location /api {\n    proxy_pass http://$upstream;\n  }\n}'), probe: 'server {' },
  'api.proto': { text: repeatTo(2000, (i) => 'message Bet' + i + ' {\n  string id = 1; // the id\n  repeated Leg legs = 2;\n}'), probe: 'message Bet0 {' },
  'main.tf': { text: repeatTo(2000, (i) => 'resource "aws_s3_bucket" "logs' + i + '" {\n  bucket = "logs-' + '$' + '{var.env}"\n  tags   = merge(local.tags, { a = true })\n}'), probe: 'resource "aws_s3_bucket" "logs0"' },
  'fix.diff': { text: repeatTo(2000, (i) => 'diff --git a/x' + i + ' b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@ func main\n-old\n+new\n same'), probe: 'diff --git a/x0' },
  'build.ps1': { text: repeatTo(2000, (i) => 'param([string]$Name' + i + ' = "x")\nif ($Name -eq $null) { Write-Host "hi" }'), probe: 'param([string]$Name0' },
  'build.bat': { text: repeatTo(2000, (i) => '@echo off\nREM build\n:loop' + i + '\nset NAME=%1'), probe: '@echo off' },
  'service.overrides': { text: repeatTo(2000, (i) => '# overrides ' + i + '\nhost' + i + ' = db.internal\nport' + i + ' = 5432'), probe: 'host0 = db.internal' },
  'App.tsx': {
    text: "import React, { useEffect, useState } from 'react';\n\n" + repeatTo(800, (i) => 'interface Props' + i + ' { id: string; count?: number }\n/** Row ' + i + '. */\nexport function Row' + i + '({ id, count = 0 }: Props' + i + '): JSX.Element {\n  const [open, setOpen] = useState<boolean>(false);\n  useEffect(() => { if (count > ' + i + ') setOpen(true); }, [count]);\n  return (\n    <div className="row" data-id={id} onClick={() => setOpen(!open)}>\n      {open ? <span>{id}</span> : null}\n    </div>\n  );\n}\n'),
    probe: "import React, { useEffect, useState } from 'react';",
  },
  'service.py': {
    text: 'import json\nfrom dataclasses import dataclass\n\n' + repeatTo(800, (i) => '@dataclass\nclass Bet' + i + ':\n    """A bet ' + i + '."""\n    amount: float = 0.0\n\n    def payout(self, odds: float) -> float:\n        if odds <= 1:\n            raise ValueError(f"bad odds {odds}")\n        return round(self.amount * odds, 2)  # rounded\n'),
    probe: 'import json',
  },
  'server.js': {
    text: "const express = require('express');\n\n" + repeatTo(800, (i) => '// route ' + i + '\napp.get("/api/' + i + '", async (req, res) => {\n  const rows = await db.query(\'SELECT * FROM t WHERE id = ?\', [req.params.id]);\n  res.json({ ok: true, rows, n: ' + i + ' });\n});\n'),
    probe: "const express = require('express');",
  },
  'build.mjs': {
    text: "import { readFile } from 'node:fs/promises';\n\n" + repeatTo(800, (i) => 'export async function step' + i + '(dir) {\n  const text = await readFile(dir + "/' + i + '.json", "utf8");\n  return JSON.parse(text).items.filter((x) => x.n > ' + i + ');\n}\n'),
    probe: "import { readFile } from 'node:fs/promises';",
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
  test(`${name}: coloured on open and the moment each line scrolls into view`, async ({ page, browserName }) => {
    test.setTimeout(240_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await boot(page);
    // Long animation frames (Chromium) over 50 ms while the file opens and scrolls.
    const loafOk = await page.evaluate(() => PerformanceObserver.supportedEntryTypes.includes('long-animation-frame'));
    if (browserName === 'chromium') expect(loafOk, 'Chromium reports long animation frames').toBe(true);
    if (loafOk) await page.evaluate(() => {
      const w = window as unknown as { __loaf: number[] };
      w.__loaf = [];
      new PerformanceObserver((l) => { for (const e of l.getEntries() as (PerformanceEntry & { scripts?: { invoker: string }[] })[]) {
          // Only frames the scroll drove (its handlers or the editor's measure frame).
          if (e.scripts?.some((s) => /onscroll|FrameRequestCallback/i.test(s.invoker))) w.__loaf.push(Math.round(e.duration));
        } })
        .observe({ type: 'long-animation-frame', buffered: false });
    });
    const item = page.getByTestId('ide-project-tree').locator(`button.tree-item[title="${name}"], button.tree-item[title$="/${name}"]`).first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    const t0 = await page.evaluate(() => performance.now());
    await item.dblclick();
    // Time to coloured: the first frame in which the probe line's tokens carry classes.
    const tColoured = await page.waitForFunction((probe) => {
      const l = [...document.querySelectorAll<HTMLElement>('.ide-editor-area .cm-line')].find((e) => e.textContent?.startsWith(probe));
      return l && l.querySelector('span[class^="ͼ"]') ? performance.now() : null;
    }, f.probe, { polling: 'raf', timeout: 30_000 }).then((h) => h.jsonValue() as Promise<number>, (e: unknown) => {
      if (REPORT_ONLY) return null; // an older build with no language for this file
      throw e;
    });
    const openMs = tColoured == null ? null : Math.round(tColoured - t0);
    // Every class on screen (the semantic layer and a language server refine names later):
    // when did the markup last change? Reported, not asserted.
    const settleMs = await page.evaluate((start) => new Promise<number>((resolve) => {
      const read = () => [...document.querySelectorAll('.ide-editor-area .cm-line')].slice(0, 80).map((l) => l.innerHTML).join('');
      let last = read(), lastAt = performance.now();
      const tick = () => {
        const now = read();
        if (now !== last) { last = now; lastAt = performance.now(); }
        if (performance.now() - lastAt > 1500) resolve(Math.round(lastAt - start)); else requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    }), t0);
    await page.evaluate(() => { (window as unknown as { __loaf: number[] }).__loaf = []; });
    const height = await page.evaluate(() => document.querySelector('.ide-editor-area .cm-scroller')!.scrollHeight);
    const late: string[] = [];
    let steps = 0;
    const scroll = async () => { for (let y = 0; y < height; y += 700) {
      const first = await visibleMarkup(page, y);
      await page.waitForTimeout(300); // ui-guards: allow — proves no colour arrives after the first frame
      const settled = await visibleMarkup(page, null);
      steps++;
      for (const [text, cls] of Object.entries(settled)) if (text in first && first[text] !== cls) late.push(`@${y}: ${text.slice(0, 60)} | ${first[text]} -> ${cls}`);
    } };
    // WebKit: every frame gap while it scrolls (no attribution to the scroll there).
    const loaf = loafOk ? (await scroll(), await page.evaluate(() => (window as unknown as { __loaf: number[] }).__loaf)) : [(await frameDeltas(page, scroll)).max];
    if (late.length) console.log(late.slice(0, 4).join('\n'));
    console.log(`HL ${JSON.stringify({ file: name, openMs, settleMs, steps, lateLines: late.length, longFrames: loaf.length, worstFrameMs: Math.max(0, ...loaf) })}`);
    if (REPORT_ONLY) return;
    expect(late, 'lines coloured after they arrived').toEqual([]);
    expect(loaf.filter((d) => d > budgetMs(50)), `scroll frames over ${budgetMs(50)} ms`).toEqual([]);
  });
}
