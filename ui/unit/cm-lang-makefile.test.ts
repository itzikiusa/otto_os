import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { StringStream } from '@codemirror/language';
import { makefileParser, type MakeState } from '../src/lib/components/cm-lang-makefile.ts';
import { cmLangNow, loadCmLang } from '../src/lib/components/cm-langs.ts';
import { isMakefilePath, langExtOf } from '../src/lib/components/cm-editor-helpers.ts';

// The IDE's Makefile language (owner, 2026-11-24: "makefile — i want it to be formatted
// as well"). Tokenizes a real-world Makefile modelled on his go_acceptance_tests one.

const SAMPLE = readFileSync(new URL('./fixtures/Makefile.sample', import.meta.url), 'utf8');

/** Every line's tokens as [text, style] (style null = plain). */
function tokenize(text: string): [string, string | null][][] {
  const st: MakeState = makefileParser.startState();
  return text.split('\n').map((line) => {
    if (!line.trim()) {
      makefileParser.blankLine(st);
      return [];
    }
    const s = new StringStream(line, 4, 2);
    const out: [string, string | null][] = [];
    while (!s.eol()) {
      const style = makefileParser.token(s, st);
      assert.ok(s.pos > s.start, `the tokenizer advances on: ${line}`);
      out.push([s.current(), style]);
      s.start = s.pos;
    }
    return out;
  });
}

const lines = tokenize(SAMPLE);
const src = SAMPLE.split('\n');
/** The style of `text` on the first line that starts with `prefix`. */
function styleOf(prefix: string, text: string): string | null {
  const i = src.findIndex((l) => l.startsWith(prefix));
  assert.ok(i >= 0, `no line starts with ${prefix}`);
  const tok = lines[i].find(([t]) => t === text);
  assert.ok(tok, `no token ${JSON.stringify(text)} on: ${src[i]}`);
  return tok[1];
}

test('Makefile names choose the Makefile language', () => {
  for (const p of ['/r/Makefile', '/r/makefile', '/r/GNUmakefile', '/r/Makefile.local', '/r/build/common.mk', '/r/x.make', '/r/x.mak']) {
    assert.ok(isMakefilePath(p), p);
    assert.equal(langExtOf(p), 'makefile', p);
  }
  for (const p of ['/r/main.go', '/r/README.md', '/r/makefile_test.go', '/r/Makefiles/notes.txt']) assert.equal(isMakefilePath(p), false, p);
});

test('the Makefile pack loads with its file type', async () => {
  assert.equal(cmLangNow('makefile', 'mysql'), null);
  await loadCmLang('makefile');
  assert.ok(cmLangNow('makefile', 'mysql'));
});

test('comments, assignments (every operator) and their definitions', () => {
  assert.equal(lines[0][0][1], 'comment');
  for (const [name, op] of [['APP_NAME', ':='], ['GIT_BRANCH', '?='], ['GO_BUILD_FLAGS', '='], ['TAGS', '+='], ['VERSION', '!=']]) {
    assert.equal(styleOf(name, name), 'propertyName.definition', name);
    assert.equal(styleOf(name, op), 'operator', `${name} ${op}`);
  }
  assert.equal(styleOf('export GOFLAGS', 'export'), 'keyword');
  assert.equal(styleOf('export GOFLAGS', 'GOFLAGS'), 'propertyName.definition');
  assert.equal(styleOf('override ENV', 'override'), 'keyword');
  assert.equal(styleOf('override ENV', 'ENV'), 'propertyName.definition');
  assert.equal(styleOf('.DEFAULT_GOAL', '.DEFAULT_GOAL'), 'meta');
  assert.equal(styleOf('all:', '## Lint, test and build'), 'comment');
});

test('references and functions: names apart from the function names', () => {
  assert.equal(styleOf('GIT_COMMIT ', '$('), 'escape');
  assert.equal(styleOf('GIT_COMMIT ', 'shell'), 'variableName.function');
  assert.equal(styleOf('GIT_COMMIT ', ')'), 'escape');
  assert.equal(styleOf('GO_BUILD_FLAGS', 'GIT_COMMIT'), 'variableName.special');
  assert.equal(styleOf('\t$(call banner', 'call'), 'variableName.function');
  assert.equal(styleOf('\t$(call banner', 'DEPLOY_TARGET'), 'variableName.special');
  assert.equal(styleOf('\t$(CC)', '$<'), 'variableName.special');
  assert.equal(styleOf('\t$(CC)', '$@'), 'variableName.special');
  assert.equal(styleOf('\trm -rf', 'wildcard'), 'variableName.function');
});

test('rules: targets, special targets and prerequisites', () => {
  assert.equal(styleOf('.PHONY', '.PHONY'), 'meta');
  assert.equal(styleOf('.PHONY', 'docker-build'), 'variableName.function');
  assert.equal(styleOf('docker-build:', 'docker-build'), 'variableName.function.definition');
  assert.equal(styleOf('docker-build:', 'build'), 'variableName.function');
  assert.equal(styleOf('%.o', '%.o'), 'variableName.function.definition');
});

test('directives', () => {
  for (const [line, word] of [['include', 'include'], ['-include', '-include'], ['ifeq', 'ifeq'], ['else ifneq', 'ifneq'], ['ifdef', 'ifdef'], ['endif', 'endif'], ['define', 'define'], ['endef', 'endef']]) {
    assert.equal(styleOf(line, word), 'keyword', line);
  }
  assert.equal(styleOf('ifeq', 'ENV'), 'variableName.special');
  assert.equal(styleOf('define', 'banner'), 'propertyName.definition');
  assert.equal(styleOf('ifdef', 'CI'), 'variableName.special');
});

test('recipes are shell with the Makefile references still coloured', () => {
  assert.equal(styleOf('\t@echo "Building', '@'), 'meta');
  assert.equal(styleOf('\t@echo "Building', 'echo'), 'builtin');
  assert.equal(styleOf('\t@echo "Building', 'APP_NAME'), 'variableName.special', 'a reference inside a "string"');
  assert.equal(styleOf('\tgo build', 'go'), 'variableName.function');
  assert.equal(styleOf('\tgo build', '-o'), 'attributeName');
  assert.equal(styleOf('\t-kubectl', '-'), 'meta');
  assert.equal(styleOf('\t@for f', '$$f'), 'variableName.special', "the shell's own $$VAR");
  assert.equal(styleOf('\t@grep', 'MAKEFILE_LIST'), 'variableName.special');
  assert.equal(styleOf('\t@grep', "'^[a-zA-Z_-]+:.*?## .*$$'"), 'string', "$$ inside '…' stays in the string");
});

test('a backslash continues the line it ends: recipe flags are not prefixes', () => {
  const i = src.findIndex((l) => l.includes('-count=1'));
  assert.deepEqual(lines[i].find(([t]) => t === '-count'), ['-count', 'attributeName']);
  const j = src.findIndex((l) => l.includes('--build-arg'));
  assert.equal(lines[j].find(([t]) => t === 'GIT_COMMIT')?.[1], 'variableName.special');
});

test('a TAB line before any rule is not a recipe; one after a rule is', () => {
  const t = tokenize('X := 1\n\tY := 2\nall:\n\techo hi\n');
  assert.equal(t[1].find(([s]) => s === 'Y')?.[1], 'propertyName.definition');
  assert.equal(t[3].find(([s]) => s === 'echo')?.[1], 'builtin');
});
