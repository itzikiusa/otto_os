<script lang="ts">
  // The one "design every state" wrapper for a loaded list/page: loading,
  // error, empty and loaded — so a failed load can never masquerade as an
  // empty list ("No X yet") and a first load never flashes the empty state.
  //
  //   <LoadState what="scheduled tasks" loading={s.loading} error={s.error}
  //              empty={s.tasks.length === 0} onretry={() => s.load()}>
  //     {#snippet emptyView()}<EmptyState title="No scheduled tasks yet" … />{/snippet}
  //     …the list…
  //   </LoadState>
  //
  // Precedence:
  //   • error + nothing to show  → inline "Couldn't load {what}" + detail + Retry
  //   • error + stale data       → the data, with a slim "refresh failed" bar + Retry
  //   • loading + nothing yet    → Skeleton (never the empty state)
  //   • empty                    → `emptyView` (or nothing)
  //   • else                     → children
  //
  // `variant` matches EmptyState: `page` for a whole pane, `panel` inside a
  // card/list, `compact` for a narrow rail (one line + Retry).
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import Skeleton from './Skeleton.svelte';

  interface Props {
    /** Noun for the headline: "Couldn't load {what}". */
    what: string;
    loading?: boolean;
    /** Human cause of the last failed load (see `loadErrorText`); null/'' = ok. */
    error?: string | null;
    /** Loaded, nothing to show. */
    empty?: boolean;
    onretry?: () => void;
    variant?: 'page' | 'panel' | 'compact';
    /** Skeleton rows on first load. */
    rows?: number;
    emptyView?: Snippet;
    children?: Snippet;
  }
  let { what, loading = false, error = null, empty = false, onretry, variant = 'panel', rows = 4, emptyView, children }: Props = $props();
</script>

{#if error && empty}
  {#if variant === 'compact'}
    <div class="ls-compact" role="alert" data-testid="load-error">
      <span class="ls-compact-text"><Icon name="warning" size={12} /> Couldn't load {what}. <span class="ls-detail">{error}</span></span>
      {#if onretry}<button class="btn small" onclick={onretry} disabled={loading}>{loading ? 'Retrying…' : 'Retry'}</button>{/if}
    </div>
  {:else}
    <div class="ls-error" class:page={variant === 'page'} role="alert" data-testid="load-error">
      <div class="ls-icon"><Icon name="warning" size={variant === 'page' ? 26 : 22} /></div>
      <h3>Couldn't load {what}</h3>
      <p class="ls-detail">{error}</p>
      {#if onretry}
        <button class="btn ls-retry" onclick={onretry} disabled={loading}>
          <Icon name="refresh" size={13} />
          {loading ? 'Retrying…' : 'Retry'}
        </button>
      {/if}
    </div>
  {/if}
{:else if loading && empty}
  <div class="ls-loading" class:page={variant === 'page'} aria-label="Loading {what}">
    <Skeleton rows={variant === 'compact' ? 2 : rows} height={variant === 'compact' ? 24 : 36} />
  </div>
{:else if empty}
  {#if emptyView}{@render emptyView()}{/if}
{:else}
  {#if error}
    <div class="ls-stale" role="status" data-testid="load-stale">
      <Icon name="warning" size={12} />
      <span class="ls-stale-text">Showing the last good load — refresh failed: {error}</span>
      {#if onretry}<button class="btn small ghost" onclick={onretry} disabled={loading}>Retry</button>{/if}
    </div>
  {/if}
  {#if children}{@render children()}{/if}
{/if}

<style>
  .ls-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 40px 24px;
    text-align: center;
    color: var(--text-dim);
  }
  .ls-error.page {
    width: 100%;
    box-sizing: border-box;
    padding: 15vh 24px 48px;
  }
  .ls-icon {
    width: 56px;
    height: 56px;
    border-radius: var(--radius-l);
    background: var(--warning-soft);
    border: 1px solid var(--border);
    display: grid;
    place-items: center;
    margin-bottom: 4px;
    color: var(--warning);
  }
  h3 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .page h3 {
    font-size: var(--fs-l);
  }
  .ls-detail {
    margin: 0;
    font-size: var(--fs-s);
    max-width: 420px;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }
  .ls-retry {
    margin-top: 8px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .ls-loading {
    padding: 8px 12px;
  }
  .ls-loading.page {
    padding: 24px;
  }
  .ls-compact {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 8px 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .ls-compact-text {
    color: var(--warning);
    overflow-wrap: anywhere;
  }
  .ls-compact .ls-detail {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .ls-stale {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 4px 12px;
    font-size: var(--fs-xs);
    color: var(--warning);
    border-block-end: 1px solid var(--border);
    background: var(--warning-soft);
  }
  .ls-stale-text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
