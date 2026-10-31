///<reference types="svelte" />
;
import { announce, focusAppContent } from '../../lib/announce.svelte';
import { rowMenu as rowMenuKeys } from '../../lib/rowMenu';
import { loadErrorText, providerErrorText } from '../../lib/loadError';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
// S3: bucket list → object browser with breadcrumb prefixes (folder rows
// first), a server-side prefix search past the loaded pages, a preview drawer
// (text / pretty JSON / CSV table / image / PDF — S3Preview) and a streamed
// Download. Writes (upload incl. drag-and-drop, delete) and presigned links
// are confirmed in-app and gated server-side (s3_write / s3_delete / s3_read)
// + audited. The current bucket + prefix live in the route
// (`#/aws/<id>/s3/<bucket>?prefix=<encoded>`) so it's deep-linkable.
import { onDestroy, untrack } from 'svelte';
import { aws } from '../../lib/stores/aws.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { awsApi, awsDownloadBlob, awsS3Upload, isLoginRequired, saveBlob } from '../../lib/api/aws';
import { ApiError } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { confirmProd } from '../../lib/confirmProd';
import S3Preview from './S3Preview.svelte';
import S3TransferBars from './S3TransferBars.svelte';
import { router } from '../../lib/router.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import { viewport } from '../../lib/stores/viewport.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import { TableWindow } from '../../lib/tableWindow.svelte';
import ViewToolbar from './ViewToolbar.svelte';
import { fmtAgo, fmtBytes, fmtDate, splitBucketSegment, awsErrorText, mergeS3Head } from './util';
import { awsLoadText } from './loadText';
import type { AwsAccount, S3Object, S3PreviewResp } from '../../lib/api/types';
;

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  };function $$render() {
  const previewBody/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  if(preview){
     { svelteHTML.createElement("div", { "class":`pv-head`,});
       { svelteHTML.createElement("strong", {   "class":`mono`,"title":preview.obj.key,});leaf(preview.obj.key); }
       { svelteHTML.createElement("span", { "class":`dim`,});fmtBytes(preview.obj.size);  fmtDate(preview.obj.last_modified); }
       { svelteHTML.createElement("div", { "class":`pv-actions`,});
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => preview && void download(preview.obj),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}  }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => preview && void copy(`s3://${bucket}/${preview.obj.key}`, 'S3 URI'),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}  }
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => preview && void presign(preview.obj),"disabled":!canRead,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});}  }
        if(!viewport.isMobile){
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => { preview = null; void focusAppContent(); },"aria-label":`Close preview`,"title":`Close preview`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});} }
        }
       }
     }
     { const $$_weiverP3S0C = __sveltets_2_ensureComponent(S3Preview); new $$_weiverP3S0C({ target: __sveltets_2_any(), props: {                "accountId":account.id,bucket,"obj":preview.obj,"data":preview.data,"loading":preview.loading,"error":preview.error,"onretry":() => preview && void openPreview(preview.obj),"ondownload":() => preview && void download(preview.obj),}});}
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
      bucketsError = awsLoadText(e);
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
      search = { q, prefixes: prev?.prefixes ?? [], objects: prev?.objects ?? [], next: prev?.next ?? null, loading: false, error: awsLoadText(e) };
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
      objError = awsLoadText(e);
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
      objError = awsLoadText(e);
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
        preview = { obj: o, data: null, loading: false, error: awsLoadText(e) };
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

  let destroyed = false;
  onDestroy(() => {
    destroyed = true;
  });

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
        // Left the view: stop polling. The job itself keeps running on the
        // daemon (the file still lands in `dir`) — only the progress UI is gone.
        if (destroyed) return;
        if (ctrl.signal.aborted) {
          job = await awsApi.s3DownloadCancel(acct, job.id);
          break;
        }
        job = await awsApi.s3DownloadJob(acct, job.id);
        if (dl) dl = { ...dl, received: job.bytes, total: job.total || dl.total };
      }
      if (job.state === 'completed') toasts.success('Downloaded', job.local_path);
      else if (job.state === 'failed') toasts.error('Couldn’t download', job.error ?? 'The download failed.');
    } catch (e) {
      toastError('Couldn’t download', e);
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
        toastError('Couldn’t download', e);
    } finally {
      if (raf) cancelAnimationFrame(raf);
      dl = null;
    }
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
    }
  }

  // ── writes + presign (A-4) ──
  function writeError(e: unknown, what: 'upload' | 'delete'): string {
    const msg = providerErrorText(e);
    if (e instanceof ApiError && e.status === 403) {
      const reason = `You don’t have ${what === 'upload' ? 'upload (s3_write)' : 'delete (s3_delete)'} access to ${bucket} — ask an admin for aws_s3:Edit on this account. (${msg})`;
      if (what === 'upload') writeDenied = reason;
      else deleteDenied = reason;
      return reason;
    }
    return awsErrorText(msg);
  }

  const PRESIGN_CHOICES = [
    { label: 'Copy link · 1 hour', value: '3600', kind: 'primary' as const },
    { label: 'Copy link · 12 hours', value: '43200' },
    { label: 'Copy link · 24 hours', value: '86400' },
    { label: 'Copy link · 7 days', value: '604800' },
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
      toasts.error('Couldn’t create the link', awsErrorText(loadErrorText(e)));
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
      toasts.error('Couldn’t delete', writeError(e, 'delete'));
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
              toasts.error(`Couldn’t upload ${f.name}`, writeError(e2, 'upload'));
              if (e2 instanceof ApiError && e2.status === 403) break;
            }
          } else {
            toasts.error(`Couldn’t upload ${f.name}`, writeError(e, 'upload'));
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

  const loginNeeded = $derived(isLoginRequired(new Error(bucketsError)) || isLoginRequired(new Error(objError)));
  // Announced at the edge (lib/announce): the search lines below are plain text.
  const searchLoading = $derived(!!search?.loading);
  const searchError = $derived(search?.error ?? '');
  $effect(() => { if (searchLoading) announce('Searching the bucket…'); });
  $effect(() => { if (searchError) announce(`Couldn’t search the bucket. ${awsErrorText(searchError)}`, { assertive: true }); });
;
async () => {

if(!bucket){
   { const $$_rablooTweiV0C = __sveltets_2_ensureComponent(ViewToolbar); const $$_rablooTweiV0 = new $$_rablooTweiV0C({ target: __sveltets_2_any(), props: {              "title":`S3`,"subtitle":buckets ? `${plural(buckets.length, 'bucket')}` : '',filter:bucketFilter,"filterPlaceholder":`Filter buckets…`,"loading":bucketsLoading,auto,"onrefresh":() => loadBuckets(),}});/*Ωignore_startΩ*/() => bucketFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'filter';/*Ωignore_startΩ*/() => auto = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'auto';}
  if(bucketsLoading && !buckets){
     { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":6,"label":`buckets`,}});} }
  } else if (bucketsError){
     { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL0 = new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {     "what":`buckets`,"error":awsErrorText(bucketsError),"empty":true,"onretry":() => void loadBuckets(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(loginNeeded){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL0.$$prop_def;/*Ωignore_endΩ*/ LoadState}
  } else if (bucketsShown.length === 0){
     { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`archive`,"title":bucketFilter ? 'No matching buckets' : 'No buckets',"body":bucketFilter ? '' : 'This account has no S3 buckets (or s3:ListAllMyBuckets is denied).',}});}
  }else{
     { svelteHTML.createElement("div", { "class":`tbl-wrap`,});
       { svelteHTML.createElement("table", { "class":`tbl`,});
         { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  } } }
         { svelteHTML.createElement("tbody", {});
             for(let b of __sveltets_2_ensureArray(bucketsShown)){b.name;
             {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('tr')));{ svelteHTML.createElement("tr", __sveltets_2_union($$action_0), {            "class":`trow`,"tabindex":0,"onclick":() => goTo(b.name, ''),"onkeydown":(e) => { if (e.key === 'Enter') goTo(b.name, ''); },"oncontextmenu":(e) => ctxMenu.show(e, [
                { label: 'Open', icon: 'folder', action: () => goTo(b.name, '') },
                { label: 'Copy S3 URI', icon: 'copy', action: () => void copy(`s3://${b.name}/`, 'S3 URI') },
              ]),});
               { svelteHTML.createElement("td", {   "class":`name`,"title":b.name,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`archive`,"size":13,}});} b.name; }
               { svelteHTML.createElement("td", { "class":`mono hide-sm`,});b.region ?? '—'; }
               { svelteHTML.createElement("td", {   "class":`dim hide-sm`,"title":fmtDate(b.creation_date),});fmtAgo(b.creation_date); }
             }}
          }
         }
       }
     }
  }
}else{
   { const $$_rablooTweiV0C = __sveltets_2_ensureComponent(ViewToolbar); const $$_rablooTweiV0 = new $$_rablooTweiV0C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"title":bucket,filter:objFilter,"filterPlaceholder":`Filter this folder…`,"loading":objLoading,auto,"onrefresh":() => refreshObjects(),actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":() => fileInput?.click(),"disabled":!!uploadBlocked,"title":uploadBlocked || `Upload files to s3://${bucket}/${prefix} (or drop them on the list)`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});}  }
       { const $$_input0 = svelteHTML.createElement("input", {            "class":`s3-file`,"type":`file`,"multiple":true,"onchange":onPick,"aria-label":`Files to upload`,"tabindex":-1,});fileInput = $$_input0;}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/() => objFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'filter';/*Ωignore_startΩ*/() => auto = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'auto';/*Ωignore_startΩ*/const {actions} = $$_rablooTweiV0.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("nav", {   "class":`crumbs`,"aria-label":`Prefix`,});
       { svelteHTML.createElement("button", {       "class":`crumb`,"onclick":() => goTo('', ''),"title":`All buckets`,"aria-label":`All buckets`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`archive`,"size":12,}});} }
       { svelteHTML.createElement("span", { "class":`sep`,});  }
       { svelteHTML.createElement("button", {    "class":`crumb`,"onclick":() => goTo(bucket, ''),});!prefix;bucket; }
         for(let c of __sveltets_2_ensureArray(crumbs)){c.prefix;
         { svelteHTML.createElement("span", { "class":`sep`,});  }
         { svelteHTML.createElement("button", {    "class":`crumb`,"onclick":() => goTo(bucket, c.prefix),});c.prefix === prefix;c.label; }
      }
     }
   ViewToolbar}

  
   { svelteHTML.createElement("div", {               "role":`group`,"aria-label":`Objects — drop files here to upload`,"class":`split`,"ondragover":onDragOver,"ondragleave":(e) => { if (e.currentTarget === e.target) dragOver = false; },"ondrop":onDrop,});preview !== null && !viewport.isMobile;dragOver;
     { const $$_div1 = svelteHTML.createElement("div", {     "class":`tbl-wrap`,"onscroll":tw.onscroll,});objWrap = $$_div1;tw.viewH= $$_div1.clientHeight;
      if(objLoading && objects.length === 0 && prefixes.length === 0){
         { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"label":`objects`,}});} }
      } else if (objError){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {     "what":`objects`,"error":awsErrorText(objError),"empty":true,"onretry":() => void loadObjects(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(loginNeeded){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/ LoadState}
      } else if (rowsShown.length === 0 && objFilter && (nextToken || search)){
         { svelteHTML.createElement("div", { "class":`s3-search-empty`,});
          if(search?.loading){
             { svelteHTML.createElement("p", { "class":`load-note`,});       prefix;objFilter.trim();  }
          } else if (search?.error){
             { svelteHTML.createElement("p", { "class":`err`,});awsErrorText(search.error);  { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void runSearch(objFilter.trim()),});  } }
          }else{
             { svelteHTML.createElement("p", { "class":`dim`,});
              if(searchedThis){    objects.length + prefixes.length;           prefix;objFilter.trim();       }else{ objects.length + prefixes.length;               }
             }
             { svelteHTML.createElement("div", { "class":`s3-search-actions`,});
              if(nextToken){ { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void loadObjects(true),"disabled":objLoading,});  }}
              if(!searchedThis){ { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => void runSearch(objFilter.trim()),});   }}
             }
          }
         }
      } else if (rowsShown.length === 0){
         { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`folder`,"title":`Empty`,"body":objFilter ? 'Nothing matches the filter.' : canWrite ? 'No objects under this prefix. Drop files here or use Upload.' : 'No objects under this prefix.',}});}
      }else{
        if(objFilter.trim() && (nextToken || search)){
           { svelteHTML.createElement("div", { "class":`s3-search-row`,});
            if(search?.loading && searchedThis){
               { svelteHTML.createElement("span", { "class":`dim`,});       prefix;objFilter.trim();  }
            } else if (search?.error && searchedThis){
               { svelteHTML.createElement("span", { "class":`err`,});awsErrorText(search.error); }
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void runSearch(objFilter.trim()),});  }
            } else if (searchedThis){
               { svelteHTML.createElement("span", { "class":`dim`,});       prefix;objFilter.trim();  }
              if(search?.next){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void runSearch(objFilter.trim(), true),});  }}
            }else{
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void runSearch(objFilter.trim()),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}     objFilter.trim();  }
               { svelteHTML.createElement("span", { "class":`dim`,});            }
            }
           }
        }
         { svelteHTML.createElement("table", { "class":`tbl`,});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", { "class":`num`,});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", { "class":`act`,}); } } }
           { svelteHTML.createElement("tbody", {});
            if(win.top){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":5,"style":`height:${win.top}px`,}); } }}
               for(let r of __sveltets_2_ensureArray(rowsWindow)){r.key;
               {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('tr')));{ svelteHTML.createElement("tr", __sveltets_2_union($$action_0), {             "class":`trow`,"tabindex":0,"onclick":() => (r.kind === 'folder' ? goTo(bucket, r.key) : void openPreview(r.obj)),"onkeydown":(e) => { if (e.key === 'Enter') r.kind === 'folder' ? goTo(bucket, r.key) : void openPreview(r.obj); },"oncontextmenu":(e) => rowMenu(e, r),});selKey === r.key;
                 { svelteHTML.createElement("td", {   "class":`name`,"title":r.key,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":r.kind === 'folder' ? 'folder' : 'file',"size":13,}});}
                  r.name;r.kind === 'folder' ? '/' : '';
                 }
                 { svelteHTML.createElement("td", { "class":`num mono`,});r.kind === 'file' ? fmtBytes(r.obj.size) : ''; }
                 { svelteHTML.createElement("td", {   "class":`dim hide-sm`,"title":r.kind === 'file' ? fmtDate(r.obj.last_modified) : '',});r.kind === 'file' ? fmtAgo(r.obj.last_modified) : ''; }
                 { svelteHTML.createElement("td", { "class":`dim mono hide-sm`,});r.kind === 'file' ? (r.obj.storage_class ?? '') : ''; }
                 { svelteHTML.createElement("td", { "class":`act`,});
                  if(r.kind === 'file'){
                     { svelteHTML.createElement("span", { "class":`s3-acts`,});
                       { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":(e) => { e.stopPropagation(); void download(r.obj); },"disabled":!canRead,"title":canRead ? 'Download' : 'You don’t have read access to this bucket',"aria-label":`Download ${r.name}`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":13,}});} }
                       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":(e) => { e.stopPropagation(); rowMenu(e, r); },"title":`More actions`,"aria-label":`More actions for ${r.name}`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
                     }
                  }
                 }
               }}
            }
            if(win.bottom){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":5,"style":`height:${win.bottom}px`,}); } }}
           }
         }
        if(nextToken){
           { svelteHTML.createElement("div", { "class":`more-row`,});
             { svelteHTML.createElement("button", {     "class":`btn`,"onclick":() => void loadObjects(true),"disabled":objLoading,});objLoading ? 'Loading more objects…' : 'Load more'; }
           }
        }
      }
     }

    if(dragOver){
       { svelteHTML.createElement("div", {   "class":`s3-drop`,"aria-hidden":`true`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":16,}});}
         { svelteHTML.createElement("span", {});uploadBlocked || `Drop to upload to s3://${bucket}/${prefix}`; }
       }
    }

    if(preview && !viewport.isMobile){
       { svelteHTML.createElement("aside", {   "class":`drawer`,"aria-label":`Object preview`,});
        ;__sveltets_2_ensureSnippet(previewBody());
       }
    }
   }

  if(preview && viewport.isMobile){
     { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":leaf(preview.obj.key),"width":720,"onclose":() => (preview = null),}});
      ;__sveltets_2_ensureSnippet(previewBody());
     Modal}
  }
}

 { const $$_sraBrefsnarT3S0C = __sveltets_2_ensureComponent(S3TransferBars); new $$_sraBrefsnarT3S0C({ target: __sveltets_2_any(), props: {     uploading,"download":dl,"oncanceldownload":() => dl?.ctrl.abort(),}});}

if(pickDirFor){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Download “${leaf(pickDirFor.key)}” (${fmtBytes(pickDirFor.size)}) to…`,"start":lastDir(),"onpick":(dir) => pickDirFor && void downloadToDir(pickDirFor, dir),"onclose":() => (pickDirFor = null),}});}
}




};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const S3Browser__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type S3Browser__SvelteComponent_ = ReturnType<typeof S3Browser__SvelteComponent_>;
/*Ωignore_endΩ*/export default S3Browser__SvelteComponent_;