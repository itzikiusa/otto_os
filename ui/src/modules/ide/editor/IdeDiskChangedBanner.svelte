<script lang="ts">
  // One line above the code when the file you are editing changed on disk under unsaved
  // edits: Compare (disk vs your buffer), Reload from disk (confirmed: it discards edits),
  // Keep mine (dismiss; the next save overwrites the disk copy knowingly). It also runs the
  // dirty-tab disk watcher (see store/ideDiskChange.svelte.ts) while the editor area is up.
  import { onDestroy } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { git } from '../../../lib/stores/git.svelte';
  import { ide, type IdeOpenTab } from '../ideStore.svelte';
  import { subscribeIdeDiskConflict, subscribeIdeFileSaved } from '../store/ideBufferPerf';
  import { ideDiskChange, type ConflictSet } from '../store/ideDiskChange.svelte';

  let { tab }: { tab: IdeOpenTab } = $props();

  // Autosave already waits on the buffer controller's conflict set; the banner joins it.
  // (The store keeps the controller private and is at its size cap, hence the cast.)
  const conflicts = (): ConflictSet | null =>
    (ide as unknown as { bufPerf?: { conflictedPaths?: Set<string> } }).bufPerf?.conflictedPaths ?? null;

  const stop = ideDiskChange.start(ide, conflicts);
  const unsubSaved = subscribeIdeFileSaved((p) => ideDiskChange.forget(p, conflicts()));
  const unsubConflict = subscribeIdeDiskConflict(() => void ideDiskChange.check(ide, conflicts()));
  onDestroy(() => { stop(); unsubSaved(); unsubConflict(); });

  // A repo watcher bump (an agent or the CLI wrote files) re-checks dirty tabs soon after.
  let revTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    void JSON.stringify(git.liveRev);
    if (revTimer !== null) clearTimeout(revTimer);
    revTimer = setTimeout(() => { revTimer = null; void ideDiskChange.check(ide, conflicts()); }, 400);
    return () => { if (revTimer !== null) clearTimeout(revTimer); };
  });

  const notice = $derived(ideDiskChange.notices[tab.path] ?? null);
  const rel = $derived(ide.relativePath(tab.path));

  function compare(): void {
    if (!notice) return;
    window.dispatchEvent(new CustomEvent('otto:ide-open-compare', {
      detail: { mode: 'clipboard', path: tab.path, clipboardText: notice.diskContent, leftLabel: 'On disk' },
    }));
  }

  async function reload(): Promise<void> {
    const ok = await confirmer.ask(`Discard your unsaved changes to ${rel} and load the version on disk?`, {
      title: 'Reload from disk', confirmLabel: 'Reload', danger: true,
    });
    if (!ok) return;
    await ide.openFile(tab.path, { reload: true });
    ideDiskChange.forget(tab.path, conflicts());
  }
</script>

{#if notice}
  <div class="ide-disk-banner" use:announceShown={`${rel} changed on disk`} data-testid="ide-disk-changed-banner">
    <Icon name="warning" size={12} />
    <span class="msg"><span class="mono path" dir="ltr">{rel}</span> changed on disk.</span>
    <span class="grow"></span>
    <button type="button" class="btn small ghost" onclick={compare} data-testid="ide-disk-compare">Compare</button>
    <button type="button" class="btn small ghost" onclick={() => void reload()} data-testid="ide-disk-reload">Reload from disk</button>
    <button type="button" class="btn small" onclick={() => ideDiskChange.keepMine(tab, conflicts())} data-testid="ide-disk-keep">Keep mine</button>
  </div>
{/if}

<style>
  .ide-disk-banner {
    display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; flex-shrink: 0;
    min-block-size: 30px; padding-block: 2px; padding-inline: var(--sp-4) var(--sp-3);
    background: var(--warning-soft); border-block-end: 1px solid var(--border);
    color: var(--text); font-size: var(--fs-xs);
  }
  .ide-disk-banner :global(svg) { color: var(--warning); flex-shrink: 0; }
  .msg { min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .path { font-weight: 600; }
  .grow { flex: 1; }
</style>
