import assert from 'node:assert/strict';
import test from 'node:test';
import { EditorState } from '@codemirror/state';
import { cmakeLang, cppLang, mesonLang } from '../src/lib/components/cm-lang-cpp.ts';
import { langKeyFor, LANG_KEYS } from '../src/lib/components/langForPath.ts';
import { detectLspCompletionTrigger, normalizeIdeLspLanguage, shouldAttachIdeLsp } from '../src/lib/components/cm-ide-lsp-completion.ts';
import { isIdeManifestPath } from '../src/lib/components/cm-ide-manifest.ts';
import { detectManifestEcosystem, parseManifestDependencies } from '../src/lib/components/cm-ide-deps-model.ts';
import {
  decodeSwitchSourceHeaderUri,
  headerSourceCandidates,
  isCppHeaderOrSourcePath,
  matchHeaderSourceFromList,
} from '../src/modules/ide/editor/ideHeaderSwitch.ts';
import { extractInstantSymbols } from '../src/modules/ide/editor/ideLangAnalyzer.ts';
import { ideLanguageLabel, ideLanguageSettingsSection, ideLspLangFor, ideLspServerName } from '../src/modules/ide/editor/ideStatusModel.ts';
import { inferTestDebugLanguage } from '../src/modules/ide/debug/ideFailedTestDebug.ts';
import { detectRepoTestEcosystem, resolveRepoTestPlan } from '../src/modules/ide/store/ideRepoTestAndBuildTags.ts';

test('C/C++/CMake/Meson syntax packs instantiate and langKeyFor maps all C/C++/Unreal paths', () => {
  assert.ok(cppLang());
  assert.ok(cmakeLang());
  assert.ok(mesonLang());

  for (const k of ['c', 'cpp', 'cmake', 'meson']) {
    assert.equal(LANG_KEYS.has(k), true, `LANG_KEYS missing ${k}`);
  }
  assert.equal(langKeyFor('/ws/src/main.cpp'), 'cpp');
  assert.equal(langKeyFor('/ws/include/engine.hpp'), 'cpp');
  assert.equal(langKeyFor('/ws/src/util.c'), 'c');
  assert.equal(langKeyFor('/ws/CMakeLists.txt'), 'cmake');
  assert.equal(langKeyFor('/ws/cmake/Flags.cmake'), 'cmake');
  assert.equal(langKeyFor('/ws/meson.build'), 'meson');
  assert.equal(langKeyFor('/ws/MyGame.uproject'), 'json');
});

test('clangd LSP normalization, attach gating, and :: / -> completion triggers', () => {
  assert.equal(normalizeIdeLspLanguage('cpp'), 'cpp');
  assert.equal(normalizeIdeLspLanguage('c'), 'cpp');
  assert.equal(normalizeIdeLspLanguage('hpp'), 'cpp');
  assert.equal(normalizeIdeLspLanguage('cxx'), 'cpp');
  assert.equal(shouldAttachIdeLsp('/ws/src/main.cpp', 'cpp', '/ws'), true);
  assert.equal(shouldAttachIdeLsp('/ws/include/a.hpp', undefined, '/ws'), true);

  const stScope = EditorState.create({ doc: 'std::' });
  const trigScope = detectLspCompletionTrigger(stScope.doc, 5, false, 'cpp');
  assert.deepEqual(trigScope?.context, { triggerKind: 2, triggerCharacter: ':' });

  const stArrow = EditorState.create({ doc: 'ptr->' });
  const trigArrow = detectLspCompletionTrigger(stArrow.doc, 5, false, 'cpp');
  assert.deepEqual(trigArrow?.context, { triggerKind: 2, triggerCharacter: '>' });
});

test('Header/source switch resolves same-dir, src/include, Unreal Private/Public, and file:// URIs', () => {
  assert.equal(isCppHeaderOrSourcePath('/ws/src/math.cpp'), true);
  assert.equal(isCppHeaderOrSourcePath('/ws/include/math.hpp'), true);
  assert.equal(isCppHeaderOrSourcePath('/ws/README.md'), false);

  assert.equal(
    decodeSwitchSourceHeaderUri('file:///Users/dev/proj/include/math%20util.hpp'),
    '/Users/dev/proj/include/math util.hpp',
  );

  const cands = headerSourceCandidates('/ws/src/math.cpp');
  assert.ok(cands.includes('/ws/src/math.hpp'));
  assert.ok(cands.includes('/ws/include/math.hpp'));

  const ueCands = headerSourceCandidates('/ue/Source/MyGame/Private/HeroActor.cpp');
  assert.ok(ueCands.includes('/ue/Source/MyGame/Public/HeroActor.h'));

  assert.equal(
    matchHeaderSourceFromList('/ws/src/math.cpp', ['/ws/include/demo/math.hpp', '/ws/src/main.cpp']),
    '/ws/include/demo/math.hpp',
  );
  assert.equal(
    matchHeaderSourceFromList('/ue/Source/MyGame/Public/HeroActor.h', ['/ue/Source/MyGame/Private/HeroActor.cpp']),
    '/ue/Source/MyGame/Private/HeroActor.cpp',
  );
});

test('C/C++ & Unreal manifests parse dependencies in vcpkg.json, conanfile.txt, CMakeLists.txt, and .uproject', () => {
  for (const p of ['/ws/vcpkg.json', '/ws/conanfile.txt', '/ws/CMakeLists.txt', '/ws/MyGame.uproject']) {
    assert.equal(isIdeManifestPath(p), true, `expected manifest: ${p}`);
    assert.equal(detectManifestEcosystem(p), 'cpp');
  }

  const vcpkg = parseManifestDependencies('/ws/vcpkg.json', '{\n  "dependencies": [\n    "fmt",\n    { "name": "nlohmann-json", "version>=": "3.11.3" }\n  ]\n}\n');
  assert.equal(vcpkg.length, 2);
  assert.equal(vcpkg[0].name, 'fmt');
  assert.equal(vcpkg[1].name, 'nlohmann-json');
  assert.equal(vcpkg[1].current_version, '3.11.3');

  const conan = parseManifestDependencies('/ws/conanfile.txt', '[requires]\nfmt/10.2.1\nspdlog/1.14.1\n');
  assert.equal(conan.length, 2);
  assert.equal(conan[0].name, 'fmt');
  assert.equal(conan[0].current_version, '10.2.1');

  const cmake = parseManifestDependencies('/ws/CMakeLists.txt', 'find_package(GTest 1.14 REQUIRED)\nFetchContent_Declare(Catch2 GIT_TAG v3.5.0)\n');
  assert.equal(cmake.length, 2);
  assert.equal(cmake[0].name, 'GTest');
  assert.equal(cmake[1].name, 'Catch2');

  const uproj = parseManifestDependencies('/ws/Demo.uproject', '{\n  "Plugins": [\n    { "Name": "EnhancedInput", "Enabled": true }\n  ]\n}\n');
  assert.equal(uproj.length, 1);
  assert.equal(uproj[0].name, 'EnhancedInput');
  assert.equal(uproj[0].current_version, 'enabled');
});

test('C/C++ status model, instant symbols, failed-test inference, and repo test plan', () => {
  assert.equal(ideLanguageLabel('/ws/src/main.cpp', 'cpp'), 'C++');
  assert.equal(ideLanguageLabel('/ws/CMakeLists.txt', 'cmake'), 'CMake');
  assert.equal(ideLanguageSettingsSection('/ws/src/main.cpp', 'cpp'), 'lang.cpp');
  assert.equal(ideLspLangFor('/ws/src/main.cpp', 'cpp'), 'cpp');
  assert.equal(ideLspServerName('cpp'), 'clangd');
  assert.equal(inferTestDebugLanguage('/ws/tests/math_test.cpp', 'ctest'), 'cpp');

  const syms = extractInstantSymbols(
    '/ws/src/actor.cpp',
    'cpp',
    'namespace game {\nclass DEMO_API HeroActor {\n};\nTEST(MathSuite, Adds) {\n}\nTEST_CASE("Catch works") {\n}\nint compute(int x) {\n  return x;\n}\n}\n',
  );
  const names = syms.map((s) => s.name);
  assert.ok(names.includes('game'));
  assert.ok(names.includes('HeroActor'));
  assert.ok(names.includes('MathSuite.Adds'));
  assert.ok(names.includes('Catch works'));
  assert.ok(names.includes('compute()'));

  const eco = detectRepoTestEcosystem({
    repoRoot: '/ws/cpp-app',
    runConfigs: [],
    detectedConfigs: [],
    dirEntries: [{ name: 'CMakeLists.txt', path: '/ws/cpp-app/CMakeLists.txt' }],
    openTabs: [],
  });
  assert.equal(eco, 'cpp_cmake');
  const plan = resolveRepoTestPlan({
    repoRoot: '/ws/cpp-app',
    mode: 'run',
    runConfigs: [],
    detectedConfigs: [],
    dirEntries: [{ name: 'CMakeLists.txt', path: '/ws/cpp-app/CMakeLists.txt' }],
    openTabs: [],
    goBuildTags: [],
  });
  assert.equal(plan.kind, 'run');
  if (plan.kind === 'run') assert.equal(plan.adhoc.runner, 'ctest');
});
