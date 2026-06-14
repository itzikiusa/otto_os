<script lang="ts">
  // One conflicted file: loads its segmented view, renders context verbatim
  // (dim) and each conflict as a ConflictHunk. When every conflict has a choice
  // the file can be "marked resolved" — we recompose the full file text (context
  // lines + each conflict's chosen lines, in segment order) and POST it.
  import type { ConflictFile, ConflictSegment } from '../../lib/api/types';
  import { git } from '../../lib/stores/git.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ConflictHunk from './ConflictHunk.svelte';

  interface Props {
    repoId: string;
    path: string;
    /** Called once the file has been marked resolved on the daemon. */
    onresolved: () => void;
  }
  let { repoId, path, onresolved }: Props = $props();

  let file = $state<ConflictFile | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let saving = $state(false);

  // Per-conflict chosen lines. Indexed by the conflict's ordinal position
  // among `conflict` segments (0-based). null = undecided.
  let choices = $state<(string[] | null)[]>([]);

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
            <ConflictHunk
              ours={seg.ours}
              theirs={seg.theirs}
              base={seg.base}
              index={ord + 1}
              {path}
              onresolve={(lines) => setChoice(ord, lines)}
            />
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
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
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
</style>
