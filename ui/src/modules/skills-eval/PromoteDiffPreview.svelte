<script lang="ts">
  // Pre-Promote Skill Diff Preview: compares the currently installed library
  // copy (if any) against the winning/selected evaluation candidate before the
  // user confirms Promote.
  import { onDestroy } from 'svelte';
  import { ApiError } from '../../lib/api/client';
  import { skillLabApi } from '../../lib/api/skillLab';
  import { loadErrorText } from '../../lib/loadError';
  import DiffView from '../../lib/components/DiffView.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';

  interface Props {
    name: string;
    candidate: string;
    invalidName?: boolean;
  }
  let { name, candidate, invalidName = false }: Props = $props();

  type LookupState =
    | { kind: 'idle' }
    | { kind: 'loading' }
    | { kind: 'new' }
    | { kind: 'found'; content: string }
    | { kind: 'error'; message: string };

  let lookup = $state<LookupState>({ kind: 'idle' });
  let diffMode = $state<'word' | 'split'>('word');
  let seq = 0;
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  onDestroy(() => {
    disposed = true;
    seq++;
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  });

  async function fetchCurrent(skillName: string): Promise<void> {
    const my = ++seq;
    lookup = { kind: 'loading' };
    try {
      const resp = await skillLabApi.getFile(skillName, 'SKILL.md');
      if (disposed || my !== seq) return;
      lookup = { kind: 'found', content: resp.content };
    } catch (e) {
      if (disposed || my !== seq) return;
      if (e instanceof ApiError && e.status === 404) {
        lookup = { kind: 'new' };
      } else {
        lookup = { kind: 'error', message: loadErrorText(e) };
      }
    }
  }

  $effect(() => {
    const trimmed = name.trim();
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    if (!trimmed || invalidName) {
      seq++;
      lookup = { kind: 'idle' };
      return;
    }
    timer = setTimeout(() => {
      timer = null;
      void fetchCurrent(trimmed);
    }, 180);
  });

  const identical = $derived(
    lookup.kind === 'found' && lookup.content.trimEnd() === candidate.trimEnd(),
  );
</script>

{#if lookup.kind !== 'idle'}
  <div class="pdp" data-testid="promote-diff-preview">
    <div class="pdp-head">
      <span class="pdp-title">Changes vs. installed library skill</span>
      {#if lookup.kind === 'found' && !identical}
        <div class="pdp-modes" role="group" aria-label="Diff layout">
          <button
            type="button"
            class="btn small ghost"
            aria-pressed={diffMode === 'word'}
            onclick={() => (diffMode = 'word')}
          >Unified</button>
          <button
            type="button"
            class="btn small ghost"
            aria-pressed={diffMode === 'split'}
            onclick={() => (diffMode = 'split')}
          >Split</button>
        </div>
      {/if}
    </div>

    {#if lookup.kind === 'loading'}
      <LoadState what="current library skill" variant="compact" loading empty rows={3} />
    {:else if lookup.kind === 'error'}
      <LoadState
        what="current library skill"
        variant="compact"
        error={lookup.message}
        empty
        onretry={() => void fetchCurrent(name.trim())}
      />
    {:else if lookup.kind === 'new'}
      <p class="pdp-note" data-testid="promote-diff-new">
        <Icon name="plus" size={12} /> New library skill — no existing copy of <strong class="mono">{name.trim()}</strong> will be overwritten.
      </p>
    {:else if identical}
      <p class="pdp-note" data-testid="promote-diff-same">
        <Icon name="check" size={12} /> Identical to the currently installed library copy.
      </p>
    {:else}
      <div class="pdp-diff" data-testid="promote-diff-body">
        <DiffView before={lookup.content} after={candidate} mode={diffMode} contextLines={3} />
      </div>
    {/if}
  </div>
{/if}

<style>
  .pdp {
    margin-block-start: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .pdp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
  }
  .pdp-title {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .pdp-modes {
    display: inline-flex;
    gap: 4px;
  }
  .pdp-modes [aria-pressed='true'] {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .pdp-note {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .pdp-diff {
    max-block-size: 280px;
    overflow: auto;
    border-radius: var(--radius-s);
  }
</style>
