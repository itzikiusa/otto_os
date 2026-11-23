<script lang="ts">
  // Drafted AI page summary panel: renders markdown safely via `renderNote`
  // (same sanitizer ReaderView uses) with Copy, Stop, and Close controls.
  import { announceShown, focusAppContent } from '../../lib/announce.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import Icon from '../../lib/components/Icon.svelte';
  import { renderNote } from '../../lib/render/markdown';
  import { browser } from '../../lib/stores/browser.svelte';

  const summaryHtml = $derived(
    browser.summary ? renderNote(browser.summary, { resolve: () => null, assetUrl: () => null }) : '',
  );

  function close(): void {
    browser.summary = '';
    void focusAppContent();
  }
</script>

{#if browser.summarizing && !browser.summary}
  <div class="summary" use:announceShown={'Summarizing this page…'}>
    <div class="summary-head"><span>Summary · drafted by Otto</span></div>
    <p class="dim-line">Summarizing this page…</p>
    <div>
      <button class="btn small" onclick={() => browser.stopSummarize()}>
        <Icon name="stop" size={12} /> Stop
      </button>
    </div>
  </div>
{:else if browser.summary}
  <div class="summary">
    <div class="summary-head">
      <span>Summary · drafted by Otto</span>
      <div class="head-actions">
        <button
          class="icon-btn"
          onclick={() => void copyWithToast(browser.summary, 'Summary')}
          aria-label="Copy summary"
          title="Copy summary"
        >
          <Icon name="copy" size={12} />
        </button>
        <button
          class="icon-btn"
          onclick={close}
          aria-label="Close summary"
          title="Close summary"
        >
          <Icon name="x" size={12} />
        </button>
      </div>
    </div>
    <p class="dim-line">
      {#if browser.summaryAt}<time datetime={browser.summaryAt}>{new Date(browser.summaryAt).toLocaleTimeString()}</time> · {/if}
      Fetched with {browser.summaryEngine || 'the browser'}
      {#if browser.summaryDegraded} · No JavaScript ran; dynamic content may be missing.{/if}
    </p>
    <!-- eslint-disable-next-line svelte/no-at-html-tags -->
    <div class="summary-body md-body">{@html summaryHtml}</div>
  </div>
{/if}

<style>
  .summary {
    margin: 10px 12px 0;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: var(--fs-m);
  }
  .summary-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
    margin-bottom: 6px;
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .dim-line {
    margin: 0 0 6px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .summary-body {
    max-height: 30vh;
    overflow-y: auto;
    color: var(--text);
    line-height: 1.5;
  }
  .summary-body :global(> :first-child) {
    margin-top: 0;
  }
  .summary-body :global(> :last-child) {
    margin-bottom: 0;
  }
</style>
