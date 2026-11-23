<script lang="ts">
  // Multi-select skill picker for MatrixView, backed by `skillsEvalApi.listSources(wsId)`
  // so matrix creation cannot fan out failing cells from comma-separated typos.
  import { onDestroy } from 'svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import { loadErrorText } from '../../lib/loadError';
  import LoadState from '../../lib/components/LoadState.svelte';
  import type { SkillSourceInfo, SkillSourceReq } from '../../lib/api/types';

  interface Props {
    wsId: string;
    selected: SkillSourceReq[];
  }
  let { wsId, selected = $bindable([]) }: Props = $props();

  let sources = $state<SkillSourceInfo[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let filter = $state('');
  let seq = 0;
  let disposed = false;

  onDestroy(() => {
    disposed = true;
    seq++;
  });

  function keyOf(s: { kind: string; reference?: string; name?: string; provider?: string | null }): string {
    return `${s.kind}\u0000${s.provider ?? ''}\u0000${s.reference ?? s.name ?? ''}`;
  }

  async function loadSources(targetWs: string): Promise<void> {
    const my = ++seq;
    loading = true;
    error = null;
    try {
      const resp = await skillsEvalApi.listSources(targetWs);
      if (disposed || my !== seq) return;
      sources = resp.sources;
      const valid = new Set(sources.map((s) => keyOf(s)));
      selected = selected.filter((s) => valid.has(keyOf(s)));
    } catch (e) {
      if (disposed || my !== seq) return;
      error = loadErrorText(e);
    } finally {
      if (!disposed && my === seq) loading = false;
    }
  }

  $effect(() => {
    const id = wsId;
    if (!id) {
      sources = [];
      error = null;
      loading = false;
      return;
    }
    void loadSources(id);
  });

  const selectedKeys = $derived(new Set(selected.map((s) => keyOf(s))));
  const visible = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return sources;
    return sources.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        (s.provider ?? '').toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q),
    );
  });

  function toggle(src: SkillSourceInfo): void {
    const k = keyOf(src);
    if (selectedKeys.has(k)) {
      selected = selected.filter((s) => keyOf(s) !== k);
    } else {
      selected = [
        ...selected,
        { kind: src.kind, reference: src.name, provider: src.provider ?? null },
      ];
    }
  }
</script>

<div class="msp" data-testid="matrix-skills">
  <LoadState
    what="skills"
    variant="compact"
    {loading}
    {error}
    empty={sources.length === 0}
    onretry={() => wsId && void loadSources(wsId)}
  >
    {#snippet emptyView()}
      <p class="msp-empty">No skills found in this workspace or library yet.</p>
    {/snippet}
    {#if sources.length > 8}
      <input
        type="search"
        dir="auto"
        class="input msp-filter"
        placeholder="Filter skills…"
        aria-label="Filter available skills"
        bind:value={filter}
      />
    {/if}
    <div class="msp-chips" role="group" aria-label="Skills to include">
      {#each visible as src (keyOf(src))}
        {@const on = selectedKeys.has(keyOf(src))}
        <label class="chip-toggle" class:on title={src.description || src.name}>
          <input type="checkbox" checked={on} onchange={() => toggle(src)} />
          <span class="mono">{src.name}</span>
          {#if src.kind === 'provider' && src.provider}
            <span class="msp-src">{src.provider}</span>
          {/if}
        </label>
      {/each}
    </div>
    {#if visible.length === 0 && filter.trim()}
      <p class="msp-empty">No skills match “{filter.trim()}”.</p>
    {/if}
  </LoadState>
</div>

<style>
  .msp {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .msp-filter {
    max-inline-size: 280px;
  }
  .msp-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    max-block-size: 168px;
    overflow-y: auto;
    padding: 2px;
  }
  .chip-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: var(--fs-s);
    color: var(--text);
    cursor: pointer;
    user-select: none;
  }
  .chip-toggle:hover {
    border-color: var(--border-strong);
  }
  .chip-toggle.on {
    border-color: var(--accent-text);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .chip-toggle input {
    margin: 0;
  }
  .msp-src {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .msp-empty {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
