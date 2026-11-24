import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { findMakefileDefinition, makefileOutline, makefileSymbols, makefileWordAt } from '../src/lib/components/makefileModel.ts';

// Makefile Structure / Go to Definition / hover, read from the text (no language server).

const SAMPLE = readFileSync(new URL('./fixtures/Makefile.sample', import.meta.url), 'utf8');
const src = SAMPLE.split('\n');
const lineOf = (prefix: string): number => src.findIndex((l) => l.startsWith(prefix)) + 1;

test('the outline: targets (recipes skipped, special targets left out), variables, includes', () => {
  const o = makefileOutline(SAMPLE);
  assert.deepEqual(o.targets.map((t) => t.name), ['all', 'build', 'test', 'lint', 'docker-build', 'deploy', '%.o', 'clean', 'help']);
  const vars = o.variables.map((v) => v.name);
  for (const v of ['SHELL', 'APP_NAME', 'GIT_COMMIT', 'GIT_BRANCH', 'GO_BUILD_FLAGS', 'TAGS', 'VERSION', 'GOFLAGS', 'CGO_ENABLED', 'ENV', '.DEFAULT_GOAL', 'DEPLOY_TARGET', 'TEST_FLAGS', 'banner']) {
    assert.ok(vars.includes(v), v);
  }
  assert.ok(!vars.includes('COMMIT'), 'a recipe line is not an assignment');
  assert.deepEqual(o.includes, ['common.mk', 'local.mk']);
  const test_ = o.targets.find((t) => t.name === 'test')!;
  assert.equal(test_.line, lineOf('test:'));
  assert.equal(test_.endLine, lineOf('\t\t-timeout'), 'a target spans its recipe, continuation lines included');
});

test('Structure rows: targets first, then variables, one row per name', () => {
  const rows = makefileSymbols(SAMPLE);
  const firstVar = rows.findIndex((r) => r.kind === 'variable');
  assert.ok(rows.slice(0, firstVar).every((r) => r.kind === 'target'));
  assert.ok(rows.slice(firstVar).every((r) => r.kind === 'variable'));
  assert.equal(rows.filter((r) => r.name === 'DEPLOY_TARGET').length, 1);
  assert.equal(rows.find((r) => r.name === 'DEPLOY_TARGET')!.line, lineOf('DEPLOY_TARGET := prod'));
});

test('the word at a column: inside $(…), a prerequisite, a target with dashes', () => {
  const l = '\tgo build $(GO_BUILD_FLAGS) -o bin/$(APP_NAME) ./cmd/...';
  assert.equal(makefileWordAt(l, l.indexOf('GO_BUILD') + 3), 'GO_BUILD_FLAGS');
  assert.equal(makefileWordAt(l, l.indexOf('GO_BUILD_FLAGS') + 'GO_BUILD_FLAGS'.length + 1), 'GO_BUILD_FLAGS', 'just after the word');
  assert.equal(makefileWordAt('deploy: docker-build', 12), 'docker-build');
  assert.equal(makefileWordAt('a   b', 3), null);
});

test('definitions: a variable at its assignment, a target at its rule, export / override names', () => {
  assert.deepEqual(pick(findMakefileDefinition(SAMPLE, 'GIT_COMMIT')), { line: lineOf('GIT_COMMIT'), column: 1, kind: 'variable' });
  assert.deepEqual(pick(findMakefileDefinition(SAMPLE, 'docker-build')), { line: lineOf('docker-build:'), column: 1, kind: 'target' });
  assert.deepEqual(pick(findMakefileDefinition(SAMPLE, 'GOFLAGS')), { line: lineOf('export GOFLAGS'), column: 8, kind: 'variable' });
  assert.deepEqual(pick(findMakefileDefinition(SAMPLE, 'ENV')), { line: lineOf('override ENV'), column: 10, kind: 'variable' });
  assert.deepEqual(pick(findMakefileDefinition(SAMPLE, 'banner')), { line: lineOf('define banner'), column: 8, kind: 'variable' });
  assert.equal(findMakefileDefinition(SAMPLE, 'GIT_COMMIT')!.text, 'GIT_COMMIT     := $(shell git rev-parse --short HEAD)');
  assert.equal(findMakefileDefinition(SAMPLE, 'LINT_VERSION'), null, 'defined in the included file only');
});

function pick(d: { line: number; column: number; kind: string } | null) {
  return d && { line: d.line, column: d.column, kind: d.kind };
}
