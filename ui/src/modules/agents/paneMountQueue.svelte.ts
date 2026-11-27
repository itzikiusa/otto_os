// Svelte 5 reactive binding for PaneMountQueue (shared by TiledView and Splits).
import { onDestroy } from 'svelte';
import { PaneMountQueue } from './paneMountQueue';

export function createPaneMountState(
  getWorkspaceId: () => string | null,
  getWanted: () => readonly string[],
  getPriority: () => string | null,
): { readonly mounted: ReadonlySet<string> } {
  let admitted = $state.raw<ReadonlySet<string>>(new Set());
  const queue = new PaneMountQueue((next) => {
    admitted = next;
  });
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
    const onVis = (): void => {
      if (document.visibilityState === 'visible') queue.wake();
    };
    document.addEventListener('visibilitychange', onVis);
    return () => document.removeEventListener('visibilitychange', onVis);
  });
  onDestroy(() => queue.destroy());
  const mounted = $derived.by(() => {
    const wanted = new Set(getWanted());
    const out = new Set<string>();
    for (const id of admitted) if (wanted.has(id)) out.add(id);
    const pri = getPriority();
    if (pri && wanted.has(pri)) out.add(pri);
    return out;
  });
  return {
    get mounted() {
      return mounted;
    },
  };
}
