<script lang="ts">
  // The S3 browser's sticky transfer bars: the upload in progress and the
  // download in progress (with Cancel). Split out of S3Browser.svelte; the
  // bars are plain text, and each transfer is announced once when it starts
  // (lib/announce — a status inserted with its text is silent in VoiceOver),
  // never per progress tick.
  import { announce } from '../../lib/announce.svelte';
  import { fmtBytes } from './util';

  interface Props {
    uploading: { done: number; total: number; name: string } | null;
    download: { key: string; received: number; total: number | null } | null;
    oncanceldownload: () => void;
  }
  let { uploading, download, oncanceldownload }: Props = $props();

  function leaf(key: string): string {
    const trimmed = key.endsWith('/') ? key.slice(0, -1) : key;
    return trimmed.slice(trimmed.lastIndexOf('/') + 1) || key;
  }
  // Keyed by the file / object, not the progress object, so a tick is silent.
  const uploadingName = $derived(uploading?.name ?? '');
  const downloadKey = $derived(download?.key ?? '');
  $effect(() => { if (uploadingName) announce(`Uploading ${uploadingName}`); });
  $effect(() => { if (downloadKey) announce(`Downloading ${leaf(downloadKey)}`); });
</script>

{#if uploading}
  <div class="dl-bar">
    <span class="mono">Uploading {uploading.name}</span>
    <progress max={uploading.total} value={uploading.done}></progress>
    <span class="dim">{uploading.done + 1} / {uploading.total}</span>
  </div>
{/if}

{#if download}
  <div class="dl-bar">
    <span class="mono">{leaf(download.key)}</span>
    <progress max={download.total ?? undefined} value={download.total ? download.received : undefined}></progress>
    <span class="dim">{fmtBytes(download.received)}{download.total ? ` / ${fmtBytes(download.total)}` : ''}</span>
    <button class="btn small" onclick={oncanceldownload}>Cancel</button>
  </div>
{/if}

<style>
  .dl-bar {
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    border-top: 1px solid var(--border);
    background: var(--surface);
    font-size: var(--fs-s);
  }
  .dl-bar progress {
    flex: 1;
    height: 6px;
  }
</style>
