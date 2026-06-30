<script lang="ts">
  // Otto-as-MCP-server: Otto exposes itself OUTWARD as an MCP server (its own
  // `otto.*` tools, grouped by feature) for an external agent over stdio. Opt-in
  // (default off), authenticated by a restricted, single-purpose token
  // (minted/rotated here and shown ONCE), with mutating tools default-disabled and
  // dangerous calls approval-gated. Per-tool enable checklist (filterable, grouped
  // by category, with per-group enable/disable) + a copy-pasteable `.mcp.json`
  // install snippet for the external agent.
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi, mcpTokensApi } from '../../lib/api/mcp';
  import { api, baseUrl } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type {
    McpOttoServerStatus,
    McpOttoToolInfo,
    McpScope,
    McpTokenInfo,
    User,
  } from '../../lib/api/types';

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

  // ---------------------------------------------------------------------------
  // HTTP transport + network exposure (R1: MCP over HTTP, not only locally)
  // ---------------------------------------------------------------------------
  const HTTP_PATH = '/api/v1/mcp/http';
  /** The transport URL reachable from THIS client (loopback when local). */
  const httpUrl = $derived(`${baseUrl()}${HTTP_PATH}`);

  let allSettings = $state<Record<string, unknown>>({});
  let netEnabled = $state(false);
  let netPort = $state(7700);
  let netBusy = $state(false);

  async function loadNetwork(): Promise<void> {
    try {
      allSettings = await api.get<Record<string, unknown>>('/settings');
      const nl = allSettings['network_listener'] as { enabled?: boolean; port?: number } | undefined;
      netEnabled = nl?.enabled ?? false;
      netPort = nl?.port ?? 7700;
    } catch {
      /* non-admin or unreachable — the HTTP panel still shows the loopback URL */
    }
  }

  async function toggleNetwork(): Promise<void> {
    netBusy = true;
    try {
      const next = !netEnabled;
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        ...allSettings,
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

  /** A ready-to-paste `claude mcp add` command for the HTTP transport. */
  function clientCommand(token: string): string {
    return `claude mcp add --transport http otto ${httpUrl} --header "Authorization: Bearer ${token}"`;
  }

  // ---------------------------------------------------------------------------
  // Multiple scoped tokens (R2/R3: many tokens, different users, different access)
  // ---------------------------------------------------------------------------
  const isMcpAdmin = $derived(auth.can('mcp', 'admin'));

  let tokens = $state<McpTokenInfo[]>([]);
  let users = $state<User[]>([]);
  let tokensLoaded = $state(false);

  // Create-token form.
  let showCreate = $state(false);
  let fOwner = $state(''); // user id; '' = the caller (self)
  let fLabel = $state('');
  let fAllowWrites = $state(false);
  let fRestrictTools = $state(false);
  let fTools = $state<Set<string>>(new Set());
  let fWorkspace = $state(''); // '' = no pin
  let creating = $state(false);
  /** A freshly created token's secret + metadata, shown ONCE. */
  let createdToken = $state<{ secret: string; info: McpTokenInfo } | null>(null);

  async function loadTokens(): Promise<void> {
    if (!isMcpAdmin) return;
    try {
      const r = await mcpTokensApi.list();
      tokens = r.tokens;
    } catch {
      /* surfaced only as an empty list — the create panel still works */
    } finally {
      tokensLoaded = true;
    }
  }

  async function loadUsers(): Promise<void> {
    if (!isMcpAdmin) return;
    try {
      users = await api.get<User[]>('/users');
    } catch {
      users = []; // not users:admin — owner defaults to self, which is fine
    }
  }

  function bare(name: string): string {
    return name.replace(/^otto\./, '');
  }

  function toggleFormTool(name: string): void {
    const b = bare(name);
    const next = new Set(fTools);
    if (next.has(b)) next.delete(b);
    else next.add(b);
    fTools = next;
  }

  async function createToken(): Promise<void> {
    creating = true;
    try {
      const scope: McpScope = {
        tools: fRestrictTools ? [...fTools] : null,
        allow_writes: fAllowWrites,
        workspace_id: fWorkspace || null,
      };
      const resp = await mcpTokensApi.create({
        user_id: fOwner || undefined,
        label: fLabel.trim() || undefined,
        scope,
      });
      createdToken = { secret: resp.token, info: resp.info };
      toasts.success('MCP token created', 'Copy it now — it is shown only once.');
      // Reset the form.
      fLabel = '';
      fTools = new Set();
      fRestrictTools = false;
      fAllowWrites = false;
      fWorkspace = '';
      fOwner = '';
      showCreate = false;
      await loadTokens();
    } catch (e) {
      toasts.error('Create failed', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  async function revokeToken(t: McpTokenInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Revoke MCP token ${t.label ? `“${t.label}” ` : ''}(${t.token_prefix}…) for ${t.username}? Any client using it stops working immediately.`,
      { title: 'Revoke token', confirmLabel: 'Revoke', danger: true },
    );
    if (!ok) return;
    try {
      await mcpTokensApi.revoke(t.id);
      toasts.success('Token revoked');
      await loadTokens();
    } catch (e) {
      toasts.error('Revoke failed', e instanceof Error ? e.message : String(e));
    }
  }

  function scopeSummary(s: McpScope): string {
    const toolPart =
      s.tools == null ? 'all tools' : `${s.tools.length} tool${s.tools.length === 1 ? '' : 's'}`;
    const writePart = s.allow_writes ? 'read + write' : 'read-only';
    const wsPart = s.workspace_id ? ` • 1 workspace` : '';
    return `${toolPart} • ${writePart}${wsPart}`;
  }

  $effect(() => {
    void loadTokens();
    void loadUsers();
    void loadNetwork();
  });
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

    <!-- HTTP transport access (R1: MCP over HTTP, not only locally) -->
    <h4 class="sec">HTTP access</h4>
    <p class="muted small">
      External MCP clients can connect over HTTP with a bearer token — no local subprocess. Reachable on
      the loopback URL below at all times; enable network access to reach it over TLS from other machines.
    </p>
    <div class="urlrow">
      <code class="url" data-testid="mcp-http-url">{httpUrl}</code>
      <button class="btn xs" onclick={() => void copy(httpUrl, 'URL')}>Copy URL</button>
    </div>
    <label class="netrow">
      <input
        type="checkbox"
        checked={netEnabled}
        disabled={netBusy || !isMcpAdmin}
        onchange={() => void toggleNetwork()}
      />
      <div class="t-meta">
        <span class="t-name">Allow network access (TLS) on port {netPort}</span>
        <span class="t-desc">
          Binds the daemon on <code>0.0.0.0:{netPort}</code> with a self-signed certificate so remote clients
          can reach <code>https://&lt;this-host&gt;:{netPort}{HTTP_PATH}</code>. Off by default; restart the daemon to apply.
        </span>
      </div>
    </label>

    <!-- Scoped tokens (R2/R3: multiple tokens, different users, different access) -->
    {#if isMcpAdmin}
      <div class="tools-head">
        <h4 class="sec">Access tokens</h4>
        <button class="btn xs" onclick={() => (showCreate = !showCreate)}>
          {showCreate ? 'Cancel' : 'New token'}
        </button>
      </div>
      <p class="muted small">
        Each token authenticates as a user and carries its own scope (tools, read-only vs writes, an optional
        workspace pin) — so different users get different access over the HTTP transport.
      </p>

      {#if createdToken}
        <div class="token-once" data-testid="mcp-created-token">
          <div class="to-head">
            <Icon name="key" size={13} />
            <strong>New token for {createdToken.info.username} — shown once. Copy it now.</strong>
            <span class="grow"></span>
            <button class="btn xs" onclick={() => void copy(createdToken!.secret, 'Token')}>Copy token</button>
            <button class="btn xs" onclick={() => void copy(clientCommand(createdToken!.secret), 'Command')}>Copy command</button>
            <button class="btn xs" onclick={() => (createdToken = null)}>Dismiss</button>
          </div>
          <code class="token">{createdToken.secret}</code>
          <code class="token cmd">{clientCommand(createdToken.secret)}</code>
        </div>
      {/if}

      {#if showCreate}
        <div class="create">
          <div class="frow">
            <label class="fld">
              <span class="lbl">Owner</span>
              <select class="inp" bind:value={fOwner}>
                <option value="">Me ({auth.me?.username ?? 'self'})</option>
                {#each users as u (u.id)}
                  <option value={u.id}>{u.username}</option>
                {/each}
              </select>
            </label>
            <label class="fld">
              <span class="lbl">Label</span>
              <input class="inp" placeholder="e.g. ci-readonly" bind:value={fLabel} />
            </label>
            <label class="fld">
              <span class="lbl">Workspace pin (optional)</span>
              <select class="inp" bind:value={fWorkspace}>
                <option value="">Any workspace</option>
                {#each ws.workspaces as w (w.id)}
                  <option value={w.id}>{w.name}</option>
                {/each}
              </select>
            </label>
          </div>
          <label class="chk">
            <input type="checkbox" bind:checked={fAllowWrites} />
            Allow mutating (write) tools — otherwise the token is read-only
          </label>
          <label class="chk">
            <input type="checkbox" bind:checked={fRestrictTools} />
            Restrict to specific tools — otherwise every enabled tool
          </label>
          {#if fRestrictTools}
            <div class="tool-pick">
              {#each groups as g (g.cat)}
                <div class="pick-grp">
                  <span class="grp-name">{g.cat}</span>
                  {#each g.tools as t (t.name)}
                    <label class="ptool">
                      <input
                        type="checkbox"
                        checked={fTools.has(bare(t.name))}
                        onchange={() => toggleFormTool(t.name)}
                      />
                      <span class="mono">{bare(t.name)}</span>{#if t.mutating}<span class="mut">mutating</span>{/if}
                    </label>
                  {/each}
                </div>
              {/each}
            </div>
          {/if}
          <div class="cactions">
            <button
              class="btn primary"
              disabled={creating}
              onclick={() => void createToken()}
              data-testid="mcp-create-token"
            >
              {creating ? 'Creating…' : 'Create token'}
            </button>
          </div>
        </div>
      {/if}

      <div class="tok-list" data-testid="mcp-tokens">
        {#if !tokens.length}
          <p class="muted small pad">{tokensLoaded ? 'No MCP tokens yet.' : 'Loading…'}</p>
        {:else}
          {#each tokens as t (t.id)}
            <div class="tok-row">
              <div class="tok-main">
                <span class="tok-label">{t.label || '(no label)'}</span>
                <span class="tok-prefix mono">{t.token_prefix}…</span>
              </div>
              <span class="tok-user">{t.username}</span>
              <span class="tok-scope muted">{scopeSummary(t.scope)}</span>
              <span class="tok-seen muted">{rel(t.last_seen_at)}</span>
              <button class="btn xs danger" onclick={() => void revokeToken(t)}>Revoke</button>
            </div>
          {/each}
        {/if}
      </div>
    {/if}
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

  /* HTTP access */
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
  .netrow {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    padding: 10px 12px;
    cursor: pointer;
  }
  .netrow input {
    margin-top: 2px;
  }

  /* Tokens */
  .create {
    display: flex;
    flex-direction: column;
    gap: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background: var(--surface);
    padding: 12px;
  }
  .frow {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1 1 160px;
  }
  .lbl {
    font-size: 11px;
    color: var(--text-dim);
  }
  .inp {
    font-size: 12px;
    padding: 5px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    background: var(--bg);
    color: var(--text);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--text);
  }
  .tool-pick {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 220px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    padding: 10px;
  }
  .pick-grp {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .ptool {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: var(--text);
  }
  .cactions {
    display: flex;
    justify-content: flex-end;
  }
  .token.cmd {
    font-size: 11px;
  }
  .tok-list {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    overflow: hidden;
  }
  .tok-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .tok-row:last-child {
    border-bottom: none;
  }
  .tok-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1 1 auto;
  }
  .tok-label {
    font-size: 12.5px;
    color: var(--text);
  }
  .tok-prefix {
    font-size: 11px;
    color: var(--text-dim);
  }
  .tok-user,
  .tok-scope,
  .tok-seen {
    font-size: 11.5px;
    flex: none;
  }
  .btn.danger {
    color: var(--danger, #c0392b);
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
    .tok-scope,
    .tok-seen {
      display: none;
    }
  }
</style>
