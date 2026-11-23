import test from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { nextStoryAfterDelete } from '../src/modules/product/storySelection.ts';
import { PanZoom } from '../src/modules/canvas/pointerZoom.ts';
import { parseDesignRoute } from '../src/modules/design-hall/model.ts';
import type { ProductStory } from '../src/modules/product/types.ts';

function fakeStory(id: string, parent_story_id: string | null = null): ProductStory {
  return {
    id,
    workspace_id: 'ws-1',
    parent_story_id,
    title: id,
    source_key: id.toUpperCase(),
  } as unknown as ProductStory;
}

test('nextStoryAfterDelete selects the first re-parented child when deleting an epic', () => {
  const epic = fakeStory('epic-1');
  const child1 = fakeStory('child-1', 'epic-1');
  const child2 = fakeStory('child-2', 'epic-1');
  const other = fakeStory('story-2');

  const next = nextStoryAfterDelete([epic, child1, child2, other], 'epic-1');
  assert.equal(next?.id, 'child-1');

  // Even when the epic only had children and no siblings, selection lands on its first child.
  const onlyChildren = nextStoryAfterDelete([epic, child1], 'epic-1');
  assert.equal(onlyChildren?.id, 'child-1');

  // Deleting the last item selects the row above it; deleting the only item returns null.
  assert.equal(nextStoryAfterDelete([epic, other], 'story-2')?.id, 'epic-1');
  assert.equal(nextStoryAfterDelete([epic], 'epic-1'), null);
});

test('LinkedCanvases.load discards stale out-of-order and post-unmount responses', async () => {
  const pending = new Map<string, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  const ctx = componentFunctions(
    new URL('../src/modules/product/LinkedCanvases.svelte', import.meta.url),
    ['load'],
    {
      storyId: 'story-a',
      scenes: [] as { id: string }[],
      loading: false,
      loadError: null as string | null,
      loadSeq: 0,
      alive: true,
      api: {
        get: (url: string) =>
          new Promise((resolve, reject) => {
            pending.set(url, { resolve, reject });
          }),
      },
      loadErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)),
    },
  );

  const pA = ctx.load('story-a');
  ctx.storyId = 'story-b';
  const pB = ctx.load('story-b');

  // Story B resolves first, then stale Story A resolves later.
  pending.get('/product/stories/story-b/linked-canvases')!.resolve([{ id: 'canvas-b' }]);
  await pB;
  assert.deepEqual(JSON.parse(JSON.stringify(ctx.scenes)), [{ id: 'canvas-b' }]);
  assert.equal(ctx.loading, false);

  pending.get('/product/stories/story-a/linked-canvases')!.resolve([{ id: 'canvas-a' }]);
  await pA;
  assert.deepEqual(JSON.parse(JSON.stringify(ctx.scenes)), [{ id: 'canvas-b' }]);

  // Post-unmount failure must not overwrite scenes or loadError.
  const pC = ctx.load('story-b');
  ctx.alive = false;
  pending.get('/product/stories/story-b/linked-canvases')!.reject(new Error('late failure'));
  await pC;
  assert.equal(ctx.loadError, null);
});

test('PanZoom.attach wires focusable surface, non-passive wheel, arrow pan and full teardown', () => {
  const listeners = new Map<string, { fn: (e: unknown) => void; opts?: unknown }>();
  const el = {
    tabIndex: -1,
    addEventListener(type: string, fn: (e: unknown) => void, opts?: unknown) {
      listeners.set(type, { fn, opts });
    },
    removeEventListener(type: string, fn: (e: unknown) => void) {
      if (listeners.get(type)?.fn === fn) listeners.delete(type);
    },
  } as unknown as HTMLElement;

  let view = { scale: 1, tx: 0, ty: 0 };
  let adjusted = false;
  let dragging = false;
  const pz = new PanZoom({
    surface: () => el,
    get: () => view,
    set: (t) => { view = t; },
    adjusted: () => adjusted,
    setAdjusted: (on) => { adjusted = on; },
    onDragging: (on) => { dragging = on; },
  });

  const detach = pz.attach(el);
  assert.equal(el.tabIndex, 0);
  assert.deepEqual(listeners.get('wheel')?.opts, { passive: false });

  let prevented = false;
  listeners.get('keydown')!.fn({
    key: 'ArrowRight',
    shiftKey: false,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    preventDefault: () => { prevented = true; },
  });
  assert.equal(prevented, true);
  assert.equal(adjusted, true);
  assert.ok(view.tx < 0);

  detach();
  assert.equal(listeners.size, 0);
  assert.equal(dragging, false);
});

test('parseDesignRoute strips query params so query-only changes keep the same artifact id', () => {
  const plain = parseDesignRoute(['design', 'a', 'art-42']);
  const fromStory = parseDesignRoute(['design', 'a', 'art-42?from=story', 'story-9']);
  assert.equal(plain.view, 'artifact');
  assert.equal(fromStory.view, 'artifact');
  if (plain.view === 'artifact' && fromStory.view === 'artifact') {
    assert.equal(plain.id, 'art-42');
    assert.equal(fromStory.id, 'art-42');
  }
});
