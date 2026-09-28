// PageHeader fit gate (perf SF-12): unchanged inputs skip the pass, collapsed
// actions keep their measured width, and same-value style writes are dropped.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { FitGate, StyleWriter, WidthCache, nodeSig, type SigNode } from '../src/lib/components/headerFit.ts';

function node(text: string, cls = 'btn', attrs: Record<string, string> = {}, kids = 0): SigNode {
  return { className: cls, textContent: text, childElementCount: kids, getAttribute: (n) => attrs[n] ?? null };
}

test('100 content-neutral mutations after a pass run no further pass', () => {
  const gate = new FitGate();
  const root = {};
  const wrap = {};
  gate.observe([{ target: root, borderBoxSize: [{ inlineSize: 900 }], contentRect: { width: 900 } }]);
  gate.observe([{ target: wrap, contentRect: { width: 420.2 } }]);
  const content = () => ['Git', nodeSig(node('Fetch')), nodeSig(node('Push'))].join('|');
  assert.equal(gate.shouldRun(gate.signature([root, wrap], content())), true, 'first pass runs');
  for (let i = 0; i < 100; i++) gate.shouldRun(gate.signature([root, wrap], content()));
  assert.equal(gate.runs, 1);
  assert.equal(gate.skips, 100);
});

test('a size change, a relabel or a restyle re-runs the pass', () => {
  const gate = new FitGate();
  const root = {};
  gate.observe([{ target: root, contentRect: { width: 900 } }]);
  assert.ok(gate.shouldRun(gate.signature([root], nodeSig(node('Fetch')))));
  gate.observe([{ target: root, contentRect: { width: 880 } }]);
  assert.ok(gate.shouldRun(gate.signature([root], nodeSig(node('Fetch')))), 'resize');
  assert.ok(gate.shouldRun(gate.signature([root], nodeSig(node('Fetch 3')))), 'text');
  assert.ok(gate.shouldRun(gate.signature([root], nodeSig(node('Fetch 3', 'btn', { style: 'display:none' })))), 'inline style');
  assert.ok(gate.shouldRun(gate.signature([root], nodeSig(node('Fetch 3', 'btn primary', { style: 'display:none' })))), 'class');
  // sub-pixel RO jitter under half a px is the same size
  gate.observe([{ target: root, contentRect: { width: 880.1 } }]);
  assert.equal(gate.shouldRun(gate.signature([root], nodeSig(node('Fetch 3', 'btn primary', { style: 'display:none' })))), false);
  gate.invalidate();
  assert.ok(gate.shouldRun(gate.signature([root], nodeSig(node('Fetch 3', 'btn primary', { style: 'display:none' })))), 'invalidate forces');
});

test('width cache hits only while the element signature is unchanged', () => {
  const c = new WidthCache<object>();
  const el = {};
  c.set(el, 'a', 64);
  assert.equal(c.get(el, 'a'), 64);
  assert.equal(c.get(el, 'b'), undefined);
  assert.equal(c.get({}, 'a'), undefined);
});

test('style writer drops same-value writes', () => {
  const w = new StyleWriter();
  const calls: string[] = [];
  const style = { setProperty: (n: string, v: string) => calls.push(`${n}=${v}`) };
  const owner = {};
  assert.equal(w.set(owner, style, '--ph-title-max', '400px'), true);
  assert.equal(w.set(owner, style, '--ph-title-max', '400px'), false);
  assert.equal(w.set(owner, style, '--ph-title-max', '410px'), true);
  assert.equal(w.set({}, style, '--ph-title-max', '410px'), true, 'per owner');
  assert.deepEqual(calls, ['--ph-title-max=400px', '--ph-title-max=410px', '--ph-title-max=410px']);
  assert.equal(w.writes, 3);
});
