// accessibility-responsive-6-05 / 6-08 / 6-09 (P10): controls that unmount
// themselves move focus to their successor; viewport shortcuts need focus.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { componentFunctions } from './componentFunctions.ts';
import { isTypingTarget, viewportOwnsKey } from '../src/lib/design/scene3d/viewportLoop.ts';

const src = (p: string) => readFileSync(join(import.meta.dirname, '..', 'src', p), 'utf8');

// Behaviour (test-quality-6w1-06): run the components' real focus handlers
// against stub elements and assert WHICH element ends up focused.
const comp = (p: string) => new URL(`../src/${p}`, import.meta.url);
const el = (name: string, log: string[]) => ({ name, focus: (o?: unknown) => log.push(o ? `${name}+${JSON.stringify(o)}` : name) });

test('site Inspector go(): selects, then focuses the row just left, else the new view head', async () => {
  const log: string[] = [];
  const selected: unknown[] = [];
  const rows: Record<string, ReturnType<typeof el>> = { b1: el('row b1', log) };
  const head = el('view head', log);
  let mounted = false; // the new view renders only after tick()
  const rootEl = {
    querySelector: (sel: string) => {
      if (!mounted) return null;
      if (sel === '[data-view-head]') return head;
      return rows[/data-item="([^"]+)"/.exec(sel)?.[1] ?? ''] ?? null;
    },
  };
  const { go } = componentFunctions(comp('modules/design-hall/site/Inspector.svelte'), ['go'], {
    rootEl,
    onselect: (s: string | null, b: string | null) => selected.push([s, b]),
    tick: async () => { mounted = true; },
    CSS: { escape: (s: string) => s },
  });
  await go('s1', null, 'b1'); // "back" from block b1 to its section
  assert.deepEqual(JSON.parse(JSON.stringify(selected)), [['s1', null]]);
  assert.deepEqual(log, ['row b1'], 'lands on the row of the block just left');
  mounted = false;
  await go('s1', 'b2'); // drill in: no row to return to
  assert.deepEqual(log, ['row b1', 'view head'], 'lands on the new view’s name, never <body>');
  mounted = false;
  await go('s1', null, 'gone'); // the block just left was deleted
  assert.deepEqual(log.at(-1), 'view head');
});

test('site Inspector: go() is the one place selection changes (structure)', () => {
  const s = src('modules/design-hall/site/Inspector.svelte');
  assert.equal([...s.matchAll(/\bonselect\(/g)].length, 1, 'only go() calls onselect');
  assert.ok([...s.matchAll(/data-view-head/g)].length >= 3, 'block, section and page views each have a focus target');
  assert.doesNotMatch(s, /aria-label="Remove link"/);
});

test('link rows: removing one focuses the Remove now in its place, else the previous, else Add', async () => {
  const log: string[] = [];
  let removes: ReturnType<typeof el>[] = [];
  // `rows` is the parent-owned prop: onchange writes it back, as the parent does.
  const ctx: Record<string, any> = componentFunctions(comp('modules/design-hall/site/LinkListField.svelte'), ['remove'], {
    keys: [1, 2, 3],
    rows: [{ label: 'A', href: '#a' }, { label: 'B', href: '#b' }, { label: 'C', href: '#c' }],
    onchange: (next: unknown[]) => { ctx.rows = next; },
    tick: async () => { removes = ctx.rows.map((r: { label: string }) => el(`remove ${r.label}`, log)); },
    listEl: {
      querySelectorAll: (sel: string) => (sel === '[data-remove-row]' ? removes : []),
      querySelector: (sel: string) => (sel === '[data-add-row]' ? el('add', log) : null),
    },
  });
  await ctx.remove(1); // B: C slides into its place
  assert.deepEqual(ctx.rows.map((r: { label: string }) => r.label), ['A', 'C']);
  assert.deepEqual([...ctx.keys], [1, 3], 'the removed row’s key goes with it');
  assert.deepEqual(log, ['remove C']);
  await ctx.remove(1); // the last row: focus the previous row's Remove
  assert.deepEqual(log, ['remove C', 'remove A']);
  await ctx.remove(0); // no rows left: Add
  assert.deepEqual(log, ['remove C', 'remove A', 'add']);
});

test('School breadcrumbs: a crumb focuses the stage BEFORE the view (and the crumb) changes', async () => {
  const log: string[] = [];
  const stage = el('stage', log);
  const handle = { toCorridor: async () => { log.push('toCorridor'); } };
  const ctx = componentFunctions(comp('modules/home/boxes/ClassroomsBox.svelte'), ['upTo'], { stage, handle });
  ctx.upTo((h: typeof handle) => h.toCorridor())();
  assert.deepEqual(log, ['stage+{"preventScroll":true}', 'toCorridor']);
  // No scene yet: focus still moves off the crumb, nothing else runs.
  const empty = componentFunctions(comp('modules/home/boxes/ClassroomsBox.svelte'), ['upTo'], { stage, handle: null });
  empty.upTo(() => { throw new Error('must not navigate without a scene'); })();
  assert.equal(log.at(-1), 'stage+{"preventScroll":true}');
});

test('3D viewport: typing in a field inside it is never a shortcut', () => {
  const field = { tagName: 'INPUT' } as unknown as EventTarget;
  const host = { contains: () => true };
  assert.equal(isTypingTarget(field), true);
  assert.equal(viewportOwnsKey(host, field), false);
  assert.equal(viewportOwnsKey(host, { tagName: 'CANVAS' } as unknown as EventTarget), true);
});
