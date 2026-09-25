// Agent UI control: docs/contracts/ui-commands.json (the ONE catalog the daemon
// also loads) ↔ the UI's registered handlers, plus the pure frame helpers.
//
// The parity half is static — it parses every `registerUiCommands('<module>',
// { name: handler, … })` call under src/ with the TypeScript AST instead of
// importing the handler modules (they pull in Svelte stores). It fails when:
//   • a catalog command has no handler, or more than one;
//   • a handler has no catalog entry (the daemon would never route to it);
//   • a handler is registered under another module than the catalog's;
//   • a uiCommands/<module>.ts file registers handlers but index.ts doesn't
//     import it (it would never load, so `hello.capabilities` would lack it).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import {
  absoluteDeadline,
  commandLabel,
  encodeResult,
  nearestDelta,
  normalizeCommand,
  providerName,
  readServerFrame,
  RESULT_MAX_BYTES,
} from '../src/lib/uiCommands/frames.ts';

const UI = fileURLToPath(new URL('..', import.meta.url));
const SRC = join(UI, 'src');
const CMD_DIR = join(SRC, 'lib', 'uiCommands');
const CATALOG = join(UI, '..', 'docs', 'contracts', 'ui-commands.json');

interface Spec {
  name: string;
  module: string;
  risk: string;
  headless?: string;
  input_schema: { type?: string; additionalProperties?: unknown };
}
const catalog = JSON.parse(readFileSync(CATALOG, 'utf8')) as { version: number; commands: Spec[] };

interface Reg {
  name: string;
  module: string;
  file: string;
}

function walk(dir: string, out: string[] = []): string[] {
  for (const f of readdirSync(dir)) {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (/\.(ts|svelte)$/.test(f)) out.push(p);
  }
  return out;
}

/** The TS source of a file (a .svelte file's <script> blocks). */
function sourceOf(file: string): string {
  const text = readFileSync(file, 'utf8');
  if (!file.endsWith('.svelte')) return text;
  return [...text.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script\s*>/gi)].map((m) => m[1]).join('\n');
}

function propName(p: ts.ObjectLiteralElementLike, sf: ts.SourceFile): string | null {
  if (ts.isSpreadAssignment(p)) return null;
  const n = p.name;
  if (!n) return null;
  if (ts.isIdentifier(n) || ts.isStringLiteral(n) || ts.isNumericLiteral(n)) return n.text;
  throw new Error(`computed handler name ${n.getText(sf)} — use a literal key`);
}

/** Every registerUiCommands(...) call in a file: its module and handler names. */
function registrations(file: string): Reg[] {
  const src = sourceOf(file);
  if (!src.includes('registerUiCommands(')) return [];
  const sf = ts.createSourceFile(file, src, ts.ScriptTarget.ES2022, true);
  // Object literals bound to a const in this file (`const handlers = {…}`).
  const consts = new Map<string, ts.ObjectLiteralExpression>();
  const out: Reg[] = [];
  const visitConsts = (n: ts.Node): void => {
    if (ts.isVariableDeclaration(n) && ts.isIdentifier(n.name) && n.initializer) {
      let init: ts.Expression = n.initializer;
      while (ts.isAsExpression(init) || ts.isSatisfiesExpression(init) || ts.isParenthesizedExpression(init)) init = init.expression;
      if (ts.isObjectLiteralExpression(init)) consts.set(n.name.text, init);
    }
    ts.forEachChild(n, visitConsts);
  };
  visitConsts(sf);
  const visit = (n: ts.Node): void => {
    if (ts.isCallExpression(n) && ts.isIdentifier(n.expression) && n.expression.text === 'registerUiCommands') {
      const [modArg, mapArg] = n.arguments;
      assert.ok(modArg && ts.isStringLiteralLike(modArg), `${relative(UI, file)}: module must be a string literal`);
      let obj: ts.Expression | undefined = mapArg;
      while (obj && (ts.isAsExpression(obj) || ts.isSatisfiesExpression(obj) || ts.isParenthesizedExpression(obj))) obj = obj.expression;
      if (obj && ts.isIdentifier(obj)) obj = consts.get(obj.text);
      assert.ok(obj && ts.isObjectLiteralExpression(obj), `${relative(UI, file)}: handlers must be an object literal (or a const bound to one)`);
      for (const p of obj.properties) {
        const name = propName(p, sf);
        if (name) out.push({ name, module: (modArg as ts.StringLiteralLike).text, file });
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return out;
}

const regs = walk(SRC).flatMap(registrations);

test('catalog: well-formed, unique names, known risks, headless only on reads', () => {
  assert.equal(catalog.version, 1);
  assert.ok(catalog.commands.length > 0);
  const seen = new Set<string>();
  for (const c of catalog.commands) {
    assert.match(c.name, /^[a-z][a-z0-9_]*$/, `bad name ${c.name}`);
    assert.ok(!seen.has(c.name), `duplicate catalog name ${c.name}`);
    seen.add(c.name);
    assert.ok(['read', 'navigate', 'local_write', 'outward'].includes(c.risk), `${c.name}: risk ${c.risk}`);
    if (c.headless !== undefined) assert.equal(c.risk, 'read', `${c.name}: headless on a ${c.risk} command`);
    assert.equal(c.input_schema.type, 'object', `${c.name}: input_schema.type`);
    assert.equal(c.input_schema.additionalProperties, false, `${c.name}: additionalProperties must be false`);
  }
});

test('every catalog command has exactly one handler, under its module', () => {
  const problems: string[] = [];
  for (const c of catalog.commands) {
    const hits = regs.filter((r) => r.name === c.name);
    if (hits.length === 0) problems.push(`${c.name}: no handler registered (module ${c.module})`);
    else if (hits.length > 1) problems.push(`${c.name}: registered ${hits.length}× (${hits.map((h) => relative(UI, h.file)).join(', ')})`);
    else if (hits[0].module !== c.module) problems.push(`${c.name}: registered under "${hits[0].module}", catalog says "${c.module}"`);
  }
  assert.deepEqual(problems, []);
});

test('every registered handler is in the catalog', () => {
  const names = new Set(catalog.commands.map((c) => c.name));
  const extra = regs.filter((r) => !names.has(r.name)).map((r) => `${r.name} (${relative(UI, r.file)})`);
  assert.deepEqual(extra, []);
});

test('uiCommands/index.ts imports every handler module', () => {
  const index = readFileSync(join(CMD_DIR, 'index.ts'), 'utf8');
  const imported = new Set([...index.matchAll(/import\s+'\.\/([\w-]+)(?:\.ts)?';/g)].map((m) => m[1]));
  const files = [...new Set(regs.filter((r) => r.file.startsWith(CMD_DIR)).map((r) => basename(r.file, '.ts')))];
  assert.deepEqual(files.filter((f) => !imported.has(f)), []);
  // …and every side-effect import names a real file.
  for (const m of imported) assert.ok(readdirSync(CMD_DIR).includes(`${m}.ts`), `index.ts imports missing ./${m}`);
});

test('frames: commands are normalised and validated', () => {
  assert.equal(normalizeCommand('otto.ui_db_run_query'), 'db_run_query');
  assert.equal(normalizeCommand('otto_ui_state'), 'state');
  assert.equal(normalizeCommand('ui_open'), 'open');
  assert.equal(normalizeCommand('db_page'), 'db_page');

  assert.deepEqual(readServerFrame({ type: 'hello_ack', conn_id: 'c1' }), { type: 'hello_ack', conn_id: 'c1' });
  assert.equal(readServerFrame({ type: 'hello_ack' }), null);
  const f = readServerFrame({
    type: 'ui_command', id: 'k1', session_id: 's1', command: 'otto.ui_db_run_query',
    agent: { session_id: 's1', title: 'Fix billing', provider: 'claude' }, args: { tab_id: 't' }, deadline_ms: 45_000,
  });
  assert.deepEqual(f, {
    type: 'ui_command', id: 'k1', session_id: 's1', command: 'db_run_query',
    agent: { session_id: 's1', title: 'Fix billing', provider: 'claude' }, args: { tab_id: 't' }, deadline_ms: 45_000,
  });
  // Missing agent / args / deadline degrade to safe defaults; arrays aren't args.
  const g = readServerFrame({ type: 'ui_command', id: 'k2', session_id: 's2', command: 'state', args: [1] });
  assert.ok(g && g.type === 'ui_command');
  assert.deepEqual(g.args, {});
  assert.equal(g.agent.session_id, 's2');
  assert.equal(g.deadline_ms, 30_000);
  assert.equal(readServerFrame({ type: 'ui_command', id: 'k3', command: 'state' }), null);
  assert.deepEqual(readServerFrame({ type: 'ui_command_cancel', id: 'k1', reason: 'Stopped' }), {
    type: 'ui_command_cancel', id: 'k1', reason: 'Stopped',
  });
  // Ordinary broadcast events are NOT UI frames.
  assert.equal(readServerFrame({ type: 'session_status', session_id: 's', status: 'idle' }), null);
  assert.equal(readServerFrame(null), null);
  assert.equal(readServerFrame('hello_ack'), null);
});

test('frames: deadlines, labels, provider names, result cap', () => {
  assert.equal(absoluteDeadline(30_000, 1_000), 31_000);
  assert.equal(absoluteDeadline(10, 1_000), 2_000); // floor 1 s
  assert.equal(absoluteDeadline(999_999, 0), 120_000); // cap at the human-wait max
  assert.equal(absoluteDeadline(1.9e12, 5), 1.9e12); // already an epoch

  assert.equal(commandLabel('db_run_query'), 'Run query');
  assert.equal(commandLabel('otto.ui_open'), 'Open a module');
  assert.equal(commandLabel('k8s_select'), 'Select');
  assert.equal(commandLabel('state'), 'Look at the window');
  assert.equal(commandLabel('wf_list_runs'), 'List runs');

  assert.equal(providerName('claude'), 'Claude');
  assert.equal(providerName('agy'), 'Antigravity');
  assert.equal(providerName(''), 'Agent');
  assert.equal(providerName('grok'), 'Grok');

  assert.equal(encodeResult(undefined), '{"ok":true,"result":null}');
  assert.equal(encodeResult({ a: 1 }), '{"ok":true,"result":{"a":1}}');
  const cyc: Record<string, unknown> = {};
  cyc.self = cyc;
  assert.equal(encodeResult(cyc), null);
  assert.equal(encodeResult('x'.repeat(RESULT_MAX_BYTES)), null);
});

test('reveal: scroll by the nearest edge, and leave a tall, partly visible target alone', () => {
  // viewport [0, 100]
  assert.equal(nearestDelta(10, 50, 0, 100), 0); // in view
  assert.equal(nearestDelta(90, 130, 0, 100), 38); // cut at the bottom → up by 30 + pad
  assert.equal(nearestDelta(-20, 30, 0, 100), -28); // cut at the top → down
  assert.equal(nearestDelta(60, 400, 0, 100), 0); // taller than the box, partly shown → stay
  assert.equal(nearestDelta(300, 700, 0, 100), 292); // taller and hidden below → show its start
  assert.equal(nearestDelta(-700, -300, 0, 100), -708); // taller and hidden above → its start
});
