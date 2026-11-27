import assert from 'node:assert/strict';
import { test } from 'node:test';
import { EditorState } from '@codemirror/state';
import { foldable } from '@codemirror/language';
import { langGutters, langKeyFor } from '../src/lib/components/langForPath.ts';
import { cmLangNow, loadCmLang } from '../src/lib/components/cm-langs.ts';
import { detectManifestEcosystem, isIdeManifestPath, parseManifestDependencies } from '../src/lib/components/cm-ide-deps-model.ts';
import { normalizeIdeLspLanguage, shouldAttachIdeLsp } from '../src/lib/components/cm-ide-lsp-completion.ts';
import { classifySemanticTokens, detectSemanticLang } from '../src/lib/components/cm-ide-semantic.ts';
import { ideFileKind, ideLanguageLabel, ideLanguageSettingsSection, ideLspLangFor, ideLspServerName } from '../src/modules/ide/editor/ideStatusModel.ts';
import { detectIdeDocKind, extractInstantSymbols } from '../src/modules/ide/editor/ideLangAnalyzer.ts';
import { nestParentName } from '../src/modules/ide/history/ideTreeNesting.ts';
import { resolveRepoTestPlan } from '../src/modules/ide/store/ideRepoTestAndBuildTags.ts';
import { inferTestDebugLanguage } from '../src/modules/ide/debug/ideFailedTestDebug.ts';
import { launchPreview } from '../src/modules/ide/agentDebug/ideAgentTarget.ts';

test('Zig and ZON language keys, gutters, and CodeMirror pack loading', async () => {
  assert.equal(langKeyFor('/proj/src/main.zig'), 'zig');
  assert.equal(langKeyFor('/proj/build.zig.zon'), 'zig');
  assert.equal(langKeyFor('/proj/noext', 'zig'), 'zig');
  assert.equal(langKeyFor('/proj/noext', 'zon'), 'zig');

  const g = langGutters('/proj/src/main.zig');
  assert.equal(g.breakpoints, true);
  assert.equal(g.lint, true);
  assert.equal(g.fold, true);

  await loadCmLang('zig');
  const ext = cmLangNow('zig', 'mysql');
  assert.ok(ext, 'zig CodeMirror extension should be loaded');

  const doc = 'pub fn main() !void {\n    const x: u32 = 42;\n    _ = x;\n}\n';
  const state = EditorState.create({ doc, extensions: [ext] });
  const range = foldable(state, 0, doc.indexOf('\n'));
  assert.ok(range && range.from < range.to, 'Zig brace block should be foldable');
});

test('Zig LSP gating, status model, outline symbols, and semantic classifier', () => {
  assert.equal(normalizeIdeLspLanguage('zig'), 'zig');
  assert.equal(normalizeIdeLspLanguage('zon'), 'zig');
  assert.equal(shouldAttachIdeLsp('/proj/src/main.zig', 'zig', '/proj'), true);
  assert.equal(shouldAttachIdeLsp('/proj/build.zig.zon', 'zig', '/proj'), true);

  assert.equal(ideFileKind('/proj/src/main.zig'), 'code');
  assert.equal(ideLanguageLabel('/proj/src/main.zig', 'zig'), 'Zig');
  assert.equal(ideLanguageSettingsSection('/proj/src/main.zig', 'zig'), 'lang.zig');
  assert.equal(ideLspLangFor('/proj/src/main.zig', 'zig'), 'zig');
  assert.equal(ideLspServerName('zig'), 'zls');

  const sample = [
    'const std = @import("std");',
    '/// Point doc',
    'pub const Point = struct {',
    '    x: i32,',
    '};',
    'pub fn add(a: i32, b: i32) i32 {',
    '    return a + b;',
    '}',
    'test "adds" {',
    '    try std.testing.expectEqual(@as(i32, 3), add(1, 2));',
    '}',
  ].join('\n');

  assert.equal(detectIdeDocKind('/proj/src/main.zig'), 'zig');
  const syms = extractInstantSymbols('/proj/src/main.zig', 'zig', sample);
  assert.deepEqual(
    syms.map((s) => [s.name, s.kind]),
    [
      ['Point', 'struct'],
      ['add', 'function'],
      ['adds', 'test'],
    ],
  );

  assert.equal(detectSemanticLang('/proj/src/main.zig'), 'zig');
  const toks = classifySemanticTokens(sample, 'zig');
  assert.ok(toks.some((t) => t.role === 'doc-comment' && t.text.includes('Point doc')));
  assert.ok(toks.some((t) => t.role === 'annotation' && t.text === '@import'));
  assert.ok(toks.some((t) => t.role === 'func-decl' && t.text === 'add'));
});

test('build.zig.zon manifest parser, nesting, test plans, and debug previews', () => {
  assert.equal(isIdeManifestPath('/proj/build.zig.zon'), true);
  assert.equal(detectManifestEcosystem('/proj/build.zig.zon'), 'zig');
  assert.equal(nestParentName('build.zig.zon'), 'build.zig');

  const zon = `.{
    .name = .demo,
    .version = "0.1.0",
    .dependencies = .{
        .zap = .{
            .url = "https://github.com/zigzap/zap/archive/refs/tags/v0.9.1.tar.gz",
            .hash = "1220abcdef",
        },
        .local_mod = .{
            .path = "../local_mod",
        },
    },
}`;
  const deps = parseManifestDependencies('/proj/build.zig.zon', zon);
  assert.deepEqual(
    deps.map((d) => [d.name, d.current_version]),
    [
      ['zap', 'v0.9.1'],
      ['local_mod', 'path:../local_mod'],
    ],
  );

  const runPlan = resolveRepoTestPlan({
    repoRoot: '/proj',
    mode: 'run',
    runConfigs: [],
    detectedConfigs: [],
    dirEntries: [{ name: 'build.zig', path: '/proj/build.zig' }],
    openTabs: [],
    goBuildTags: [],
  });
  assert.equal(runPlan.kind, 'run');
  if (runPlan.kind === 'run') assert.equal(runPlan.adhoc.runner, 'zig_test');

  const dbgPlan = resolveRepoTestPlan({
    repoRoot: '/proj',
    mode: 'debug',
    runConfigs: [],
    detectedConfigs: [],
    dirEntries: [{ name: 'build.zig', path: '/proj/build.zig' }],
    openTabs: [],
    goBuildTags: [],
  });
  assert.equal(dbgPlan.kind, 'debug');
  if (dbgPlan.kind === 'debug') assert.equal(dbgPlan.language, 'zig');

  assert.equal(inferTestDebugLanguage('/proj/src/main.zig'), 'zig');
  assert.equal(inferTestDebugLanguage(null, 'zig_test'), 'zig');
  assert.equal(
    launchPreview({ language: 'zig', program: '/proj/src/main.zig', root: '/proj', test_name: 'adds' }),
    'zig test --test-filter "adds" src/main.zig',
  );
});
