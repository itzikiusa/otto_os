import assert from 'node:assert/strict';
import { test } from 'node:test';
import { EditorState } from '@codemirror/state';
import { langGutters, langKeyFor } from '../src/lib/components/langForPath.ts';
import { kotlinLang } from '../src/lib/components/cm-lang-kotlin.ts';
import { detectSemanticLang } from '../src/lib/components/cm-ide-semantic-model.ts';
import {
  detectLspCompletionTrigger,
  normalizeIdeLspLanguage,
  shouldAttachIdeLsp,
} from '../src/lib/components/cm-ide-lsp-completion.ts';
import {
  detectManifestEcosystem,
  isIdeManifestPath,
  parseManifestDependencies,
} from '../src/lib/components/cm-ide-deps-model.ts';
import {
  ideLanguageLabel,
  ideLanguageSettingsSection,
  ideLspLangFor,
  ideLspServerName,
} from '../src/modules/ide/editor/ideStatusModel.ts';
import { detectIdeDocKind, extractInstantSymbols } from '../src/modules/ide/editor/ideLangAnalyzer.ts';
import { inferTestDebugLanguage } from '../src/modules/ide/debug/ideFailedTestDebug.ts';
import { buildDedupedBulkRepos } from '../src/modules/ide/shell/bulkRepoModel.ts';

test('Kotlin files map to kotlin language pack, semantic model, status strip, and LSP', () => {
  assert.equal(langKeyFor('src/main/kotlin/com/acme/App.kt'), 'kt');
  assert.equal(langKeyFor('build.gradle.kts'), 'kts');
  assert.equal(langGutters('src/main/kotlin/App.kt').breakpoints, true);

  assert.equal(detectSemanticLang('src/main/kotlin/App.kt'), 'kotlin');
  assert.equal(detectIdeDocKind('src/main/kotlin/App.kt'), 'kotlin');

  assert.equal(normalizeIdeLspLanguage('kt'), 'kotlin');
  assert.equal(normalizeIdeLspLanguage('kts'), 'kotlin');
  assert.equal(shouldAttachIdeLsp('/ws/src/main/kotlin/App.kt', 'kotlin', '/ws'), true);

  assert.equal(ideLanguageLabel('src/main/kotlin/App.kt'), 'Kotlin');
  assert.equal(ideLanguageSettingsSection('src/main/kotlin/App.kt', 'kotlin'), 'lang.kotlin');
  assert.equal(ideLspLangFor('src/main/kotlin/App.kt', 'kotlin'), 'kotlin');
  assert.equal(ideLspServerName('kotlin'), 'kotlin-language-server');
  assert.equal(inferTestDebugLanguage('src/test/kotlin/AppTest.kt'), 'kotlin');

  const bulk = buildDedupedBulkRepos([
    { name: 'kt-svc', path: '/ws/kt-svc', kind: 'kotlin', has_git: true, has_go_mod: false, has_pom_or_gradle: true },
  ]);
  assert.deepEqual(bulk.techByPath['/ws/kt-svc'], ['java', 'kotlin']);

  // CodeMirror kotlinLang builds an EditorState cleanly
  const state = EditorState.create({
    doc: '@file:JvmName("App")\npackage com.acme\n\ndata class User(val id: Long)\nsuspend fun fetch(): User = User(1L)\n',
    extensions: [kotlinLang()],
  });
  assert.ok(state.doc.lines >= 5);

  // Trigger characters: `::` (method reference) and `@` (annotations)
  const refDoc = EditorState.create({ doc: 'String::' }).doc;
  assert.deepEqual(detectLspCompletionTrigger(refDoc, refDoc.length, false, 'kotlin'), {
    from: refDoc.length,
    context: { triggerKind: 2, triggerCharacter: ':' },
  });
  const annDoc = EditorState.create({ doc: '@' }).doc;
  assert.deepEqual(detectLspCompletionTrigger(annDoc, annDoc.length, false, 'kotlin'), {
    from: annDoc.length,
    context: { triggerKind: 2, triggerCharacter: '@' },
  });
});

test('Kotlin instant symbols extract data classes, companion objects, extension funs, and Kotest blocks', () => {
  const src = [
    'package com.acme',
    'data class Order(val id: Int)',
    'sealed interface Status',
    'object Registry {',
    '    companion object {',
    '        suspend fun String.normalize(): String = trim()',
    '    }',
    '}',
    'class OrderSpec : StringSpec({',
    '    "computes total" {',
    '        assert(true)',
    '    }',
    '})',
  ].join('\n');

  const syms = extractInstantSymbols('src/test/kotlin/com/acme/OrderSpec.kt', 'kotlin', src);
  const names = syms.map((s) => s.name);
  assert.ok(names.includes('Order'), `missing Order in ${JSON.stringify(names)}`);
  assert.ok(names.includes('Status'), `missing Status in ${JSON.stringify(names)}`);
  assert.ok(names.includes('Registry'), `missing Registry in ${JSON.stringify(names)}`);
  assert.ok(names.includes('Companion'), `missing Companion in ${JSON.stringify(names)}`);
  assert.ok(names.includes('normalize()'), `missing normalize() in ${JSON.stringify(names)}`);
  assert.ok(names.includes('computes total'), `missing Kotest block in ${JSON.stringify(names)}`);
});

test('Gradle Kotlin DSL and gradle/libs.versions.toml manifests parse dependencies cleanly', () => {
  assert.equal(isIdeManifestPath('/ws/build.gradle.kts'), true);
  assert.equal(isIdeManifestPath('/ws/gradle/libs.versions.toml'), true);
  assert.equal(detectManifestEcosystem('/ws/gradle/libs.versions.toml'), 'gradle');

  const gradleKts = [
    'dependencies {',
    '    implementation(kotlin("stdlib"))',
    '    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")',
    '    ksp("com.google.dagger:hilt-compiler:2.52")',
    '    testImplementation("io.kotest:kotest-runner-junit5:5.9.1")',
    '}',
  ].join('\n');
  const ktsDeps = parseManifestDependencies('/ws/build.gradle.kts', gradleKts);
  assert.equal(ktsDeps.length, 4);
  assert.ok(ktsDeps.some((d) => d.name === 'org.jetbrains.kotlin:kotlin-stdlib'));
  assert.ok(ktsDeps.some((d) => d.name === 'io.kotest:kotest-runner-junit5' && d.indirect));

  const versionCatalog = [
    '[versions]',
    'coroutines = "1.9.0"',
    'kotest = "5.9.1"',
    '',
    '[libraries]',
    'kotlinx-coroutines = { module = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version.ref = "coroutines" }',
    'kotest-runner = { group = "io.kotest", name = "kotest-runner-junit5", version.ref = "kotest" }',
    'junit = "org.junit.jupiter:junit-jupiter:5.11.0"',
  ].join('\n');
  const catDeps = parseManifestDependencies('/ws/gradle/libs.versions.toml', versionCatalog);
  assert.equal(catDeps.length, 3);
  assert.ok(
    catDeps.some(
      (d) =>
        d.name === 'org.jetbrains.kotlinx:kotlinx-coroutines-core' &&
        d.current_version === '1.9.0' &&
        d.property_name === 'coroutines',
    ),
  );
  assert.ok(catDeps.some((d) => d.name === 'org.junit.jupiter:junit-jupiter' && d.current_version === '5.11.0'));
});

