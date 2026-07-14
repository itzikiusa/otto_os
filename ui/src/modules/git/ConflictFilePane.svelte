<script lang="ts">
  // One conflicted file: loads its segmented view, renders context verbatim
  // (dim) and each conflict as a ConflictHunk (A/B side + per-line picking).
  // Below the hunks sits a GitKraken-style OUTPUT pane: a live preview of the
  // recomposed file (context + chosen lines), with "conflict k of m" navigation
  // that scrolls the hunk list. When every conflict has a choice the file can
  // be "marked resolved" — we recompose the full file text and POST it.
  import type { ConflictFile, ConflictSegment } from '../../lib/api/types';
  import { git } from '../../lib/stores/git.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ConflictHunk from './ConflictHunk.svelte';

  interface Props {
    repoId: string;
    path: string;
    /** Side labels for the A/B pickers (checked-out branch vs merge source). */
    oursLabel?: string;
    theirsLabel?: string;
    /** Called once the file has been marked resolved on the daemon. */
    onresolved: () => void;
  }
  let { repoId, path, oursLabel = 'OURS', theirsLabel = 'THEIRS', onresolved }: Props = $props();

  let file = $state<ConflictFile | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let saving = $state(false);

  // Per-conflict chosen lines. Indexed by the conflict's ordinal position
  // among `conflict` segments (0-based). null = undecided.
  //
  // $state.raw, NOT $state: the values come from ConflictHunk as plain arrays
  // and setChoice's no-change guard compares them by IDENTITY. A deep proxy
  // would re-wrap the stored array, the guard would never match, and the
  // hunk's onresolve effect → setChoice → re-render cycle would spin until
  // Svelte aborts it (effect_update_depth_exceeded). The array is only ever
  // replaced wholesale, so raw loses nothing.
  let choices = $state.raw<(string[] | null)[]>([]);

  $effect(() => {
    // Re-load whenever the selected file changes.
    const id = repoId;
    const p = path;
    loading = true;
    loadError = null;
    file = null;
    choices = [];
    void git
      .getConflictFile(id, p)
      .then((f) => {
        file = f;
        const conflictCount = f.segments.filter((s) => s.kind === 'conflict').length;
        choices = new Array(conflictCount).fill(null);
        current = 0;
      })
      .catch((e) => {
        loadError = e instanceof Error ? e.message : String(e);
      })
      .finally(() => {
        loading = false;
      });
  });

  // Map a segment array index → its conflict ordinal (or -1 for context).
  function conflictOrdinal(segments: ConflictSegment[], segIdx: number): number {
    let ord = -1;
    for (let i = 0; i <= segIdx; i++) {
      if (segments[i].kind === 'conflict') ord++;
    }
    return ord;
  }

  const conflictCount = $derived(
    file ? file.segments.filter((s) => s.kind === 'conflict').length : 0
  );
  const decidedCount = $derived(choices.filter((c) => c !== null).length);
  const allDecided = $derived(conflictCount > 0 && decidedCount === conflictCount);

  function setChoice(ordinal: number, lines: string[] | null): void {
    if (choices[ordinal] === lines) return;
    const next = [...choices];
    next[ordinal] = lines;
    choices = next;
  }

  // ── Conflict navigation (Output pane header) ────────────────────────────────
  // 0-based index of the "current" conflict; prev/next scroll its hunk into
  // view in the hunk list so long files stay navigable.
  let current = $state(0);
  let hunkEls: (HTMLElement | null)[] = $state([]);

  function goTo(ord: number): void {
    if (conflictCount === 0) return;
    const next = ((ord % conflictCount) + conflictCount) % conflictCount;
    current = next;
    hunkEls[next]?.scrollIntoView({ behavior: 'smooth', block: 'center' });
  }

  // ── Output preview ──────────────────────────────────────────────────────────
  let outputOpen = $state(true);

  type OutSeg =
    | { kind: 'context'; text: string }
    | { kind: 'resolved'; ord: number; text: string; deleted: boolean }
    | { kind: 'unresolved'; ord: number };

  /** The live recomposition shown in the Output pane: context verbatim, each
   *  conflict either its chosen lines or an explicit "unresolved" marker. */
  const outputSegs = $derived.by((): OutSeg[] => {
    if (!file) return [];
    const out: OutSeg[] = [];
    let ord = -1;
    for (const seg of file.segments) {
      if (seg.kind === 'context') {
        if (seg.lines.length > 0) out.push({ kind: 'context', text: seg.lines.join('\n') });
      } else {
        ord++;
        const choice = choices[ord];
        if (choice === null || choice === undefined) {
          out.push({ kind: 'unresolved', ord });
        } else {
          out.push({
            kind: 'resolved',
            ord,
            text: choice.join('\n'),
            deleted: choice.length === 0,
          });
        }
      }
    }
    return out;
  });

  /**
   * Recompose the full file text from the segments + the user's choices:
   * context segments contribute their lines verbatim; conflict segments
   * contribute the chosen lines. Lines are joined with "\n" and a trailing
   * newline is added (git's normalised file form).
   */
  function composeContent(): string {
    if (!file) return '';
    const out: string[] = [];
    let ord = -1;
    for (const seg of file.segments) {
      if (seg.kind === 'context') {
        out.push(...seg.lines);
      } else {
        ord++;
        out.push(...(choices[ord] ?? []));
      }
    }
    return out.length === 0 ? '' : out.join('\n') + '\n';
  }

  async function markResolved(): Promise<void> {
    if (!file || !allDecided || saving) return;
    saving = true;
    try {
      const content = composeContent();
      await git.resolveConflict(repoId, path, content);
      toasts.success('File resolved', path);
      onresolved();
    } catch (e) {
      toasts.error('Resolve failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="pane">
  <div class="pane-head">
    <span class="mono pane-path" title={path}>{path}</span>
    <span class="grow"></span>
    {#if conflictCount > 0}
      <span class="progress" class:done={allDecided}>
        {decidedCount}/{conflictCount} resolved
      </span>
    {/if}
    <button
      class="btn small primary"
      disabled={!allDecided || saving}
      onclick={markResolved}
      title={allDecided ? 'Mark this file resolved' : 'Resolve every conflict first'}
    >
      {saving ? 'Saving…' : 'Mark file resolved'}
    </button>
  </div>

  <div class="pane-body">
    {#if loading}
      <div style="padding: 12px"><Skeleton rows={8} height={20} /></div>
    {:else if loadError}
      <div class="load-error">
        <Icon name="info" size={14} />
        <span>Failed to load conflict: {loadError}</span>
      </div>
    {:else if file}
      {#if file.is_binary}
        <div class="binary dim">
          Binary file — choose a side via the file list, or resolve it on the command line.
        </div>
      {:else}
        {#each file.segments as seg, si (si)}
          {#if seg.kind === 'context'}
            {#if seg.lines.length > 0}
              <pre class="context mono">{seg.lines.join('\n')}</pre>
            {/if}
          {:else}
            {@const ord = conflictOrdinal(file.segments, si)}
            <div class="hunk-anchor" bind:this={hunkEls[ord]}>
              <ConflictHunk
                ours={seg.ours}
                theirs={seg.theirs}
                base={seg.base}
                index={ord + 1}
                {path}
                {oursLabel}
                {theirsLabel}
                onresolve={(lines) => setChoice(ord, lines)}
              />
            </div>
          {/if}
        {/each}
        {#if conflictCount === 0}
          <div class="dim" style="padding: 16px; font-size: 12px">
            No conflict markers in this file. Mark it resolved to continue.
          </div>
        {/if}
      {/if}
    {/if}
  </div>

  <!-- ── Output: live preview of the resolved file ── -->
  {#if file && !file.is_binary && conflictCount > 0}
    <div class="output" class:open={outputOpen}>
      <div class="output-bar">
        <button
          class="output-toggle"
          onclick={() => (outputOpen = !outputOpen)}
          aria-expanded={outputOpen}
        >
          <Icon name={outputOpen ? 'chevronDown' : 'chevronRight'} size={12} />
          <span class="output-title">Output</span>
        </button>
        <span class="grow"></span>
        <span class="output-nav">
          <span class="output-count">conflict {Math.min(current + 1, conflictCount)} of {conflictCount}</span>
          <button
            class="nav-btn"
            onclick={() => goTo(current - 1)}
            title="Previous conflict"
            aria-label="Previous conflict"
          >
            <Icon name="chevronUp" size={12} />
          </button>
          <button
            class="nav-btn"
            onclick={() => goTo(current + 1)}
            title="Next conflict"
            aria-label="Next conflict"
          >
            <Icon name="chevronDown" size={12} />
          </button>
        </span>
      </div>
      {#if outputOpen}
        <div class="output-body">
          {#each outputSegs as seg, i (i)}
            {#if seg.kind === 'context'}
              <pre class="out-context mono">{seg.text}</pre>
            {:else if seg.kind === 'resolved'}
              {#if seg.deleted}
                <div class="out-deleted dim mono">‹conflict {seg.ord + 1}: block removed›</div>
              {:else}
                <pre class="out-resolved mono">{seg.text}</pre>
              {/if}
            {:else}
              <button class="out-unresolved" onclick={() => goTo(seg.ord)} title="Jump to conflict {seg.ord + 1}">
                <Icon name="merge" size={11} />
                conflict {seg.ord + 1} — unresolved
              </button>
            {/if}
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .pane-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    flex-shrink: 0;
  }
  .pane-path {
    font-size: 12px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .progress {
    font-size: 10.5px;
    font-weight: 600;
    color: var(--text-dim);
    padding: 2px 7px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    white-space: nowrap;
  }
  .progress.done {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .pane-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .hunk-anchor {
    scroll-margin: 12px;
  }
  .context {
    margin: 0;
    padding: 4px 10px;
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 4%, transparent);
    border-radius: var(--radius-s);
    white-space: pre-wrap;
    word-break: break-all;
    overflow-x: auto;
  }
  .binary {
    padding: 16px;
    font-size: 12px;
  }
  .load-error {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 16px;
    font-size: 12px;
    color: var(--status-exited);
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* ── Output preview pane ── */
  .output {
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .output.open {
    flex-basis: 34%;
    max-height: 42%;
  }
  .output-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .output-toggle {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    cursor: pointer;
    padding: 2px 0;
  }
  .output-nav {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .output-count {
    font-size: 10.5px;
    font-weight: 600;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .nav-btn {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
  }
  .nav-btn:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .output-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 6px 10px;
  }
  .out-context {
    margin: 0;
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-dim);
    white-space: pre-wrap;
    word-break: break-all;
  }
  .out-resolved {
    margin: 0;
    font-size: 11px;
    line-height: 1.5;
    color: var(--text);
    background: color-mix(in srgb, var(--status-working) 10%, transparent);
    border-inline-start: 2px solid color-mix(in srgb, var(--status-working) 60%, transparent);
    padding: 1px 8px;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .out-deleted {
    font-size: 10.5px;
    font-style: italic;
    padding: 1px 8px;
    border-inline-start: 2px solid var(--border);
  }
  .out-unresolved {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border: 1px dashed color-mix(in srgb, var(--status-warn) 55%, transparent);
    border-radius: var(--radius-s);
    background: var(--status-warn-soft);
    color: var(--status-warn);
    font-size: 11px;
    font-weight: 600;
    padding: 3px 8px;
    margin: 3px 0;
    cursor: pointer;
    text-align: start;
  }

  /* ── Mobile + tablet (≤1024px) ── */
  @media (max-width: 1024px) {
    .pane-head {
      flex-wrap: wrap;
      gap: 6px 10px;
    }
    .pane-path {
      flex-basis: 100%;
      font-size: 13px;
    }
    .pane-head .btn {
      min-height: 40px;
    }
    .pane-body {
      padding: 10px;
      -webkit-overflow-scrolling: touch;
    }
    .context {
      font-size: 12.5px;
      word-break: break-word;
      overflow-wrap: anywhere;
    }
    .output.open {
      flex-basis: 40%;
      max-height: 45%;
    }
    .nav-btn {
      width: 32px;
      height: 32px;
    }
    .output-toggle {
      min-height: 32px;
    }
  }
</style>
