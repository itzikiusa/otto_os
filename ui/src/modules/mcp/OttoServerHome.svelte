<script lang="ts">
  // Otto's built-in MCP server is the control plane's home: session attachment,
  // the outward catalog, what sessions see, and external-client setup.
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { mcpCpExtraApi, type McpGatewayToolRow } from './cp-api';
  import ExposePanel from './ExposePanel.svelte';
  import type { McpOttoServerStatus, McpOttoToolInfo, McpSessionAttach } from '../../lib/api/types';

  interface Props {
    wsId: string | null;
  }

  let { wsId }: Props = $props();

  let status = $state<McpOttoServerStatus | null>(null);
  let loading = $state(false);
  let saving = $state(false);
  let loadError = $state<string | null>(null);
  let filter = $state('');
  let attach = $state<McpSessionAttach | null>(null);
  let attachBusy = $state(false);
  let attachError = $state<string | null>(null);
  let gatewayTools = $state<McpGatewayToolRow[]>([]);
  let gatewayLoading = $state(false);
  let gatewayError = $state<string | null>(null);
  let exposeOpen = $state(false);

  const isMcpAdmin = $derived(auth.can('mcp', 'admin'));
  const tools = $derived(status?.tools ?? []);
  const enabledNames = $derived(new Set(tools.filter((tool) => tool.enabled).map((tool) => tool.name)));
  const mutatingCount = $derived(tools.filter((tool) => tool.mutating).length);
  const attached = $derived(attach?.attached ?? true);
  const gatewayNames = $derived([...new Set(gatewayTools.map((tool) => tool.name))]);
  const shownGatewayNames = $derived(gatewayNames.slice(0, 60));

  const filtered = $derived.by(() => {
    const query = filter.trim().toLowerCase();
    return query
      ? tools.filter((tool) => `${tool.name} ${tool.description}`.toLowerCase().includes(query))
      : tools;
  });
  const groups = $derived.by(() => {
    const order: string[] = [];
    const grouped = new Map<string, McpOttoToolInfo[]>();
    for (const tool of filtered) {
      const category = tool.category || 'Other';
      if (!grouped.has(category)) {
        grouped.set(category, []);
        order.push(category);
      }
      grouped.get(category)!.push(tool);
    }
    return order.map((cat) => ({ cat, tools: grouped.get(cat)! }));
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      status = await mcpCpApi.cpOttoServer();
    } catch (e) {
      status = null;
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function patch(body: Parameters<typeof mcpCpApi.cpUpdateOttoServer>[0]): Promise<void> {
    saving = true;
    try {
      const next = await mcpCpApi.cpUpdateOttoServer(body);
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

  async function setCategory(categoryTools: McpOttoToolInfo[], enable: boolean): Promise<void> {
    const next = new Set(enabledNames);
    for (const tool of categoryTools) {
      if (enable) next.add(tool.name);
      else next.delete(tool.name);
    }
    await patch({ tools: [...next] });
  }

  async function loadAttach(id: string): Promise<void> {
    attach = null;
    attachError = null;
    try {
      const next = await mcpCpExtraApi.sessionAttach(id);
      if (wsId === id) attach = next;
    } catch (e) {
      if (wsId === id) attachError = e instanceof Error ? e.message : String(e);
    }
  }

  async function updateAttach(enabled: boolean): Promise<void> {
    const id = wsId;
    if (!id) return;
    attachBusy = true;
    attachError = null;
    try {
      attach = await mcpCpExtraApi.setSessionAttach(id, { enabled });
      toasts.success(
        enabled ? 'Attached to sessions' : 'Detached from sessions',
        'Applies to sessions started from now on.',
      );
    } catch (e) {
      attachError = e instanceof Error ? e.message : String(e);
    } finally {
      attachBusy = false;
    }
  }

  async function loadGatewayTools(id: string): Promise<void> {
    gatewayLoading = true;
    gatewayError = null;
    try {
      const result = await mcpCpExtraApi.gatewayTools(id);
      if (wsId === id) gatewayTools = result.tools;
    } catch (e) {
      if (wsId === id) {
        gatewayTools = [];
        gatewayError = e instanceof Error ? e.message : String(e);
      }
    } finally {
      if (wsId === id) gatewayLoading = false;
    }
  }

  async function copyGatewayTools(): Promise<void> {
    try {
      await copyTextOrThrow(gatewayNames.join('\n'));
      toasts.success('Tool names copied');
    } catch {
      toasts.error('Copy failed', 'Select and copy manually.');
    }
  }

  $effect(() => {
    void load();
  });

  $effect(() => {
    const id = wsId;
    attach = null;
    attachError = null;
    gatewayTools = [];
    gatewayError = null;
    gatewayLoading = false;
    if (id) {
      void loadAttach(id);
      void loadGatewayTools(id);
    }
  });
</script>

<div class="otto">
  <section class="hero">
    <div class="hl-title">
      <Icon name="plug" size={16} />
      <span>Otto's built-in MCP server</span>
    </div>

    {#if wsId}
      <label class="switchrow">
        <input
          type="checkbox"
          data-testid="mcp-session-attach"
          checked={attach?.attached ?? true}
          disabled={!wsId || attachBusy || !isMcpAdmin}
          onchange={(event) => void updateAttach(event.currentTarget.checked)}
        />
        <span class="switchcopy">
          <strong>Attach to sessions in {ws.current?.name ?? 'this workspace'}</strong>
          <span class="muted small">
            Sessions started in this workspace get Otto's read-only tools (ottod mcp-tools). Applies to sessions started from now on.
          </span>
        </span>
      </label>
    {:else}
      <p class="muted small">Select a workspace to manage session attachment.</p>
    {/if}
    {#if attachError}<p class="warn">Could not load session attachment: {attachError}</p>{/if}

    <label class="switchrow">
      <input
        type="checkbox"
        data-testid="mcp-outward-enabled"
        checked={status?.enabled ?? false}
        disabled={saving || !status || !isMcpAdmin}
        onchange={() => void toggleEnabled()}
      />
      <span class="switchcopy">
        <strong>Expose to external clients</strong>
        <span class="muted small">
          Serve the otto.* tools below to external MCP clients over stdio or HTTP, authenticated by an access token. Mutating tools stay off by default; dangerous calls are approval-gated; every call is audited.
        </span>
      </span>
    </label>
    <p class="counts muted small">
      {tools.length} tools · {enabledNames.size} exposed · {mutatingCount} mutating
    </p>
  </section>

  {#if loading && !status}
    <p class="muted pad">Loading…</p>
  {/if}
  {#if loadError}
    <p class="warn">Could not load the external tool catalog: {loadError}</p>
  {/if}

  <div class="tools-head">
    <div>
      <h4 class="sec">External tool catalog</h4>
      <p class="muted small catalog-note">
        Applies to external clients only — sessions always get Otto's built-in read-only tool set.
      </p>
    </div>
    <input
      class="filter"
      type="search"
      placeholder="Filter tools…"
      bind:value={filter}
      aria-label="Filter tools"
    />
  </div>
  {#each groups as group (group.cat)}
    <div class="grp">
      <div class="grp-head">
        <span class="grp-name">{group.cat}</span>
        <span class="grp-count muted">
          {group.tools.filter((tool) => tool.enabled).length}/{group.tools.length}
        </span>
        <span class="grow"></span>
        <button
          class="btn xs"
          disabled={saving || !status || !isMcpAdmin}
          onclick={() => void setCategory(group.tools, true)}
        >All</button>
        <button
          class="btn xs"
          disabled={saving || !status || !isMcpAdmin}
          onclick={() => void setCategory(group.tools, false)}
        >None</button>
      </div>
      <div class="tool-list">
        {#each group.tools as tool (tool.name)}
          <label class="tool">
            <input
              type="checkbox"
              checked={tool.enabled}
              disabled={saving || !status || !isMcpAdmin}
              onchange={() => void toggleTool(tool.name)}
            />
            <span class="t-meta">
              <span class="t-name mono">
                {tool.name}{#if tool.mutating}<span class="mut">mutating</span>{/if}
              </span>
              <span class="t-desc">{tool.description}</span>
            </span>
          </label>
        {/each}
      </div>
    </div>
  {/each}
  {#if status && !groups.length}
    <p class="muted small pad">No tools match “{filter}”.</p>
  {/if}

  <section class="panel" data-testid="mcp-sessions-panel">
    <div class="tools-head">
      <h4 class="sec">What my sessions see</h4>
      {#if wsId}
        <div class="panel-actions">
          <button
            class="btn xs"
            disabled={gatewayLoading || !gatewayNames.length}
            onclick={() => void copyGatewayTools()}
          >Copy</button>
          <button
            class="btn xs"
            disabled={gatewayLoading}
            onclick={() => void loadGatewayTools(wsId!)}
          >{gatewayLoading ? 'Refreshing…' : 'Refresh'}</button>
        </div>
      {/if}
    </div>
    <p class="small">
      Built-in tools: {attached ? 'attached' : 'not attached'} — Otto's read-only tool server (<code>ottod mcp-tools</code>).
    </p>
    {#if !wsId}
      <p class="muted small">Select a workspace to inspect its gateway tools.</p>
    {:else if gatewayError}
      <p class="muted small">Could not load gateway tools: {gatewayError}</p>
    {:else}
      <p class="small">Gateway tools from governed external servers: {gatewayNames.length}</p>
      {#if shownGatewayNames.length}
        <div class="gateway-list">
          {#each shownGatewayNames as name (name)}<code>{name}</code>{/each}
          {#if gatewayNames.length > shownGatewayNames.length}
            <span class="muted small">+{gatewayNames.length - shownGatewayNames.length} more</span>
          {/if}
        </div>
      {/if}
    {/if}
  </section>

  <div class="expose-wrap">
    <button
      class="disclose"
      data-testid="mcp-expose-toggle"
      aria-expanded={exposeOpen}
      onclick={() => (exposeOpen = !exposeOpen)}
    >
      <span class:open={exposeOpen}>▸</span>
      Connect an external client
    </button>
    {#if exposeOpen}<ExposePanel {groups} {isMcpAdmin} />{/if}
  </div>
</div>

<style>
  .otto {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 820px;
  }
  .hero,
  .panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background: var(--surface);
    padding: 14px;
  }
  .hl-title {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
  }
  .switchrow {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .switchrow > input {
    margin-top: 2px;
  }
  .switchcopy {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .switchcopy strong {
    font-size: 12.5px;
    color: var(--text);
  }
  .counts,
  .catalog-note,
  .panel p {
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
  .panel-actions,
  .gateway-list {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .gateway-list code {
    font-size: 10.5px;
    padding: 2px 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
  }
  .expose-wrap {
    display: flex;
    flex-direction: column;
  }
  .disclose {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 11px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
    font-weight: 600;
    text-align: start;
  }
  .disclose[aria-expanded='true'] {
    border-radius: var(--radius-m, 8px) var(--radius-m, 8px) 0 0;
  }
  .disclose span {
    display: inline-block;
    transition: transform 120ms ease;
  }
  .disclose span.open {
    transform: rotate(90deg);
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
    .tools-head {
      align-items: stretch;
      flex-direction: column;
    }
    .filter {
      flex-basis: auto;
      width: 100%;
      box-sizing: border-box;
    }
    .panel-actions {
      align-self: flex-start;
    }
  }
</style>
