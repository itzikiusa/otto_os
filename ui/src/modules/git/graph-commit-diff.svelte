<script lang="ts" module>
  import type { DiffResp, FileDiff, Hunk as H } from '../../lib/api/types';
  import { WeightedLru, diffKey } from './graph-diff-cache';
  import { api } from '../../lib/api/client';

  // The diff contract's summary-mode fields (`hunks_omitted`, `too_large`,
  // `total_*`) are all optional, so an older daemon's full response fits too.
  type SumFile = FileDiff;
  type SumResp = DiffResp;

  type Body =
    | { kind: 'loading' }
    | { kind: 'ready'; hunks: H[]; rows: number }
    | { kind: 'too_large'; lines: number; hard: boolean }
    | { kind: 'error'; message: string };

  // Module scope = one cache for every graph instance / tab (a commit's diff is
  // immutable per sha). Summaries weigh their file count plus any inline lines
  // (an old daemon that ignores `summary=true`); file patches weigh their rows.
  const summaryCache = new WeightedLru<SumResp>(20_000, (r) => {
    let w = r.files.length;
    for (const f of r.files) for (const h of f.hunks) w += h.lines.length;
    return w;
  });
  const bodyCache = new WeightedLru<Body>(60_000, (b) => (b.kind === 'ready' ? b.rows : 0) + 1);

  /** Summary requests started by `prefetchCommitSummary`, joined by the pane
   *  when it mounts for the same commit. */
  const summaryFlights = new Map<string, Promise<SumResp>>();

  function summaryUrl(repoId: string, sha: string): string {
    return `/repos/${repoId}/diff?target=${encodeURIComponent(`commit:${sha}`)}&summary=true`;
  }

  /** Warm the file list of `sha` while the pane is still held back (the
   *  graph's branch click waits out the double-click window before mounting
   *  the diff): the summary is one small `--numstat` read, so by the time the
   *  window closes it has usually landed and the list paints from cache in
   *  the same frame. Returns an abort for the double-click (= checkout) case. */
  export function prefetchCommitSummary(repoId: string, sha: string): () => void {
    const key = diffKey(repoId, sha);
    if (summaryCache.get(key) || summaryFlights.has(key)) return () => {};
    const ctl = new AbortController();
    const p = api
      .get<SumResp>(summaryUrl(repoId, sha), ctl.signal)
      .then((r) => {
        summaryCache.set(key, r);
        return r;
      })
      .finally(() => {
        if (summaryFlights.get(key) === p) summaryFlights.delete(key);
      });
    summaryFlights.set(key, p);
    p.catch(() => {}); // a joiner handles failures; an unjoined abort is expected
    return () => ctl.abort();
  }
</script>

<script lang="ts">
  import { plural } from '../../lib/plural';
  // The commit-detail diff pane of the Git graph (the right-hand panel a branch
  // or commit click opens). Built to stay flat no matter how big the commit is:
  //
  //  1. SUMMARY FIRST — `?summary=true` returns only the file list with the
  //     server's own +/- counts (numstat), so the list paints in one small
  //     round-trip. Nothing here ever walks diff lines to count them.
  //  2. PATCHES ON DEMAND — a file's hunks are fetched (`?path=` + `old_path=`
  //     for renames) only when it is OPEN and near the viewport
  //     (IntersectionObserver), at most MAX_FETCHES at a time, all abortable.
  //  3. COLLAPSED BY DEFAULT above COLLAPSE_ALL_FILES / COLLAPSE_ALL_LINES, and
  //     any single file over COLLAPSE_FILE_LINES; derived synchronously, so the
  //     first render never builds rows it is about to throw away.
  //  4. CAPPED — at most FILE_PAGE file headers and ROW_CAP rows per file are
  //     mounted, each with a "Show more"; `too_large` files offer "Load anyway".
  //  5. SMALL COMMITS IN ONE GO — when the summary shows an ordinary commit
  //     (not `collapseAll`), every file's hunks come from ONE (daemon-capped,
  //     memoized) full request instead of one request per file.
  //
  // Defensive against a daemon that predates `summary=true`: if the response
  // carries full hunks (no `hunks_omitted`), they are used in place — still
  // collapsed and capped — instead of being fetched again.
  import { untrack } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { isAbortError } from '../../lib/api/client';
  import type { Hunk, DiffLine } from '../../lib/api/types';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { fileFor } from './diff-load';

  interface Props {
    repoId: string;
    /** Commit to show; `null` = selected but not loading yet (the branch-click
     *  double-click window) — renders the skeleton. */
    sha: string | null;
    /** Selected but deliberately NOT loaded (a double-click checked the branch
     *  out): show a button instead of fetching. */
    held?: boolean;
    onresume?: () => void;
    onfiletools?: (e: MouseEvent, path: string) => void;
    /** Out: number of files in the loaded summary (mobile section header). */
    fileCount?: number | null;
  }
  let { repoId, sha, held = false, onresume, onfiletools, fileCount = $bindable(null) }: Props = $props();

  const COLLAPSE_ALL_FILES = 40;
  const COLLAPSE_ALL_LINES = 2000;
  const COLLAPSE_FILE_LINES = 400;
  const FILE_PAGE = 200;
  const ROW_CAP = 400;
  const ROW_STEP = 1500;
  const MAX_FETCHES = 4;
  /** Estimated px per diff row — only sizes the placeholder of a file that is
   *  open but not fetched yet, so off-screen files don't all intersect at once. */
  const EST_ROW_PX = 17;

  let summary = $state.raw<SumResp | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let reloadTick = $state(0);
  let filePage = $state(FILE_PAGE);
  // Keyed per file path. SvelteMap values are NOT deep-proxied — a Body's
  // hunks stay plain objects.
  const bodies = new SvelteMap<string, Body>();
  const openOverride = new SvelteMap<string, boolean>();
  const rowLimit = new SvelteMap<string, number>();

  /** Bumped per (repo, sha) load; every async continuation checks it. */
  let gen = 0;
  let loadCtl: AbortController | null = null;
  let queue: { f: SumFile; full: boolean; gen: number }[] = [];
  let active = 0;

  $effect(() => {
    const id = repoId;
    const s = sha;
    const h = held;
    void reloadTick;
    untrack(() => reset());
    if (!s || h) return;
    const ctl = new AbortController();
    loadCtl = ctl;
    untrack(() => void loadSummary(id, s, ctl));
    return () => ctl.abort();
  });

  $effect(() => {
    fileCount = summary ? summary.files.length : null;
  });

  function reset(): void {
    gen++;
    queue = [];
    loadCtl?.abort();
    loadCtl = null;
    summary = null;
    error = null;
    loading = false;
    filePage = FILE_PAGE;
    bodies.clear();
    openOverride.clear();
    rowLimit.clear();
  }

  async function loadSummary(id: string, s: string, ctl: AbortController): Promise<void> {
    const my = gen;
    const key = diffKey(id, s);
    const cached = summaryCache.get(key);
    if (cached) {
      summary = cached;
      void loadSmallBodies(id, s, cached, ctl, my);
      return;
    }
    loading = true;
    try {
      // Join a prefetch in flight (see `prefetchCommitSummary`); if it was
      // aborted or failed, fetch on our own.
      const flight = summaryFlights.get(key);
      const joined = flight ? await flight.catch(() => null) : null;
      const resp = joined ?? (await api.get<SumResp>(summaryUrl(id, s), ctl.signal));
      if (my !== gen) return;
      summaryCache.set(key, resp);
      summary = resp;
      void loadSmallBodies(id, s, resp, ctl, my);
    } catch (e) {
      if (my !== gen || isAbortError(e)) return;
      error = (e instanceof Error ? e.message : String(e)) || 'The diff could not be loaded.';
    } finally {
      if (my === gen) loading = false;
    }
  }

  /** Is this summary an ordinary commit (the same test as `collapseAll`)? */
  function isSmall(r: SumResp): boolean {
    if (r.files.length > COLLAPSE_ALL_FILES) return false;
    let add = 0;
    let del = 0;
    for (const f of r.files) {
      const c = countOf(f);
      add += c.add;
      del += c.del;
    }
    return (r.total_added ?? add) + (r.total_deleted ?? del) <= COLLAPSE_ALL_LINES;
  }

  /** An ordinary commit: fetch every pending file's hunks in ONE request
   *  (instead of 1 + N per-file ones). Runs in the same tick the summary is
   *  assigned, so the files are marked loading BEFORE the first render's
   *  intersection callbacks could queue per-file fetches. Anything the bulk
   *  response doesn't carry (capped, omitted, or a failure) falls back to the
   *  per-file path. */
  async function loadSmallBodies(id: string, s: string, r: SumResp, ctl: AbortController, my: number): Promise<void> {
    if (!isSmall(r)) return;
    const pending: SumFile[] = [];
    for (const f of r.files) {
      if (!needsFetch(f)) continue;
      const hit = bodyCache.get(diffKey(id, s, f.path, ''));
      if (hit) bodies.set(f.path, hit);
      else pending.push(f);
    }
    if (pending.length === 0) return;
    for (const f of pending) bodies.set(f.path, { kind: 'loading' });
    let resp: SumResp | null = null;
    try {
      resp = await api.get<SumResp>(`/repos/${id}/diff?target=${encodeURIComponent(`commit:${s}`)}`, ctl.signal);
    } catch (e) {
      if (my !== gen || isAbortError(e)) return;
    }
    if (my !== gen) return;
    for (const f of pending) {
      const hit = resp ? fileFor(resp, f.path) : null;
      if (hit && !hit.too_large && !hit.hunks_omitted) {
        const body: Body = { kind: 'ready', hunks: hit.hunks, rows: rowsOf(hit.hunks) };
        bodyCache.set(diffKey(id, s, f.path, ''), body);
        bodies.set(f.path, body);
      } else {
        bodies.delete(f.path);
        // The intersection callback already fired for these; queue directly.
        if (isOpen(f)) enqueue(f);
      }
    }
  }

  // ── Counts / defaults (derived synchronously from the summary) ────────────
  /** Server counts; the line walk is only a fallback for a provider that
   *  leaves them null (the summary itself never needs hunks). */
  function countOf(f: SumFile): { add: number; del: number } {
    if (f.added != null && f.deleted != null) return { add: f.added, del: f.deleted };
    let add = 0;
    let del = 0;
    for (const h of f.hunks) for (const l of h.lines) {
      if (l.origin === 'add') add++;
      else if (l.origin === 'del') del++;
    }
    return { add, del };
  }

  const stats = $derived.by(() => {
    const m = new Map<string, { add: number; del: number }>();
    for (const f of summary?.files ?? []) m.set(f.path, countOf(f));
    return m;
  });
  const totals = $derived.by(() => {
    const s = summary;
    let add = 0;
    let del = 0;
    for (const c of stats.values()) {
      add += c.add;
      del += c.del;
    }
    return { add: s?.total_added ?? add, del: s?.total_deleted ?? del };
  });
  /** Everything starts collapsed on a big commit; otherwise only big files. */
  const collapseAll = $derived(
    (summary?.files.length ?? 0) > COLLAPSE_ALL_FILES || totals.add + totals.del > COLLAPSE_ALL_LINES,
  );
  const shownFiles = $derived(summary ? summary.files.slice(0, filePage) : []);

  function isOpen(f: SumFile): boolean {
    const o = openOverride.get(f.path);
    if (o !== undefined) return o;
    if (collapseAll || f.too_large) return false;
    const c = stats.get(f.path);
    return !!c && c.add + c.del <= COLLAPSE_FILE_LINES;
  }

  /** Hunks the summary response already carried (old daemon / no summary). */
  function inlineBody(f: SumFile): Body | null {
    if (f.hunks_omitted || f.too_large || f.is_binary) return null;
    if (f.hunks.length === 0 && (f.added ?? 0) + (f.deleted ?? 0) > 0) return null;
    return { kind: 'ready', hunks: f.hunks, rows: rowsOf(f.hunks) };
  }

  function bodyOf(f: SumFile): Body | undefined {
    return bodies.get(f.path) ?? inlineBody(f) ?? (f.too_large ? tooLarge(f, false) : undefined);
  }

  function tooLarge(f: SumFile, hard: boolean): Body {
    const c = stats.get(f.path) ?? countOf(f);
    return { kind: 'too_large', lines: c.add + c.del, hard };
  }

  function rowsOf(hunks: Hunk[]): number {
    let n = 0;
    for (const h of hunks) n += h.lines.length;
    return n;
  }

  function needsFetch(f: SumFile): boolean {
    return !f.is_binary && !f.too_large && !bodies.has(f.path) && inlineBody(f) === null;
  }

  // ── Per-file patch fetch (bounded concurrency, abortable, cached) ─────────
  function enqueue(f: SumFile, full = false): void {
    if (!sha) return;
    bodies.set(f.path, { kind: 'loading' });
    queue.push({ f, full, gen });
    pump();
  }

  function pump(): void {
    while (active < MAX_FETCHES && queue.length > 0) {
      const job = queue.shift()!;
      if (job.gen !== gen) continue;
      active++;
      void fetchBody(job.f, job.full, job.gen).finally(() => {
        active--;
        pump();
      });
    }
  }

  async function fetchBody(f: SumFile, full: boolean, my: number): Promise<void> {
    const s = sha;
    const id = repoId;
    const ctl = loadCtl;
    if (!s || !ctl) return;
    const key = diffKey(id, s, f.path, full ? 'full' : '');
    const cached = bodyCache.get(key);
    if (cached) {
      if (my === gen) bodies.set(f.path, cached);
      return;
    }
    const q = new URLSearchParams({ target: `commit:${s}`, path: f.path });
    if (f.old_path) q.set('old_path', f.old_path);
    if (full) q.set('full', 'true');
    try {
      const r = await api.get<SumResp>(`/repos/${id}/diff?${q}`, ctl.signal);
      if (my !== gen) return;
      const hit = fileFor(r, f.path) ?? r.files[0];
      let body: Body;
      if (!hit) body = { kind: 'ready', hunks: [], rows: 0 };
      else if (hit.too_large || hit.hunks_omitted) body = tooLarge(f, full);
      else body = { kind: 'ready', hunks: hit.hunks, rows: rowsOf(hit.hunks) };
      bodyCache.set(key, body);
      bodies.set(f.path, body);
    } catch (e) {
      if (my !== gen || isAbortError(e)) return;
      bodies.set(f.path, { kind: 'error', message: e instanceof Error ? e.message : String(e) });
    }
  }

  function retryFile(f: SumFile): void {
    bodies.delete(f.path);
    enqueue(f);
  }

  function toggle(f: SumFile): void {
    const next = !isOpen(f);
    openOverride.set(f.path, next);
    // A click means the header is on screen — fetch now, not on the next
    // intersection callback (which won't fire: visibility didn't change).
    if (next && needsFetch(f)) enqueue(f);
  }

  // ── Near-viewport detection ───────────────────────────────────────────────
  let io: IntersectionObserver | null = null;
  const byEl = new WeakMap<Element, SumFile>();

  function onIntersect(entries: IntersectionObserverEntry[]): void {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      const f = byEl.get(e.target);
      if (f && isOpen(f) && needsFetch(f)) enqueue(f);
    }
  }

  /** `use:nearView={file}` on each file block. */
  function nearView(node: HTMLElement, f: SumFile) {
    byEl.set(node, f);
    if (typeof IntersectionObserver === 'undefined') {
      if (isOpen(f) && needsFetch(f)) enqueue(f);
    } else {
      io ??= new IntersectionObserver(onIntersect, {
        root: node.closest('.detail-diff'),
        rootMargin: '400px 0px',
      });
      io.observe(node);
    }
    return {
      update(next: SumFile) {
        byEl.set(node, next);
      },
      destroy() {
        io?.unobserve(node);
      },
    };
  }
  $effect(() => () => {
    io?.disconnect();
    io = null;
  });

  // ── Rendering helpers ─────────────────────────────────────────────────────
  /** The first `limit` rows of a file, hunk headers kept with their lines. */
  function capHunks(hunks: Hunk[], limit: number): Hunk[] {
    const out: Hunk[] = [];
    let left = limit;
    for (const h of hunks) {
      if (left <= 0) break;
      out.push(h.lines.length <= left ? h : { header: h.header, lines: h.lines.slice(0, left) });
      left -= h.lines.length;
    }
    return out;
  }

  function placeholderPx(f: SumFile): number {
    const c = stats.get(f.path);
    return Math.min((c ? c.add + c.del : 1) + 6, ROW_CAP) * EST_ROW_PX;
  }

  function lineClass(origin: DiffLine['origin']): string {
    if (origin === 'add') return 'dl-add';
    if (origin === 'del') return 'dl-del';
    return 'dl-ctx';
  }

  function lineSign(origin: DiffLine['origin']): string {
    if (origin === 'add') return '+';
    if (origin === 'del') return '−';
    return ' ';
  }
</script>

<div class="detail-diff">
  {#if held}
    <div class="dd-held">
      <button class="btn small" onclick={() => onresume?.()}>Show this commit’s changes</button>
    </div>
  {:else if !sha || loading || (!summary && !error)}
    <div class="detail-diff-loading">
      <Skeleton rows={8} height={20} />
    </div>
  {:else if error}
    <LoadState what="this commit’s changes" {error} empty onretry={() => reloadTick++} />
  {:else if summary}
    <!-- Keyed on the summary object: a new commit remounts the blocks, so each
         one is observed afresh (a reused block whose path also changed in the
         previous commit would never get a new intersection callback). -->
    {#key summary}
      {#if summary.files.length === 0}
        <div class="dim dd-empty">No file changes.</div>
      {:else}
        <div class="diff-summary-bar">
          <span class="dim">{plural(summary.files.length, 'file')}</span>
          <span class="ds-add">+{totals.add}</span>
          <span class="ds-del">−{totals.del}</span>
          {#if collapseAll}
            <span class="dim dd-hint" title="Large commit — open a file to load its changes">collapsed · large commit</span>
          {/if}
        </div>

        {#each shownFiles as file (file.path)}
          {@const st = stats.get(file.path) ?? { add: 0, del: 0 }}
          {@const open = isOpen(file)}
          {@const body = open ? bodyOf(file) : undefined}
          <div class="df-block" class:open use:nearView={file}>
            <div class="df-head-row">
              <button class="df-head" onclick={() => toggle(file)} title={file.path} aria-expanded={open}>
                <span class="df-chevron dim" aria-hidden="true"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} /></span>
                <span class="mono df-path" dir="ltr">
                  {#if file.old_path}<span class="df-rename-from">{file.old_path}</span><span class="df-rename-arrow"> → </span>{/if}{file.path}
                </span>
                <span class="grow"></span>
                <span class="ds-add">+{st.add}</span>
                <span class="ds-del">−{st.del}</span>
              </button>
              <button
                class="df-tools"
                title="File actions"
                aria-label="File actions"
                onclick={(e) => { e.stopPropagation(); onfiletools?.(e, file.path); }}
              ><Icon name="more" size={13} /></button>
            </div>

            {#if open}
              {#if file.is_binary}
                <div class="df-note dim">Binary file — no text diff.</div>
              {:else if body?.kind === 'ready'}
                {@const limit = rowLimit.get(file.path) ?? ROW_CAP}
                {#if body.rows === 0}
                  <div class="df-note dim">No text changes.</div>
                {:else}
                  <div class="df-hunks" dir="ltr">
                    {#each capHunks(body.hunks, limit) as hunk, hi (hi)}
                      <div class="hunk-header mono">{hunk.header}</div>
                      <table class="dl-table">
                        <tbody>
                          {#each hunk.lines as line, li (li)}
                            <tr class="dl-row {lineClass(line.origin)}">
                              <td class="dl-gut dl-old">{line.old_line ?? ''}</td>
                              <td class="dl-gut dl-new">{line.new_line ?? ''}</td>
                              <td class="dl-sign">{lineSign(line.origin)}</td>
                              <td class="dl-code mono">{line.content}</td>
                            </tr>
                          {/each}
                        </tbody>
                      </table>
                    {/each}
                  </div>
                  {#if body.rows > limit}
                    <div class="df-note df-more">
                      <span class="dim">{(body.rows - limit).toLocaleString()} more lines hidden</span>
                      <button class="btn small" onclick={() => rowLimit.set(file.path, limit + ROW_STEP)}>
                        Show {Math.min(ROW_STEP, body.rows - limit).toLocaleString()} more
                      </button>
                    </div>
                  {/if}
                {/if}
              {:else if body?.kind === 'too_large'}
                <div class="df-note df-more">
                  {#if body.hard}
                    <span class="dim">Too large to show here — {body.lines.toLocaleString()} changed lines.</span>
                  {:else}
                    <span class="dim">Large file — {body.lines.toLocaleString()} changed lines.</span>
                    <button class="btn small" onclick={() => enqueue(file, true)}>Load anyway</button>
                  {/if}
                </div>
              {:else if body?.kind === 'error'}
                <div class="df-note df-more">
                  <span class="dd-err">Couldn’t load this file’s changes: {body.message}</span>
                  <button class="btn small" onclick={() => retryFile(file)}>Retry</button>
                </div>
              {:else}
                <div class="df-pending" style:min-height="{placeholderPx(file)}px" aria-busy="true">
                  <Skeleton rows={2} height={16} />
                </div>
              {/if}
            {/if}
          </div>
        {/each}

        {#if summary.files.length > filePage}
          <div class="df-note df-more dd-files-more">
            <span class="dim">{(summary.files.length - filePage).toLocaleString()} more files</span>
            <button class="btn small" onclick={() => (filePage += FILE_PAGE)}>Show {Math.min(FILE_PAGE, summary.files.length - filePage)} more files</button>
          </div>
        {/if}
      {/if}
    {/key}
  {/if}
</div>

<style>
  /* Diff area (the pane's own scroll container). */
  .detail-diff {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .detail-diff-loading,
  .dd-held {
    padding: 12px;
  }
  .dd-empty {
    padding: 18px;
    font-size: var(--fs-s);
    text-align: center;
  }
  .diff-summary-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    font-size: var(--fs-xs);
    position: sticky;
    top: 0;
    z-index: 1;
  }
  .dd-hint {
    margin-inline-start: auto;
  }
  .ds-add {
    color: var(--success);
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .ds-del {
    color: var(--danger);
    font-weight: 600;
    font-size: var(--fs-xs);
  }

  /* Per-file block. `content-visibility: auto` lets the engine skip layout
     and paint for blocks scrolled out of view; `auto` in the intrinsic size
     remembers each block's last rendered height so the scrollbar stays put. */
  .df-block {
    border-bottom: 1px solid var(--border);
    content-visibility: auto;
    contain-intrinsic-size: auto 27px;
  }
  .df-block.open {
    contain-intrinsic-size: auto 300px;
  }
  /* The header is a ROW, not a single button: the ⋯ file-tools button can't be
     nested inside the collapse button (invalid HTML, and the click would fold
     the file instead of opening the menu). */
  .df-head-row {
    display: flex;
    align-items: stretch;
    background: var(--surface-2);
  }
  .df-tools {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-m);
    line-height: 1;
    cursor: pointer;
  }
  .df-tools:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 7%, var(--surface-2));
  }
  .df-head {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    width: 100%;
    padding: 4px 10px;
    border: none;
    background: var(--surface-2);
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text);
    text-align: start;
    transition: background var(--dur-fast);
  }
  .df-head:hover {
    background: color-mix(in srgb, var(--accent) 7%, var(--surface-2));
  }
  .df-chevron {
    display: inline-flex;
    flex-shrink: 0;
  }
  /* Collapsed points in the reading direction: Icon mirrors chevronRight
     under RTL itself (DIRECTIONAL), so no flip here. */
  /* Rename separator: mirror the glyph in place so "from → to" reads right. */
  :global([dir='rtl']) .df-rename-arrow {
    display: inline-block;
    transform: scaleX(-1);
  }
  .df-path {
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
    color: var(--text);
  }
  .df-note {
    padding: 8px 12px;
    font-size: var(--fs-xs);
  }
  .df-more {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    border-top: 1px dashed var(--border);
  }
  .dd-files-more {
    border-top: none;
  }
  .dd-err {
    color: var(--danger);
  }
  .df-pending {
    padding: 8px 12px;
  }
  .df-hunks {
    overflow-x: auto;
  }

  .hunk-header {
    padding: 2px 10px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 7%, var(--surface));
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }

  /* Diff line table. Row count per file is capped (ROW_CAP + "Show more"), so
     auto table layout — which keeps long lines horizontally scrollable — only
     ever measures a bounded number of cells. */
  .dl-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-xs);
    line-height: 1.5;
  }
  .dl-gut {
    width: 34px;
    min-width: 34px;
    text-align: end;
    padding-block: 0; padding-inline: 2px 4px;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    user-select: none;
    vertical-align: top;
    border-inline-end: 1px solid var(--border);
  }
  .dl-sign {
    width: 14px;
    text-align: center;
    color: var(--text-dim);
    user-select: none;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    vertical-align: top;
    padding: 0 1px;
  }
  .dl-code {
    padding-block: 0; padding-inline: 2px 8px;
    white-space: pre;
    word-break: normal;
    user-select: text;
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  tr.dl-row.dl-add {
    background: color-mix(in srgb, var(--success) 11%, transparent);
  }
  tr.dl-row.dl-add .dl-sign {
    color: var(--success);
  }
  tr.dl-row.dl-del {
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
  tr.dl-row.dl-del .dl-sign {
    color: var(--danger);
  }
  tr.dl-row.dl-ctx {
    color: var(--text-dim);
  }

  .grow {
    flex: 1;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* Phone / tablet (the graph's stacked accordion, see GraphView `.mobile`). */
  @media (max-width: 1024px) {
    :global(.mobile) .detail-diff { -webkit-overflow-scrolling: touch; overscroll-behavior: contain; }
    :global(.mobile) .diff-summary-bar { font-size: var(--fs-m); padding: 8px 12px; }
    :global(.mobile) .ds-add,
    :global(.mobile) .ds-del { font-size: var(--fs-m); }
    :global(.mobile) .df-head { font-size: var(--fs-m); padding: 8px 12px; }
    :global(.mobile) .df-path { font-size: var(--fs-m); }
    :global(.mobile) .hunk-header { font-size: var(--fs-s); padding: 4px 10px; }
    /* table-layout:fixed pins the gutter/sign columns to their declared widths
       and hands the rest to the code column, so a long unbroken line wraps
       INSIDE that column instead of widening the table past the viewport. */
    :global(.mobile) .df-hunks { overflow-x: hidden; }
    :global(.mobile) .dl-table { font-size: var(--fs-s); table-layout: fixed; width: 100%; }
    :global(.mobile) .dl-code {
      font-size: var(--fs-s);
      width: auto;
      white-space: pre-wrap;
      word-break: break-word;
      overflow-wrap: anywhere;
      overflow: visible;
      text-overflow: clip;
    }
    :global(.mobile) .dl-gut { font-size: var(--fs-xs); width: 30px; min-width: 30px; }
    :global(.mobile) .dl-sign { font-size: var(--fs-s); }
  }
</style>
