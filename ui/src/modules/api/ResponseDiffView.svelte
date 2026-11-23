<script lang="ts">
  // Response diff panel: compares the current API response against either a
  // pinned baseline or the immediately previous send in this tab using Otto's
  // shared <DiffView> with canonical key-sorted JSON formatting.
  import DiffView from '../../lib/components/DiffView.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { formatBytes } from '../../lib/metric-format';
  import { formatForDiff, summarizeResponseDelta, type ResponseSnapshot } from './responseDiff';

  interface Props {
    current: ResponseSnapshot;
    pinned: ResponseSnapshot | null;
    previous: ResponseSnapshot | null;
    onpin: () => void;
    onunpin: () => void;
  }
  let { current, pinned, previous, onpin, onunpin }: Props = $props();

  let target = $state<'pinned' | 'previous'>('pinned');
  let mode = $state<'word' | 'split'>('word');
  let canonical = $state(true);

  const effectiveTarget = $derived<'pinned' | 'previous' | null>(
    target === 'pinned'
      ? (pinned ? 'pinned' : previous ? 'previous' : null)
      : (previous ? 'previous' : pinned ? 'pinned' : null),
  );
  const baseline = $derived(effectiveTarget === 'pinned' ? pinned : effectiveTarget === 'previous' ? previous : null);
  const delta = $derived(baseline ? summarizeResponseDelta(baseline, current, canonical) : null);
  const beforeFmt = $derived(baseline ? formatForDiff(baseline.body, baseline.content_type, canonical) : null);
  const afterFmt = $derived(formatForDiff(current.body, current.content_type, canonical));

  function fmtSignedMs(ms: number): string {
    return `${ms >= 0 ? '+' : ''}${Math.round(ms)} ms`;
  }
  function fmtSignedBytes(b: number): string {
    return `${b >= 0 ? '+' : '−'}${formatBytes(Math.abs(b))}`;
  }
</script>

<div class="rdiff">
  {#if !baseline || !delta || !beforeFmt}
    <EmptyState
      icon="columns"
      title="No response to compare yet"
      body="Pin this response as a baseline, or send the request again to diff against the previous response."
    >
      <button class="btn small primary" onclick={onpin}><Icon name="pin" size={12} /> Pin current as baseline</button>
    </EmptyState>
  {:else}
    <div class="rdiff-bar">
      {#if pinned && previous}
        <div class="segmented" role="group" aria-label="Compare against">
          <button class:active={effectiveTarget === 'pinned'} aria-pressed={effectiveTarget === 'pinned'} onclick={() => (target = 'pinned')}>Baseline</button>
          <button class:active={effectiveTarget === 'previous'} aria-pressed={effectiveTarget === 'previous'} onclick={() => (target = 'previous')}>Previous</button>
        </div>
      {:else}
        <span class="rdiff-src">vs <strong>{baseline.label}</strong></span>
      {/if}

      <div class="rdiff-chips">
        {#if delta.statusChanged}
          <Badge tone="warn" label={`${delta.statusFrom} → ${delta.statusTo}`} />
        {:else}
          <Badge tone="ok" label={`HTTP ${delta.statusTo}`} />
        {/if}
        <Badge label={fmtSignedMs(delta.durationDeltaMs)} title="Latency delta" />
        <Badge label={fmtSignedBytes(delta.sizeDeltaBytes)} title="Body size delta" />
        {#if delta.identical}
          <Badge tone="ok" label="Identical body" />
        {/if}
      </div>

      <span class="grow"></span>

      <button class="pill-toggle" class:on={canonical} aria-pressed={canonical} onclick={() => (canonical = !canonical)} title="Sort JSON object keys alphabetically before diffing">
        Sort JSON keys
      </button>
      <div class="segmented" role="group" aria-label="Diff layout">
        <button class:active={mode === 'word'} aria-pressed={mode === 'word'} onclick={() => (mode = 'word')}>Unified</button>
        <button class:active={mode === 'split'} aria-pressed={mode === 'split'} onclick={() => (mode = 'split')}>Split</button>
      </div>
      <button class="btn small ghost" onclick={onpin} title="Replace the pinned baseline with the current response"><Icon name="pin" size={12} /> Pin</button>
      {#if pinned}
        <button class="btn small ghost" onclick={onunpin} title="Clear pinned baseline">Clear</button>
      {/if}
    </div>

    {#if baseline.too_large || current.too_large}
      <p class="rdiff-note">One of the responses exceeded the inline body limit — download both responses to compare full payloads.</p>
    {:else if delta.identical}
      <div class="rdiff-same">
        <Icon name="check" size={16} />
        <span>No differences{canonical && afterFmt.isJson ? ' (canonical key-sorted JSON)' : ''}.</span>
      </div>
    {:else}
      <div class="rdiff-body" dir="ltr">
        <DiffView before={beforeFmt.text} after={afterFmt.text} {mode} contextLines={4} />
      </div>
    {/if}
  {/if}
</div>

<style>
  .rdiff {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    gap: 8px;
  }
  .rdiff-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .rdiff-src {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .rdiff-chips {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
  }
  .rdiff-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 10px 10px;
  }
  .rdiff-same,
  .rdiff-note {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 16px;
    color: var(--text-dim);
    font-size: var(--fs-m);
    margin: 0;
  }
</style>
