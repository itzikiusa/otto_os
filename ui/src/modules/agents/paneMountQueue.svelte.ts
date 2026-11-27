// Svelte 5 reactive bindings for PaneMountQueue (shared by TiledView and Splits).
import { untrack } from 'svelte';
import { PaneMountQueue, selectLiveTiles, type PaneMountQueueOpts } from './paneMountQueue';

export function createPaneMountState(
  getWorkspaceId: () => string | null,
  getWanted: () => readonly string[],
  getPriority: () => string | null,
  queueOpts?: PaneMountQueueOpts,
): { readonly admitted: ReadonlySet<string>; readonly mounted: ReadonlySet<string> } {
  let admitted = $state.raw<ReadonlySet<string>>(new Set());
  const queue = new PaneMountQueue((next) => {
    admitted = next;
  }, queueOpts);
  let lastWs: string | null | undefined = undefined;
  $effect(() => {
    const wid = getWorkspaceId();
    if (lastWs !== undefined && lastWs !== wid) queue.reset();
    lastWs = wid;
  });
  $effect(() => {
    queue.sync(getWanted(), getPriority());
  });
  $effect(() => {
    if (typeof document === 'undefined') return () => queue.destroy();
    const onVis = (): void => {
      if (document.visibilityState === 'visible') queue.wake();
    };
    document.addEventListener('visibilitychange', onVis);
    return () => {
      document.removeEventListener('visibilitychange', onVis);
      queue.destroy();
    };
  });
  const mounted = $derived.by(() => {
    const wanted = new Set(getWanted());
    const out = new Set<string>();
    for (const id of admitted) if (wanted.has(id)) out.add(id);
    const pri = getPriority();
    if (pri && wanted.has(pri)) out.add(pri);
    return out;
  });
  return {
    get admitted() {
      return admitted;
    },
    get mounted() {
      return mounted;
    },
  };
}

/** One-directional TiledView mount graph: selection reads the queue's
 *  independent `admitted` state (never the derived `mounted` set) so cold
 *  evaluation is strictly acyclic (BUG-18) while already-mounted visible tiles
 *  stay mounted across exit until scrolled off or evicted (OBS-1). */
export function createTiledMountState<T extends { id: string }>(opts: {
  getWorkspaceId: () => string | null;
  getOrdered: () => readonly T[];
  getActiveId: () => string | null;
  getPinned: () => ReadonlySet<string>;
  getVisible: () => ReadonlySet<string>;
  isThread: (item: T) => boolean;
  isInactive: (item: T) => boolean;
  maxLive: number;
  queueOpts?: PaneMountQueueOpts;
}): {
  readonly liveOrder: readonly string[];
  readonly liveIds: ReadonlySet<string>;
  readonly mountedIds: ReadonlySet<string>;
} {
  const mounts = createPaneMountState(
    opts.getWorkspaceId,
    () => liveOrder,
    () => priorityTileId,
    opts.queueOpts,
  );
  const liveOrder: string[] = $derived(
    selectLiveTiles(opts.getOrdered(), {
      activeId: opts.getActiveId(),
      pinned: opts.getPinned(),
      visible: opts.getVisible(),
      retained: untrack(() => mounts.admitted),
      isThread: opts.isThread,
      isInactive: opts.isInactive,
      maxLive: opts.maxLive,
    }),
  );
  const liveIds = $derived(new Set(liveOrder));
  const priorityTileId: string | null = $derived(
    opts.getActiveId() && liveIds.has(opts.getActiveId()!) ? opts.getActiveId() : (liveOrder[0] ?? null),
  );
  return {
    get liveOrder() {
      return liveOrder;
    },
    get liveIds() {
      return liveIds;
    },
    get mountedIds() {
      return mounts.mounted;
    },
  };
}
