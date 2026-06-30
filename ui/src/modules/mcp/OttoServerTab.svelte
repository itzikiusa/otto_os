<script lang="ts">
  // Otto-as-MCP-server: Otto exposes itself OUTWARD as an MCP server (its own
  // `otto.*` tools, grouped by feature) for an external agent over stdio. Opt-in
  // (default off), authenticated by a restricted, single-purpose token
  // (minted/rotated here and shown ONCE), with mutating tools default-disabled and
  // dangerous calls approval-gated. Per-tool enable checklist (filterable, grouped
  // by category, with per-group enable/disable) + a copy-pasteable `.mcp.json`
  // install snippet for the external agent.
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { McpOttoServerStatus, McpOttoToolInfo } from '../../lib/api/types';

  // A small fallback catalog shown only if the daemon's admin route isn't
  // reachable. The live, full catalog (every feature category) comes from the
  // daemon's `GET /mcp/otto-server`.
  const CATALOG: McpOttoToolInfo[] = [
    { name: 'otto.search_codebase', description: 'ripgrep over the workspace (file:line hits)', mutating: false, enabled: false, category: 'Code & Context' },
    { name: 'otto.list_workflows', description: 'list a workspace’s workflows', mutating: false, enabled: false, category: 'Workflows' },
    { name: 'otto.list_broker_clusters', description: 'list a workspace’s Kafka clusters', mutating: false, enabled: false, category: 'Message Brokers' },
    { name: 'otto.query_db_readonly', description: 'read-only SQL against a connection', mutating: false, enabled: false, category: 'Database' },
    { name: 'otto.get_proof_pack', description: 'evidence bundle for a branch / PR / goal loop', mutating: false, enabled: false, category: 'Code & Context' },
    { name: 'otto.ask_human_approval', description: 'create a pending human-approval request', mutating: false, enabled: false, category: 'Approvals' },
  ];

  let status = $state<McpOttoServerStatus | null>(null);
  let loading = $state(false);
  let saving = $state(false);
  let loadError = $state<string | null>(null);
  let filter = $state('');
  /** A freshly minted token, shown ONCE in this session. */
  let mintedToken = $state<string | null>(null);

  const tools = $derived(status && status.tools.length ? status.tools : CATALOG);
  const enabledNames = $derived(new Set(tools.filter((t) => t.enabled).map((t) => t.name)));

  // Tools filtered by the search box, then grouped by `category` (first-seen order;
  // tools with no category fall into "Other").
  const filtered = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return q ? tools.filter((t) => `${t.name} ${t.description}`.toLowerCase().includes(q)) : tools;
  });
  const groups = $derived.by(() => {
    const order: string[] = [];
    const map = new Map<string, McpOttoToolInfo[]>();
    for (const t of filtered) {
      const cat = t.category || 'Other';
      if (!map.has(cat)) {
        map.set(cat, []);
        order.push(cat);
      }
      map.get(cat)!.push(t);
    }
    return order.map((cat) => ({ cat, tools: map.get(cat)! }));
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      status = await mcpCpApi.cpOttoServer();
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load();
  });

  async function patch(body: Parameters<typeof mcpCpApi.cpUpdateOttoServer>[0]): Promise<void> {
    saving = true;
    try {
      const next = await mcpCpApi.cpUpdateOttoServer(body);
      if (next.token) mintedToken = next.token;
      // The PATCH reply carries the token once; don't keep it on `status`.
      status = { ...next, token: null };
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function toggleEnabled(): Promise<void> {
    await patch({ enabled: !(status?.enabled ?? false) });
  }

  async function toggleTool(name: string): Promise<void> {
    const next = new Set(enabledNames);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    await patch({ tools: [...next] });
  }

  /** Enable/disable every tool in one category at once. */
  async function setCategory(catTools: McpOttoToolInfo[], enable: boolean): Promise<void> {
    const next = new Set(enabledNames);
    for (const t of catTools) {
      if (enable) next.add(t.name);
      else next.delete(t.name);
    }
    await patch({ tools: [...next] });
  }

  async function rotate(): Promise<void> {
    if (status?.has_token) {
      const ok = await confirmer.ask(
        'Rotate the outward-server token? The current token stops working immediately and any agent using it must be updated.',
        { title: 'Rotate token', confirmLabel: 'Rotate', danger: true },
      );
      if (!ok) return;
    }
    await patch({ rotate_token: true });
    if (mintedToken) toasts.success('Token minted', 'Copy it now — it is shown only once.');
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      toasts.success(`${what} copied`);
    } catch {
      toasts.error('Copy failed', 'Select and copy manually.');
    }
  }

  const tokenForSnippet = $derived(
    mintedToken ?? (status?.token_prefix ? `${status.token_prefix}…` : 'YOUR_OTTO_MCP_TOKEN'),
  );
  const snippet = $derived(
    JSON.stringify(
      {
        mcpServers: {
          otto: {
            command: 'ottod',
            args: ['mcp-server'],
            env: { OTTO_API_TOKEN: tokenForSnippet },
          },
        },
      },
      null,
      2,
    ),
  );
</script>

<div class="otto">
  {#if loading && !status}
    <p class="muted pad">Loading…</p>
  {:else}
    {#if loadError}
      <p class="warn">
        The outward Otto-MCP-server admin endpoint isn't reachable ({loadError}). The catalog below
        reflects the documented 8 <code>otto.*</code> tools; controls will work once the daemon route is live.
      </p>
    {/if}

    <div class="hero">
      <div class="hero-left">
        <div class="hl-title">
          <Icon name="plug" size={16} />
          <span>Otto as an MCP server</span>
          {#if status}
            <span class="state {status.enabled ? 'on' : 'off'}">{status.enabled ? 'enabled' : 'disabled'}</span>
          {/if}
        </div>
        <p class="muted small">
          Expose Otto's own <code>otto.*</code> tools — across every feature (workflows, brokers, git,
          issues, swarm, vault, …) — to an external agent over stdio. Opt-in; mutating tools stay off by
          default; dangerous calls are approval-gated; every call is audited.
        </p>
      </div>
      <div class="hero-actions">
        <button class="btn" onclick={() => void toggleEnabled()} disabled={saving || !status}>
          {status?.enabled ? 'Disable' : 'Enable'}
        </button>
        <button class="btn primary" onclick={() => void rotate()} disabled={saving}>
          {status?.has_token ? 'Rotate token' : 'Mint token'}
        </button>
      </div>
    </div>

    {#if mintedToken}
      <div class="token-once">
        <div class="to-head">
          <Icon name="key" size={13} />
          <strong>New token — shown once. Copy it now.</strong>
          <span class="grow"></span>
          <button class="btn xs" onclick={() => void copy(mintedToken!, 'Token')}>Copy</button>
          <button class="btn xs" onclick={() => (mintedToken = null)}>Dismiss</button>
        </div>
        <code class="token">{mintedToken}</code>
      </div>
    {:else if status?.has_token}
      <p class="token-hint muted small">A token is configured (<code>{status.token_prefix}…</code>). Rotate to mint a fresh one.</p>
    {/if}

    <div class="tools-head">
      <h4 class="sec">Tools</h4>
      <input
        class="filter"
        type="search"
        placeholder="Filter tools…"
        bind:value={filter}
        aria-label="Filter tools"
      />
    </div>
    {#each groups as g (g.cat)}
      <div class="grp">
        <div class="grp-head">
          <span class="grp-name">{g.cat}</span>
          <span class="grp-count muted">{g.tools.filter((t) => t.enabled).length}/{g.tools.length}</span>
          <span class="grow"></span>
          <button class="btn xs" disabled={saving || !status} onclick={() => void setCategory(g.tools, true)}>All</button>
          <button class="btn xs" disabled={saving || !status} onclick={() => void setCategory(g.tools, false)}>None</button>
        </div>
        <div class="tool-list">
          {#each g.tools as t (t.name)}
            <label class="tool">
              <input
                type="checkbox"
                checked={t.enabled}
                disabled={saving || !status}
                onchange={() => void toggleTool(t.name)}
              />
              <div class="t-meta">
                <span class="t-name mono">{t.name}{#if t.mutating}<span class="mut">mutating</span>{/if}</span>
                <span class="t-desc">{t.description}</span>
              </div>
            </label>
          {/each}
        </div>
      </div>
    {/each}
    {#if !groups.length}
      <p class="muted small pad">No tools match “{filter}”.</p>
    {/if}

    <h4 class="sec">Install snippet</h4>
    <p class="muted small">Add this to the external agent's <code>.mcp.json</code> (e.g. another machine's Claude/Copilot).</p>
    <div class="snippet">
      <button class="btn xs copy" onclick={() => void copy(snippet, '.mcp.json')}><Icon name="file" size={12} /> Copy</button>
      <pre>{snippet}</pre>
    </div>
  {/if}
</div>

<style>
  .otto {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 820px;
  }
  .hero {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background: var(--surface);
    padding: 14px;
  }
  .hero-left {
    min-width: 0;
  }
  .hl-title {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
  }
  .state {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 1px 6px;
    border-radius: 4px;
  }
  .state.on {
    background: color-mix(in srgb, var(--status-working, #28c840) 18%, transparent);
    color: var(--status-working, #28c840);
  }
  .state.off {
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
  }
  .hero-left p {
    margin: 6px 0 0;
    max-width: 520px;
  }
  .hero-actions {
    display: flex;
    gap: 8px;
    flex: none;
  }
  .token-once {
    border: 1px solid color-mix(in srgb, #e0a000 45%, transparent);
    background: color-mix(in srgb, #e0a000 10%, transparent);
    border-radius: var(--radius-m, 8px);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .to-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--text);
  }
  .token {
    font-family: var(--font-mono);
    font-size: 12px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    padding: 8px 10px;
    word-break: break-all;
    color: var(--text);
  }
  .token-hint {
    margin: 0;
  }
  .sec {
    margin: 4px 0 0;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .tools-head {
    display: flex;
    align-items: center;
    gap: 12px;
    justify-content: space-between;
  }
  .filter {
    flex: 0 1 240px;
    font-size: 12px;
    padding: 5px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    background: var(--bg);
    color: var(--text);
  }
  .grp {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .grp-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 2px 0;
  }
  .grp-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }
  .grp-count {
    font-size: 11px;
  }
  .grow {
    flex: 1 1 auto;
  }
  .tool-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    overflow: hidden;
  }
  .tool {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    cursor: pointer;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .tool:last-child {
    border-bottom: none;
  }
  .tool:hover {
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .tool input {
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
  .mut {
    margin-inline-start: 8px;
    font-size: 9px;
    text-transform: uppercase;
    color: #e0a000;
    background: color-mix(in srgb, #e0a000 16%, transparent);
    border-radius: 4px;
    padding: 0 5px;
  }
  .t-desc {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
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
  .btn.xs {
    font-size: 11px;
    padding: 3px 8px;
  }
  .warn {
    margin: 0;
    font-size: 12.5px;
    color: #e0a000;
    background: color-mix(in srgb, #e0a000 12%, transparent);
    border: 1px solid color-mix(in srgb, #e0a000 35%, transparent);
    border-radius: var(--radius-s, 6px);
    padding: 10px 12px;
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: 11.5px;
  }
  .pad {
    padding: 16px;
  }

  @media (max-width: 640px) {
    .hero {
      flex-direction: column;
    }
    .hero-actions {
      width: 100%;
    }
    .hero-actions .btn {
      flex: 1;
    }
  }
</style>
