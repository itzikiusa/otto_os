// Opening a child thread from its card or its sidebar row: in place, or
// beside its parent in the split (1-up → 2-up → grid, the layout Threads
// already has). A phone has no split: the child opens in place and the
// header's "‹ <parent>" chip (ThreadParentChip.svelte) leads back.
import { router } from '../../../lib/router.svelte';
import { viewport } from '../../../lib/stores/viewport.svelte';
import { threads } from '../threadsStore.svelte';

export function openChildThread(id: string, beside = false): void {
  if (router.module !== 'threads') {
    router.go(`threads/${id}`);
    return;
  }
  threads.selectThread(id, { split: beside && !viewport.isPhone });
}
