<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  // S3 (read-only): bucket list → object browser with breadcrumb prefixes
  // (folder rows first), a preview drawer (text / pretty JSON / CSV table) and
  // a streamed Download. The current bucket + prefix live in the route
  // (`#/aws/<id>/s3/<bucket>?prefix=<encoded>`) so it's deep-linkable.
  import { untrack } from 'svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { awsApi, awsDownloadBlob, isLoginRequired, saveBlob } from '../../lib/api/aws';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import JsonTree from '../database/JsonTree.svelte';
  import ViewToolbar from './ViewToolbar.svelte';
  import { fmtAgo, fmtBytes, fmtDate, splitBucketSegment } from './util';
  import type { AwsAccount, S3Object, S3PreviewResp } from '../../lib/api/types';

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  }
  let { account, onsignin }: Props = $props();

  // Route-driven bucket + prefix.
  const routeSeg = $derived(router.parts[3]);
  const bucket = $derived(splitBucketSegment(routeSeg)[0]);
  const prefix = $derived(splitBucketSegment(routeSeg)[1]);
  $effect(() => { if (bucket) void resourceAccess.load('aws_account', account.id, `bucket:${bucket}`); });
  const canRead = $derived(resourceAccess.can('aws_account', account.id, 's3_read', 'aws_s3', 'view', `bucket:${bucket}`));

  function goTo(b: string, p: string): void {
    const seg = p ? `${encodeURIComponent(b)}?prefix=${encodeURIComponent(p)}` : encodeURIComponent(b);
    router.go(`aws/${account.id}/s3/${seg}`);
  }

  // ── buckets ──
  const buckets = $derived(aws.s3Buckets[account.id] ?? null);
  let bucketsLoading = $state(false);
  let bucketsError = $state('');
  let bucketFilter = $state('');
  let auto = $state(false);
  const bucketsShown = $derived.by(() => {
    const q = bucketFilter.trim().toLowerCase();
    const list = buckets ?? [];
    return q ? list.filter((b) => b.name.toLowerCase().includes(q)) : list;
  });

  async function loadBuckets(): Promise<void> {
    bucketsLoading = true;
    try {
      await aws.loadS3Buckets(account.id);
      bucketsError = '';
    } catch (e) {
      bucketsError = e instanceof Error ? e.message : String(e);
    } finally {
      bucketsLoading = false;
    }
  }

  // ── objects ──
  let prefixes = $state<string[]>([]);
  let objects = $state<S3Object[]>([]);
  let nextToken = $state<string | null>(null);
  let objLoading = $state(false);
  let objError = $state('');
  let objFilter = $state('');
  const rowsShown = $derived.by(() => {
    const q = objFilter.trim().toLowerCase();
    const folders = prefixes.map((p) => ({ kind: 'folder' as const, name: leaf(p), key: p }));
    const files = objects
      .filter((o) => o.key !== prefix) // the "directory marker" object itself
      .map((o) => ({ kind: 'file' as const, name: leaf(o.key), key: o.key, obj: o }));
    const all = [...folders, ...files];
    return q ? all.filter((r) => r.name.toLowerCase().includes(q)) : all;
  });

  function leaf(key: string): string {
    const trimmed = key.endsWith('/') ? key.slice(0, -1) : key;
    return trimmed.slice(trimmed.lastIndexOf('/') + 1) || key;
  }

  async function loadObjects(more = false): Promise<void> {
    if (!bucket) return;
    objLoading = true;
    try {
      const r = await awsApi.s3Objects(account.id, bucket, prefix, more ? nextToken : undefined);
      prefixes = more ? [...prefixes, ...r.prefixes] : r.prefixes;
      objects = more ? [...objects, ...r.objects] : r.objects;
      nextToken = r.is_truncated ? (r.next_token ?? null) : null;
      objError = '';
    } catch (e) {
      objError = e instanceof Error ? e.message : String(e);
    } finally {
      objLoading = false;
    }
  }

  // Load on mount + whenever the route bucket/prefix changes. Reads
  // bucket/prefix (deps); the loaders' own writes are untracked.
  $effect(() => {
    const b = bucket;
    void prefix;
    untrack(() => {
      if (!buckets && !bucketsLoading) void loadBuckets();
      if (b) {
        preview = null;
        void loadObjects();
      }
    });
  });

  const crumbs = $derived.by(() => {
    const parts = prefix.split('/').filter(Boolean);
    return parts.map((p, i) => ({ label: p, prefix: parts.slice(0, i + 1).join('/') + '/' }));
  });

  // ── preview drawer ──
  let preview = $state<{ obj: S3Object; data: S3PreviewResp | null; loading: boolean; error: string } | null>(null);
  const previewKind = $derived.by<'json' | 'csv' | 'text' | 'binary' | null>(() => {
    const d = preview?.data;
    if (!d) return null;
    if (d.binary) return 'binary';
    const ct = (d.content_type ?? '').toLowerCase();
    const key = preview?.obj.key.toLowerCase() ?? '';
    if (ct.includes('json') || key.endsWith('.json') || key.endsWith('.ndjson')) return 'json';
    if (ct.includes('csv') || key.endsWith('.csv') || key.endsWith('.tsv')) return 'csv';
    return 'text';
  });
  const previewJson = $derived.by<unknown>(() => {
    if (previewKind !== 'json' || !preview?.data?.text) return undefined;
    try {
      return JSON.parse(preview.data.text);
    } catch {
      return undefined;
    }
  });
  const previewCsv = $derived.by<string[][]>(() => {
    if (previewKind !== 'csv' || !preview?.data?.text) return [];
    const sep = preview.obj.key.toLowerCase().endsWith('.tsv') ? '\t' : ',';
    return preview.data.text
      .split(/\r?\n/)
      .filter((l) => l.length)
      .slice(0, 200)
      .map((l) => l.split(sep));
  });

  async function openPreview(o: S3Object): Promise<void> {
    if (!canRead) return;
    preview = { obj: o, data: null, loading: true, error: '' };
    try {
      const d = await awsApi.s3Preview(account.id, bucket, o.key);
      if (preview?.obj.key === o.key) preview = { obj: o, data: d, loading: false, error: '' };
    } catch (e) {
      if (preview?.obj.key === o.key)
        preview = { obj: o, data: null, loading: false, error: e instanceof Error ? e.message : String(e) };
    }
  }

  // ── download ──
  let dl = $state<{ key: string; received: number; total: number | null; ctrl: AbortController } | null>(null);
  async function download(o: S3Object): Promise<void> {
    if (!canRead) return;
    if (dl) {
      toasts.warn('A download is already running', leaf(dl.key));
      return;
    }
    const ctrl = new AbortController();
    dl = { key: o.key, received: 0, total: o.size || null, ctrl };
    try {
      const { blob, filename } = await awsDownloadBlob(
        awsApi.s3DownloadPath(account.id, bucket, o.key),
        (received, total) => {
          if (dl) dl = { ...dl, received, total: total ?? dl.total };
        },
        ctrl.signal,
      );
      saveBlob(blob, filename ?? leaf(o.key));
      toasts.success('Downloaded', leaf(o.key));
    } catch (e) {
      if (!(e instanceof DOMException && e.name === 'AbortError'))
        toasts.error('Download failed', e instanceof Error ? e.message : String(e));
    } finally {
      dl = null;
    }
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch (e) {
      toasts.error('Copy failed', e instanceof Error ? e.message : String(e));
    }
  }

  function rowMenu(e: MouseEvent | KeyboardEvent, r: (typeof rowsShown)[number]): void {
    if (r.kind === 'folder') {
      ctxMenu.show(e, [
        { label: 'Open', icon: 'folder', action: () => goTo(bucket, r.key) },
        { label: 'Copy S3 URI', icon: 'copy', action: () => void copy(`s3://${bucket}/${r.key}`, 'S3 URI') },
      ]);
      return;
    }
    ctxMenu.show(e, [
      { label: 'Preview', disabled: !canRead, icon: 'eye', action: () => void openPreview(r.obj) },
      { label: 'Download', disabled: !canRead, icon: 'arrowDown', action: () => void download(r.obj) },
      { separator: true },
      { label: 'Copy key', icon: 'copy', action: () => void copy(r.key, 'key') },
      { label: 'Copy S3 URI', icon: 'copy', action: () => void copy(`s3://${bucket}/${r.key}`, 'S3 URI') },
    ]);
  }

  const loginNeeded = $derived(
    isLoginRequired(new Error(bucketsError)) || isLoginRequired(new Error(objError)),
  );
</script>

{#if !bucket}
  <ViewToolbar
    title="S3"
    subtitle={`${buckets?.length ?? 0} buckets`}
    bind:filter={bucketFilter}
    filterPlaceholder="Filter buckets…"
    loading={bucketsLoading}
    bind:auto
    onrefresh={() => void loadBuckets()}
  />
  {#if bucketsLoading && !buckets}
    <div class="pad"><Skeleton rows={6} /></div>
  {:else if bucketsError}
    <EmptyState icon="cloud" title="Couldn't list buckets" body={bucketsError} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void loadBuckets()} />
  {:else if bucketsShown.length === 0}
    <EmptyState icon="archive" title={bucketFilter ? 'No matching buckets' : 'No buckets'} body={bucketFilter ? '' : 'This account has no S3 buckets (or s3:ListAllMyBuckets is denied).'} />
  {:else}
    <div class="tbl-wrap">
      <table class="tbl">
        <thead><tr><th>Bucket</th><th class="hide-sm">Region</th><th class="hide-sm">Created</th></tr></thead>
        <tbody>
          {#each bucketsShown as b (b.name)}
            <tr
              class="trow"
              tabindex="0"
              onclick={() => goTo(b.name, '')}
              onkeydown={(e) => { if (e.key === 'Enter') goTo(b.name, ''); }}
              oncontextmenu={(e) => ctxMenu.show(e, [
                { label: 'Open', icon: 'folder', action: () => goTo(b.name, '') },
                { label: 'Copy S3 URI', icon: 'copy', action: () => void copy(`s3://${b.name}/`, 'S3 URI') },
              ])}
            >
              <td class="name"><Icon name="archive" size={13} /> {b.name}</td>
              <td class="mono hide-sm">{b.region ?? '—'}</td>
              <td class="dim hide-sm" title={fmtDate(b.creation_date)}>{fmtAgo(b.creation_date)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
{:else}
  <ViewToolbar
    title={bucket}
    bind:filter={objFilter}
    filterPlaceholder="Filter this folder…"
    loading={objLoading}
    bind:auto
    onrefresh={() => void loadObjects()}
  >
    <nav class="crumbs" aria-label="Prefix">
      <button class="crumb" onclick={() => goTo('', '')} title="All buckets"><Icon name="archive" size={12} /></button>
      <span class="sep">/</span>
      <button class="crumb" class:cur={!prefix} onclick={() => goTo(bucket, '')}>{bucket}</button>
      {#each crumbs as c (c.prefix)}
        <span class="sep">/</span>
        <button class="crumb" class:cur={c.prefix === prefix} onclick={() => goTo(bucket, c.prefix)}>{c.label}</button>
      {/each}
    </nav>
  </ViewToolbar>

  <div class="split" class:with-drawer={preview !== null && !viewport.isMobile}>
    <div class="tbl-wrap">
      {#if objLoading && objects.length === 0 && prefixes.length === 0}
        <div class="pad"><Skeleton rows={8} /></div>
      {:else if objError}
        <EmptyState icon="cloud" title="Couldn't list objects" body={objError} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void loadObjects()} />
      {:else if rowsShown.length === 0}
        <EmptyState icon="folder" title="Empty" body={objFilter ? 'Nothing matches the filter.' : 'No objects under this prefix.'} />
      {:else}
        <table class="tbl">
          <thead><tr><th>Name</th><th class="num">Size</th><th class="hide-sm">Modified</th><th class="hide-sm">Class</th><th class="act"></th></tr></thead>
          <tbody>
            {#each rowsShown as r (r.key)}
              <tr
                class="trow"
                class:sel={preview?.obj.key === r.key}
                tabindex="0"
                onclick={() => (r.kind === 'folder' ? goTo(bucket, r.key) : void openPreview(r.obj))}
                onkeydown={(e) => { if (e.key === 'Enter') r.kind === 'folder' ? goTo(bucket, r.key) : void openPreview(r.obj); }}
                oncontextmenu={(e) => rowMenu(e, r)}
              >
                <td class="name" title={r.key}>
                  <Icon name={r.kind === 'folder' ? 'folder' : 'file'} size={13} />
                  {r.name}{r.kind === 'folder' ? '/' : ''}
                </td>
                <td class="num mono">{r.kind === 'file' ? fmtBytes(r.obj.size) : ''}</td>
                <td class="dim hide-sm" title={r.kind === 'file' ? fmtDate(r.obj.last_modified) : ''}>{r.kind === 'file' ? fmtAgo(r.obj.last_modified) : ''}</td>
                <td class="dim mono hide-sm">{r.kind === 'file' ? (r.obj.storage_class ?? '') : ''}</td>
                <td class="act">
                  {#if r.kind === 'file'}
                    <button class="icon-btn" onclick={(e) => { e.stopPropagation(); void download(r.obj); }} disabled={!canRead} title="Download" aria-label={`Download ${r.name}`}><Icon name="arrowDown" size={13} /></button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if nextToken}
          <div class="more-row">
            <button class="ghost" onclick={() => void loadObjects(true)} disabled={objLoading}>{objLoading ? 'Loading…' : 'Load more'}</button>
          </div>
        {/if}
      {/if}
    </div>

    {#if preview && !viewport.isMobile}
      <aside class="drawer" aria-label="Object preview">
        {@render previewBody()}
      </aside>
    {/if}
  </div>

  {#if preview && viewport.isMobile}
    <Modal title={leaf(preview.obj.key)} width={720} onclose={() => (preview = null)}>
      {@render previewBody()}
    </Modal>
  {/if}
{/if}

{#if dl}
  <div class="dl-bar" role="status">
    <span class="mono">{leaf(dl.key)}</span>
    <progress max={dl.total ?? undefined} value={dl.total ? dl.received : undefined}></progress>
    <span class="dim">{fmtBytes(dl.received)}{dl.total ? ` / ${fmtBytes(dl.total)}` : ''}</span>
    <button class="ghost sm" onclick={() => dl?.ctrl.abort()}>Cancel</button>
  </div>
{/if}

{#snippet previewBody()}
  {#if preview}
    <div class="pv-head">
      <strong class="mono" title={preview.obj.key}>{leaf(preview.obj.key)}</strong>
      <span class="dim">{fmtBytes(preview.obj.size)} · {fmtDate(preview.obj.last_modified)}</span>
      <div class="pv-actions">
        <button class="ghost sm" onclick={() => preview && void download(preview.obj)}><Icon name="arrowDown" size={12} /> Download</button>
        <button class="ghost sm" onclick={() => preview && void copy(`s3://${bucket}/${preview.obj.key}`, 'S3 URI')}><Icon name="copy" size={12} /> URI</button>
        {#if !viewport.isMobile}
          <button class="icon-btn" onclick={() => (preview = null)} aria-label="Close preview"><Icon name="x" size={13} /></button>
        {/if}
      </div>
    </div>
    {#if preview.loading}
      <Skeleton rows={6} />
    {:else if preview.error}
      <p class="err">{preview.error}</p>
    {:else if previewKind === 'binary'}
      <p class="dim">Binary content ({preview.data?.content_type ?? 'unknown type'}) — download to open it.</p>
    {:else if previewKind === 'json' && previewJson !== undefined}
      <div class="pv-body mono"><JsonTree value={previewJson} /></div>
    {:else if previewKind === 'csv' && previewCsv.length}
      <div class="pv-body">
        <table class="csv">
          <thead><tr>{#each previewCsv[0] as h, i (i)}<th>{h}</th>{/each}</tr></thead>
          <tbody>
            {#each previewCsv.slice(1) as row, ri (ri)}
              <tr>{#each row as c, ci (ci)}<td>{c}</td>{/each}</tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <pre class="pv-body text">{preview.data?.text ?? ''}</pre>
    {/if}
    {#if preview.data?.truncated}
      <p class="dim">Preview truncated to the first 64 KiB.</p>
    {/if}
  {/if}
{/snippet}

<style>
  .pad {
    padding: 12px;
  }
  .tbl-wrap {
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: auto;
  }
  .tbl {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  .tbl th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface);
    text-align: left;
    font-weight: 600;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  .tbl td {
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 420px;
  }
  .tbl .num {
    text-align: right;
  }
  .tbl .act {
    width: 32px;
    text-align: right;
  }
  .trow {
    cursor: pointer;
  }
  .trow:hover,
  .trow:focus-visible {
    background: var(--surface-2);
    outline: none;
  }
  .trow.sel {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .name :global(svg) {
    vertical-align: -2px;
    margin-right: 6px;
  }
  .dim {
    color: var(--text-dim);
  }
  .err {
    color: var(--status-exited);
    font-size: 12.5px;
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-wrap: wrap;
    font-size: 12.5px;
  }
  .crumb {
    border: 0;
    background: transparent;
    color: var(--accent);
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 4px;
    font: inherit;
    font-size: 12.5px;
    display: inline-flex;
    align-items: center;
  }
  .crumb:hover {
    background: var(--surface-2);
  }
  .crumb.cur {
    color: var(--text);
    font-weight: 600;
    cursor: default;
  }
  .sep {
    color: var(--text-dim);
  }
  .split {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
  }
  .split.with-drawer {
    grid-template-columns: minmax(0, 1fr) minmax(280px, 42%);
  }
  .drawer {
    border-left: 1px solid var(--border);
    background: var(--surface);
    min-height: 0;
    overflow: auto;
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pv-head {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12.5px;
  }
  .pv-head strong {
    word-break: break-all;
  }
  .pv-actions {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .pv-body {
    font-size: 12px;
    overflow: auto;
    max-height: 60vh;
  }
  .pv-body.text {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: var(--font-mono);
    background: var(--bg);
    padding: 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
  }
  .csv {
    border-collapse: collapse;
    font-size: 11.5px;
    font-family: var(--font-mono);
  }
  .csv th,
  .csv td {
    border: 1px solid var(--border);
    padding: 2px 6px;
    white-space: nowrap;
  }
  .csv th {
    position: sticky;
    top: 0;
    background: var(--surface-2);
  }
  .more-row {
    display: flex;
    justify-content: center;
    padding: 10px;
  }
  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    font-size: 12.5px;
  }
  .ghost.sm {
    padding: 3px 8px;
    font-size: 12px;
  }
  .icon-btn {
    display: inline-grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .dl-bar {
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    border-top: 1px solid var(--border);
    background: var(--surface);
    font-size: 12px;
  }
  .dl-bar progress {
    flex: 1;
    height: 6px;
  }
  @media (max-width: 640px) {
    .hide-sm {
      display: none;
    }
  }
</style>
