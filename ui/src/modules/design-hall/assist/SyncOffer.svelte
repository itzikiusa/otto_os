<script lang="ts" module>
  import type { DesignVariantRun as Run } from '../../../lib/api/types';
  /** After "Edit here", a newer source arrives as a side run named `sync-…`. */
  export function isSyncRun(run: Pick<Run, 'run_id'>): boolean {
    return run.run_id.startsWith('sync-');
  }
</script>

<script lang="ts">
  // A newer version of the source an adopted design came from (a Product
  // story attachment or a Canvas board). The daemon keeps it aside as a
  // one-version `sync-…` run — it is NOT an agent variant, so it reads as
  // "Newer version from …" with Accept (fast-forward main to it, like any
  // variant) or Keep mine (hide it here; nothing changes on the design).
  import Icon from '../../../lib/components/Icon.svelte';
  import { ApiError } from '../../../lib/api/client';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { lsGet, lsSet } from '../../../lib/storage';
  import type { DesignArtifact, DesignVariantRun, DesignVersion } from '../../../lib/api/types';
  import type { CompareSide } from '../CompareModal.svelte';
  import { acceptVariant } from './api';

  interface Props {
    artifact: DesignArtifact;
    run: DesignVariantRun;
    head: DesignVersion | null;
    readonly: boolean;
    dirty: boolean;
    oncompare: (left: CompareSide, right: CompareSide) => void;
  }
  let { artifact, run, head, readonly, dirty, oncompare }: Props = $props();

  const version = $derived(run.versions[0] ?? null);
  const from = $derived(artifact.source_kind === 'canvas_scene' ? 'Canvas' : 'Product');
  const keyOf = (v: string) => `otto.design.sync-kept.${v}`;
  let kept = $state(false);
  $effect(() => {
    kept = !!version && lsGet(keyOf(version.id)) === '1';
  });
  let busy = $state(false);
  const accepted = $derived(!!run.accepted_version_id);

  async function accept(): Promise<void> {
    if (!version || busy) return;
    if (dirty) {
      const ok = await confirmer.ask(`Accept the newer version from ${from}? It becomes the current version; your unsaved edits stay in the editor.`, {
        title: 'Accept newer version', confirmLabel: 'Accept', danger: false,
      });
      if (!ok) return;
    }
    busy = true;
    try {
      let res;
      try {
        res = await acceptVariant(artifact.id, version.id);
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 409 && /moved/i.test(e.message))) throw e;
        const ok = await confirmer.ask(`This design changed after the newer ${from} version arrived. Accept it on top? The current version stays in history — nothing is lost.`, {
          title: 'Accept newer version', confirmLabel: 'Accept on top', danger: false,
        });
        if (!ok) return;
        res = await acceptVariant(artifact.id, version.id, true);
      }
      toasts.success(`Accepted the ${from} version as v${res.version.seq}`);
    } catch (e) {
      toastError('Couldn’t accept the newer version', e);
    } finally {
      busy = false;
    }
  }

  function keepMine(): void {
    if (!version) return;
    lsSet(keyOf(version.id), '1');
    kept = true;
  }
</script>

{#if version && !kept}
  <div class="sync" role="group" aria-label={`Newer version from ${from}`} data-testid="design-sync-offer">
    <div class="head">
      <Icon name="refresh" size={14} />
      <strong>Newer version from {from}</strong>
      <span class="dim">{new Date(version.created_at).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })}</span>
    </div>
    {#if accepted}
      <p class="dim"><Icon name="check" size={12} /> Accepted.</p>
    {:else}
      <p class="dim">The original changed after you took this design over. Your version stays current until you accept this one.</p>
      <div class="acts">
        <button class="btn small primary" onclick={() => void accept()} disabled={readonly || busy} data-testid="design-sync-accept">{busy ? 'Accepting…' : 'Accept'}</button>
        <button class="btn small" onclick={keepMine} disabled={busy} data-testid="design-sync-keep">Keep mine</button>
        {#if head}
          <button class="btn small ghost" onclick={() => oncompare({ artifact, versionId: head.id }, { artifact, versionId: version.id })}>
            <Icon name="columns" size={12} /> Compare
          </button>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .sync {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-inline-start: 3px solid var(--info);
    border-radius: var(--radius-m);
    background: var(--bg);
    font-size: var(--fs-s);
  }
  .head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }
  .head > :global(svg) {
    color: var(--info);
  }
  .dim {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
</style>
