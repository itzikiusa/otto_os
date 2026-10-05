// Cluster workspace single-key shortcuts (a11y P3): scoped to focus inside the
// workspace (or nowhere), and the letters can be switched off.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SINGLE_KEYS, inWorkspace } from '../src/modules/kubernetes/workspaceKeys.ts';

class FakeNode {
  parent: FakeNode | null;
  constructor(parent: FakeNode | null = null) {
    this.parent = parent;
  }
}
(globalThis as { Node?: unknown }).Node ??= FakeNode;
const root = Object.assign(new FakeNode(), { contains(n: FakeNode) { for (let c: FakeNode | null = n; c; c = c.parent) if (c === root) return true; return false; } });
const doc = { body: new FakeNode(), documentElement: new FakeNode() } as unknown as Document;

test('keys count only inside the workspace, or with focus on nothing', () => {
  const NodeCtor = (globalThis as unknown as { Node: typeof FakeNode }).Node;
  const inside = new NodeCtor(root as unknown as FakeNode);
  const elsewhere = new NodeCtor(null);
  const r = root as unknown as Element;
  assert.equal(inWorkspace(inside as unknown as EventTarget, r, doc), true);
  assert.equal(inWorkspace((doc as unknown as { body: EventTarget }).body, r, doc), true, 'body focus (nothing focused) still works');
  assert.equal(inWorkspace(elsewhere as unknown as EventTarget, r, doc), false, 'sidebar / another pane');
  assert.equal(inWorkspace(inside as unknown as EventTarget, undefined, doc), false, 'unmounted workspace');
});

test('the opt-out covers the letters but not Enter / Escape', () => {
  for (const k of ['l', 's', 'd', 'y', 'j', 'k', 'r', 'n', '/', '?']) assert.ok(SINGLE_KEYS.has(k), k);
  assert.ok(!SINGLE_KEYS.has('Enter'));
  assert.ok(!SINGLE_KEYS.has('Escape'));
});
