<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import { api, baseUrl } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import TokensPanel from './TokensPanel.svelte';
  import type { McpOttoToolInfo } from '../../lib/api/types';

  interface Props {
    groups: { cat: string; tools: McpOttoToolInfo[] }[];
    isMcpAdmin: boolean;
  }

  let { groups, isMcpAdmin }: Props = $props();

  const HTTP_PATH = '/api/v1/mcp/http';
  const httpUrl = $derived(`${baseUrl()}${HTTP_PATH}`);
  const httpCommand = $derived(
    `claude mcp add --transport http otto ${httpUrl} --header "Authorization: Bearer YOUR_OTTO_MCP_TOKEN"`,
  );
  const snippet = JSON.stringify(
    {
      mcpServers: {
        otto: {
          command: 'ottod',
          args: ['mcp-server'],
          env: { OTTO_API_TOKEN: 'YOUR_OTTO_MCP_TOKEN' },
        },
      },
    },
    null,
    2,
  );

  const loopbackPort = $derived.by(() => {
    try {
      return Number(new URL(baseUrl()).port) || 7700;
    } catch {
      return 7700;
    }
  });

  let networkOpen = $state(false);
  let networkError = $state(false);
  let netEnabled = $state(false);
  let netPort = $state(7701);
  let netPortTouched = $state(false);
  let netBusy = $state(false);
  const portConflict = $derived(netPort === loopbackPort);

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`${what} copied`);
    } catch {
      toasts.error('Copy failed', 'Select and copy manually.');
    }
  }

  async function loadNetwork(): Promise<void> {
    networkError = false;
    try {
      const settings = await api.get<Record<string, unknown>>('/settings');
      const listener = settings['network_listener'] as
        | { enabled?: boolean; port?: number }
        | undefined;
      netEnabled = listener?.enabled ?? false;
      if (listener?.port != null) {
        netPort = listener.port;
        netPortTouched = true;
      } else if (!netPortTouched) {
        netPort = loopbackPort + 1;
      }
    } catch {
      networkError = true;
    }
  }

  async function toggleNetwork(): Promise<void> {
    if (!netEnabled && portConflict) {
      toasts.error(
        'Pick a different port',
        `The network port must differ from the daemon's loopback port (${loopbackPort}).`,
      );
      return;
    }
    netBusy = true;
    try {
      const next = !netEnabled;
      await api.put<Record<string, unknown>>('/settings', {
        network_listener: { enabled: next, port: netPort },
      });
      netEnabled = next;
      toasts.success(
        next ? 'Network access enabled' : 'Network access disabled',
        'Restart the daemon to apply the listener change.',
      );
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    } finally {
      netBusy = false;
    }
  }

  $effect(() => {
    void loadNetwork();
  });
</script>

<div class="expose" data-testid="mcp-expose-panel">
  <h4 class="sec">Install snippet</h4>
  <p class="muted small">
    Add this to the external agent's <code>.mcp.json</code> and replace the token placeholder.
  </p>
  <div class="snippet">
    <button class="btn xs copy" onclick={() => void copy(snippet, '.mcp.json')}>
      <Icon name="file" size={12} /> Copy
    </button>
    <pre>{snippet}</pre>
  </div>

  <h4 class="sec">HTTP access</h4>
  <p class="muted small">
    External MCP clients can connect over HTTP with a bearer token — no local subprocess. The
    loopback URL is always available; opt in to network access for remote clients.
  </p>
  <div class="urlrow">
    <code class="url" data-testid="mcp-http-url">{httpUrl}</code>
    <button class="btn xs" onclick={() => void copy(httpUrl, 'URL')}>Copy URL</button>
  </div>
  <div class="urlrow">
    <code class="url">{httpCommand}</code>
    <button class="btn xs" onclick={() => void copy(httpCommand, 'Command')}>Copy command</button>
  </div>

  <button
    class="subdisclose"
    aria-expanded={networkOpen}
    onclick={() => (networkOpen = !networkOpen)}
  >
    <span class:open={networkOpen}>▸</span>
    Network access (advanced)
  </button>
  {#if networkOpen}
    <div class="network">
      {#if networkError}
        <p class="muted small">Network settings need settings:admin.</p>
      {:else}
        <label class="netrow">
          <input
            type="checkbox"
            checked={netEnabled}
            disabled={netBusy || !isMcpAdmin || (!netEnabled && portConflict)}
            onchange={() => void toggleNetwork()}
          />
          <div class="t-meta">
            <span class="t-name">
              Allow network access (TLS) on port
              <input
                class="port"
                type="number"
                min="1"
                max="65535"
                bind:value={netPort}
                oninput={() => (netPortTouched = true)}
                disabled={netEnabled || !isMcpAdmin}
                aria-label="Network port"
              />
            </span>
            <span class="t-desc">
              Binds the daemon on <code>0.0.0.0:{netPort}</code> with a self-signed certificate so
              remote clients can reach <code>https://&lt;this-host&gt;:{netPort}{HTTP_PATH}</code>.
              Off by default; restart the daemon to apply.
            </span>
            {#if portConflict}
              <span class="t-warn">
                Choose a port other than the daemon's loopback port ({loopbackPort}) — they would collide.
              </span>
            {/if}
          </div>
        </label>
      {/if}
    </div>
  {/if}

  {#if isMcpAdmin}
    <TokensPanel {groups} />
  {/if}
</div>

<style>
  .expose {
    display: flex;
    flex-direction: column;
    gap: 10px;
    border: 1px solid var(--border);
    border-top: none;
    border-radius: 0 0 var(--radius-m, 8px) var(--radius-m, 8px);
    background: var(--surface);
    padding: 14px;
  }
  .sec {
    margin: 4px 0 0;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .snippet {
    position: relative;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background: var(--bg);
  }
  .snippet pre {
    margin: 0;
    padding: 12px;
    font-family: var(--font-mono);
    font-size: 12px;
    overflow: auto;
    color: var(--text);
  }
  .copy {
    position: absolute;
    top: 8px;
    inset-inline-end: 8px;
  }
  .urlrow {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .url {
    flex: 1 1 auto;
    font-family: var(--font-mono);
    font-size: 12px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    padding: 8px 10px;
    color: var(--text);
    word-break: break-all;
  }
  .subdisclose {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 9px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    background: var(--bg);
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .subdisclose span {
    display: inline-block;
    transition: transform 120ms ease;
  }
  .subdisclose span.open {
    transform: rotate(90deg);
  }
  .network {
    padding-inline-start: 8px;
  }
  .netrow {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    padding: 10px 12px;
    cursor: pointer;
  }
  .netrow > input {
    margin-top: 2px;
  }
  .t-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .t-name {
    font-size: 12.5px;
    color: var(--text);
  }
  .t-desc {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .port {
    width: 72px;
    font-size: 12px;
    padding: 2px 6px;
    margin-inline-start: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    background: var(--bg);
    color: var(--text);
  }
  .t-warn {
    font-size: 11.5px;
    color: #e0a000;
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: 11.5px;
  }
  .btn.xs {
    font-size: 11px;
    padding: 3px 8px;
  }

  @media (max-width: 640px) {
    .urlrow {
      align-items: stretch;
      flex-direction: column;
    }
    .urlrow .btn {
      align-self: flex-start;
    }
  }
</style>
