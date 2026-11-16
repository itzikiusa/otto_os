import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  extractGoBuildTagsFromSource,
  resolveIdeLspModuleRoot,
  rewriteGoBuildTagDiagnostic,
} from '../src/lib/components/cm-ide-lsp-completion.ts';
import {
  effortsForModel,
  FALLBACK_PROVIDER_MODELS,
  modelsForProvider,
  providerFamily,
  supportsEffortSelection,
  supportsInteractionMode,
  supportsModelSelection,
} from '../src/lib/providerModels.ts';
import {
  computeInlineVariableEdit,
  computeIntroduceVariableEdit,
  detectInlineRefactorHints,
  transformStringManipulation,
} from '../src/lib/components/cm-ide-refactor-core.ts';
import { splitStreamingMarkdown } from '../src/modules/agents/conversation/streamingMd.ts';
import {
  mergeGoBuildTagsIntoFlags,
  normalizeGoBuildTags,
  resolveRepoGoBuildTags,
  resolveRepoRootForPath,
  resolveRepoTestPlan,
} from '../src/modules/ide/store/ideRepoTestAndBuildTags.ts';

describe('Round 4 — Antigravity model catalog & Orchestrator entries', () => {
  it('exposes all 9 Antigravity models immediately on cold start including Orchestrator (Carbon) and Orchestrator (Argon)', () => {
    const agy = modelsForProvider('agy', { provider_models: {} });
    assert.equal(FALLBACK_PROVIDER_MODELS.agy.length, 9);
    assert.equal(agy.length, 9);

    const ids = agy.map((m) => m.id);
    assert.deepEqual(ids, [
      'gemini-4-carbon',
      'gemini-4-argon',
      'gemini-4-hydrogen',
      'gemini-pro-orchestrator',
      'gemini-pro-orchestrator-argon',
      'gemini-3.8-flash',
      'claude-opus-5-5',
      'claude-sonnet-5-5',
      'gpt-oss-120b-medium',
    ]);

    const carbonOrch = agy.find((m) => m.id === 'gemini-pro-orchestrator');
    const argonOrch = agy.find((m) => m.id === 'gemini-pro-orchestrator-argon');
    assert.equal(carbonOrch?.label, 'Orchestrator (Carbon)');
    assert.equal(argonOrch?.label, 'Orchestrator (Argon)');
    assert.equal(supportsEffortSelection('agy', null), true);
    assert.equal(supportsEffortSelection('agy-work', null), true);
    assert.deepEqual(effortsForModel('agy', 'gemini-pro-orchestrator', null), []);
    assert.deepEqual(effortsForModel('agy', 'gemini-pro-orchestrator-argon', null), []);
    assert.deepEqual(effortsForModel('agy', 'gemini-4-carbon', null), ['low', 'medium', 'high']);
  });
});

describe('Round 4 — Incremental streaming markdown splitter', () => {
  it('splits completed paragraphs from active streaming tail without breaking open code fences or reference links', () => {
    const shortMd = 'Short paragraph still under threshold.\n\nTail words';
    assert.deepEqual(splitStreamingMarkdown(shortMd), { settled: '', tail: shortMd });

    const p1 = `${'First completed paragraph with prose and details. '.repeat(8).trim()}\n\n`;
    const p2 = '```rust\nfn demo() -> i32 { 42 }\n```\n\n';
    const tail = 'Currently streaming trailing sentence...';
    const combined = `${p1}${p2}${tail}`;
    const split = splitStreamingMarkdown(combined);
    assert.equal(split.settled, `${p1}${p2}`);
    assert.equal(split.tail, tail);
    assert.equal(split.settled + split.tail, combined);

    // Unclosed trailing code fence must stay inside `tail` so the fence never renders torn across passes.
    const unclosedFence = `${p1}\`\`\`go\nfunc partial() {\n\n  stillInsideFence := 1`;
    const fenceSplit = splitStreamingMarkdown(unclosedFence);
    assert.equal(fenceSplit.settled, p1);
    assert.equal(fenceSplit.tail, '```go\nfunc partial() {\n\n  stillInsideFence := 1');

    // Reference-style link definitions fall back to single-pass rendering.
    const withRefDef = `${p1}See [guide][g].\n\n[g]: https://example.com`;
    assert.deepEqual(splitStreamingMarkdown(withRefDef), { settled: '', tail: withRefDef });
  });
});

describe('Round 4 — Go build tags, diagnostics rewrite, and outside-of-root LSP module scoping', () => {
  it('extracts custom //go:build and // +build tags while ignoring standard OS/arch constraints', () => {
    const src = [
      '//go:build component && (linux || darwin) && go1.22',
      '// +build component,integration !windows',
      '',
      'package tests',
      '//go:build ignored_after_package',
    ].join('\n');
    assert.deepEqual(extractGoBuildTagsFromSource(src), ['component', 'integration']);
  });

  it('rewrites misleading gopls "No packages found for open file" diagnostics with detected build tags', () => {
    const src = '//go:build component\n\npackage tests\n';
    const rewritten = rewriteGoBuildTagDiagnostic(
      'No packages found for open file /Users/dev/koala-justslots-go/tests/assign_frb_component_test.go.',
      undefined,
      src,
    );
    assert.equal(rewritten.isBuildTagDiag, true);
    assert.deepEqual(rewritten.tags, ['component']);
    assert.match(rewritten.message, /File requires Go build tag "component"/);

    const normal = rewriteGoBuildTagDiagnostic('undefined: client', 'UndeclaredName', src);
    assert.equal(normal.isBuildTagDiag, false);
    assert.equal(normal.message, 'undefined: client');
  });

  it('scopes outside-of-root files to their own directory when not under any attached project folder', () => {
    assert.equal(
      resolveIdeLspModuleRoot(
        '/ws/primary-repo',
        '/ws/other-repo/pkg/client.go',
        ['/ws/primary-repo', '/ws/attached-repo'],
      ),
      '/ws/other-repo/pkg',
    );
    assert.equal(
      resolveIdeLspModuleRoot(
        '/ws/primary-repo',
        '/ws/attached-repo/internal/svc.go',
        ['/ws/primary-repo', '/ws/attached-repo'],
      ),
      '/ws/attached-repo',
    );
  });
});

describe('Round 4 — Per-repository Go build tags & Run/Coverage/Debug All Tests planner', () => {
  const roots = [
    { path: '/ws/umbrella', name: 'umbrella', is_primary: true },
    { path: '/ws/koala-justslots-go', name: 'koala-justslots-go', is_primary: false },
    { path: '/ws/otto-rust', name: 'otto-rust', is_primary: false },
  ];

  it('normalizes, resolves, and merges Go build tags strictly per repository', () => {
    assert.deepEqual(
      normalizeGoBuildTags(['-tags=component, integration', 'component', 'e2e_v2', 'bad;rm']),
      ['component', 'integration', 'e2e_v2'],
    );

    const repo = resolveRepoRootForPath(
      '/ws/umbrella',
      roots,
      '/ws/koala-justslots-go/tests/assign_component_test.go',
    );
    assert.deepEqual(repo, { path: '/ws/koala-justslots-go', name: 'koala-justslots-go' });

    const env = {
      go_build_tags: ['umbrella_tag'],
      module_go_build_tags: {
        '/ws/koala-justslots-go': ['component'],
      },
      env_vars: [],
    };
    assert.deepEqual(
      resolveRepoGoBuildTags(
        env,
        '/ws/umbrella',
        '/ws/koala-justslots-go',
        '//go:build integration\npackage tests\n',
      ),
      ['component', 'integration'],
    );
    assert.deepEqual(
      mergeGoBuildTagsIntoFlags(['-race', '-tags=smoke'], ['component', 'smoke']),
      ['-race', '-tags=smoke,component'],
    );
  });

  it('builds per-repository Run All Tests, Coverage, and Debug All Tests plans without cross-repo leakage', () => {
    const goCov = resolveRepoTestPlan({
      repoRoot: '/ws/koala-justslots-go',
      repoName: 'koala-justslots-go',
      mode: 'coverage',
      runConfigs: [],
      detectedConfigs: [],
      dirEntries: [{ name: 'go.mod', path: '/ws/koala-justslots-go/go.mod' }],
      openTabs: [],
      goBuildTags: ['component'],
    });
    assert.equal(goCov.kind, 'run');
    assert.equal(goCov.adhoc.cwd, '/ws/koala-justslots-go');
    assert.equal(goCov.adhoc.runner, 'go_test');
    assert.equal(goCov.adhoc.target_path, './...');
    assert.equal(goCov.adhoc.coverage, true);
    assert.deepEqual(goCov.adhoc.build_flags, ['-tags=component']);

    const goDebug = resolveRepoTestPlan({
      repoRoot: '/ws/koala-justslots-go',
      repoName: 'koala-justslots-go',
      mode: 'debug',
      runConfigs: [],
      detectedConfigs: [],
      dirEntries: [{ name: 'go.mod', path: '/ws/koala-justslots-go/go.mod' }],
      openTabs: [],
      goBuildTags: ['component'],
    });
    assert.equal(goDebug.kind, 'debug');
    assert.equal(goDebug.language, 'go');
    assert.equal(goDebug.launch.cwd, '/ws/koala-justslots-go');
    assert.equal(goDebug.launch.program, '/ws/koala-justslots-go/...');
    assert.deepEqual(goDebug.launch.build_flags, ['-tags=component']);

    const rustRun = resolveRepoTestPlan({
      repoRoot: '/ws/otto-rust',
      repoName: 'otto-rust',
      mode: 'run',
      runConfigs: [],
      detectedConfigs: [],
      dirEntries: [{ name: 'Cargo.toml', path: '/ws/otto-rust/Cargo.toml' }],
      openTabs: [],
      goBuildTags: ['component'],
    });
    assert.equal(rustRun.kind, 'run');
    assert.equal(rustRun.adhoc.cwd, '/ws/otto-rust');
    assert.equal(rustRun.adhoc.runner, 'cargo_test');
    assert.equal(rustRun.adhoc.coverage, false);
    assert.equal(rustRun.adhoc.build_flags, undefined);
  });
});

describe('Round 4 — Custom Claude provider family & model/effort support (claudework)', () => {
  it('recognizes claudework without hyphen as Claude family for models, reasoning effort, and access mode', () => {
    assert.equal(providerFamily('claudework'), 'claude');
    assert.equal(providerFamily('claude-work'), 'claude');
    assert.equal(supportsModelSelection('claudework'), true);
    assert.equal(supportsEffortSelection('claudework'), true);
    assert.equal(supportsInteractionMode('claudework'), true);
    const models = modelsForProvider('claudework', { provider_models: {} });
    assert.ok(models.length > 0, 'claudework should inherit Claude fallback models when live catalog is empty');
  });
});

describe('Round 4 — IDE CodeVision inline refactor hints & editor refactorings', () => {
  it('detects in-place declaration renames (R✎ Rename usages) and struct Implement interface hints', () => {
    const baseline = [
      'package client',
      '',
      'type GenericHttpResponse struct {',
      '\tHeaders http.Header',
      '\tBody    []byte',
      '}',
    ].join('\n');
    const edited = [
      'package client',
      '',
      'type GenericHttpResponse struct {',
      '\taaHeaders http.Header',
      '\tBody      []byte',
      '}',
    ].join('\n');

    const hints = detectInlineRefactorHints(edited, baseline, 'go');
    const renameHint = hints.find((h) => h.kind === 'rename-usages');
    assert.deepEqual(renameHint, {
      kind: 'rename-usages',
      line: 4,
      column: 2,
      oldName: 'Headers',
      newName: 'aaHeaders',
    });

    assert.equal(hints.find((h) => h.kind === 'implement-interface'), undefined, 'a plain data struct gets no "Implement interface" hint');

    const withIface = `type Responder interface {\n\tRespond() error\n}\n\n${edited}`;
    assert.deepEqual(detectInlineRefactorHints(withIface, null, 'go').find((h) => h.kind === 'implement-interface'), {
      kind: 'implement-interface',
      line: 7,
      column: 1,
      symbol: 'GenericHttpResponse',
    });
    const ts = 'interface Shape { area(): number }\nclass Square {}\n';
    assert.equal(detectInlineRefactorHints(ts, null, 'typescript').filter((h) => h.kind === 'implement-interface').length, 1);
    assert.equal(detectInlineRefactorHints('class Square {}\n', null, 'typescript').length, 0);
  });

  it('transforms string cases and computes Introduce Variable / Inline Variable edits', () => {
    assert.equal(transformStringManipulation('genericHttpResponse', 'snake'), 'generic_http_response');
    assert.equal(transformStringManipulation('generic_http_response', 'pascal'), 'GenericHttpResponse');
    assert.equal(transformStringManipulation('genericHttpResponse', 'screaming_snake'), 'GENERIC_HTTP_RESPONSE');

    const goSrc = 'func demo() {\n\tresp := fetchUser(ctx, 42)\n\tuse(resp)\n}\n';
    const exprStart = goSrc.indexOf('fetchUser(ctx, 42)');
    const exprEnd = exprStart + 'fetchUser(ctx, 42)'.length;
    const intro = computeIntroduceVariableEdit(goSrc, exprStart, exprEnd, 'go', false);
    assert.ok(intro);
    assert.equal(intro.changes.length, 2);
    assert.equal(intro.changes[0].insert, '\textracted := fetchUser(ctx, 42)\n');
    assert.equal(intro.changes[1].insert, 'extracted');

    const inlineEdit = computeInlineVariableEdit(goSrc, goSrc.indexOf('resp :='));
    assert.ok(inlineEdit);
    assert.equal(inlineEdit.changes.length, 2);
    assert.equal(inlineEdit.changes[0].insert, '');
    assert.equal(inlineEdit.changes[1].insert, '(fetchUser(ctx, 42))');
  });
});
