<script lang="ts">
  import { onMount } from 'svelte';
  import { okfMetadata } from './okfMetadata';
  let { frontmatter }: { frontmatter: unknown } = $props();
  let now = $state(Date.now());
  onMount(() => { const timer = setInterval(() => now = Date.now(), 30_000); return () => clearInterval(timer); });
  const meta = $derived(okfMetadata(frontmatter, now));
</script>
<div class="knowledge" aria-label="Knowledge provenance">
  <div class="badges"><span>{meta.tier}</span><span>{meta.status}</span>{#if meta.stale}<span class="warning">Stale</span>{/if}</div>
  {#if meta.generatedAt}<p>Content updated <time>{meta.generatedAt}</time>{#if meta.generatedBy} by {meta.generatedBy}{/if}</p>{/if}
  {#if meta.staleAt}<p class:warning={meta.stale || meta.invalidDeadline}>Stale after {meta.staleAt}{#if meta.invalidDeadline} (invalid timestamp){/if}</p>{/if}
  {#if meta.changedSinceVerification}<p class="warning">Content changed after the latest recorded verification.</p>{/if}
  {#each meta.verified as event, i (i)}<p>Verified by {event.by} · <time>{event.at}</time></p>{/each}
  {#if meta.sources.length}
    <details><summary>Sources ({meta.sources.length})</summary>
      {#each meta.sources as source, i (i)}
        <div class="source"><strong>{source.title || source.id || 'Source'}</strong><div>{source.resource}</div>
          {#if source.author || source.modified}<small>{source.author} {source.modified}</small>{/if}
        </div>
      {/each}
    </details>
  {/if}
  <p class="hint">Verification is declared in this note; it is not an authenticated approval.</p>
  {#if meta.attestedComputation}<p class="hint">Computation definition only. Otto has not run or attested this computation.</p>{/if}
</div>
<style>
  .knowledge { padding: 5px 8px; font-size: 11.5px; overflow-wrap: anywhere; }
  .badges { display: flex; flex-wrap: wrap; gap: 5px; }
  .badges span { border: 1px solid var(--border); padding: 2px 5px; border-radius: 4px; }
  p { margin: 7px 0; } .hint, small { color: var(--text-dim); }
  .warning { color: var(--status-warn); } summary { cursor: pointer; }
  .source { border-inline-start: 2px solid var(--border); padding-inline-start: 7px; margin-block: 8px; }
</style>
