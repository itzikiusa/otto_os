<script lang="ts">
  import { announceShown } from '../../../lib/announce.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';

  interface Props {
    surface: 'agents' | 'threads';
    effectiveProvider: string;
    titleText: string;
    titleTooltip: string;
    statsText: string;
    narrowHead: boolean;
    hasChanges: boolean;
    searchOpen: boolean;
    query: string;
    searchLabel: string;
    partialSearch: boolean;
    hitsCount: number;
    hitIdx: number;
    noMatches: boolean;
    loadingEarlier: boolean;
    onquerychange: (q: string) => void;
    onsearchkey: (e: KeyboardEvent) => void;
    onprevhit: () => void;
    onnexthit: () => void;
    onopensearch: () => void;
    onclosesearch: () => void;
    onopenchanges: () => void;
    onopenmenu: (e: MouseEvent | KeyboardEvent) => void;
    onloadearlier: () => void;
  }

  let {
    surface,
    effectiveProvider,
    titleText,
    titleTooltip,
    statsText,
    narrowHead,
    hasChanges,
    searchOpen,
    query,
    searchLabel,
    partialSearch,
    hitsCount,
    hitIdx,
    noMatches,
    loadingEarlier,
    onquerychange,
    onsearchkey,
    onprevhit,
    onnexthit,
    onopensearch,
    onclosesearch,
    onopenchanges,
    onopenmenu,
    onloadearlier,
  }: Props = $props();

  let searchEl = $state<HTMLInputElement | null>(null);

  export function selectInput(): void {
    searchEl?.select();
  }
</script>

{#if surface !== 'threads'}
  <header class="conv-head" class:folded={narrowHead && searchOpen}>
    {#if effectiveProvider && hasProviderIcon(effectiveProvider)}
      <ProviderIcon provider={effectiveProvider} size={13} />
    {/if}
    <span class="conv-title" title={titleTooltip}>{titleText}</span>
    {#if statsText}
      <span class="stats" title="turns · tool calls · cost · tokens in/out · duration">{statsText}</span>
    {/if}
    <span class="grow"></span>
    {#if hasChanges && !(narrowHead && searchOpen)}
      <button
        class="icon-btn"
        title="All changes in this conversation"
        aria-label="All changes in this conversation"
        data-all-changes
        onclick={onopenchanges}
      >
        <Icon name="split" size={12} />
      </button>
    {/if}
    {#if searchOpen}
      <div class="search" class:wide={narrowHead} role="search">
        <Icon name="search" size={12} />
        <input
          dir="auto"
          bind:this={searchEl}
          value={query}
          oninput={(e) => onquerychange(e.currentTarget.value)}
          class="search-in"
          placeholder={searchLabel}
          aria-label={searchLabel}
          aria-describedby={partialSearch ? 'conv-search-scope' : undefined}
          onkeydown={onsearchkey}
        />
        <span class="search-n" data-search-hits={hitsCount} aria-live="polite">
          {hitsCount ? `${hitIdx + 1}/${hitsCount}` : noMatches && !partialSearch ? 'No matches' : ''}
        </span>
        <button class="icon-btn" title="Previous match" aria-keyshortcuts="Shift+Enter" aria-label="Previous match" disabled={!hitsCount} onclick={onprevhit}><Icon name="chevronUp" size={12} /></button>
        <button class="icon-btn" title="Next match" aria-keyshortcuts="Enter" aria-label="Next match" disabled={!hitsCount} onclick={onnexthit}><Icon name="chevronDown" size={12} /></button>
        <button class="icon-btn" title="Close search" aria-label="Close search" aria-keyshortcuts="Escape" onclick={onclosesearch}><Icon name="x" size={12} /></button>
      </div>
    {:else if !narrowHead}
      <button class="icon-btn" title="Search this conversation" aria-keyshortcuts="Meta+F" aria-label="Search this conversation" onclick={onopensearch}><Icon name="search" size={12} /></button>
    {/if}
    <button class="icon-btn" aria-label="Conversation options" title="Conversation options" aria-haspopup="menu" data-conv-menu onclick={onopenmenu}><Icon name="more" size={12} /></button>
  </header>
{:else if searchOpen}
  <div class="conv-search-row">
    <div class="search wide" role="search">
      <Icon name="search" size={12} />
      <input
        dir="auto"
        bind:this={searchEl}
        value={query}
        oninput={(e) => onquerychange(e.currentTarget.value)}
        class="search-in"
        placeholder={searchLabel}
        aria-label={searchLabel}
        aria-describedby={partialSearch ? 'conv-search-scope' : undefined}
        onkeydown={onsearchkey}
      />
      <span class="search-n" data-search-hits={hitsCount} aria-live="polite">
        {hitsCount ? `${hitIdx + 1}/${hitsCount}` : noMatches && !partialSearch ? 'No matches' : ''}
      </span>
      <button class="icon-btn" title="Previous match" aria-keyshortcuts="Shift+Enter" aria-label="Previous match" disabled={!hitsCount} onclick={onprevhit}><Icon name="chevronUp" size={12} /></button>
      <button class="icon-btn" title="Next match" aria-keyshortcuts="Enter" aria-label="Next match" disabled={!hitsCount} onclick={onnexthit}><Icon name="chevronDown" size={12} /></button>
      <button class="icon-btn" title="Close search" aria-label="Close search" aria-keyshortcuts="Escape" onclick={onclosesearch}><Icon name="x" size={12} /></button>
    </div>
  </div>
{/if}
{#if searchOpen && partialSearch}
  <div class="search-scope" data-testid="conv-search-scope">
    <span id="conv-search-scope" use:announceShown={noMatches ? 'No matches in the loaded messages.' : ''}>
      {noMatches ? 'No matches in the loaded messages.' : 'Only the loaded messages are searched.'}
    </span>
    <button class="btn small ghost" disabled={loadingEarlier} onclick={onloadearlier}>
      {loadingEarlier ? 'Loading earlier messages…' : 'Load earlier messages'}
    </button>
  </div>
{/if}

<style>
  .conv-head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding-block: 0;
    padding-inline: 12px 6px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .conv-search-row {
    display: flex;
    align-items: center;
    padding-block: 4px;
    padding-inline: 10px;
    background: var(--surface);
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .conv-title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 0 1 auto;
  }
  .stats {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 0 2 auto;
  }
  .grow {
    flex: 1;
  }
  .search {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--border));
    border-radius: var(--radius-s);
    background: var(--bg);
    padding-block: 0;
    padding-inline: 6px 4px;
    height: 22px;
    color: var(--text-dim);
    min-width: 0;
  }
  .search:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .search-scope {
    flex: none;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding-block: 4px;
    padding-inline: 12px;
    border-block-end: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .search-in {
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    width: 180px;
    min-width: 0;
  }
  .search-n {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 28px;
    text-align: center;
    white-space: nowrap;
  }
  @container (max-width: 360px) {
    .search-in {
      width: 110px;
    }
  }
  @container (max-width: 560px) {
    .stats {
      display: none;
    }
  }
  .search.wide {
    flex: 1;
  }
  .search.wide .search-in {
    width: 100%;
    flex: 1;
  }
  .conv-head.folded .conv-title {
    display: none;
  }
</style>
