import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import { test } from 'node:test';
import {
  applyFallbackManifestVersionUpdate,
  detectManifestEcosystem,
  isIdeManifestPath,
  isVersionOutdated,
  parseManifestDependencies,
} from '../src/lib/components/cm-ide-deps.ts';
import {
  analyzeJavaDiagnostics,
  classifySemanticTokens,
  computeImportFoldRange,
  computeRainbowBrackets,
  detectSemanticLang,
} from '../src/lib/components/cm-ide-semantic.ts';
import {
  IDE_COLOR_SCHEME_OPTIONS,
  isDarkIdeScheme,
  resolveEffectiveIdeScheme,
} from '../src/lib/components/cm-ide-theme.ts';

function roleMap(doc: string, spans: ReturnType<typeof classifySemanticTokens>): Map<string, Set<string>> {
  const map = new Map<string, Set<string>>();
  for (const s of spans) {
    const text = doc.slice(s.from, s.to);
    let set = map.get(text);
    if (!set) {
      set = new Set();
      map.set(text, set);
    }
    set.add(s.role);
  }
  return map;
}

test('Go semantic classifier distinguishes package prefix, struct/pkg fields, func decls, func calls, and params', () => {
  const goCode = [
    'package main',
    '',
    'import (',
    '\t"github.com/shopspring/decimal"',
    '\t"promo/internal/app"',
    ')',
    '',
    'var (',
    '\tappName = "promotions"',
    '\tVersion = "1.0.0"',
    '\tBuild   = "42"',
    ')',
    '',
    'func runWithCtx(ctx Context, retries int) {',
    '\tdecimal.MarshalJSONWithoutQuotes = true',
    '\tdecimal.DivisionPrecision = 4',
    '\tapp.InitApp(appName, Version, retries)',
    '}',
  ].join('\n');

  const t0 = performance.now();
  const spans = classifySemanticTokens(goCode, 'go');
  const elapsed = performance.now() - t0;
  assert.ok(elapsed < 25, `expected fast viewport semantic scan, took ${elapsed.toFixed(2)}ms`);

  const byIdent = roleMap(goCode, spans);
  // Package prefixes must be neutral foreground (`fg`), NEVER coral-red or purple field!
  assert.ok(byIdent.get('decimal')?.has('fg'));
  assert.ok(byIdent.get('app')?.has('fg'));

  // Exported/struct fields & package-level vars must be JetBrains purple `field`!
  assert.ok(byIdent.get('MarshalJSONWithoutQuotes')?.has('field'));
  assert.ok(byIdent.get('DivisionPrecision')?.has('field'));
  assert.ok(byIdent.get('appName')?.has('field'));
  assert.ok(byIdent.get('Version')?.has('field'));

  // Function decl vs function call vs parameter must each get distinct roles!
  assert.ok(byIdent.get('runWithCtx')?.has('func-decl'));
  assert.ok(byIdent.get('InitApp')?.has('func-call'));
  assert.ok(byIdent.get('retries')?.has('param'));
  assert.ok(byIdent.get('Context')?.has('type'));
});

test('Java semantic classifier colors annotations, fields, Javadoc @param tags, method decls, calls, and params', () => {
  const javaCode = [
    'package com.gamescale.reports;',
    '',
    'import java.util.List;',
    'import java.util.Map;',
    'import lombok.extern.slf4j.Slf4j;',
    'import org.springframework.stereotype.Service;',
    '',
    '@Slf4j',
    '@Service',
    'public class OperatorReportsFacade {',
    '    private final GetOperatorReportsService getOperatorReportsService;',
    '',
    '    /**',
    '     * Merges operator reports.',
    '     * @param mergedOperators input operator rows',
    '     */',
    '    public Map<String, Integer> getMergedOperatorReports(List<String> mergedOperators) {',
    '        log.debug("count={}", mergedOperators.size());',
    '        return getOperatorReportsService.fetchAll(mergedOperators);',
    '    }',
    '}',
  ].join('\n');

  const spans = classifySemanticTokens(javaCode, 'java');
  const byIdent = roleMap(javaCode, spans);

  assert.ok(byIdent.get('@Slf4j')?.has('annotation'));
  assert.ok(byIdent.get('@Service')?.has('annotation'));
  assert.ok(byIdent.get('GetOperatorReportsService')?.has('type'));
  assert.ok(byIdent.get('getOperatorReportsService')?.has('field'));
  assert.ok(byIdent.get('@param')?.has('doc-tag'));
  assert.ok(byIdent.get('getMergedOperatorReports')?.has('func-decl'));
  assert.ok(byIdent.get('log')?.has('field'));
  assert.ok(byIdent.get('debug')?.has('func-call'));
  assert.ok(byIdent.get('fetchAll')?.has('func-call'));
  assert.ok(byIdent.get('mergedOperators')?.has('param'));
});

test('All 8 IDE languages map and classify cleanly (TS, Rust, Python, Shell, JSON, CSV + Go/Java)', () => {
  assert.equal(detectSemanticLang('src/index.mts'), 'ts');
  assert.equal(detectSemanticLang('crates/lib.rs'), 'rust');
  assert.equal(detectSemanticLang('app/worker.pyi'), 'python');
  assert.equal(detectSemanticLang('scripts/run.zsh'), 'shell');
  assert.equal(detectSemanticLang('data/config.jsonl'), 'json');
  assert.equal(detectSemanticLang('data/report.csv'), 'csv');
  assert.equal(detectSemanticLang('jdt://contents/java.base/java.util/List.class'), 'java');

  const tsSpans = classifySemanticTokens('@Injectable()\nexport class AuthSvc {\n  login(user: UserDto) { return fetchToken(user); }\n}', 'ts');
  const tsRoles = roleMap('@Injectable()\nexport class AuthSvc {\n  login(user: UserDto) { return fetchToken(user); }\n}', tsSpans);
  assert.ok(tsRoles.get('@Injectable')?.has('annotation'));
  assert.ok(tsRoles.get('fetchToken')?.has('func-call'));

  const rsDoc = '#[derive(Debug)]\npub fn compute(limit: usize) -> Result<usize, Error> { helper(limit) }';
  const rsRoles = roleMap(rsDoc, classifySemanticTokens(rsDoc, 'rust'));
  assert.ok(rsRoles.get('compute')?.has('func-decl'));
  assert.ok(rsRoles.get('helper')?.has('func-call'));

  const pyDoc = '@dataclass\ndef process_batch(items: list[str]) -> int:\n    return len(items)\n';
  const pyRoles = roleMap(pyDoc, classifySemanticTokens(pyDoc, 'python'));
  assert.ok(pyRoles.get('@dataclass')?.has('annotation'));
  assert.ok(pyRoles.get('process_batch')?.has('func-decl'));

  const shDoc = '#!/usr/bin/env bash\ndeploy_app() {\n  echo "$APP_ENV"\n}\n';
  const shRoles = roleMap(shDoc, classifySemanticTokens(shDoc, 'shell'));
  assert.ok(shRoles.get('deploy_app')?.has('func-decl'));
  assert.ok(shRoles.get('$APP_ENV')?.has('param'));

  const jsonDoc = '{\n  "service": "otto",\n  "port": 7700\n}';
  const jsonRoles = roleMap(jsonDoc, classifySemanticTokens(jsonDoc, 'json'));
  assert.ok(jsonRoles.get('"service"')?.has('field'));
  assert.equal(classifySemanticTokens('a,b,c\n1,2,3', 'csv').length, 0);
});

test('Java client-side import analyzer catches bogus packages, duplicates, and unused imports while honoring Lombok @Slf4j', () => {
  const javaWithIssues = [
    'package demo;',
    '',
    'import nonexistent.bogus.FakeClient;',
    'import java.util.List;',
    'import java.util.List;',
    'import java.util.Optional;',
    'import lombok.extern.slf4j.Slf4j;',
    '',
    '@Slf4j',
    'public class Demo {',
    '    public void run(List<String> names) {',
    '        log.info("names={}", names.size());',
    '    }',
    '}',
  ].join('\n');

  const { diagnostics, unresolvedIdents } = analyzeJavaDiagnostics(javaWithIssues);
  assert.ok(unresolvedIdents.has('FakeClient'));
  assert.ok(diagnostics.some((d) => d.severity === 'error' && d.line === 3 && d.quickFixes.length > 0));
  assert.ok(diagnostics.some((d) => d.message.toLowerCase().includes('duplicate') && d.line === 5));
  assert.ok(diagnostics.some((d) => d.message.toLowerCase().includes('unused') && d.line === 6));
  // Lombok @Slf4j IS used via `@Slf4j` + `log.info(...)`, so line 7 must NOT be flagged!
  assert.ok(!diagnostics.some((d) => d.line === 7));
});

test('Rainbow brackets cycle 4 nesting levels and flag unmatched closers; import fold skips error blocks', () => {
  const expr = 'fn({ a: [1, (2 + 3)] }) )';
  const brackets = computeRainbowBrackets(expr);
  const levels = brackets.map((b) => b.level);
  assert.ok(levels.includes(1));
  assert.ok(levels.includes(2));
  assert.ok(levels.includes(3));
  assert.ok(levels.includes(4));
  assert.equal(brackets[brackets.length - 1].level, 'unmatched');

  const goImports = 'package main\n\nimport (\n\t"fmt"\n\t"os"\n\t"strings"\n)\n\nfunc main() {}\n';
  const fold = computeImportFoldRange(goImports, 'go');
  assert.ok(fold && fold.startLine === 3 && fold.endLine === 7);
  // Error inside import block prevents hiding broken imports!
  assert.equal(computeImportFoldRange(goImports, 'go', new Set([4])), null);
});

test('Manifest dependency parser covers Go, Maven, Gradle, npm, Cargo, requirements.txt, and pyproject.toml', () => {
  assert.equal(isIdeManifestPath('/repo/go.mod'), true);
  assert.equal(isIdeManifestPath('/repo/requirements-dev.txt'), true);
  assert.equal(detectManifestEcosystem('/repo/build.gradle.kts'), 'gradle');

  const goMod = 'module demo\n\nrequire (\n\t// Deprecated: use google.golang.org/protobuf\n\tgithub.com/golang/protobuf v1.5.2\n\tgolang.org/x/sync v0.7.0 // indirect\n)\n';
  const goDeps = parseManifestDependencies('/repo/go.mod', goMod);
  assert.equal(goDeps.length, 2);
  assert.equal(goDeps[0].deprecated, true);
  assert.equal(goDeps[1].indirect, true);
  assert.equal(isVersionOutdated('v0.7.0', 'v0.8.0'), true);
  assert.ok(applyFallbackManifestVersionUpdate(goMod, goDeps[1], 'v0.8.0').includes('golang.org/x/sync v0.8.0'));

  const pomXml = '<project>\n<dependencies>\n<dependency>\n<groupId>org.springframework.boot</groupId>\n<artifactId>spring-boot-starter-web</artifactId>\n</dependency>\n</dependencies>\n</project>';
  const pomDeps = parseManifestDependencies('/ws/parent/service/pom.xml', pomXml);
  assert.equal(pomDeps.length, 1);
  assert.equal(pomDeps[0].name, 'org.springframework.boot:spring-boot-starter-web');
  assert.equal(pomDeps[0].managed_in?.path, '/ws/parent/pom.xml');

  assert.equal(parseManifestDependencies('/ws/build.gradle', 'dependencies {\n  implementation("io.netty:netty-all:4.1.100.Final")\n}').length, 1);
  assert.equal(parseManifestDependencies('/ws/package.json', '{\n  "dependencies": {\n    "svelte": "^5.0.0"\n  }\n}').length, 1);
  assert.equal(parseManifestDependencies('/ws/Cargo.toml', '[dependencies]\nserde = { version = "1.0.210", features = ["derive"] }\n').length, 1);
  assert.equal(parseManifestDependencies('/ws/requirements.txt', 'fastapi==0.115.0\nuvicorn>=0.30.0\n').length, 2);
  assert.equal(parseManifestDependencies('/ws/pyproject.toml', '[project]\ndependencies = [\n  "pydantic>=2.9.0",\n]\n').length, 1);
});

test('6 curated IDE color schemes resolve Auto against preferred dark & light schemes', () => {
  assert.equal(IDE_COLOR_SCHEME_OPTIONS.length, 6);
  assert.equal(
    resolveEffectiveIdeScheme(
      { colorScheme: 'auto', darkColorScheme: 'jetbrains-darcula', lightColorScheme: 'vscode-light-modern' },
      'dark',
    ),
    'jetbrains-darcula',
  );
  assert.equal(
    resolveEffectiveIdeScheme(
      { colorScheme: 'auto', darkColorScheme: 'jetbrains-darcula', lightColorScheme: 'vscode-light-modern' },
      'light',
    ),
    'vscode-light-modern',
  );
  assert.equal(isDarkIdeScheme('vscode-dark-modern'), true);
  assert.equal(isDarkIdeScheme('jetbrains-islands-light'), false);
});
