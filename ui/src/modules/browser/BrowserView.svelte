<script lang="ts">
  // Browser module page: tab strip + URL bar + reader-mode page. Mirrors the
  // shape of VaultPage/LoopsPage (a thin view over a $state store).

  import { ws } from '../../lib/stores/workspace.svelte';
  import { browser } from '../../lib/stores/browser.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import TabStrip from './TabStrip.svelte';
  import ReaderView from './ReaderView.svelte';

  let urlInput = $state('');
  let summarizing = $state(false);
  let summary = $state('');

  // (Re)load the tab list when the workspace changes.
  $effect(() => {
    const id = ws.currentId;
    if (id) void browser.loadTabs(id);
  });

  // Keep the URL bar in sync with the active tab.
  $effect(() => {
    urlInput = browser.activeTab?.url ?? urlInput;
  });

  function normalize(raw: string): string {
    const t = raw.trim();
    if (!t) return t;
    return /^[a-z][a-z0-9+.-]*:\/\//i.test(t) ? t : `https://${t}`;
  }

  async function go(): Promise<void> {
    const url = normalize(urlInput);
    if (!url) return;
    urlInput = url;
    try {
      await browser.navigate(url);
      summary = '';
    } catch (e) {
      toasts.error('Failed to load page', e instanceof Error ? e.message : undefined);
    }
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') void go();
  }

  function newTab(): void {
    urlInput = '';
    browser.deselect();
  }

  async function doSummarize(): Promise<void> {
    const url = browser.activeTab?.url;
    if (!url) return;
    summarizing = true;
    try {
      const resp = await browser.summarize(url);
      summary = resp.summary;
    } catch (e) {
      toasts.error('Summarize failed', e instanceof Error ? e.message : undefined);
    } finally {
      summarizing = false;
    }
  }
</script>

<div class="browser">
  <TabStrip onnew={newTab} />

  <div class="urlbar">
    <input
      type="text"
      placeholder="Enter a URL and press Enter…"
      bind:value={urlInput}
      onkeydown={onkeydown}
    />
    <button class="btn" onclick={go} title="Go">
      <Icon name="external" size={14} />
    </button>
    <button
      class="btn"
      onclick={doSummarize}
      disabled={!browser.activeTab || summarizing}
      title="Summarize"
    >
      <Icon name="zap" size={14} />
    </button>
  </div>

  {#if summary}
    <div class="summary">
      <div class="summary-head">
        <span>Summary</span>
        <button class="close" onclick={() => (summary = '')}><Icon name="x" size={12} /></button>
      </div>
      <p>{summary}</p>
    </div>
  {/if}

  <ReaderView page={browser.page} loading={browser.loadingPage} error={browser.pageError} />
</div>

<style>
  .browser {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .urlbar {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border);
  }
  .urlbar input {
    flex: 1;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 0.4rem 0.6rem;
    font: inherit;
    font-size: 0.85rem;
  }
  .btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .summary {
    margin: 0.6rem 0.75rem 0;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: 0.85rem;
  }
  .summary-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--text-dim);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: 0.35rem;
  }
  .summary p {
    margin: 0;
    color: var(--text);
    line-height: 1.5;
  }
  .close {
    display: flex;
    background: transparent;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
  }
</style>
