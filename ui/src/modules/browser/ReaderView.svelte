<script lang="ts">
  // Reader-mode page render. The fetched page's markdown is rendered through
  // the SAME sanitizing renderer Vault's reading view uses (`renderNote` from
  // `modules/vault/mdRender.ts`) — never `{@html}` on raw page content. No
  // wikilink/attachment resolution applies here (this isn't a vault note), so
  // `resolve`/`assetUrl` are no-ops.

  import { renderNote } from '../vault/mdRender';
  import type { BrowserPage } from '../../lib/api/types';

  let { page, loading, error }: { page: BrowserPage | null; loading: boolean; error: string } =
    $props();

  const html = $derived(
    page ? renderNote(page.markdown, { resolve: () => null, assetUrl: () => null }) : '',
  );
</script>

<div class="reader">
  {#if loading}
    <p class="muted">Loading…</p>
  {:else if error}
    <div class="error">{error}</div>
  {:else if !page}
    <div class="empty">
      <p>Enter a URL above to fetch it in reader mode.</p>
    </div>
  {:else}
    {#if page.degraded}
      <div class="degraded">
        Degraded fetch — no JavaScript ran ({page.engine}). Some content may be missing.
      </div>
    {/if}
    <article class="page">
      <h1 class="page-title">{page.title || page.url}</h1>
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      {@html html}
    </article>
  {/if}
</div>

<style>
  .reader {
    flex: 1;
    overflow-y: auto;
    padding: 1rem 1.25rem 2rem;
  }
  .muted {
    color: var(--text-dim);
    padding: 0.75rem 0;
  }
  .error {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
    border-radius: var(--radius-s);
    padding: 0.6rem 0.75rem;
    font-size: 0.85rem;
  }
  .empty {
    color: var(--text-dim);
    padding: 2rem 0;
    text-align: center;
  }
  .degraded {
    background: color-mix(in srgb, var(--status-warn) 16%, transparent);
    color: var(--status-warn);
    border-radius: var(--radius-s);
    padding: 0.5rem 0.75rem;
    font-size: 0.82rem;
    margin-bottom: 0.75rem;
  }
  .page {
    max-width: 72ch;
    margin: 0 auto;
    color: var(--text);
    line-height: 1.6;
  }
  .page-title {
    font-size: 1.4rem;
    margin: 0 0 1rem;
  }
</style>
