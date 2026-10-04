<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  // S3: bucket list → object browser with breadcrumb prefixes (folder rows
  // first), a server-side prefix search past the loaded pages, a preview drawer
  // (text / pretty JSON / CSV table / image / PDF — S3Preview) and a streamed
  // Download. Writes (upload incl. drag-and-drop, delete) and presigned links
  // are confirmed in-app and gated server-side (s3_write / s3_delete / s3_read)
  // + audited. The current bucket + prefix live in the route
  // (`#/aws/<id>/s3/<bucket>?prefix=<encoded>`) so it's deep-linkable.
  import { untrack } from 'svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { awsApi, awsDownloadBlob, awsS3Upload, isLoginRequired, saveBlob } from '../../lib/api/aws';
  import { ApiError } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { confirmProd } from '../../lib/confirmProd';
  import S3Preview from './S3Preview.svelte';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { TableWindow } from '../../lib/tableWindow.svelte';
  import ViewToolbar from './ViewToolbar.svelte';
  import { fmtAgo, fmtBytes, fmtDate, splitBucketSegment, awsErrorText, mergeS3Head } from './util';
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
  const canWrite = $derived(resourceAccess.can('aws_account', account.id, 's3_write', 'aws_s3', 'edit', `bucket:${bucket}`));
  const canDelete = $derived(resourceAccess.can('aws_account', account.id, 's3_delete', 'aws_s3', 'edit', `bucket:${bucket}`));
  /** Set when the daemon refused a write (403) — the reason the write
   *  actions are disabled for the rest of this view. */
  let writeDenied = $state('');
  let deleteDenied = $state('');
  const isProd = $derived(account.environment === 'prod');

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
  // Raw (not deep-proxied) and appended with `concat`: "Load more" grows a
  // flat prefix to 100k objects; the rows are windowed below.
  let prefixes = $state.raw<string[]>([]);
  let objects = $state.raw<S3Object[]>([]);
  let nextToken = $state<string | null>(null);
  /** Pages in `objects`/`prefixes` (1 after a listing, +1 per "Load more"). */
  let pagesLoaded = 0;
  let objLoading = $state(false);
  let objError = $state('');
  let objectRequest = 0;
  let objFilter = $state('');
  const rowsShown = $derived.by(() => {
    const q = objFilter.trim().toLowerCase();
    // Server prefix-search hits for the current filter join the loaded rows.
    const s = search && search.q === objFilter.trim() ? search : null;
    const seen = new Set<string>();
    const allPrefixes = s ? prefixes.concat(s.prefixes) : prefixes;
    const allObjects = s ? objects.concat(s.objects) : objects;
    const folders = allPrefixes
      .filter((p) => !seen.has(p) && (seen.add(p), true))
      .map((p) => ({ kind: 'folder' as const, name: leaf(p), key: p }));
    const files = allObjects
      .filter((o) => o.key !== prefix) // the "directory marker" object itself
      .filter((o) => !seen.has(o.key) && (seen.add(o.key), true))
      .map((o) => ({ kind: 'file' as const, name: leaf(o.key), key: o.key, obj: o }));
    const all = [...folders, ...files];
    return q ? all.filter((r) => r.name.toLowerCase().includes(q)) : all;
  });

  // ── server-side search (A-2) ──
  // The filter above only sees loaded pages. When more pages exist, the
  // bucket itself is searched by key prefix (`prefix + q`, case-sensitive —
  // S3 has no substring search): automatically 400 ms after typing (unless
  // the query has a `*`), or via the explicit "Search the bucket" row.
  let search = $state<{
    q: string;
    prefixes: string[];
    objects: S3Object[];
    next: string | null;
    loading: boolean;
    error: string;
  } | null>(null);
  let searchCtrl: AbortController | null = null;

  async function runSearch(q: string, more = false): Promise<void> {
    if (!bucket || !q) return;
    searchCtrl?.abort();
    const ctrl = new AbortController();
    searchCtrl = ctrl;
    const prev = more && search?.q === q ? search : null;
    search = { q, prefixes: prev?.prefixes ?? [], objects: prev?.objects ?? [], next: prev?.next ?? null, loading: true, error: '' };
    try {
      const r = await awsApi.s3Objects(account.id, bucket, prefix + q, prev?.next ?? undefined, undefined, ctrl.signal);
      if (ctrl.signal.aborted) return;
      search = {
        q,
        prefixes: (prev?.prefixes ?? []).concat(r.prefixes),
        objects: (prev?.objects ?? []).concat(r.objects),
        next: r.is_truncated ? (r.next_token ?? null) : null,
        loading: false,
        error: '',
      };
    } catch (e) {
      if (ctrl.signal.aborted || (e instanceof DOMException && e.name === 'AbortError')) return;
      search = { q, prefixes: prev?.prefixes ?? [], objects: prev?.objects ?? [], next: prev?.next ?? null, loading: false, error: e instanceof Error ? e.message : String(e) };
    }
  }

  $effect(() => {
    const q = objFilter.trim();
    const more = nextToken !== null;
    void bucket;
    void prefix;
    if (!q) {
      untrack(() => {
        searchCtrl?.abort();
        search = null;
      });
      return;
    }
    if (!more || q.includes('*')) return;
    const t = setTimeout(() => {
      if (untrack(() => search?.q) !== q) void runSearch(q);
    }, 400);
    return () => clearTimeout(t);
  });
  const searchedThis = $derived(search !== null && search.q === objFilter.trim());

  // Window the object table (only the visible slice + spacers is in the DOM).
  const tw = new TableWindow();
  let objWrap = $state<HTMLDivElement | null>(null);
  const win = $derived(tw.range(rowsShown.length));
  const rowsWindow = $derived(rowsShown.slice(win.start, win.end));
  $effect(() => {
    void rowsWindow;
    tw.measure(objWrap);
  });
  // ⌘F over every (filtered) object, not just the mounted slice.
  $effect(() =>
    tw.findRows(
      () => objWrap,
      () => rowsShown,
      (r) =>
        r.kind === 'folder'
          ? `${r.name}/`
          : [r.name, fmtBytes(r.obj.size), fmtAgo(r.obj.last_modified), r.obj.storage_class ?? ''].join('\n'),
      'tbody tr.trow',
    ),
  );

  function leaf(key: string): string {
    const trimmed = key.endsWith('/') ? key.slice(0, -1) : key;
    return trimmed.slice(trimmed.lastIndexOf('/') + 1) || key;
  }

  async function loadObjects(more = false, keepScroll = false): Promise<void> {
    if (!bucket) return;
    objLoading = true;
    const request = ++objectRequest;
    try {
      const r = await awsApi.s3Objects(account.id, bucket, prefix, more ? nextToken : undefined);
      if (request !== objectRequest) return;
      prefixes = more ? prefixes.concat(r.prefixes) : r.prefixes;
      objects = more ? objects.concat(r.objects) : r.objects;
      pagesLoaded = more ? pagesLoaded + 1 : 1;
      if (!more && !keepScroll) tw.reset(objWrap);
      nextToken = r.is_truncated ? (r.next_token ?? null) : null;
      objError = '';
    } catch (e) {
      if (request !== objectRequest) return;
      objError = e instanceof Error ? e.message : String(e);
    } finally {
      if (request === objectRequest) objLoading = false;
    }
  }

  /** Refresh / auto-refresh: re-read the first page and merge it over what is
   *  loaded — "Load more" pages and the scroll position survive (was: back to
   *  page 1 and the top on every 10 s auto-refresh tick). */
  async function refreshObjects(): Promise<void> {
    if (!bucket) return;
    if (pagesLoaded <= 1) return loadObjects(false, true);
    objLoading = true;
    const request = ++objectRequest;
    try {
      const r = await awsApi.s3Objects(account.id, bucket, prefix);
      if (request !== objectRequest) return;
      const merged = mergeS3Head({ prefixes, objects }, r);
      prefixes = merged.prefixes;
      objects = merged.objects;
      // Everything fits one page now: nothing left to page in.
      if (!r.is_truncated) {
        nextToken = null;
        pagesLoaded = 1;
      }
      objError = '';
    } catch (e) {
      if (request !== objectRequest) return;
      objError = e instanceof Error ? e.message : String(e);
    } finally {
      if (request === objectRequest) objLoading = false;
    }
  }

  // Load on mount + whenever the route bucket/prefix changes. Reads
  // bucket/prefix (deps); the loaders' own writes are untracked.
  $effect(() => {
    const b = bucket;
    void prefix;
    untrack(() => {
      if (!buckets && !bucketsLoading) void loadBuckets();
      objectRequest++;
      searchCtrl?.abort();
      search = null;
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
  const selKey = $derived(preview?.obj.key ?? '');
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
  // Objects over BIG_DOWNLOAD go daemon-side straight to a local folder (the
  // in-webview path holds every chunk plus a Blob — up to 2× the object in
  // WKWebView memory); smaller ones stream into a Blob as before.
  const BIG_DOWNLOAD = 100 * 1024 * 1024;
  const DL_DIR_KEY = 'otto_s3_download_dir';
  let dl = $state<{ key: string; received: number; total: number | null; ctrl: AbortController; job?: string } | null>(null);
  let pickDirFor = $state<S3Object | null>(null);
  function lastDir(): string {
    try {
      return localStorage.getItem(DL_DIR_KEY) || '~/Downloads';
    } catch {
      return '~/Downloads';
    }
  }

  async function downloadToDir(o: S3Object, dir: string): Promise<void> {
    pickDirFor = null;
    if (dl) {
      toasts.warn('A download is already running', leaf(dl.key));
      return;
    }
    try {
      localStorage.setItem(DL_DIR_KEY, dir);
    } catch {
      /* remembered dir is a convenience */
    }
    const ctrl = new AbortController();
    const acct = account.id;
    dl = { key: o.key, received: 0, total: o.size || null, ctrl };
    try {
      let job = await awsApi.s3DownloadTo(acct, bucket, o.key, dir);
      if (dl) dl = { ...dl, job: job.id };
      while (job.state === 'running') {
        await new Promise((r) => setTimeout(r, 700));
        if (ctrl.signal.aborted) {
          job = await awsApi.s3DownloadCancel(acct, job.id);
          break;
        }
        job = await awsApi.s3DownloadJob(acct, job.id);
        if (dl) dl = { ...dl, received: job.bytes, total: job.total || dl.total };
      }
      if (job.state === 'completed') toasts.success('Downloaded', job.local_path);
      else if (job.state === 'failed') toasts.error('Download failed', job.error ?? 'The download failed.');
    } catch (e) {
      toasts.error('Download failed', e instanceof Error ? e.message : String(e));
    } finally {
      dl = null;
    }
  }

  async function download(o: S3Object): Promise<void> {
    if (!canRead) return;
    if (dl) {
      toasts.warn('A download is already running', leaf(dl.key));
      return;
    }
    // Download-to writes a file on the daemon host, so it needs aws_s3:Edit
    // (r3-10-01). A View-only user keeps the streamed browser download.
    if (o.size > BIG_DOWNLOAD && auth.can('aws_s3', 'edit')) {
      pickDirFor = o;
      return;
    }
    const ctrl = new AbortController();
    dl = { key: o.key, received: 0, total: o.size || null, ctrl };
    // Progress lands once per frame, not per 64 KiB chunk (~32k state writes
    // for a 2 GiB object).
    let pending: { received: number; total: number | null } | null = null;
    let raf = 0;
    try {
      const { blob, filename } = await awsDownloadBlob(
        awsApi.s3DownloadPath(account.id, bucket, o.key),
        (received, total) => {
          pending = { received, total };
          if (raf) return;
          raf = requestAnimationFrame(() => {
            raf = 0;
            if (dl && pending) dl = { ...dl, received: pending.received, total: pending.total ?? dl.total };
          });
        },
        ctrl.signal,
      );
      saveBlob(blob, filename ?? leaf(o.key));
      toasts.success('Downloaded', leaf(o.key));
    } catch (e) {
      if (!(e instanceof DOMException && e.name === 'AbortError'))
        toasts.error('Download failed', e instanceof Error ? e.message : String(e));
    } finally {
      if (raf) cancelAnimationFrame(raf);
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

  // ── writes + presign (A-4) ──
  function writeError(e: unknown, what: 'upload' | 'delete'): string {
    const msg = e instanceof Error ? e.message : String(e);
    if (e instanceof ApiError && e.status === 403) {
      const reason = `You don't have ${what === 'upload' ? 'upload (s3_write)' : 'delete (s3_delete)'} access to ${bucket} — ask an admin for aws_s3:Edit on this account. (${msg})`;
      if (what === 'upload') writeDenied = reason;
      else deleteDenied = reason;
      return reason;
    }
    return awsErrorText(msg);
  }

  const PRESIGN_CHOICES = [
    { label: '1 hour', value: '3600', kind: 'primary' as const },
    { label: '12 hours', value: '43200' },
    { label: '24 hours', value: '86400' },
    { label: '7 days', value: '604800' },
  ];

  async function presign(o: S3Object): Promise<void> {
    const { value } = await confirmer.choose(
      `Create a presigned download link for s3://${bucket}/${o.key}? Anyone holding the link can download the object until it expires. The link is recorded in the audit log.`,
      { title: 'Copy presigned link', options: PRESIGN_CHOICES },
    );
    if (!value) return;
    try {
      const r = await awsApi.s3Presign(account.id, bucket, o.key, Number(value));
      await copyTextOrThrow(r.url);
      toasts.success('Presigned link copied', `Expires ${fmtDate(r.expires_at)}`);
      if (r.warning) toasts.warn('Link may expire early', r.warning);
    } catch (e) {
      toasts.error('Couldn’t create the link', awsErrorText(e instanceof Error ? e.message : String(e)));
    }
  }

  async function deleteObject(o: S3Object): Promise<void> {
    const where = `s3://${bucket}/${o.key}`;
    // Prod asks the user to type the object key (and the server re-checks it).
    const ok = await confirmProd({
      env: account.environment,
      verb: 'Delete',
      title: isProd ? 'Delete object on production?' : 'Delete object?',
      where: `${where} · ${account.name}`,
      what: 'This can’t be undone unless the bucket is versioned.',
      typed: isProd ? o.key : undefined,
      danger: true,
    });
    if (!ok) return;
    const confirm: string | undefined = isProd ? o.key : undefined;
    try {
      await awsApi.s3Delete(account.id, bucket, o.key, confirm);
      if (preview?.obj.key === o.key) preview = null;
      objects = objects.filter((x) => x.key !== o.key);
      if (search) search = { ...search, objects: search.objects.filter((x) => x.key !== o.key) };
      toasts.success('Deleted', where);
    } catch (e) {
      toasts.error('Delete failed', writeError(e, 'delete'));
    }
  }

  let fileInput = $state<HTMLInputElement | null>(null);
  let dragOver = $state(false);
  let uploading = $state<{ done: number; total: number; name: string } | null>(null);
  const uploadBlocked = $derived(
    !canWrite ? 'You don’t have upload access to this bucket' : writeDenied || (uploading ? 'An upload is running' : ''),
  );

  async function uploadFiles(files: File[]): Promise<void> {
    if (!bucket || files.length === 0) return;
    if (uploadBlocked) {
      toasts.warn('Can’t upload', uploadBlocked);
      return;
    }
    const dest = `s3://${bucket}/${prefix}`;
    const what = files.length === 1 ? files[0].name : `${files.length} files`;
    if (isProd) {
      const ok = await confirmProd({
        env: account.environment,
        verb: 'Upload',
        where: `${dest} · ${account.name}`,
        what,
      });
      if (!ok) return;
    }
    let okCount = 0;
    try {
      for (const [i, f] of files.entries()) {
        uploading = { done: i, total: files.length, name: f.name };
        const key = prefix + f.name;
        try {
          await awsS3Upload(account.id, bucket, key, f);
          okCount++;
        } catch (e) {
          if (e instanceof ApiError && e.status === 409) {
            const replace = await confirmProd({
              env: account.environment,
              verb: 'Replace',
              title: isProd ? 'Replace existing object on production?' : 'Replace existing object?',
              where: `s3://${bucket}/${key} · ${account.name}`,
              what: `Already exists — replace it with the file you picked (${fmtBytes(f.size)}).`,
              danger: true,
            });
            if (!replace) continue;
            try {
              await awsS3Upload(account.id, bucket, key, f, { overwrite: true });
              okCount++;
            } catch (e2) {
              toasts.error(`Upload failed: ${f.name}`, writeError(e2, 'upload'));
              if (e2 instanceof ApiError && e2.status === 403) break;
            }
          } else {
            toasts.error(`Upload failed: ${f.name}`, writeError(e, 'upload'));
            if (e instanceof ApiError && e.status === 403) break;
          }
        }
      }
    } finally {
      uploading = null;
    }
    if (okCount) {
      toasts.success(okCount === 1 ? 'Uploaded' : `Uploaded ${okCount} files`, dest);
      void refreshObjects();
    }
  }

  function onPick(e: Event): void {
    const input = e.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    void uploadFiles(files);
  }
  function hasFiles(e: DragEvent): boolean {
    return Array.from(e.dataTransfer?.types ?? []).includes('Files');
  }
  function onDragOver(e: DragEvent): void {
    if (!bucket || !hasFiles(e)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = uploadBlocked ? 'none' : 'copy';
    dragOver = true;
  }
  function onDrop(e: DragEvent): void {
    if (!hasFiles(e)) return;
    e.preventDefault();
    dragOver = false;
    void uploadFiles(Array.from(e.dataTransfer?.files ?? []));
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
      { label: 'Copy presigned link…', disabled: !canRead, icon: 'link', action: () => void presign(r.obj) },
      { separator: true },
      {
        label: 'Delete…',
        icon: 'trash',
        danger: true,
        disabled: !canDelete || !!deleteDenied,
        action: () => void deleteObject(r.obj),
      },
    ]);
  }

  const loginNeeded = $derived(
    isLoginRequired(new Error(bucketsError)) || isLoginRequired(new Error(objError)),
  );
</script>

{#if !bucket}
  <ViewToolbar
    title="S3"
    subtitle={buckets ? `${buckets.length} bucket${buckets.length === 1 ? '' : 's'}` : ''}
    bind:filter={bucketFilter}
    filterPlaceholder="Filter buckets…"
    loading={bucketsLoading}
    bind:auto
    onrefresh={() => loadBuckets()}
  />
  {#if bucketsLoading && !buckets}
    <div class="pad" role="status"><p class="load-note">Loading buckets…</p><Skeleton rows={6} /></div>
  {:else if bucketsError}
    <EmptyState actionKind={loginNeeded ? 'primary' : 'secondary'} icon="warning" title="Couldn't list buckets" body={awsErrorText(bucketsError)} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void loadBuckets()} />
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
              <td class="name" title={b.name}><Icon name="archive" size={13} /> {b.name}</td>
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
    onrefresh={() => refreshObjects()}
  >
    {#snippet actions()}
      <button
        class="btn small"
        onclick={() => fileInput?.click()}
        disabled={!!uploadBlocked}
        title={uploadBlocked || `Upload files to s3://${bucket}/${prefix} (or drop them on the list)`}
      ><Icon name="arrowUp" size={12} /> Upload</button>
      <input class="s3-file" type="file" multiple bind:this={fileInput} onchange={onPick} aria-label="Files to upload" tabindex="-1" />
    {/snippet}
    <nav class="crumbs" aria-label="Prefix">
      <button class="crumb" onclick={() => goTo('', '')} title="All buckets" aria-label="All buckets"><Icon name="archive" size={12} /></button>
      <span class="sep">/</span>
      <button class="crumb" class:cur={!prefix} onclick={() => goTo(bucket, '')}>{bucket}</button>
      {#each crumbs as c (c.prefix)}
        <span class="sep">/</span>
        <button class="crumb" class:cur={c.prefix === prefix} onclick={() => goTo(bucket, c.prefix)}>{c.label}</button>
      {/each}
    </nav>
  </ViewToolbar>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="split"
    class:with-drawer={preview !== null && !viewport.isMobile}
    class:drag-over={dragOver}
    ondragover={onDragOver}
    ondragleave={(e) => { if (e.currentTarget === e.target) dragOver = false; }}
    ondrop={onDrop}
  >
    <div class="tbl-wrap" bind:this={objWrap} bind:clientHeight={tw.viewH} onscroll={tw.onscroll}>
      {#if objLoading && objects.length === 0 && prefixes.length === 0}
        <div class="pad" role="status"><p class="load-note">Loading objects…</p><Skeleton rows={8} /></div>
      {:else if objError}
        <EmptyState actionKind={loginNeeded ? 'primary' : 'secondary'} icon="warning" title="Couldn't list objects" body={awsErrorText(objError)} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void loadObjects()} />
      {:else if rowsShown.length === 0 && objFilter && (nextToken || search)}
        <div class="s3-search-empty">
          {#if search?.loading}
            <p class="load-note" role="status">Searching the bucket for keys starting with “{prefix}{objFilter.trim()}”…</p>
          {:else if search?.error}
            <p class="err" role="alert">{awsErrorText(search.error)} <button class="btn small" onclick={() => void runSearch(objFilter.trim())}>Retry search</button></p>
          {:else}
            <p class="dim">
              {#if searchedThis}
                No match in the {objects.length + prefixes.length} loaded rows, and no key in the bucket starts with “{prefix}{objFilter.trim()}” (bucket search matches the key prefix, case-sensitive).
              {:else}
                Showing {objects.length + prefixes.length} loaded — more objects exist. Load more, or search the bucket by key prefix (case-sensitive).
              {/if}
            </p>
            <div class="s3-search-actions">
              {#if nextToken}<button class="btn small" onclick={() => void loadObjects(true)} disabled={objLoading}>Load more</button>{/if}
              {#if !searchedThis}<button class="btn small primary" onclick={() => void runSearch(objFilter.trim())}>Search the bucket</button>{/if}
            </div>
          {/if}
        </div>
      {:else if rowsShown.length === 0}
        <EmptyState icon="folder" title="Empty" body={objFilter ? 'Nothing matches the filter.' : canWrite ? 'No objects under this prefix. Drop files here or use Upload.' : 'No objects under this prefix.'} />
      {:else}
        {#if objFilter.trim() && (nextToken || search)}
          <div class="s3-search-row">
            {#if search?.loading && searchedThis}
              <span class="dim" role="status">Searching the bucket for keys starting with “{prefix}{objFilter.trim()}”…</span>
            {:else if search?.error && searchedThis}
              <span class="err" role="alert">{awsErrorText(search.error)}</span>
              <button class="btn small" onclick={() => void runSearch(objFilter.trim())}>Retry search</button>
            {:else if searchedThis}
              <span class="dim">Includes bucket matches for keys starting with “{prefix}{objFilter.trim()}” (case-sensitive).</span>
              {#if search?.next}<button class="btn small" onclick={() => void runSearch(objFilter.trim(), true)}>More matches</button>{/if}
            {:else}
              <button class="btn small" onclick={() => void runSearch(objFilter.trim())}><Icon name="search" size={12} /> Search the bucket for “{objFilter.trim()}”</button>
              <span class="dim">Only loaded pages are filtered. Bucket search matches the key prefix, case-sensitive.</span>
            {/if}
          </div>
        {/if}
        <table class="tbl">
          <thead><tr><th>Name</th><th class="num">Size</th><th class="hide-sm">Modified</th><th class="hide-sm">Class</th><th class="act"></th></tr></thead>
          <tbody>
            {#if win.top}<tr class="tw-spacer" aria-hidden="true"><td colspan="5" style="height:{win.top}px"></td></tr>{/if}
            {#each rowsWindow as r (r.key)}
              <tr
                class="trow"
                class:sel={selKey === r.key}
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
                    <span class="s3-acts">
                      <button class="icon-btn" onclick={(e) => { e.stopPropagation(); void download(r.obj); }} disabled={!canRead} title={canRead ? 'Download' : 'You don’t have read access to this bucket'} aria-label={`Download ${r.name}`}><Icon name="arrowDown" size={13} /></button>
                      <button class="icon-btn" onclick={(e) => { e.stopPropagation(); rowMenu(e, r); }} title="More actions" aria-label={`More actions for ${r.name}`}><Icon name="more" size={13} /></button>
                    </span>
                  {/if}
                </td>
              </tr>
            {/each}
            {#if win.bottom}<tr class="tw-spacer" aria-hidden="true"><td colspan="5" style="height:{win.bottom}px"></td></tr>{/if}
          </tbody>
        </table>
        {#if nextToken}
          <div class="more-row">
            <button class="btn" onclick={() => void loadObjects(true)} disabled={objLoading}>{objLoading ? 'Loading…' : 'Load more'}</button>
          </div>
        {/if}
      {/if}
    </div>

    {#if dragOver}
      <div class="s3-drop" aria-hidden="true">
        <Icon name="arrowUp" size={18} />
        <span>{uploadBlocked || `Drop to upload to s3://${bucket}/${prefix}`}</span>
      </div>
    {/if}

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

{#if uploading}
  <div class="dl-bar" role="status">
    <span class="mono">Uploading {uploading.name}</span>
    <progress max={uploading.total} value={uploading.done}></progress>
    <span class="dim">{uploading.done + 1} / {uploading.total}</span>
  </div>
{/if}

{#if dl}
  <div class="dl-bar" role="status">
    <span class="mono">{leaf(dl.key)}</span>
    <progress max={dl.total ?? undefined} value={dl.total ? dl.received : undefined}></progress>
    <span class="dim">{fmtBytes(dl.received)}{dl.total ? ` / ${fmtBytes(dl.total)}` : ''}</span>
    <button class="btn small" onclick={() => dl?.ctrl.abort()}>Cancel</button>
  </div>
{/if}

{#if pickDirFor}
  <FolderPicker
    title={`Download “${leaf(pickDirFor.key)}” (${fmtBytes(pickDirFor.size)}) to…`}
    start={lastDir()}
    onpick={(dir) => pickDirFor && void downloadToDir(pickDirFor, dir)}
    onclose={() => (pickDirFor = null)}
  />
{/if}

{#snippet previewBody()}
  {#if preview}
    <div class="pv-head">
      <strong class="mono" title={preview.obj.key}>{leaf(preview.obj.key)}</strong>
      <span class="dim">{fmtBytes(preview.obj.size)} · {fmtDate(preview.obj.last_modified)}</span>
      <div class="pv-actions">
        <button class="btn small" onclick={() => preview && void download(preview.obj)}><Icon name="arrowDown" size={12} /> Download</button>
        <button class="btn small" onclick={() => preview && void copy(`s3://${bucket}/${preview.obj.key}`, 'S3 URI')}><Icon name="copy" size={12} /> URI</button>
        <button class="btn small" onclick={() => preview && void presign(preview.obj)} disabled={!canRead}><Icon name="link" size={12} /> Link</button>
        {#if !viewport.isMobile}
          <button class="icon-btn" onclick={() => (preview = null)} aria-label="Close preview" title="Close preview"><Icon name="x" size={13} /></button>
        {/if}
      </div>
    </div>
    <S3Preview
      accountId={account.id}
      {bucket}
      obj={preview.obj}
      data={preview.data}
      loading={preview.loading}
      error={preview.error}
      onretry={() => preview && void openPreview(preview.obj)}
      ondownload={() => preview && void download(preview.obj)}
    />
  {/if}
{/snippet}

<style>
  .load-note {
    margin: 0 0 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
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
    font-size: var(--fs-m);
  }
  .tbl th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface);
    text-align: start;
    font-weight: 600;
    font-size: var(--fs-xs);
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
    text-align: end;
  }
  .tbl .act {
    width: 60px;
    text-align: end;
  }
  .trow {
    cursor: pointer;
  }
  .tbl .tw-spacer td {
    padding: 0;
    border: 0;
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
    margin-inline-end: 6px;
  }
  .dim {
    color: var(--text-dim);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-m);
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-wrap: wrap;
    font-size: var(--fs-m);
  }
  .crumb {
    border: 0;
    background: transparent;
    color: var(--accent-text);
    cursor: pointer;
    padding: 2px 4px;
    border-radius: var(--radius-s);
    font: inherit;
    font-size: var(--fs-m);
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
    border-inline-start: 1px solid var(--border);
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
    font-size: var(--fs-m);
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
  .split {
    position: relative;
  }
  .split.drag-over .tbl-wrap {
    outline: 2px dashed var(--accent-text);
    outline-offset: -4px;
  }
  .s3-drop {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    pointer-events: none;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    color: var(--accent-text);
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .s3-file {
    display: none;
  }
  .s3-acts {
    display: inline-flex;
    gap: 4px;
  }
  .s3-search-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-s);
  }
  .s3-search-empty {
    padding: 24px 16px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    text-align: center;
    font-size: var(--fs-m);
  }
  .s3-search-actions {
    display: flex;
    gap: 8px;
  }
  .more-row {
    display: flex;
    justify-content: center;
    padding: 10px;
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
    font-size: var(--fs-s);
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
