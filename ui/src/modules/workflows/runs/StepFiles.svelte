<script lang="ts">
  // The files a step wrote (ux.md §4.4): the run's context files named
  // `step{N}-…`, read in place; "All run files" widens to every file of the
  // run (folders open in place). A binary file (its read has no text) offers
  // Download.
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { authedBlobUrl } from '../../../lib/api/client';
  import { browseRunContext, readRunContext } from '../../../lib/api/workflows';
  import { downloadBlobUrl } from '../../../lib/downloadBlob';
  import { loadErrorText } from '../../../lib/loadError';
  import { toastError } from '../../../lib/toastError';
  import type { FsEntry, FsRead } from '../../../lib/api/types';

  interface Props {
    runId: string;
    contextDir: string | null | undefined;
    /** `step3-checkout-pr-branch`; null lists every top-level file. */
    stepFile: string | null;
  }
  let { runId, contextDir, stepFile }: Props = $props();

  let all = $state(false);
  /** The folder "All run files" shows ('' = the run's root). */
  let dir = $state('');
  let files = $state<FsEntry[] | null>(null);
  let error = $state<string | null>(null);
  let open = $state<{ path: string; read: FsRead | null; error: string | null } | null>(null);
  let reload = $state(0);

  // Another step's files: its open preview and folder go with it.
  $effect(() => {
    void [runId, stepFile];
    open = null;
    all = false;
    dir = '';
  });
  $effect(() => {
    const prefix = all ? null : stepFile;
    const path = all ? dir : '';
    void reload;
    files = null;
    error = null;
    const ac = new AbortController();
    browseRunContext(runId, path, ac.signal).then(
      (b) => (files = b.entries.filter((e) => (prefix ? !e.is_dir && e.name.startsWith(prefix) : !e.name.startsWith('.')))),
      (e) => {
        if (!ac.signal.aborted) error = loadErrorText(e);
      },
    );
    return () => ac.abort();
  });

  async function show(path: string): Promise<void> {
    open = { path, read: null, error: null };
    try {
      const read = await readRunContext(runId, path);
      if (open?.path === path) open.read = read;
    } catch (e) {
      if (open?.path === path) open.error = loadErrorText(e);
    }
  }

  async function download(path: string): Promise<void> {
    try {
      const url = await authedBlobUrl(`/workflow-runs/${encodeURIComponent(runId)}/context/read?path=${encodeURIComponent(path)}&download=1`);
      downloadBlobUrl(url, path.split('/').pop() ?? 'file');
    } catch (e) {
      toastError('Couldn’t download the file', e);
    }
  }
</script>

{#if all && dir}
  <button type="button" class="btn small back" onclick={() => (dir = dir.split('/').slice(0, -1).join('/'))}><Icon name="chevronLeft" size={12} />{dir.split('/').slice(0, -1).join('/') || 'All run files'}</button>
{/if}
<LoadState what="the files" loading={files === null && !error} {error} empty={!files?.length} onretry={() => reload++}>
  {#snippet emptyView()}<p class="none">{all ? 'This folder is empty.' : stepFile ? 'This step wrote no files.' : 'This run has no files.'}</p>{/snippet}
  <ul class="files">
    {#each files ?? [] as f (f.path)}
      <li>
        <button type="button" class="file" class:on={open?.path === f.path} onclick={() => (f.is_dir ? (dir = f.path) : show(f.path))}>
          <Icon name={f.is_dir ? 'folder' : 'file'} size={13} /><span class="ellipsis" dir="auto">{f.name}</span>
        </button>
      </li>
    {/each}
  </ul>
</LoadState>
{#if open}
  {#if open.error}
    {@const path = open.path}
    <LoadState what="the file" variant="compact" error={open.error} empty onretry={() => void show(path)} />
  {:else if open.read && open.read.content}
    <pre class="content" dir="auto">{open.read.content}</pre>
    {#if open.read.truncated}<p class="none">Showing the first 400 KB.</p>{/if}
  {:else if open.read}
    {@const path = open.path}
    <p class="none">This file has no text preview. <button type="button" class="btn small" onclick={() => download(path)}><Icon name="download" size={12} />Download</button></p>
  {/if}
{/if}
{#if stepFile && contextDir}
  <button type="button" class="btn small" aria-pressed={all} onclick={() => ((all = !all), (dir = ''))}>{all ? 'This step’s files' : 'All run files'}</button>
{/if}

<style>
  .files {
    margin: 0;
    margin-block-end: var(--sp-4);
    padding: 0;
    list-style: none;
  }
  .file {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    inline-size: 100%;
    min-block-size: 32px;
    padding-inline: var(--sp-4);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .file:hover {
    background: var(--hover);
  }
  .file.on {
    background: var(--accent-soft);
  }
  .content {
    margin: 0;
    margin-block-end: var(--sp-4);
    padding: var(--sp-5);
    max-block-size: 480px;
    overflow: auto;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .none {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    margin: 0;
    padding-block: var(--sp-4);
    font-size: var(--fs-m);
    color: var(--text-dim);
  }
  .back {
    margin-block-end: var(--sp-4);
  }
  @media (pointer: coarse) {
    .file {
      min-block-size: 44px;
    }
  }
</style>
