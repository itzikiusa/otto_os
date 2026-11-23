<script lang="ts">
  // Opt-in Live DevTools drawer for RemoteLiveView: Console, Network, and
  // point-based Element Inspector (with "Mark element" integration into
  // Otto's Browser annotations).
  import Icon from '../../../lib/components/Icon.svelte';
  import Tabs, { type TabItem } from '../../../lib/components/Tabs.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import type {
    BrowserConsoleEntry,
    BrowserInspectedElement,
    BrowserNetworkEntry,
  } from '../../../lib/api/types';

  export type InspectorTab = 'console' | 'network' | 'element';

  interface Props {
    consoleEntries: BrowserConsoleEntry[];
    networkEntries: BrowserNetworkEntry[];
    inspected: BrowserInspectedElement | null;
    inspectPicking: boolean;
    activeTab?: InspectorTab;
    ontogglepick: () => void;
    onclear: () => void;
    onmark?: (el: BrowserInspectedElement) => void;
    onclose: () => void;
  }

  let {
    consoleEntries,
    networkEntries,
    inspected,
    inspectPicking,
    activeTab = $bindable<InspectorTab>('console'),
    ontogglepick,
    onclear,
    onmark,
    onclose,
  }: Props = $props();

  let levelFilter = $state<'all' | 'error' | 'warning'>('all');

  const tabs = $derived<readonly TabItem<InspectorTab>[]>([
    { id: 'console', label: 'Console', count: consoleEntries.length || undefined },
    { id: 'network', label: 'Network', count: networkEntries.length || undefined },
    { id: 'element', label: 'Element' },
  ]);

  const filteredConsole = $derived.by(() => {
    if (levelFilter === 'all') return consoleEntries;
    return consoleEntries.filter((e) => e.level === levelFilter);
  });

  function statusTone(entry: BrowserNetworkEntry): 'err' | 'warn' | 'ok' | 'dim' {
    if (entry.failed) return 'err';
    if (entry.status === null || entry.status === undefined) return 'dim';
    if (entry.status >= 400) return 'err';
    if (entry.status >= 300) return 'warn';
    return 'ok';
  }
</script>

<section class="insp" aria-label="Live page inspector">
  <div class="insp-head">
    <Tabs
      label="Inspector tabs"
      size="s"
      idBase="browser-insp"
      {tabs}
      value={activeTab}
      onchange={(id) => (activeTab = id)}
    >
      {#snippet trailing()}
        <div class="insp-actions">
          <button
            class="icon-btn insp-icon"
            class:picking={inspectPicking}
            onclick={ontogglepick}
            aria-label={inspectPicking ? 'Stop picking element' : 'Pick element in page'}
            aria-pressed={inspectPicking}
            title={inspectPicking ? 'Click any element in the page to inspect it' : 'Pick an element in the page'}
          >
            <Icon name="target" size={13} />
          </button>
          {#if activeTab === 'console'}
            <select class="insp-filter" bind:value={levelFilter} aria-label="Filter console level">
              <option value="all">All</option>
              <option value="error">Errors</option>
              <option value="warning">Warnings</option>
            </select>
          {/if}
          {#if activeTab !== 'element'}
            <button
              class="icon-btn insp-icon"
              onclick={onclear}
              aria-label="Clear inspector logs"
              title="Clear logs"
            >
              <Icon name="trash" size={13} />
            </button>
          {/if}
          <button
            class="icon-btn insp-icon"
            onclick={onclose}
            aria-label="Close inspector"
            title="Close inspector"
          >
            <Icon name="x" size={13} />
          </button>
        </div>
      {/snippet}
    </Tabs>
  </div>

  <div class="insp-body scroll-thin" role="tabpanel" id="browser-insp-panel-{activeTab}" aria-labelledby="browser-insp-tab-{activeTab}">
    {#if activeTab === 'console'}
      {#if filteredConsole.length === 0}
        <p class="insp-empty">No console output yet. Reload or interact with the page to capture logs.</p>
      {:else}
        <ul class="insp-list">
          {#each filteredConsole as entry (entry.seq)}
            <li class="insp-log" class:err={entry.level === 'error'} class:warn={entry.level === 'warning'}>
              <span class="insp-lvl">{entry.level}</span>
              <span class="insp-msg" dir="ltr">{entry.text}</span>
              {#if entry.source}
                <span class="insp-src" dir="ltr" title={entry.source}>{entry.source}</span>
              {/if}
              <button
                class="icon-btn insp-copy"
                onclick={() => void copyWithToast(entry.text, 'console message')}
                aria-label="Copy console message"
                title="Copy message"
              >
                <Icon name="copy" size={12} />
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if activeTab === 'network'}
      {#if networkEntries.length === 0}
        <p class="insp-empty">No network requests captured yet. Query strings are redacted for safety.</p>
      {:else}
        <ul class="insp-list">
          {#each networkEntries as net (net.request_id)}
            {@const tone = statusTone(net)}
            <li class="insp-net">
              <span class="insp-method" dir="ltr">{net.method}</span>
              <span class="insp-status {tone}" dir="ltr">
                {net.failed ? 'ERR' : (net.status ?? '…')}
              </span>
              <span class="insp-url" dir="ltr" title={net.url}>{net.url}</span>
              <span class="insp-type">{net.resource_type}</span>
              <span class="insp-dur" dir="ltr">
                {net.duration_ms !== null && net.duration_ms !== undefined ? `${Math.round(net.duration_ms)} ms` : '—'}
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    {:else}
      {#if !inspected}
        <div class="insp-empty-el">
          <p class="insp-empty">
            {inspectPicking
              ? 'Click any element in the live page to inspect its selector, box and computed styles.'
              : 'Select the target tool above and click an element in the live page.'}
          </p>
          {#if !inspectPicking}
            <button class="btn small" onclick={ontogglepick}>
              <Icon name="target" size={12} /> Pick element
            </button>
          {/if}
        </div>
      {:else}
        <div class="insp-el">
          <div class="insp-el-top">
            <code class="insp-sel" dir="ltr">&lt;{inspected.tag}&gt; {inspected.selector}</code>
            <span class="insp-box" dir="ltr">
              {Math.round(inspected.rect.width)}×{Math.round(inspected.rect.height)}
            </span>
            <button
              class="btn small"
              onclick={() => inspected && void copyWithToast(inspected.selector, 'CSS selector')}
            >
              <Icon name="copy" size={12} /> Selector
            </button>
            {#if onmark}
              <button class="btn small primary" onclick={() => inspected && onmark(inspected)}>
                <Icon name="note" size={12} /> Mark element
              </button>
            {/if}
          </div>
          <div class="insp-grid">
            <div><span class="insp-k">Display</span> <code dir="ltr">{inspected.styles.display}</code></div>
            <div><span class="insp-k">Position</span> <code dir="ltr">{inspected.styles.position}</code></div>
            <div><span class="insp-k">Font</span> <code dir="ltr">{inspected.styles.font}</code></div>
            <div><span class="insp-k">Color</span> <code dir="ltr">{inspected.styles.color}</code></div>
          </div>
          {#if inspected.text}
            <p class="insp-excerpt">{inspected.text}</p>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
</section>

<style>
  .insp {
    display: flex;
    flex-direction: column;
    height: 190px;
    min-height: 120px;
    max-height: 45%;
    border-top: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    flex: none;
    min-width: 0;
  }
  .insp-head {
    padding: 0 8px;
    min-width: 0;
  }
  .insp-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .insp-icon {
    width: 24px;
    height: 24px;
  }
  .insp-icon.picking {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .insp-filter {
    height: 24px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
  }
  .insp-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 8px;
  }
  .insp-empty {
    margin: 12px 4px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .insp-empty-el {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 4px;
  }
  .insp-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .insp-log,
  .insp-net {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-radius: var(--radius-s);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    min-width: 0;
  }
  .insp-log:hover,
  .insp-net:hover {
    background: var(--bg);
  }
  .insp-log.err {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .insp-log.warn {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .insp-lvl {
    flex-shrink: 0;
    text-transform: uppercase;
    min-width: 48px;
    color: var(--text-dim);
  }
  .insp-log.err .insp-lvl { color: var(--danger); }
  .insp-log.warn .insp-lvl { color: var(--warning); }
  .insp-msg,
  .insp-url {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .insp-src,
  .insp-type,
  .insp-dur {
    flex-shrink: 0;
    color: var(--text-dim);
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .insp-copy {
    width: 24px;
    height: 24px;
    flex-shrink: 0;
  }
  .insp-method {
    flex-shrink: 0;
    min-width: 40px;
    font-weight: 600;
  }
  .insp-status {
    flex-shrink: 0;
    min-width: 30px;
    font-variant-numeric: tabular-nums;
  }
  .insp-status.ok { color: var(--success); }
  .insp-status.warn { color: var(--warning); }
  .insp-status.err { color: var(--danger); }
  .insp-status.dim { color: var(--text-dim); }
  .insp-el {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 6px 4px;
  }
  .insp-el-top {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .insp-sel {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 2px 6px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .insp-box {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .insp-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 6px;
    font-size: var(--fs-xs);
  }
  .insp-k {
    color: var(--text-dim);
    margin-inline-end: 4px;
  }
  .insp-excerpt {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  @media (max-height: 500px) {
    .insp {
      height: 140px;
    }
  }
</style>
