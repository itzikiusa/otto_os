<script lang="ts">
  // Shared response viewer: status pill (colored by class) + duration + size +
  // content-type, then Body / Headers tabs. Pretty-prints JSON bodies.
  import Icon from '../../lib/components/Icon.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';

  interface Props {
    compact?: boolean;
  }
  let { compact = false }: Props = $props();

  const resp = $derived(apiClient.lastResponse);

  type Tab = 'body' | 'headers';
  let tab: Tab = $state('body');

  function statusClass(status: number): string {
    if (status >= 200 && status < 300) return 'ok';
    if (status >= 300 && status < 400) return 'redirect';
    if (status >= 400 && status < 500) return 'client';
    if (status >= 500) return 'server';
    return 'none';
  }

  function isJson(ct: string | null, body: string): boolean {
    if (ct && /\bjson\b/i.test(ct)) return true;
    const t = body.trim();
    return (t.startsWith('{') && t.endsWith('}')) || (t.startsWith('[') && t.endsWith(']'));
  }

  const prettyBody = $derived.by(() => {
    if (!resp) return '';
    if (isJson(resp.content_type, resp.body)) {
      try {
        return JSON.stringify(JSON.parse(resp.body), null, 2);
      } catch {
        return resp.body;
      }
    }
    return resp.body;
  });

  function fmtSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<div class="viewer" class:compact>
  {#if !resp}
    <div class="empty">
      <Icon name="send" size={compact ? 20 : 26} />
      <span>Send a request to see the response.</span>
    </div>
  {:else}
    <div class="resp-head">
      <span class="status-pill {statusClass(resp.status)}">
        {resp.status}{#if resp.status_text}&nbsp;{resp.status_text}{/if}
      </span>
      <span class="meta"><Icon name="clock" size={11} />{resp.duration_ms} ms</span>
      <span class="meta"><Icon name="box" size={11} />{fmtSize(resp.size_bytes)}</span>
      {#if resp.content_type}
        <span class="meta mono ellipsis ct" title={resp.content_type}>{resp.content_type}</span>
      {/if}
    </div>

    <div class="rtabs" role="tablist">
      <button class="rtab" class:active={tab === 'body'} role="tab" aria-selected={tab === 'body'} onclick={() => (tab = 'body')}>Body</button>
      <button class="rtab" class:active={tab === 'headers'} role="tab" aria-selected={tab === 'headers'} onclick={() => (tab = 'headers')}>
        Headers <span class="hcount">{resp.headers.length}</span>
      </button>
    </div>

    <div class="rbody">
      {#if tab === 'body'}
        {#if prettyBody.trim() === ''}
          <div class="empty-mini">Empty response body.</div>
        {:else}
          <pre class="body-pre mono">{prettyBody}</pre>
        {/if}
      {:else}
        <table class="htable mono">
          <tbody>
            {#each resp.headers as h, i (i)}
              <tr>
                <td class="hkey">{h.key}</td>
                <td class="hval">{h.value}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  {/if}
</div>

<style>
  .viewer {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .resp-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 2px 8px;
    flex-wrap: wrap;
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
    height: 20px;
    padding: 0 9px;
    border-radius: 999px;
    font-size: 11.5px;
    font-weight: 700;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .status-pill.ok {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .status-pill.redirect {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .status-pill.client {
    background: color-mix(in srgb, #d2691e 20%, transparent);
    color: #d2691e;
  }
  .status-pill.server {
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    color: var(--status-exited);
  }
  .meta {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .ct {
    max-width: 220px;
  }
  .rtabs {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border);
  }
  .rtab {
    height: 26px;
    padding: 0 12px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .rtab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .hcount {
    font-size: 10px;
    color: var(--text-dim);
  }
  .rbody {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding-top: 8px;
  }
  .body-pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--text);
  }
  .empty-mini {
    font-size: 12px;
    color: var(--text-dim);
    padding: 8px 2px;
  }
  .htable {
    width: 100%;
    border-collapse: collapse;
    user-select: text;
  }
  .htable td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    font-size: 11.5px;
    vertical-align: top;
    word-break: break-word;
  }
  .hkey {
    color: var(--accent);
    width: 34%;
    font-weight: 600;
  }
  .hval {
    color: var(--text);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
