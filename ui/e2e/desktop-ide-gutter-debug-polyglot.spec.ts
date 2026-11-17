import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, symlinkSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// The person's path for every language: open the file from the tree, click the
// breakpoint gutter on two lines, gutter ▶ → Debug, and the Debug panel must
// stop on each line with that line's locals. Every file sits a few folders deep
// (the Go cells, root to second root, live in desktop-ide-gutter-debug-go).

const SHOTS = process.env.OTTO_E2E_DEBUG_SHOTS;

const has = (bin: string): boolean => {
  try {
    execFileSync('/bin/sh', ['-c', `command -v ${bin}`], {
      env: { ...process.env, PATH: `${process.env.PATH}:${join(homedir(), '.cargo', 'bin')}:${join(homedir(), '.sdkman', 'candidates', 'maven', 'current', 'bin')}` },
      stdio: 'ignore',
    });
    return true;
  } catch {
    return false;
  }
};

interface Cell {
  name: string;
  /** Project folder → files (relative path → content). */
  files: Record<string, string>;
  open: string;
  /** The gutter ▶ to use: the last of its kind is the innermost test (a class / `mod tests` comes first). */
  gutterKind: 'main' | 'test';
  menu: RegExp;
  stops: Array<{ line: number; text: string; local: string }>;
  needs: string[];
  /** A `node_modules` to link in: there is no offline `npm install`. */
  nodeModules?: string | null;
  /** Prepares the project after its files are written (a venv, a Gradle wrapper); false skips. */
  setup?: (root: string) => boolean;
  /** Also run it from the gutter first; the Run & tests panel must show this. */
  runPasses?: RegExp;
}

const sh = (cmd: string, args: string[], cwd: string, env: Record<string, string> = {}): boolean => {
  try {
    execFileSync(cmd, args, { cwd, stdio: 'ignore', env: { ...process.env, ...env }, timeout: 240_000 });
    return true;
  } catch {
    return false;
  }
};

/** The newest fully downloaded Gradle (`*.zip.ok`) in the wrapper cache, and JDK 17. */
function cachedGradle(): { gradle: string; jdk: string } | null {
  const dists = join(homedir(), '.gradle', 'wrapper', 'dists');
  if (!existsSync(dists)) return null;
  const dirs = (p: string): string[] => readdirSync(p, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name);
  const homes = dirs(dists).flatMap((d) =>
    dirs(join(dists, d)).flatMap((h) => {
      const dir = join(dists, d, h);
      const ok = readdirSync(dir).find((f) => f.endsWith('-bin.zip.ok'));
      const home = ok ? join(dir, ok.replace('-bin.zip.ok', '')) : '';
      return ok && existsSync(join(home, 'bin', 'gradle')) ? [home] : [];
    }),
  );
  const ver = (h: string): number[] => (h.split('gradle-').pop() ?? '').split('.').map(Number);
  homes.sort((a, b) => {
    const [x, y] = [ver(a), ver(b)];
    for (let i = 0; i < Math.max(x.length, y.length); i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) - (y[i] ?? 0);
    return 0;
  });
  let jdk = '';
  try {
    jdk = execFileSync('/usr/libexec/java_home', ['-v', '17'], { encoding: 'utf8' }).trim();
  } catch { /* none */ }
  return homes.length && jdk ? { gradle: join(homes[homes.length - 1], 'bin', 'gradle'), jdk } : null;
}
const GRADLE = cachedGradle();

/** An already-installed `node_modules` holding `pkg`: the npx cache, or another checkout on this Mac. */
function installedNodeModules(pkg: string): string | null {
  const npx = join(homedir(), '.npm', '_npx');
  const cands = [
    ...(existsSync(npx) ? readdirSync(npx).map((d) => join(npx, d, 'node_modules')) : []),
    ...readdirSync(homedir()).map((d) => join(homedir(), d, 'node_modules')),
  ];
  return cands.find((nm) => existsSync(join(nm, pkg, 'package.json')) && existsSync(join(nm, '.bin', pkg))) ?? null;
}

const TEST_BODY = (imports: string): string =>
  `${imports}function add(a, b) {\n  return a + b;\n}\n\ntest('adds numbers', () => {\n  const left = 20;\n  const sum = add(left, 22);\n  expect(sum).toBe(42);\n  console.log('JS_TEST_DONE');\n});\n`;

const CELLS: Cell[] = [
  {
    name: 'python script in scripts/sub',
    files: {
      'scripts/sub/tool.py': 'def main():\n    base = 40\n    total = base + 2\n    print("PY_DEEP_DONE", total)\n\n\nif __name__ == "__main__":\n    main()\n',
    },
    open: 'scripts/sub/tool.py',
    gutterKind: 'main',
    menu: /^Debug/,
    stops: [{ line: 3, text: 'total = base + 2', local: 'base' }, { line: 4, text: 'print(', local: 'total' }],
    needs: ['python3'],
  },
  {
    name: 'rust test in src/a/b.rs',
    files: {
      'Cargo.toml': '[package]\nname = "rs-deep"\nversion = "0.1.0"\nedition = "2021"\n',
      'src/lib.rs': 'pub mod a;\n',
      'src/a/mod.rs': 'pub mod b;\n',
      'src/a/b.rs': 'pub fn mul(a: i32, b: i32) -> i32 {\n    a * b\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn multiplies_deep() {\n        let left = 6;\n        let product = mul(left, 7);\n        assert_eq!(product, 42);\n        println!("RS_DEEP_DONE");\n    }\n}\n',
    },
    open: 'src/a/b.rs',
    gutterKind: 'test',
    menu: /^Debug/,
    stops: [{ line: 12, text: 'let product', local: 'left' }, { line: 13, text: 'assert_eq!', local: 'product' }],
    needs: ['cargo', 'lldb'],
  },
  {
    name: 'java JUnit 5 test in a deeper package of a sub-module',
    files: {
      'pom.xml': '<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>com.acme</groupId><artifactId>parent</artifactId><version>1.0.0</version><packaging>pom</packaging><modules><module>core</module></modules><properties><maven.compiler.release>17</maven.compiler.release><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties><build><pluginManagement><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-resources-plugin</artifactId><version>3.3.1</version></plugin><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.2.5</version></plugin></plugins></pluginManagement></build></project>\n',
      'core/pom.xml': '<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><parent><groupId>com.acme</groupId><artifactId>parent</artifactId><version>1.0.0</version></parent><artifactId>core</artifactId><dependencies><dependency><groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><version>5.10.5</version><scope>test</scope></dependency></dependencies></project>\n',
      'core/src/main/java/com/acme/core/Calc.java': 'package com.acme.core;\n\npublic class Calc {\n    public static int add(int a, int b) {\n        return a + b;\n    }\n}\n',
      'core/src/test/java/com/acme/core/deep/inner/DeepTest.java': 'package com.acme.core.deep.inner;\n\nimport static org.junit.jupiter.api.Assertions.assertEquals;\n\nimport com.acme.core.Calc;\nimport org.junit.jupiter.api.Test;\n\nclass DeepTest {\n    @Test\n    void addsDeep() {\n        int left = 30;\n        int sum = Calc.add(left, 12);\n        assertEquals(42, sum);\n        System.out.println("JUNIT5_DEEP_DONE");\n    }\n}\n',
    },
    open: 'core/src/test/java/com/acme/core/deep/inner/DeepTest.java',
    gutterKind: 'test',
    menu: /^Debug/,
    stops: [{ line: 12, text: 'int sum', local: 'left' }, { line: 13, text: 'assertEquals', local: 'sum' }],
    needs: ['java', 'mvn'],
  },
  {
    name: 'shell script in bin/sub',
    files: {
      'bin/sub/run.sh': '#!/usr/bin/env bash\nadd() {\n  echo $(( $1 + $2 ))\n}\nbase=40\ntotal=$(add "$base" 2)\necho "SH_MAIN_DONE $total"\n',
    },
    open: 'bin/sub/run.sh',
    gutterKind: 'main',
    menu: /^Debug/,
    stops: [{ line: 6, text: 'total=$(add', local: 'base' }, { line: 7, text: 'echo "SH_MAIN_DONE', local: 'total' }],
    needs: ['bash'],
  },
];

async function openFromTree(page: Page, rel: string): Promise<void> {
  const segs = rel.split('/');
  const tree = page.getByTestId('ide-project-tree');
  for (let i = 1; i <= segs.length; i++) {
    const r = segs.slice(0, i).join('/');
    const item = tree.locator(`button.tree-item[title="${r}"], button.tree-item[title$="/${r}"]`).first();
    await expect(item).toBeVisible({ timeout: 20_000 });
    if (i === segs.length) await item.dblclick();
    else if ((await item.getAttribute('aria-expanded')) !== 'true') await item.click();
  }
}

async function clickBreakpoint(page: Page, line: number): Promise<void> {
  const num = page.locator('.cm-editor:visible .cm-lineNumbers .cm-gutterElement').filter({ hasText: new RegExp(`^${line}$`) }).first();
  await num.scrollIntoViewIfNeeded();
  const g = await page.locator('.cm-editor:visible .cm-ide-bp-gutter').first().boundingBox();
  const n = await num.boundingBox();
  expect(g && n).toBeTruthy();
  await page.mouse.click(g!.x + g!.width / 2, n!.y + n!.height / 2);
}

CELLS.push(
  {
    name: 'pytest test in tests/sub',
    // The code and a fixture live at the project root: the test must run from pytest's rootdir.
    files: {
      'calc.py': 'def add(a, b):\n    return a + b\n',
      'conftest.py': 'import pytest\n\n\n@pytest.fixture\ndef base():\n    return 20\n',
      'tests/sub/test_calc.py': 'from calc import add\n\n\ndef test_adds_numbers(base):\n    left = base\n    total = add(left, 22)\n    assert total == 42\n    print("PY_TEST_DONE")\n',
    },
    open: 'tests/sub/test_calc.py',
    gutterKind: 'test',
    menu: /^Debug/,
    stops: [{ line: 6, text: 'total = add', local: 'left' }, { line: 7, text: 'assert total', local: 'total' }],
    needs: ['python3'],
    // The IDE runs the project's .venv; pip serves pytest from its cache when it holds it.
    setup: (root) => sh('python3', ['-m', 'venv', '.venv'], root) && sh(join(root, '.venv', 'bin', 'pip'), ['install', '-q', '--disable-pip-version-check', 'pytest'], root),
    runPasses: /1 passed|PASSED|passed/,
  },
  {
    name: 'gradle JUnit test in a deeper package',
    files: {
      'settings.gradle': "rootProject.name = 'gj'\ninclude 'lib'\n",
      'lib/build.gradle': "plugins { id 'java' }\nrepositories { mavenLocal(); mavenCentral() }\ndependencies { testImplementation 'junit:junit:4.13.2' }\ntest { testLogging { events 'passed'; showStandardStreams = true } }\n",
      'lib/src/main/java/com/acme/gj/Calc.java': 'package com.acme.gj;\n\npublic class Calc {\n    public static int add(int a, int b) {\n        return a + b;\n    }\n}\n',
      'lib/src/test/java/com/acme/gj/deep/CalcTest.java': 'package com.acme.gj.deep;\n\nimport static org.junit.Assert.assertEquals;\n\nimport com.acme.gj.Calc;\nimport org.junit.Test;\n\npublic class CalcTest {\n    @Test\n    public void addsNumbers() {\n        int left = 20;\n        int sum = Calc.add(left, 22);\n        assertEquals(42, sum);\n        System.out.println("GRADLE_JUNIT_DONE");\n    }\n}\n',
    },
    open: 'lib/src/test/java/com/acme/gj/deep/CalcTest.java',
    gutterKind: 'test',
    menu: /^Debug/,
    stops: [{ line: 12, text: 'int sum', local: 'left' }, { line: 13, text: 'assertEquals', local: 'sum' }],
    needs: ['java'],
    setup: (root) => GRADLE !== null && sh(GRADLE.gradle, ['wrapper', '--offline', '-q'], root, { JAVA_HOME: GRADLE.jdk }),
    runPasses: /GRADLE_JUNIT_DONE|PASSED|passed/,
  },
  {
    name: 'vitest test in src/deep',
    files: {
      'package.json': '{"name":"m","type":"module","devDependencies":{"vitest":"*"}}\n',
      'src/deep/calc.test.ts': TEST_BODY("import { expect, test } from 'vitest';\n\n"),
    },
    open: 'src/deep/calc.test.ts',
    gutterKind: 'test',
    menu: /^Debug/,
    stops: [{ line: 9, text: 'const sum', local: 'left' }, { line: 10, text: 'expect(sum)', local: 'sum' }],
    needs: ['node'],
    nodeModules: installedNodeModules('vitest'),
  },
  {
    name: 'jest test in src/deep',
    files: {
      'package.json': '{"name":"m","devDependencies":{"jest":"*"}}\n',
      'src/deep/calc.test.js': TEST_BODY(''),
    },
    open: 'src/deep/calc.test.js',
    gutterKind: 'test',
    menu: /^Debug/,
    stops: [{ line: 7, text: 'const sum', local: 'left' }, { line: 8, text: 'expect(sum)', local: 'sum' }],
    needs: ['node'],
    nodeModules: installedNodeModules('jest'),
  },
);

for (const cell of CELLS) {
  test(`gutter Debug stops on the breakpoints: ${cell.name}`, async ({ page }) => {
    test.skip(!cell.needs.every(has), `needs ${cell.needs.join(', ')}`);
    test.skip(cell.nodeModules === null, 'no installed node_modules with the test runner');
    test.setTimeout(240_000);
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-gutter-polyglot-')));
    for (const [rel, src] of Object.entries(cell.files)) {
      mkdirSync(join(root, rel, '..'), { recursive: true });
      writeFileSync(join(root, rel), src);
    }
    if (cell.nodeModules) symlinkSync(cell.nodeModules, join(root, 'node_modules'));
    test.skip(cell.setup !== undefined && !cell.setup(root), 'project setup (venv / Gradle wrapper) failed');
    const { ctx, base } = await apiCtx();
    const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: cell.name, root_path: root } });
    expect(r.ok(), await r.text()).toBe(true);
    const ws = ((await r.json()) as { id: string }).id;
    const extraPath = [join(homedir(), '.cargo', 'bin'), join(homedir(), '.sdkman', 'candidates', 'maven', 'current', 'bin'), '/opt/homebrew/bin', '/usr/local/bin']
      .filter(existsSync).join(':');
    await ctx.put(`${base}/api/v1/ide/environments`, {
      data: {
        target: { kind: 'local', workspace_id: ws, root, connection_id: null },
        extra_path: extraPath,
        // Gradle 8.3 (the newest fully cached one) does not run on a newer JDK.
        env_vars: GRADLE && existsSync(join(root, 'gradlew')) ? [{ key: 'JAVA_HOME', value: GRADLE.jdk, secret: false }] : [],
      },
    });

    await page.addInitScript((wsId) => {
      localStorage.setItem('otto_workspace', wsId as string);
      localStorage.setItem('otto_rail_expanded', '0');
    }, ws);
    await page.goto('/#/ide');
    await openFromTree(page, cell.open);
    await expect(page.locator('.cm-editor:visible .cm-content').first()).toContainText(cell.stops[0].text);
    for (const s of cell.stops) await clickBreakpoint(page, s.line);
    const slug = cell.name.replace(/[^a-z0-9]+/gi, '-').toLowerCase();

    if (cell.runPasses) {
      const runStarted = page.waitForResponse(
        (res) => res.request().method() === 'POST' && /\/api\/v1\/ide\/runs(\?|$)/.test(res.url()),
        { timeout: 30_000 },
      );
      const gutterRun = page.locator(`.cm-editor:visible .cm-gutters .cm-ide-gutter-run-btn[data-gutter-kind="${cell.gutterKind}"]`).last();
      await expect(gutterRun).toBeVisible({ timeout: 30_000 });
      await gutterRun.click();
      await page.locator('.ctx-menu').getByRole('menuitem', { name: /^Run '/ }).first().click();
      const runId = ((await (await runStarted).json()) as { run: { id: string } }).run.id;
      let run: { status: string; output: string[] } = { status: 'running', output: [] };
      await expect.poll(async () => {
        run = ((await (await ctx.get(`${base}/api/v1/ide/runs/${runId}`)).json()) as { run: typeof run }).run;
        return run.status;
      }, { timeout: 240_000 }).not.toBe('running');
      expect(run.status, run.output.slice(-20).join('\n')).toBe('passed');
      expect(run.output.join('\n')).toMatch(cell.runPasses);
      if (SHOTS) await page.screenshot({ path: join(SHOTS, `${slug}-run.png`) });
    }

    const started = page.waitForResponse(
      (res) => res.request().method() === 'POST' && /\/api\/v1\/ide\/debug\/sessions(\?|$)/.test(res.url()),
      { timeout: 30_000 },
    );
    const runBtn = page.locator(`.cm-editor:visible .cm-gutters .cm-ide-gutter-run-btn[data-gutter-kind="${cell.gutterKind}"]`).last();
    await expect(runBtn).toBeVisible({ timeout: 30_000 });
    await runBtn.click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: cell.menu }).first().click();
    const resp = await started;
    expect(resp.ok(), await resp.text()).toBe(true);
    const sid = ((await resp.json()) as { session: { id: string } }).session.id;

    const panel = page.getByTestId('ide-debugger-panel');
    try {
      for (const [i, s] of cell.stops.entries()) {
        await expect(page.locator('.cm-ide-paused-line').first()).toContainText(s.text, { timeout: i === 0 ? 180_000 : 30_000 });
        await expect(panel).toContainText(/Paused/i);
        await expect(panel).toContainText(s.local);
        if (SHOTS) await page.screenshot({ path: join(SHOTS, `${slug}-stop${i + 1}.png`) });
        await panel.getByRole('button', { name: /^Resume program/ }).click();
      }
      await expect(panel).toContainText(/Terminated/i, { timeout: 60_000 });
    } finally {
      await ctx.post(`${base}/api/v1/ide/debug/sessions/${sid}/command`, { data: { action: 'stop' } }).catch(() => {});
      await ctx.dispose();
    }
  });
}
