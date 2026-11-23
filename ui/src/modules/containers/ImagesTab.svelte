<script lang="ts">
  // Images tab: list local Docker images, pull from a registry, inspect
  // digests/tags, launch a container from an image, delete an image, or prune
  // dangling/unused images. All destructive actions confirm with danger: true.
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { containersApi } from '../../lib/api/containers';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { copyWithToast } from '../../lib/clipboard';
  import { formatBytes } from '../../lib/metric-format';
  import { plural } from '../../lib/plural';
  import { rel } from '../../lib/stores/now.svelte';
  import type { ImageSummary } from '../../lib/api/types/containers';
  import { filterImages, imageDisplayName } from './containers-util';

  interface Props {
    images: readonly ImageSummary[];
    loading: boolean;
    error: string | null;
    pulling: boolean;
    onrefresh: () => void;
    onpull: () => void;
    onrunimage: (reference: string) => void;
  }

  let { images, loading, error, pulling, onrefresh, onpull, onrunimage }: Props = $props();

  let query = $state('');
  let onlyDangling = $state(false);
  let busyId = $state<string | null>(null);
  let pruning = $state(false);
  let inspected = $state<ImageSummary | null>(null);

  const visible = $derived(filterImages(images, query, onlyDangling));
  const totalBytes = $derived(images.reduce((sum, i) => sum + i.size_bytes, 0));

  async function removeImage(img: ImageSummary): Promise<void> {
    const { repo, tag } = imageDisplayName(img);
    const label = repo === '<untagged>' ? img.short_id : `${repo}:${tag}`;
    const ok = await confirmer.ask(
      img.in_use
        ? `“${label}” is referenced by a container. Force removing it may leave stopped containers unable to restart.`
        : `Remove image “${label}” (${formatBytes(img.size_bytes)}) from the local Docker engine?`,
      {
        title: 'Remove image',
        confirmLabel: img.in_use ? 'Force remove image' : 'Remove image',
        danger: true,
      },
    );
    if (!ok) return;
    busyId = img.id;
    try {
      await containersApi.removeImage(img.id, { force: img.in_use });
      toasts.success('Image removed', label);
      onrefresh();
    } catch (e) {
      toastError('Couldn’t remove image', e);
    } finally {
      busyId = null;
    }
  }

  async function pruneImages(): Promise<void> {
    const ok = await confirmer.ask(
      'Remove all dangling (untagged) images not used by any container? This frees local disk space.',
      {
        title: 'Prune dangling images',
        confirmLabel: 'Prune images',
        danger: true,
      },
    );
    if (!ok) return;
    pruning = true;
    try {
      const res = await containersApi.pruneImages({ danglingOnly: true });
      const count = res.deleted.length;
      toasts.success(
        count > 0 ? `Pruned ${plural(count, 'image')}` : 'No dangling images to prune',
        res.space_reclaimed_bytes > 0 ? `Reclaimed ${formatBytes(res.space_reclaimed_bytes)}` : undefined,
      );
      onrefresh();
    } catch (e) {
      toastError('Couldn’t prune images', e);
    } finally {
      pruning = false;
    }
  }
</script>

<div class="ct-res-view">
  <div class="ct-res-bar">
    <div class="ct-res-search">
      <Icon name="search" size={13} />
      <input
        class="ct-res-input"
        type="search"
        dir="auto"
        aria-label="Filter images"
        placeholder="Filter images by repository, tag, or ID…"
        bind:value={query}
      />
      {#if query}
        <button
          type="button"
          class="icon-btn"
          aria-label="Clear image filter"
          title="Clear image filter"
          onclick={() => (query = '')}
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>

    <div class="segmented" role="group" aria-label="Image filter">
      <button
        type="button"
        class:active={!onlyDangling}
        aria-pressed={!onlyDangling}
        onclick={() => (onlyDangling = false)}
      >
        All
      </button>
      <button
        type="button"
        class:active={onlyDangling}
        aria-pressed={onlyDangling}
        onclick={() => (onlyDangling = true)}
      >
        Dangling
      </button>
    </div>

    <div class="ct-res-actions">
      <button type="button" class="btn small" disabled={pulling} onclick={onpull}>
        <Icon name="download" size={12} />
        {pulling ? 'Pulling…' : 'Pull image…'}
      </button>
      <button type="button" class="btn small danger" disabled={pruning || images.length === 0} onclick={() => void pruneImages()}>
        <Icon name="trash" size={12} />
        {pruning ? 'Pruning…' : 'Prune…'}
      </button>
    </div>
  </div>

  <LoadState
    what="images"
    {loading}
    {error}
    empty={images.length === 0}
    onretry={onrefresh}
  >
    {#snippet emptyView()}
      <EmptyState
        icon="layers"
        title="No local images"
        body="Pull an image from a registry to inspect or run it here."
        actionLabel="Pull image…"
        actionIcon="download"
        actionKind="secondary"
        onaction={onpull}
      />
    {/snippet}

    {#if visible.length === 0}
      <EmptyState
        icon="search"
        title="No matching images"
        body="Clear the search filter to see all local images."
        actionLabel="Clear filter"
        actionKind="secondary"
        onaction={() => {
          query = '';
          onlyDangling = false;
        }}
      />
    {:else}
      <div class="ct-table-wrap">
        <table class="ct-table">
          <thead>
            <tr>
              <th>Repository</th>
              <th>Tag</th>
              <th>ID</th>
              <th>Status</th>
              <th>Created</th>
              <th class="ct-num">Size</th>
              <th class="ct-col-act"><span class="sr-only">Actions</span></th>
            </tr>
          </thead>
          <tbody>
            {#each visible as img (img.id)}
              {@const { repo, tag } = imageDisplayName(img)}
              {@const ref = repo === '<untagged>' ? img.short_id : `${repo}:${tag}`}
              <tr>
                <td class="ct-mono ct-strong" dir="ltr" title={repo}>{repo}</td>
                <td class="ct-mono" dir="ltr" title={tag}>{tag}</td>
                <td>
                  <button
                    type="button"
                    class="ct-id-copy"
                    dir="ltr"
                    title="Copy image ID"
                    onclick={() => void copyWithToast(img.id, 'image ID')}
                  >
                    {img.short_id}
                  </button>
                </td>
                <td>
                  {#if img.in_use}
                    <Badge tone="ok" label="In use" />
                  {:else if img.dangling}
                    <Badge tone="warn" label="Dangling" />
                  {:else}
                    <Badge tone="neutral" label="Unused" />
                  {/if}
                </td>
                <td class="ct-dim">{rel(img.created * 1000)}</td>
                <td class="ct-num">{formatBytes(img.size_bytes)}</td>
                <td class="ct-col-act">
                  <div class="ct-row-acts">
                    <button
                      type="button"
                      class="icon-btn"
                      aria-label={`Run ${ref}`}
                      title={`Run ${ref}`}
                      onclick={() => onrunimage(ref)}
                    >
                      <Icon name="play" size={12} />
                    </button>
                    <button
                      type="button"
                      class="icon-btn"
                      aria-label={`Inspect ${ref}`}
                      title={`Inspect ${ref}`}
                      onclick={() => (inspected = img)}
                    >
                      <Icon name="info" size={13} />
                    </button>
                    <button
                      type="button"
                      class="icon-btn ct-danger-icon"
                      disabled={busyId === img.id}
                      aria-label={`Remove ${ref}`}
                      title={`Remove ${ref}`}
                      onclick={() => void removeImage(img)}
                    >
                      <Icon name="trash" size={13} />
                    </button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="ct-res-foot">
        <span>{plural(visible.length, 'image')}</span>
        <span>·</span>
        <span>Total {formatBytes(totalBytes)}</span>
      </div>
    {/if}
  </LoadState>
</div>

{#if inspected}
  {@const img = inspected}
  <Modal title="Image details" width={540} onclose={() => (inspected = null)}>
    <dl class="ct-inspect-dl">
      <dt>Image ID</dt>
      <dd class="ct-mono" dir="ltr">{img.id}</dd>
      <dt>Size</dt>
      <dd>{formatBytes(img.size_bytes)}</dd>
      <dt>Created</dt>
      <dd>{rel(img.created * 1000)}</dd>
      <dt>Tags</dt>
      <dd class="ct-mono" dir="ltr">
        {#if img.repo_tags.length > 0}
          {img.repo_tags.join('\n')}
        {:else}
          —
        {/if}
      </dd>
      <dt>Digests</dt>
      <dd class="ct-mono" dir="ltr">
        {#if img.repo_digests.length > 0}
          {img.repo_digests.join('\n')}
        {:else}
          —
        {/if}
      </dd>
    </dl>
    {#snippet footer()}
      <button type="button" class="btn" onclick={() => void copyWithToast(img.id, 'image ID')}>
        <Icon name="copy" size={12} /> Copy ID
      </button>
      <button type="button" class="btn primary" onclick={() => (inspected = null)}>Close</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .ct-res-view {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-inline-size: 0;
  }
  .ct-res-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .ct-res-search {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1 1 220px;
    min-inline-size: 160px;
    max-inline-size: 360px;
    block-size: 28px;
    padding-inline: 8px 4px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .ct-res-search:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .ct-res-input {
    flex: 1;
    min-inline-size: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
  }
  .ct-res-input::placeholder {
    color: var(--text-dim);
  }
  .ct-res-actions {
    margin-inline-start: auto;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ct-table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .ct-table {
    inline-size: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .ct-table th {
    text-align: start;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    padding: 8px 10px;
    border-block-end: 1px solid var(--border);
    white-space: nowrap;
  }
  .ct-table td {
    padding: 6px 10px;
    border-block-end: 1px solid var(--border);
    vertical-align: middle;
    max-inline-size: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ct-table tbody tr:last-child td {
    border-block-end: 0;
  }
  .ct-table tbody tr:hover {
    background: var(--hover);
  }
  .ct-num {
    text-align: end;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .ct-col-act {
    inline-size: 88px;
    text-align: end;
  }
  .ct-row-acts {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .ct-id-copy {
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ct-id-copy:hover {
    color: var(--accent-text);
  }
  .ct-mono {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .ct-strong {
    font-weight: 500;
  }
  .ct-dim {
    color: var(--text-dim);
    white-space: nowrap;
  }
  .ct-danger-icon:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .ct-res-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .ct-inspect-dl {
    margin: 0;
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    gap: 8px 12px;
    font-size: var(--fs-s);
  }
  .ct-inspect-dl dt {
    color: var(--text-dim);
    font-weight: 500;
  }
  .ct-inspect-dl dd {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .sr-only {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
  }
</style>
