<script lang="ts">
  // "Where it went › Session": the top sessions as two-line rows — title (it
  // opens the session) … cost; kind · workspace · provider/model · tokens · last
  // active under it. Windowed (VirtualList), so raising the summary's session
  // limit stays DOM-bounded; ⌘F still reaches every row (findText). Two lines
  // instead of seven columns: nothing scrolls sideways, on a phone or not.
  import Badge from '../../lib/components/Badge.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import { kindLabel } from '../../lib/labels';
  import { formatCount, formatUsd } from '../../lib/metric-format';
  import { openAgentSession } from '../../lib/openAgentSession';
  import type { SessionUsage } from '../../lib/api/usage.svelte';
  import { breakdownTitle, fmtLastActive } from './tokenParts';

  let { sessions }: { sessions: SessionUsage[] } = $props();

  // A row with no Otto session behind it: run outside Otto, or deleted.
  const unknownLabel = (s: SessionUsage) => (s.workspace_id === 'external' ? 'Session outside Otto' : 'Deleted session');
  const name = (s: SessionUsage) => s.title ?? (s.kind != null ? s.session_id.slice(0, 12) : unknownLabel(s));
</script>

{#if sessions.length === 0}
  <p class="dim small">No sessions in this window.</p>
{:else}
  <VirtualList
    items={sessions}
    estimateHeight={56}
    class="sess-vlist"
    findText={(s) => [name(s), s.kind ?? '', s.workspace_name ?? '', s.provider, s.model ?? ''].join('\n')}
  >
    {#snippet row(s)}
      {@const open = s.kind != null || s.title != null}
      <div class="sess-row" title={s.session_id}>
        <div class="l1">
          {#if open}
            <button class="sess-name sess-open ellip" onclick={() => void openAgentSession(s.session_id)} aria-label="Open session {name(s)}">{name(s)}</button>
          {:else}
            <span class="sess-name ellip">{name(s)}</span>
          {/if}
          <span class="cost" title={s.fallback_priced ? 'Estimated: this model isn’t in the rate table, so it is priced at the Opus tier' : undefined}>
            {formatUsd(s.cost_usd)}{#if s.fallback_priced}<span class="est"><Badge tone="warn" label="Est." /></span>{/if}
          </span>
        </div>
        <div class="l2 ellip">
          {#if s.kind}{kindLabel(s.kind)} · {/if}{#if s.workspace_name}{s.workspace_name} · {/if}{s.provider}{#if s.model} · <span class="mono">{s.model}</span>{/if}
          · <span title={breakdownTitle(s)}>{formatCount(s.total_tokens)} tokens</span> · {fmtLastActive(s.last_active)}
        </div>
      </div>
    {/snippet}
  </VirtualList>
{/if}

<style>
  :global(.sess-vlist) {
    max-block-size: 448px;
  }
  .sess-row {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--sp-1);
    block-size: 56px;
    padding-inline: var(--sp-4);
    border-block-end: 1px solid var(--separator);
    min-inline-size: 0;
  }
  .sess-row:hover {
    background: var(--hover);
  }
  .l1 {
    display: flex;
    align-items: baseline;
    gap: var(--sp-5);
    min-inline-size: 0;
  }
  .sess-name {
    flex: 1;
    color: var(--text);
    font-size: var(--fs-m);
  }
  .sess-open {
    border: none;
    background: none;
    padding: 0;
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .sess-open:hover {
    color: var(--accent-text);
    text-decoration: underline;
  }
  .cost {
    flex: none;
    font-variant-numeric: tabular-nums;
    font-weight: 600;
  }
  .est {
    margin-inline-start: var(--sp-2);
  }
  .l2 {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .ellip {
    display: block;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    margin: 0;
    font-size: var(--fs-s);
  }
</style>
