import { expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

export const R2_SCREENSHOT_DIR =
  '/Users/itziklavon/.gemini/antigravity-cli/brain/eaf6440b-3dbd-4661-9f38-63372630db72/scratch/ide-r2-shots';

export interface IdeTargetSpec {
  kind: 'local' | 'ssh';
  workspace_id?: string | null;
  root: string;
  connection_id?: string | null;
}

export interface Round2WorkspaceSeed {
  base: string;
  token: string;
  workspaceId: string;
  workspaceRoot: string;
  target: IdeTargetSpec;
}

export const GO_MAIN_SOURCE = [
  'package main',
  '',
  'import (',
  '\t"fmt"',
  '\t"example.com/ottoideround2/decimal"',
  ')',
  '',
  'var Version = "2.4.0"',
  '',
  'type AppBootstrap struct{}',
  '',
  '// InitApp initializes ledger bootstrap metadata.',
  'func (AppBootstrap) InitApp(name string, ver string) string {',
  '\treturn fmt.Sprintf("%s@%s", name, ver)',
  '}',
  '',
  '// NeighborAboveDoc belongs strictly to NeighborAbove and must never leak into ComputeBonus hover!',
  'func NeighborAbove() int {',
  '\treturn 10',
  '}',
  '',
  '// ComputeBonus calculates deterministic operator settlement bonus in cents.',
  '// Supports multi-currency tiering with zero rounding drift.',
  'func ComputeBonus(amount int, rate int) int {',
  '\tnested := ((amount + (rate * 2)) * (1 + (rate % 3)))',
  '\treturn nested',
  '}',
  '',
  '// NeighborBelowDoc belongs strictly to NeighborBelow and must never leak into ComputeBonus hover!',
  'func NeighborBelow() int {',
  '\treturn 99',
  '}',
  '',
  'func main() {',
  '\tapp := AppBootstrap{}',
  '\tappName := "otto-ledger"',
  '\tdecimal.MarshalJSONWithoutQuotes = true',
  '\tbanner := app.InitApp(appName, Version)',
  '\ttotal := ComputeBonus(100, 5)',
  '\tfmt.Println(banner, total)',
  '}',
  '',
].join('\n');

export const JAVA_BONUS_SOURCE = [
  'package com.otto;',
  '',
  'import lombok.extern.slf4j.Slf4j;',
  'import org.springframework.stereotype.Service;',
  'import com.nonexistent.FakeClass;',
  '',
  '@Slf4j',
  '@Service',
  'public class BonusService {',
  '    private final FooService fooService;',
  '',
  '    public BonusService(FooService fooService) {',
  '        this.fooService = fooService;',
  '    }',
  '',
  '    /**',
  '     * Settles bonus payout for the operator.',
  '     * @param operatorId unique operator identifier',
  '     * @return settled cents',
  '     */',
  '    public int settleBonus(String operatorId, int baseCents) {',
  '        log.debug("Settling bonus for {}", operatorId);',
  '        int mult = this.fooService.lookupMultiplier(operatorId);',
  '        return baseCents * mult;',
  '    }',
  '}',
  '',
].join('\n');

export const PY_PIPELINE_SOURCE = [
  'import json',
  '',
  '',
  'def compute_payout(amount: int, bonus: int) -> int:',
  '    return amount + bonus',
  '',
  '',
  'def run_settlement() -> int:',
  '    print(json.dumps({"level": "info", "event": "settlement.start", "operator": "acme", "amount": 40}), flush=True)',
  '    settled = compute_payout(40, 2)',
  '    doubled = settled * 2',
  '    return doubled',
  '',
  '',
  'if __name__ == "__main__":',
  '    print(json.dumps({"level": "info", "event": "settlement.done", "total": run_settlement()}), flush=True)',
  '',
].join('\n');

export function seedRound2Files(dir: string): void {
  for (const sub of ['app', 'decimal', 'pyapp', 'web', 'scripts', 'notes', 'src/main/java/com/otto']) {
    mkdirSync(join(dir, sub), { recursive: true });
  }

  writeFileSync(
    join(dir, 'decimal', 'decimal.go'),
    ['package decimal', '', 'var MarshalJSONWithoutQuotes = false', ''].join('\n'),
  );

  writeFileSync(
    join(dir, 'go.mod'),
    [
      'module example.com/ottoideround2',
      '',
      'go 1.22',
      '',
      'require (',
      '\tgithub.com/shopspring/decimal v1.4.0',
      '\tgolang.org/x/sync v0.8.0',
      ')',
      '',
    ].join('\n'),
  );
  writeFileSync(join(dir, 'app', 'main.go'), GO_MAIN_SOURCE);

  writeFileSync(
    join(dir, 'pom.xml'),
    [
      '<?xml version="1.0" encoding="UTF-8"?>',
      '<project xmlns="http://maven.apache.org/POM/4.0.0">',
      '  <modelVersion>4.0.0</modelVersion>',
      '  <groupId>com.otto</groupId>',
      '  <artifactId>otto-bonus-service</artifactId>',
      '  <version>1.0.0</version>',
      '  <dependencies>',
      '    <dependency>',
      '      <groupId>org.projectlombok</groupId>',
      '      <artifactId>lombok</artifactId>',
      '      <version>1.18.34</version>',
      '    </dependency>',
      '    <dependency>',
      '      <groupId>org.springframework</groupId>',
      '      <artifactId>spring-context</artifactId>',
      '      <version>6.1.12</version>',
      '    </dependency>',
      '  </dependencies>',
      '</project>',
      '',
    ].join('\n'),
  );

  writeFileSync(
    join(dir, 'src/main/java/com/otto', 'FooService.java'),
    [
      'package com.otto;',
      '',
      'public class FooService {',
      '    public int lookupMultiplier(String operatorId) {',
      '        return operatorId == null ? 1 : 2;',
      '    }',
      '}',
      '',
    ].join('\n'),
  );
  writeFileSync(join(dir, 'src/main/java/com/otto', 'BonusService.java'), JAVA_BONUS_SOURCE);
  writeFileSync(join(dir, 'pyapp', '__init__.py'), '');
  writeFileSync(join(dir, 'pyapp', 'pipeline.py'), PY_PIPELINE_SOURCE);

  writeFileSync(
    join(dir, 'package.json'),
    JSON.stringify({ name: 'otto-ide-r2-e2e', version: '1.0.0', type: 'module' }, null, 2) + '\n',
  );
  writeFileSync(
    join(dir, 'web', 'worker.mjs'),
    [
      'export function transformBatch(items) {',
      '  const mapped = items.map((x) => x * 2);',
      '  return mapped.reduce((a, b) => a + b, 0);',
      '}',
      '',
      "console.log('worker-total:', transformBatch([10, 11]));",
      '',
    ].join('\n'),
  );

  writeFileSync(
    join(dir, 'scripts', 'deploy.sh'),
    [
      '#!/usr/bin/env bash',
      'set -euo pipefail',
      '',
      'APP_ENV="${1:-staging}"',
      'RELEASE_TAG="v2.4.0"',
      'echo "Deploying ${RELEASE_TAG} to ${APP_ENV}"',
      '',
    ].join('\n'),
  );

  writeFileSync(join(dir, 'notes', 'roadmap.md'), '# Otto IDE Round 2\n\nInitial committed baseline.\n');

  for (const argv of [
    ['init', '-b', 'main'],
    ['config', 'user.email', 'ide-r2@otto.local'],
    ['config', 'user.name', 'Otto IDE Round 2'],
    ['config', 'commit.gpgsign', 'false'],
    ['add', '.'],
    ['commit', '-m', 'chore: seed polyglot round2 workspace'],
  ]) {
    execFileSync('git', argv, { cwd: dir, stdio: 'ignore' });
  }
}

export async function daemonCall<T>(
  ctx: APIRequestContext,
  base: string,
  token: string,
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

export async function createRound2Seed(): Promise<Round2WorkspaceSeed> {
  const workspaceRoot = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-r2-suite-')));
  seedRound2Files(workspaceRoot);

  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', {
      name: 'Otto IDE Round 2 E2E',
      root_path: workspaceRoot,
    });
    // Also register git repo with Otto so Integrated Git dock resolves immediately.
    await daemonCall(c.ctx, c.base, c.token, 'POST', '/repos', {
      path: workspaceRoot,
      workspace_id: ws.id,
    }).catch(() => undefined);

    const target: IdeTargetSpec = {
      kind: 'local',
      workspace_id: ws.id,
      root: workspaceRoot,
      connection_id: null,
    };
    const extraPath = [join(homedir(), 'go', 'bin'), '/opt/homebrew/bin', '/usr/local/go/bin', '/usr/local/bin']
      .filter(existsSync)
      .join(':');
    await daemonCall(c.ctx, c.base, c.token, 'PUT', '/ide/environments', {
      target,
      extra_path: extraPath,
      env_vars: [],
    });
    await daemonCall(c.ctx, c.base, c.token, 'POST', '/ide/run-configs', {
      target,
      id: 'py-settlement-debug',
      name: 'Python Settlement Debug',
      runner: 'python',
      command: 'python3',
      args: [],
      cwd: workspaceRoot,
      target_path: join(workspaceRoot, 'pyapp', 'pipeline.py'),
    });

    return { base: c.base, token: c.token, workspaceId: ws.id, workspaceRoot, target };
  } finally {
    await c.ctx.dispose();
  }
}

export async function saveR2Shot(page: Page, name: string): Promise<string> {
  mkdirSync(R2_SCREENSHOT_DIR, { recursive: true });
  const out = join(R2_SCREENSHOT_DIR, name);
  await page.screenshot({ path: out, fullPage: false });
  return out;
}

export async function bootR2Ide(
  page: Page,
  seed: Round2WorkspaceSeed,
  scheme: 'dark' | 'light' = 'dark',
): Promise<void> {
  await page.addInitScript(
    ({ wsId, root, uiScheme }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_scheme', uiScheme);
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.agent.provider', 'claude');
      localStorage.setItem(
        'otto.ide.namedProjects.v1',
        JSON.stringify([
          {
            id: 'proj-r2',
            name: 'Otto IDE Round 2 Project',
            folders: [{ path: root, name: 'otto-r2' }],
            updatedAt: Date.now(),
          },
        ]),
      );
    },
    { wsId: seed.workspaceId, root: seed.workspaceRoot, uiScheme: scheme },
  );
  await page.goto('/#/ide');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('ide-project-tree')).toBeVisible({ timeout: 20_000 });
}

export async function openR2File(page: Page, ...segments: string[]): Promise<void> {
  const tree = page.getByTestId('ide-project-tree');
  await expect(tree).toBeVisible({ timeout: 15_000 });
  for (let i = 0; i < segments.length; i++) {
    const rel = segments.slice(0, i + 1).join('/');
    const item = tree.locator(`button.tree-item[title="${rel}"], button.tree-item[title$="/${rel}"]`).first();
    await expect(item).toBeVisible({ timeout: 15_000 });
    if (i === segments.length - 1) {
      await item.dblclick();
    } else if ((await item.getAttribute('aria-expanded')) !== 'true') {
      await item.click();
    }
  }
}

export function editorContent(page: Page): Locator {
  return page.locator('.ide-editor-area .cm-editor .cm-content').first();
}
