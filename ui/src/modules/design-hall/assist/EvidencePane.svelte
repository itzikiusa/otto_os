<script lang="ts">
  // Evidence for one learned rule / proposal: a vertical timeline of the design
  // signals it was drawn from (newest first) — when, what kind, which design,
  // a one-line summary and a way back to the design. Evidence ids that aren't
  // in the loaded signal window are counted, never invented.
  import Icon from '../../../lib/components/Icon.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import type { DesignSignal } from '../../../lib/api/types';
  import ArtifactThumb from '../ArtifactThumb.svelte';
  import { signalSummary, signalTone } from '../model';
  import { library } from '../library.svelte';
  import { assistSignalSummary, signalKindLabel } from './model';

  interface Props {
    title: string;
    rationale: string;
    evidence: string[];
    signals: Record<string, DesignSignal>;
    onclose?: () => void;
  }
  let { title, rationale, evidence, signals, onclose }: Props = $props();

  const items = $derived(
    evidence
      .map((id) => signals[id])
      .filter((s): s is DesignSignal => !!s)
      .sort((a, b) => b.created_at.localeCompare(a.created_at)),
  );
  const missing = $derived(evidence.length - items.length);
  const summary = (s: DesignSignal) => assistSignalSummary(s.kind, s.payload) ?? signalSummary(s);
  const day = (iso: string) => new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
</script>

<aside class="evidence" aria-label="Evidence" data-testid="design-learned-evidence">
  <div class="head">
    <span class="k">Evidence</span>
    {#if onclose}
      <button class="icon-btn" onclick={onclose} aria-label="Close evidence" title="Close"><Icon name="x" size={12} /></button>
    {/if}
  </div>
  <h3>{title}</h3>
  {#if rationale}<p class="why"><Icon name="clock" size={12} /> {rationale}</p>{/if}
  {#if items.length === 0}
    <p class="dim">{evidence.length ? 'These signals are older than the loaded log.' : 'No evidence was recorded with this rule.'}</p>
  {:else}
    <ol class="timeline">
      {#each items as s (s.id)}
        {@const hit = library.hitOf(s.artifact_id)}
        <li>
          <span class="dot" aria-hidden="true"></span>
          <div class="ev card">
            <div class="row">
              <strong title={new Date(s.created_at).toLocaleString()}>{day(s.created_at)}</strong>
              <span class="kind tone-{signalTone(s.kind)}">{signalKindLabel(s.kind)}</span>
              <span class="dim when">{rel(s.created_at)}</span>
            </div>
            <div class="art">{hit?.artifact.title ?? 'A design'}</div>
            {#if hit}
              <div class="pic"><ArtifactThumb artifact={hit.artifact} versionId={s.version_id} live compact /></div>
            {/if}
            <p class="sum">{summary(s)}</p>
            <a href={`#/design/a/${encodeURIComponent(s.artifact_id)}`}>Open the design</a>
          </div>
        </li>
      {/each}
    </ol>
  {/if}
  {#if missing > 0 && items.length > 0}
    <p class="dim">{missing} more signal{missing === 1 ? '' : 's'} beyond the loaded log.</p>
  {/if}
</aside>

<style>
  .evidence {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .k {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  h3 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    line-height: 1.3;
  }
  .why {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .why :global(svg) {
    vertical-align: -2px;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
    margin: 0;
  }
  .timeline {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .timeline li {
    position: relative;
    padding-inline-start: 18px;
  }
  .timeline li::before {
    content: '';
    position: absolute;
    inset-inline-start: 4px;
    inset-block: 14px -12px;
    border-inline-start: 1px solid var(--border);
  }
  .timeline li:last-child::before {
    display: none;
  }
  .dot {
    position: absolute;
    inset-inline-start: 0;
    inset-block-start: 10px;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 2px solid var(--accent);
    background: var(--surface);
  }
  .ev {
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .when {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
  }
  .kind {
    font-size: var(--fs-xs);
    font-weight: 500;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .tone-ok {
    color: var(--success);
    background: var(--success-soft);
  }
  .tone-bad {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .tone-warn {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .tone-info {
    color: var(--info);
    background: var(--info-soft);
  }
  .art {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .pic {
    width: 120px;
    aspect-ratio: 4 / 3;
    border-radius: var(--radius-s);
    overflow: hidden;
    border: 1px solid var(--border);
  }
  .sum {
    margin: 0;
    font-size: var(--fs-s);
  }
  a {
    font-size: var(--fs-s);
    color: var(--accent-text);
    text-decoration: none;
  }
  a:hover {
    text-decoration: underline;
  }
</style>
