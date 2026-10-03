<script lang="ts">
  // `git blame` for one file, opened from a diff header's ⋯ menu (via
  // `gitBridge`). The daemon returns one row per RUN of consecutive lines
  // with the run's source text, so each row shows who/when next to the code
  // itself (syntax-highlighted, deferred). Clicking the commit selects it in
  // the graph; "Blame before this change" re-blames the file at the commit's
  // parent (following renames via porcelain `previous`), with Back to return.
  import type { BlameLine, BlameResp } from '../../lib/api/types';
  import { api } from '../../lib/api/client';
  import { gitBridge } from './gitBridge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { TableWindow } from '../../lib/tableWindow.svelte';
  import { ensureHljs, langFromPath } from '../../lib/hl';
  import { DeferredHighlighter } from './diff-highlight.svelte';

  interface Props {
    repoId: string;
    path: string;
    /** Revision to blame; the daemon defaults to HEAD. */
    rev?: string;
    onclose: () => void;
  }
  let { repoId, path, rev, onclose }: Props = $props();

  /** "Blame before this change" hops, newest last; empty = the props target. */
  let hops = $state<{ rev: string; path: string }[]>([]);
  // A new file/rev from the parent starts a fresh trail.
  $effect(() => {
    void path;
    void rev;
    hops = [];
  });
  const target = $derived(hops.at(-1) ?? { rev: rev ?? 'HEAD', path });

  const hl = new DeferredHighlighter();
  let hlReady = $state(false);
  $effect(() => {
    void ensureHljs().then(() => (hlReady = true));
    return () => hl.clear();
  });
  const lang = $derived(hlReady ? langFromPath(target.path) : null);

  let blame = $state.raw<BlameResp | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let retryRev = $state(0);

  $effect(() => {
    void retryRev;
    const id = repoId;
    const p = target.path;
    const r = target.rev;
    loading = true;
    error = null;
    // Switching files aborts the previous blame; a late response is dropped.
    const ctl = new AbortController();
    api
      .get<BlameResp>(
        `/repos/${id}/blame?path=${encodeURIComponent(p)}&rev=${encodeURIComponent(r)}`,
        ctl.signal,
      )
      .then((b) => {
        if (!ctl.signal.aborted) blame = b;
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted) return;
        blame = null;
        error = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) loading = false;
      });
    return () => ctl.abort();
  });

  // Windowed: a generated or long-lived file blames to 20k+ runs, and every
  // run was a real <tr> (≈670 ms to mount). Past 400 runs only the rows in
  // view (+ overscan) render between two spacer rows (`TableWindow`).
  const tw = new TableWindow(400, 15);
  let bodyEl = $state<HTMLDivElement | undefined>();
  const lines = $derived(blame?.lines ?? []);
  /** One display row per SOURCE line (uniform height, which `TableWindow`
   *  needs); the run's commit shows on its first line only. An older daemon
   *  without `text` gets one row per run with its line range. */
  interface Row {
    run: BlameLine;
    first: boolean;
    n: string;
    code: string | null;
  }
  const rows = $derived.by((): Row[] => {
    const out: Row[] = [];
    for (const run of lines) {
      const text = run.text ?? [];
      if (text.length === 0) out.push({ run, first: true, n: range(run.line_start, run.count), code: null });
      else text.forEach((t, k) => out.push({ run, first: k === 0, n: String(run.line_start + k), code: t }));
    }
    return out;
  });
  const win = $derived(tw.range(rows.length));
  $effect(() => {
    void win;
    tw.measure(bodyEl);
  });
  // ⌘F over every blame run, not just the mounted slice.
  $effect(() =>
    tw.findRows(
      () => bodyEl,
      () => rows,
      (r) => [r.n, r.first ? `${r.run.author}\n${r.run.short_sha}\n${day(r.run.at)}` : '', r.code ?? r.run.summary].join('\n'),
    ),
  );
  // A new blame starts at the top.
  $effect(() => {
    void blame;
    tw.reset(bodyEl);
  });
  /** Widest line range, so the gutter keeps one width while rows window in
   *  and out (auto table layout sizes columns from the MOUNTED rows only). */
  const gutterCh = $derived.by(() => {
    let max = 0;
    for (const l of lines) max = Math.max(max, l.line_start + l.count - 1);
    // Per-line numbers; a text-less (older daemon) run shows "a–b".
    return lines.some((l) => !l.text?.length) ? String(max).length * 2 + 1 : String(max).length;
  });

  function day(at: string): string {
    const d = new Date(at);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString();
  }

  function blameBefore(l: BlameLine): void {
    if (!l.previous) return;
    hops = [...hops, { rev: l.previous.sha, path: l.previous.path }];
  }

  /** "12" for a single line, "12–15" for a run. */
  function range(start: number, count: number): string {
    return count > 1 ? `${start}–${start + count - 1}` : `${start}`;
  }
</script>

<section class="bl" aria-label="Blame">
  <header class="bl-head">
    <Icon name="user" size={13} />
    <span class="bl-title mono" title={target.path}>{target.path}</span>
    {#if blame}<span class="chip mono">{blame.rev.length === 40 ? blame.rev.slice(0, 8) : blame.rev}</span>{/if}
    <span class="grow"></span>
    {#if hops.length > 0}
      <button
        class="btn small ghost"
        onclick={() => (hops = hops.slice(0, -1))}
        title="Back to the previous blame"
        aria-label="Back to the previous blame"
      >
        <Icon name="undo" size={12} /> Back
      </button>
    {/if}
    <button class="icon-btn" onclick={onclose} aria-label="Close blame" title="Close blame">
      <Icon name="x" size={14} />
    </button>
  </header>

  <div class="bl-body" bind:this={bodyEl} bind:clientHeight={tw.viewH} onscroll={tw.onscroll}>
    {#if loading}
      <div class="bl-pad"><Skeleton rows={6} height={22} /></div>
    {:else if error}
      <div class="bl-pad"><LoadState what="blame" {error} empty onretry={() => retryRev++} variant="compact" /></div>
    {:else if !blame || lines.length === 0}
      <p class="bl-msg">Nothing to blame — the file is empty at this revision.</p>
    {:else}
      <table class="bl-table" style="--bl-gutter:{gutterCh}ch">
        <tbody>
          {#if win.top}<tr class="tw-spacer" aria-hidden="true"><td colspan="4" style="height:{win.top}px"></td></tr>{/if}
          {#each rows.slice(win.start, win.end) as r, j (`${r.run.sha}-${r.n}-${win.start + j}`)}
            {@const l = r.run}
            <tr class:bl-run-start={r.first}>
              <td class="bl-gutter mono">{r.n}</td>
              <td class="bl-who">
                {#if r.first}
                  <button
                    class="bl-commit"
                    onclick={() => gitBridge.focusCommit(repoId, l.sha)}
                    title={l.summary}
                  >
                    <span class="bl-author">{l.author}</span>
                    <span class="mono bl-sha">{l.short_sha}</span>
                    <span class="bl-date">{day(l.at)}</span>
                  </button>
                {/if}
              </td>
              <td class="bl-act">
                {#if r.first && l.previous}
                  <button
                    class="icon-btn bl-before"
                    onclick={() => blameBefore(l)}
                    title="Blame before this change ({l.short_sha}^)"
                    aria-label="Blame before this change ({l.short_sha})"
                  >
                    <Icon name="clock" size={12} />
                  </button>
                {/if}
              </td>
              {#if r.code === null}
                <td class="bl-code bl-summary" title={l.summary}>{l.summary}</td>
              {:else}
                <td class="bl-code mono" title={l.summary}>{@html hl.html(r.code, lang)}</td>
              {/if}
            </tr>
          {/each}
          {#if win.bottom}<tr class="tw-spacer" aria-hidden="true"><td colspan="4" style="height:{win.bottom}px"></td></tr>{/if}
        </tbody>
      </table>
    {/if}
  </div>
</section>

<style>
  .bl {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--surface);
    border-inline-start: 1px solid var(--border);
  }
  .bl-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  .bl-title {
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .bl-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .bl-pad {
    padding: 10px;
  }
  .bl-msg {
    padding: 14px 12px;
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .bl-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-xs);
  }
  .bl-table tr:hover {
    background: var(--surface-2);
  }
  .tw-spacer td {
    padding: 0;
    border: 0;
  }
  .bl-gutter {
    width: 1%;
    min-width: var(--bl-gutter);
    box-sizing: content-box;
    white-space: nowrap;
    padding: 1px 8px;
    color: var(--text-dim);
    text-align: end;
    vertical-align: top;
    border-inline-end: 1px solid var(--border);
  }
  .bl-who {
    width: 1%;
    white-space: nowrap;
    padding: 0 8px;
    vertical-align: top;
  }
  .bl-commit {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    padding: 1px 0;
    cursor: pointer;
  }
  .bl-commit:hover .bl-sha {
    color: var(--accent-text);
  }
  .bl-sha,
  .bl-date {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .bl-table tr.bl-run-start td {
    border-block-start: 1px solid var(--border);
  }
  .bl-act {
    width: 1%;
    padding: 1px 2px;
    vertical-align: top;
  }
  .bl-before {
    opacity: 0.6;
  }
  .bl-before:hover,
  .bl-before:focus-visible {
    opacity: 1;
  }
  .bl-code {
    padding: 1px 8px;
    vertical-align: top;
    overflow: hidden;
    max-width: 0;
    color: var(--text);
    white-space: pre;
    text-overflow: ellipsis;
  }
  .bl-summary {
    color: var(--text-dim);
    font-family: inherit;
  }
</style>
