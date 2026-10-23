import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parse } from 'svelte/compiler';
import { loadSource } from './sourceHarness.ts';

// architecture-1-09: the collapsed-row ref popover moved out of GraphView into
// git/graph/refPopoverState.svelte.ts (controller + viewport clamp) and
// git/graph/RefPopover.svelte (markup). These pin what it did inline: it never
// leaves the window, Escape and the backdrop close it, a row double-click
// checks the ref out and a right-click opens the ref's full menu.

const graph = (name: string) => new URL(`../src/modules/git/graph/${name}`, import.meta.url);
const refMenus = loadSource(new URL('../src/modules/git/ref-menus.ts', import.meta.url), {});
const mod = loadSource(graph('refPopoverState.svelte.ts'), { '../ref-menus': refMenus });

const commit = { sha: 'c1', refs: [], parents: [], subject: 'Tip', short_sha: 'c1', author: 'a', date: '' };
const refs = {
  local: [{ name: 'main', sha: 'c1', is_current: true, remote: false }, { name: 'topic', sha: 'c1', remote: false }],
  remote: [{ name: 'origin/main', sha: 'c1', remote: true }],
  tags: [{ name: 'v1', sha: 'c1' }],
};

function setup(selectedSha: string | null = null) {
  const log: string[] = [];
  const host = {
    refs,
    selectedSha,
    worktreeByBranch: new Map([['wt-branch', { path: '/wt', branch: 'wt-branch' }]]),
    selectCommit: (c: { sha: string }, o: { deferDiff?: boolean }) => log.push(`select ${c.sha} defer=${o.deferDiff}`),
    holdDiff: () => log.push('holdDiff'),
    checkout: (name: string, create: boolean) => { log.push(`checkout ${name} create=${create}`); },
    checkoutRemoteRef: (label: string) => { log.push(`checkoutRemote ${label}`); },
    openWorktree: (w: { path: string }) => { log.push(`openWorktree ${w.path}`); },
    branchMenu: (_e: unknown, b: { name: string; remote?: boolean }) => log.push(`branchMenu ${b.remote ? 'remote' : 'local'}:${b.name}`),
    tagMenu: (_e: unknown, t: { name: string }) => log.push(`tagMenu ${t.name}`),
  };
  return { pop: mod.createRefPopover(host), log };
}

function mouse() {
  const calls: string[] = [];
  const trigger = { focus: () => calls.push('focus'), getBoundingClientRect: () => ({ left: 40, bottom: 120 }) };
  return {
    calls,
    e: { currentTarget: trigger, preventDefault: () => calls.push('preventDefault'), stopPropagation: () => calls.push('stopPropagation') },
  };
}

test('a popover taller than the window is capped and clamped inside it, never flipped off-screen', () => {
  const view = { w: 1024, h: 768 };
  // Anchored near the bottom-right corner: the cap is a third of the window
  // (not the 60 px left under the anchor), and the measured popover is moved
  // up and left so every row stays reachable.
  const maxH = mod.popoverMaxHeight(view.h, 700);
  assert.equal(maxH, 256);
  assert.deepEqual({ ...mod.clampPopover({ x: 900, y: 700 }, { w: 340, h: maxH }, view) }, { x: 676, y: 504 });
  // A list taller than the whole window (before the cap applies) pins to the
  // top-left gap instead of a negative top.
  assert.deepEqual({ ...mod.clampPopover({ x: 900, y: 700 }, { w: 340, h: 1200 }, view) }, { x: 676, y: 8 });
  // Room to spare: the anchor stands, and the cap is the room under it (+160).
  assert.equal(mod.popoverMaxHeight(view.h, 100), 752);
  assert.deepEqual({ ...mod.clampPopover({ x: 40, y: 124 }, { w: 220, h: 200 }, view) }, { x: 40, y: 124 });
});

test('the expander opens the popover at its own rect and focuses it; close clears it', () => {
  const { pop } = setup();
  const m = mouse();
  const chips = [
    { kind: 'head', label: 'main', current: true },
    { kind: 'local', label: 'topic', current: false },
    { kind: 'tag', label: 'v1', current: false },
  ];
  pop.open(m.e, commit, chips);
  assert.deepEqual(m.calls, ['stopPropagation', 'preventDefault', 'focus']);
  assert.equal(pop.menu.x, 40);
  assert.equal(pop.menu.y, 124);
  assert.deepEqual(pop.menu.branches.map((c: { label: string }) => c.label), ['main', 'topic']);
  assert.deepEqual(pop.menu.tags.map((c: { label: string }) => c.label), ['v1']);
  pop.close();
  assert.equal(pop.menu, null);
});

test('Escape inside the popover dialog runs its close (dialogFocus)', () => {
  let onKey: ((e: unknown) => void) | null = null;
  const node = { querySelectorAll: () => [], contains: () => false, getClientRects: () => [1] };
  const globals = {
    document: { addEventListener() {}, querySelectorAll: () => [node], activeElement: null, querySelector: () => null },
    window: { addEventListener: (_t: string, fn: (e: unknown) => void) => { onKey = fn; }, removeEventListener() {} },
    requestAnimationFrame: () => 1,
    cancelAnimationFrame() {},
    HTMLElement: class {},
    Element: class {},
  };
  const { dialogFocus } = loadSource(new URL('../src/lib/dialogFocus.ts', import.meta.url), {}, globals);
  const { pop } = setup();
  pop.open(mouse().e, commit, [{ kind: 'local', label: 'topic', current: false }, { kind: 'tag', label: 'v1', current: false }]);
  dialogFocus(node, pop.close);
  const stops: string[] = [];
  onKey!({ key: 'Escape', defaultPrevented: false, preventDefault: () => stops.push('pd'), stopImmediatePropagation: () => stops.push('sip') });
  assert.equal(pop.menu, null);
  assert.deepEqual(stops, ['pd', 'sip']);
});

test('the backdrop button and the dialog both close through the controller (structure)', () => {
  const [file] = ['RefPopover.svelte'];
  const ast = parse(readFileSync(graph(file), 'utf8'), { modern: true });
  const elements: any[] = ast.fragment.nodes.filter((n: { type: string }) => n.type === 'RegularElement');
  const attr = (el: any, name: string) => el.attributes.find((a: any) => a.name === name);
  const backdrop = elements.find((el: any) => attr(el, 'class')?.value[0]?.data === 'ref-pop-backdrop');
  assert.equal(backdrop.name, 'button');
  assert.equal(attr(backdrop, 'onclick').value.expression.property.name, 'close');
  const dialog = elements.find((el: any) => attr(el, 'role')?.value[0]?.data === 'dialog');
  assert.equal(attr(dialog, 'aria-modal').value[0].data, 'true');
  const focus = dialog.attributes.find((a: any) => a.type === 'UseDirective');
  assert.equal(focus.name, 'dialogFocus');
  assert.equal(focus.expression.property.name, 'close');
  // The clamped corner and the height cap are applied inline.
  const style = attr(dialog, 'style').value.map((v: any) => v.data ?? `{${v.expression.name}}`).join('');
  assert.equal(style, 'left: {x}px; top: {y}px; max-height: {maxH}px;');
});

test('a row click selects the commit (deferred diff) and is a no-op when it is already selected', () => {
  const a = setup();
  a.pop.rowSelect(commit);
  assert.deepEqual(a.log, ['select c1 defer=true']);
  const b = setup('c1');
  b.pop.rowSelect(commit);
  assert.deepEqual(b.log, []);
});

test('a row double-click checks the ref out, closes the popover and holds the diff', () => {
  const { pop, log } = setup();
  pop.open(mouse().e, commit, [{ kind: 'local', label: 'topic', current: false }]);
  pop.rowCheckout({ kind: 'local', label: 'topic', current: false });
  assert.equal(pop.menu, null);
  pop.rowCheckout({ kind: 'remote', label: 'origin/main', current: false });
  pop.rowCheckout({ kind: 'tag', label: 'v1', current: false });
  pop.rowCheckout({ kind: 'local', label: 'wt-branch', current: false, worktree: true });
  // A worktree flag whose tree is gone falls back to a plain checkout.
  pop.rowCheckout({ kind: 'local', label: 'gone', current: false, worktree: true });
  assert.deepEqual(log, [
    'holdDiff', 'checkout topic create=false',
    'holdDiff', 'checkoutRemote origin/main',
    'holdDiff', 'checkout v1 create=false',
    'holdDiff', 'openWorktree /wt',
    'holdDiff', 'checkout gone create=false',
  ]);
});

test('a row right-click opens that ref’s own menu, keeping local and remote twins apart', () => {
  const { pop, log } = setup();
  const m = mouse();
  pop.rowMenu(m.e, { kind: 'head', label: 'main', current: true });
  pop.rowMenu(m.e, { kind: 'remote', label: 'origin/main', current: false });
  pop.rowMenu(m.e, { kind: 'tag', label: 'v1', current: false });
  pop.rowMenu(m.e, { kind: 'local', label: 'deleted-meanwhile', current: false });
  assert.deepEqual(log, ['branchMenu local:main', 'branchMenu remote:origin/main', 'tagMenu v1']);
  assert.equal(m.calls.filter((c) => c === 'preventDefault').length, 4);
  assert.equal(m.calls.filter((c) => c === 'stopPropagation').length, 4);
});
