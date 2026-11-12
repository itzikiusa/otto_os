<script lang="ts">
  // One compact row for another agent at work — a subagent, or a thread this
  // agent opened or messaged: provider icon · name · model / effort · status
  // · elapsed · one-line result · Open. The row itself toggles `expanded`
  // when the host gives it a body (a subagent's activity); Open is its own
  // button. T3 Code's ProviderSubagentBar / V2LifecycleRow, in Otto's tokens.
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';

  interface Props {
    provider?: string | null;
    name: string;
    /** "gpt-5 · high", "Explore", … */
    sub?: string | null;
    /** `ask`: it waits on a person (an approval or a question). */
    tone: 'live' | 'done' | 'failed' | 'off' | 'ask';
    status: string;
    elapsed?: string | null;
    result?: string | null;
    /** Given: the row toggles its body. */
    expanded?: boolean;
    ontoggle?: () => void;
    /** Given: an Open button (⇧-click handled by the host). */
    onopen?: (e: MouseEvent) => void;
    openLabel?: string;
    /** Given: an "Open beside" icon button (a child thread). */
    onbeside?: () => void;
    /** Given: a Stop icon button while it works. */
    onstop?: () => void;
    testid?: string;
  }
  let { provider = null, name, sub = null, tone, status, elapsed = null, result = null, expanded = false, ontoggle, onopen, openLabel = 'Open', onbeside, onstop, testid }: Props = $props();
</script>

{#snippet inner()}
  {#if provider && hasProviderIcon(provider)}<ProviderIcon {provider} size={13} />{:else}<Icon name="assistant" size={13} />{/if}
  <span class="ar-name ellipsis" dir="auto">{name}</span>
  {#if sub}<span class="ar-sub ellipsis">{sub}</span>{/if}
  <span class="ar-state">
    {#if tone === 'live'}<span class="spinner" aria-hidden="true"></span>{:else if tone === 'done'}<Icon name="check" size={12} />{:else if tone === 'failed'}<Icon name="x" size={12} />{:else if tone === 'ask'}<Icon name="warning" size={12} />{/if}
    {status}{#if elapsed} · {elapsed}{/if}
  </span>
  {#if result}<span class="ar-result ellipsis" dir="auto" title={result}>{result}</span>{/if}
{/snippet}

<div class="ar" data-tone={tone} data-testid={testid}>
  {#if ontoggle}
    <button type="button" class="ar-main" aria-expanded={expanded} onclick={ontoggle}>
      {@render inner()}
      <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
  {:else}
    <div class="ar-main">{@render inner()}</div>
  {/if}
  {#if onstop}
    <button type="button" class="icon-btn" aria-label="Stop {name}" title="Stop" data-testid="runtime-item-stop" onclick={onstop}><Icon name="stop" size={12} /></button>
  {/if}
  {#if onbeside}
    <button type="button" class="icon-btn" aria-label="Open {name} beside" title="Open beside (Split)" data-testid="runtime-item-beside" onclick={onbeside}><Icon name="split" size={12} /></button>
  {/if}
  {#if onopen}
    <button type="button" class="btn small ghost ar-open" title="{openLabel} (⇧-click: beside)" data-testid="runtime-item-open" onclick={onopen}>{openLabel}</button>
  {/if}
</div>

<style>
  .ar {
    display: flex;
    align-items: center;
    gap: 4px;
    min-inline-size: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .ar-main {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-inline-size: 0;
    min-block-size: 30px;
    padding: 2px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: inherit;
    font: inherit;
    text-align: start;
  }
  button.ar-main {
    cursor: pointer;
  }
  button.ar-main:hover {
    background: var(--hover);
  }
  button.ar-main:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .ar-name {
    flex: 0 1 auto;
    min-inline-size: 3em;
    color: var(--text);
    font-weight: 500;
  }
  .ar-sub {
    flex: 0 2 auto;
    min-inline-size: 0;
    font-size: var(--fs-xs);
  }
  .ar-state {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .ar[data-tone='live'] .ar-state {
    color: var(--text);
  }
  .ar[data-tone='done'] .ar-state {
    color: var(--success);
  }
  .ar[data-tone='failed'] .ar-state {
    color: var(--danger);
  }
  .ar[data-tone='ask'] .ar-state {
    color: var(--warning);
  }
  .ar-state .spinner {
    --spinner-size: 10px;
  }
  .ar-result {
    flex: 1 1 0;
    min-inline-size: 0;
    font-size: var(--fs-xs);
  }
  .ar-open {
    flex: none;
  }
  /* A narrow column: the result drops first, then the model. */
  @container (max-width: 420px) {
    .ar-result,
    .ar-sub {
      display: none;
    }
  }
</style>
