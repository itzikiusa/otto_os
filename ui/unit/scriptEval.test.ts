import test from 'node:test';
import assert from 'node:assert/strict';
import { evalScript } from '../src/lib/api/scriptEval.ts';

test('evalScript supports expressions, templates, destructuring, loops, and closures without eval/Function', () => {
  const out: unknown[] = [];
  const log = (...args: unknown[]) => out.push(args.join(' '));
  evalScript(`
    const data = { token: 'abc-123', items: [1, 2, 3] };
    const { token, items } = data;
    const sum = items.reduce((acc, n) => acc + n, 0);
    let label = '';
    for (const n of items) {
      if (n === 2) continue;
      label += \`[\${n}]\`;
    }
    function greet(who = 'world') {
      return \`hi \${who}: \${token} (\${sum}) \${label}\`;
    }
    log(greet('otto'));
    log(data?.missing?.deep ?? 'fallback');
    log(/^abc-\\d+$/.test(token));
  `, { log });

  assert.deepEqual(out, [
    'hi otto: abc-123 (6) [1][3]',
    'fallback',
    'true',
  ]);
});

test('evalScript blocks prototype/constructor escape and omits Function/eval/fetch globals', () => {
  const seen: unknown[] = [];
  evalScript(`
    seen.push(({}).constructor);
    seen.push(({}).__proto__);
    seen.push(typeof Function);
    seen.push(typeof eval);
    seen.push(typeof fetch);
    seen.push(typeof globalThis);
  `, { seen });

  assert.deepEqual(seen, [undefined, undefined, 'undefined', 'undefined', 'undefined', 'undefined']);

  assert.throws(
    () => evalScript(`const o = {}; o.__proto__ = { polluted: 1 };`, {}),
    /Cannot set property '__proto__'/,
  );
  assert.throws(
    () => evalScript(`const o = {}; o['constructor'] = 1;`, {}),
    /Cannot set property 'constructor'/,
  );
});

test('evalScript stops infinite loops with step budget error', () => {
  assert.throws(
    () => evalScript(`while (true) {}`, {}, 500),
    /step budget/i,
  );
});

test('evalScript supports try/catch/finally, switch, and object/array spread', () => {
  const out: string[] = [];
  evalScript(`
    const base = { a: 1 };
    const merged = { ...base, b: 2 };
    const arr = [...[10, 20], 30];
    try {
      throw new Error('boom');
    } catch (e) {
      out.push('caught:' + e.message);
    } finally {
      out.push('finally:' + merged.a + ',' + merged.b + ',' + arr.length);
    }
    switch (merged.b) {
      case 1: out.push('one'); break;
      case 2: out.push('two'); break;
      default: out.push('other');
    }
  `, { out });

  assert.deepEqual(out, ['caught:boom', 'finally:1,2,3', 'two']);
});
