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

test('C# and F# language keys, gutters, and CodeMirror pack loading', async () => {
  assert.equal(langKeyFor('/proj/src/Program.cs'), 'cs');
  assert.equal(langKeyFor('/proj/src/Script.csx'), 'csx');
  assert.equal(langKeyFor('/proj/src/Pages/Home.razor'), 'cs');
  assert.equal(langKeyFor('/proj/src/Math.fs'), 'fs');
  assert.equal(langKeyFor('/proj/src/Script.fsx'), 'fsx');
  assert.equal(langKeyFor('/proj/noext', 'csharp'), 'cs');
  assert.equal(langKeyFor('/proj/noext', 'fsharp'), 'fs');

  const g = langGutters('/proj/src/Program.cs');
  assert.equal(g.breakpoints, true);
  assert.equal(g.lint, true);
  assert.equal(g.fold, true);

  await loadCmLang('cs');
  const csExt = cmLangNow('cs', 'mysql');
  assert.ok(csExt, 'C# CodeMirror extension should be loaded');

  const doc = 'public class Settlement {\n    public int Add(int a, int b) {\n        return a + b;\n    }\n}\n';
  const state = EditorState.create({ doc, extensions: [csExt] });
  const range = foldable(state, 0, doc.indexOf('\n'));
  assert.ok(range && range.from < range.to, 'C# brace block should be foldable');

  await loadCmLang('fs');
  const fsExt = cmLangNow('fs', 'mysql');
  assert.ok(fsExt, 'F# CodeMirror extension should be loaded');
});

test('C# and F# LSP gating, status model, outline symbols, and semantic classifier', () => {
  assert.equal(normalizeIdeLspLanguage('csharp'), 'csharp');
  assert.equal(normalizeIdeLspLanguage('cs'), 'csharp');
  assert.equal(normalizeIdeLspLanguage('fsharp'), 'fsharp');
  assert.equal(normalizeIdeLspLanguage('fs'), 'fsharp');
  assert.equal(shouldAttachIdeLsp('/proj/src/Program.cs', 'csharp', '/proj'), true);
  assert.equal(shouldAttachIdeLsp('/proj/src/Lib.fs', 'fsharp', '/proj'), true);

  assert.equal(ideFileKind('/proj/src/Program.cs'), 'code');
  assert.equal(ideFileKind('/proj/src/Lib.fs'), 'code');
  assert.equal(ideLanguageLabel('/proj/src/Program.cs', 'csharp'), 'C#');
  assert.equal(ideLanguageLabel('/proj/src/Lib.fs', 'fsharp'), 'F#');
  assert.equal(ideLanguageSettingsSection('/proj/src/Program.cs', 'csharp'), 'lang.dotnet');
  assert.equal(ideLanguageSettingsSection('/proj/src/Lib.fs', 'fsharp'), 'lang.dotnet');
  assert.equal(ideLspLangFor('/proj/src/Program.cs', 'csharp'), 'csharp');
  assert.equal(ideLspLangFor('/proj/src/Lib.fs', 'fsharp'), 'fsharp');
  assert.equal(ideLspServerName('csharp'), 'csharp-ls');
  assert.equal(ideLspServerName('fsharp'), 'fsautocomplete');

  const sample = [
    'namespace Otto.Payments;',
    '/// <summary>Computes bonuses.</summary>',
    'public class SettlementService {',
    '    [Fact]',
    '    public int ComputeBonus(int amount, int rate) {',
    '        return amount + rate;',
    '    }',
    '}',
  ].join('\n');

  assert.equal(detectIdeDocKind('/proj/src/SettlementService.cs'), 'csharp');
  const syms = extractInstantSymbols('/proj/src/SettlementService.cs', 'csharp', sample);
  assert.ok(syms.some((s) => s.name === 'SettlementService' && s.kind === 'class'));
  assert.ok(syms.some((s) => s.name === 'ComputeBonus'));

  assert.equal(detectSemanticLang('/proj/src/SettlementService.cs'), 'csharp');
  const toks = classifySemanticTokens(sample, 'csharp');
  assert.ok(toks.some((t) => t.role === 'doc-comment' && t.text.includes('Computes bonuses')));
});

test('NuGet manifest parser, nesting, test plans, and debug previews', () => {
  assert.equal(isIdeManifestPath('/proj/App.csproj'), true);
  assert.equal(isIdeManifestPath('/proj/Lib.fsproj'), true);
  assert.equal(isIdeManifestPath('/proj/Directory.Packages.props'), true);
  assert.equal(detectManifestEcosystem('/proj/App.csproj'), 'nuget');
  assert.equal(nestParentName('appsettings.Development.json'), 'appsettings.json');
  assert.equal(nestParentName('Counter.razor.cs'), 'Counter.razor');

  const csproj = `<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
    <PackageReference Include="xunit" Version="2.9.2" />
  </ItemGroup>
</Project>`;
  const deps = parseManifestDependencies('/proj/App.csproj', csproj);
  assert.deepEqual(
    deps.map((d) => [d.name, d.current_version]),
    [
      ['Newtonsoft.Json', '13.0.3'],
      ['xunit', '2.9.2'],
    ],
  );

  const runPlan = resolveRepoTestPlan({
    repoRoot: '/proj',
    mode: 'run',
    runConfigs: [],
    detectedConfigs: [],
    dirEntries: [{ name: 'App.csproj', path: '/proj/App.csproj' }],
    openTabs: [],
    goBuildTags: [],
  });
  assert.equal(runPlan.kind, 'run');
  if (runPlan.kind === 'run') assert.equal(runPlan.adhoc.runner, 'dotnet_test');

  const dbgPlan = resolveRepoTestPlan({
    repoRoot: '/proj',
    mode: 'debug',
    runConfigs: [],
    detectedConfigs: [],
    dirEntries: [{ name: 'App.csproj', path: '/proj/App.csproj' }],
    openTabs: [],
    goBuildTags: [],
  });
  assert.equal(dbgPlan.kind, 'debug');
  if (dbgPlan.kind === 'debug') {
    assert.equal(dbgPlan.language, 'csharp');
    assert.equal(dbgPlan.launch.is_test, true);
  }

  assert.equal(inferTestDebugLanguage('/proj/Tests/CalcTests.cs'), 'csharp');
  assert.equal(inferTestDebugLanguage('/proj/Tests/CalcTests.fs'), 'fsharp');
  assert.equal(inferTestDebugLanguage(null, 'dotnet_test'), 'csharp');
  assert.equal(
    launchPreview({ language: 'csharp', program: '/proj/Tests/CalcTests.cs', root: '/proj', test_name: 'AddsNumbers' }),
    'dotnet test --filter "FullyQualifiedName~AddsNumbers" Tests/CalcTests.cs',
  );
});

test('BUG-17: buildDedupedBulkRepos preserves dotnet badge on Unreal repos with generated .sln', async () => {
  const { buildDedupedBulkRepos } = await import('../src/modules/ide/shell/bulkRepoModel.ts');
  const { techByPath } = buildDedupedBulkRepos([
    { name: 'UnrealWithSln', path: '/ws/UnrealWithSln', kind: 'unreal', has_go_mod: false, has_pom_or_gradle: false, has_git: true, has_dotnet: true },
    { name: 'UnrealFresh', path: '/ws/UnrealFresh', kind: 'unreal', has_go_mod: false, has_pom_or_gradle: false, has_git: true, has_dotnet: false },
  ]);
  assert.deepEqual(techByPath['/ws/UnrealWithSln'], ['cpp', 'dotnet']);
  assert.deepEqual(techByPath['/ws/UnrealFresh'], ['cpp']);
});

test('C# and F# LSP trust refusal surfaces the .NET trust opt-in in the status strip', async () => {
  const { lspHandshakeFailureReason } = await import('../src/lib/components/cm-ide-lsp-reconnect.ts');
  const { ideLspItem } = await import('../src/modules/ide/editor/ideStatusModel.ts');
  for (const lang of ['csharp', 'fsharp']) {
    assert.equal(lspHandshakeFailureReason(lang), 'trust_or_unavailable');
    const item = ideLspItem(lang, { lang, rootPath: '/app', phase: 'unavailable', reason: 'trust_or_unavailable' });
    assert.ok(item?.title.includes('OTTO_IDE_TRUST_DOTNET_LSP=1'), lang);
  }
});
