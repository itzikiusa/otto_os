import { expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

export const WORKFLOW_SCREENSHOT_DIR =
  process.env.OTTO_IDE_SHOTS_DIR ?? join(tmpdir(), 'otto-ide-workflow-shots');

export interface IdeTargetSpec {
  kind: 'local' | 'ssh';
  workspace_id?: string | null;
  root: string;
  connection_id?: string | null;
}

export interface PolyglotWorkspaceSeed {
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
  '\t"example.com/ottoidepolyglot/decimal"',
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

export const ZIG_MAIN_SOURCE = [
  'const std = @import("std");',
  '',
  'pub const Settlement = struct {',
  '    amount: i32,',
  '    bonus: i32,',
  '',
  '    pub fn total(self: Settlement) i32 {',
  '        return self.amount + self.bonus;',
  '    }',
  '};',
  '',
  'pub fn computeBonus(amount: i32, rate: i32) i32 {',
  '    return amount + rate * 2;',
  '}',
  '',
  'pub fn triggerPanic() noreturn {',
  '    @panic("deliberate zig settlement panic");',
  '}',
  '',
  'pub fn main() !void {',
  '    const item = Settlement{ .amount = 40, .bonus = computeBonus(10, 5) };',
  '    const total = item.total();',
  '    std.debug.print("ZIG_SETTLEMENT_DONE:{d}\\n", .{total});',
  '}',
  '',
  'test "settlement total" {',
  '    const item = Settlement{ .amount = 30, .bonus = 12 };',
  '    try std.testing.expectEqual(@as(i32, 42), item.total());',
  '}',
  '',
].join('\n');

export function seedPolyglotFiles(dir: string): void {
  for (const sub of ['app', 'decimal', 'pyapp', 'zigapp/src', 'cppapp/include', 'cppapp/src', 'web', 'scripts', 'notes', 'src/main/java/com/otto']) {
    mkdirSync(join(dir, sub), { recursive: true });
  }

  writeFileSync(
    join(dir, 'decimal', 'decimal.go'),
    ['package decimal', '', 'var MarshalJSONWithoutQuotes = false', ''].join('\n'),
  );

  writeFileSync(
    join(dir, 'go.mod'),
    [
      'module example.com/ottoidepolyglot',
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
    join(dir, 'zigapp', 'build.zig'),
    [
      'const std = @import("std");',
      '',
      'pub fn build(b: *std.Build) void {',
      '    const target = b.standardTargetOptions(.{});',
      '    const optimize = b.standardOptimizeOption(.{});',
      '    const exe = b.addExecutable(.{',
      '        .name = "zigapp",',
      '        .root_source_file = b.path("src/main.zig"),',
      '        .target = target,',
      '        .optimize = optimize,',
      '    });',
      '    b.installArtifact(exe);',
      '    const run_step = b.step("run", "Run the app");',
      '    run_step.dependOn(&b.addRunArtifact(exe).step);',
      '    const unit_tests = b.addTest(.{',
      '        .root_source_file = b.path("src/main.zig"),',
      '        .target = target,',
      '        .optimize = optimize,',
      '    });',
      '    const test_step = b.step("test", "Run unit tests");',
      '    test_step.dependOn(&b.addRunArtifact(unit_tests).step);',
      '}',
      '',
    ].join('\n'),
  );
  writeFileSync(
    join(dir, 'zigapp', 'build.zig.zon'),
    [
      '.{',
      '    .name = .zigapp,',
      '    .version = "0.1.0",',
      '    .dependencies = .{},',
      '    .paths = .{ "build.zig", "build.zig.zon", "src" },',
      '}',
      '',
    ].join('\n'),
  );
  writeFileSync(join(dir, 'zigapp', 'src', 'main.zig'), ZIG_MAIN_SOURCE);

  writeFileSync(
    join(dir, 'cppapp', 'CMakeLists.txt'),
    [
      'cmake_minimum_required(VERSION 3.16)',
      'project(cppapp LANGUAGES CXX)',
      'set(CMAKE_CXX_STANDARD 20)',
      'set(CMAKE_EXPORT_COMPILE_COMMANDS ON)',
      'add_executable(cppapp src/main.cpp src/calc.cpp)',
      'target_include_directories(cppapp PRIVATE include)',
      'target_compile_options(cppapp PRIVATE -g -O0)',
      'enable_testing()',
      'add_test(NAME cppapp_smoke COMMAND cppapp)',
      '',
    ].join('\n'),
  );
  writeFileSync(
    join(dir, 'cppapp', 'include', 'calc.hpp'),
    '#pragma once\nnamespace calc {\nint add(int a, int b);\n}\n',
  );
  writeFileSync(
    join(dir, 'cppapp', 'src', 'calc.cpp'),
    '#include "calc.hpp"\nnamespace calc {\nint add(int a, int b) { return a + b; }\n}\n',
  );
  writeFileSync(
    join(dir, 'cppapp', 'src', 'main.cpp'),
    '#include <iostream>\n#include "calc.hpp"\nint main() {\n    int left = 30;\n    int total = calc::add(left, 12);\n    std::cout << "CPP_SETTLEMENT_DONE:" << total << "\\n";\n    return 0;\n}\n',
  );

  mkdirSync(join(dir, 'kotlinapp', 'gradle'), { recursive: true });
  mkdirSync(join(dir, 'kotlinapp', 'src', 'main', 'kotlin', 'com', 'otto'), { recursive: true });
  mkdirSync(join(dir, 'kotlinapp', 'src', 'test', 'kotlin', 'com', 'otto'), { recursive: true });
  writeFileSync(
    join(dir, 'kotlinapp', 'settings.gradle.kts'),
    'rootProject.name = "kotlinapp"\n',
  );
  writeFileSync(
    join(dir, 'kotlinapp', 'gradle', 'libs.versions.toml'),
    '[versions]\ncoroutines = "1.9.0"\nkotest = "5.9.1"\n\n[libraries]\nkotlinx-coroutines = { module = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version.ref = "coroutines" }\nkotest-runner = { module = "io.kotest:kotest-runner-junit5", version.ref = "kotest" }\n',
  );
  writeFileSync(
    join(dir, 'kotlinapp', 'build.gradle.kts'),
    'plugins {\n    kotlin("jvm") version "2.1.20"\n}\ndependencies {\n    implementation(kotlin("stdlib"))\n    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")\n    testImplementation("io.kotest:kotest-runner-junit5:5.9.1")\n}\n',
  );
  writeFileSync(
    join(dir, 'kotlinapp', 'src', 'main', 'kotlin', 'com', 'otto', 'App.kt'),
    'package com.otto\n\ndata class Settlement(val amount: Int)\n\nfun main() {\n    val s = Settlement(42)\n    println("KOTLIN_DONE:${s.amount}")\n}\n',
  );
  writeFileSync(
    join(dir, 'kotlinapp', 'src', 'test', 'kotlin', 'com', 'otto', 'AppSpec.kt'),
    'package com.otto\n\nimport io.kotest.core.spec.style.StringSpec\n\nclass AppSpec : StringSpec({\n    "computes settlement" {\n        assert(Settlement(42).amount == 42)\n    }\n})\n',
  );

  writeFileSync(
    join(dir, 'package.json'),
    JSON.stringify({ name: 'otto-ide-workflows-e2e', version: '1.0.0', type: 'module' }, null, 2) + '\n',
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

  writeFileSync(join(dir, 'notes', 'roadmap.md'), '# Otto IDE Workflows\n\nInitial committed baseline.\n');

  for (const argv of [
    ['init', '-b', 'main'],
    ['config', 'user.email', 'ide-workflows@otto.local'],
    ['config', 'user.name', 'Otto IDE Workflows'],
    ['config', 'commit.gpgsign', 'false'],
    ['add', '.'],
    ['commit', '-m', 'chore: seed polyglot ide workspace'],
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

export async function createPolyglotSeed(): Promise<PolyglotWorkspaceSeed> {
  const workspaceRoot = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-workflows-suite-')));
  seedPolyglotFiles(workspaceRoot);

  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', {
      name: 'Otto IDE Workflows E2E',
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

export async function saveWorkflowShot(page: Page, name: string): Promise<string> {
  mkdirSync(WORKFLOW_SCREENSHOT_DIR, { recursive: true });
  const out = join(WORKFLOW_SCREENSHOT_DIR, name);
  await page.screenshot({ path: out, fullPage: false });
  return out;
}

export async function bootPolyglotIde(
  page: Page,
  seed: PolyglotWorkspaceSeed,
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
            id: 'proj-polyglot',
            name: 'Otto IDE Polyglot Project',
            folders: [{ path: root, name: 'otto-polyglot' }],
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

export async function openPolyglotFile(page: Page, ...segments: string[]): Promise<void> {
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
