<script lang="ts">
  // The Tools view (#/mcp/otto): Otto's own MCP server — session attachment,
  // exposing it to external clients, the auto-approve rules, the tool catalog
  // (ToolCatalog: collapsed, searchable, windowed), what sessions see, and
  // external-client setup. An outage is ONE notice line with one Retry.
  import Icon from '../../lib/components/Icon.svelte';
  import { toastError } from '../../lib/toastError';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import { mcpCpExtraApi, type McpGatewayToolRow } from './cp-api';
  import ExposePanel from './ExposePanel.svelte';
  import ToolCatalog from './ToolCatalog.svelte';
  import { byCategory } from './toolCatalog';
  import AutoApprovePanel from './AutoApprovePanel.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { attachView, builtInToolsLine, gatewayView } from './sessionAttach';
  import { loadErrorText } from '../../lib/loadError';
  import type {
    McpAutoApproveRef,
    McpOttoServerStatus,
    McpOttoToolInfo,
    McpSessionAttach,
  } from '../../lib/api/types';

  interface Props {
    wsId: string | null;
  }

  let { wsId }: Props = $props();

  let status = $state<McpOttoServerStatus | null>(null);
  let loading = $state(false);
  let saving = $state(false);
  let loadError = $state<string | null>(null);
  let attach = $state<McpSessionAttach | null>(null);
  let attachBusy = $state(false);
  let attachGeneration = 0;
  let attachError = $state<string | null>(null);
  const attachState = $derived(attachView(wsId, attach, attachError));
  let gatewayTools = $state<McpGatewayToolRow[]>([]);
  let gatewayLoading = $state(false);
  let gatewayError = $state<string | null>(null);
  let exposeOpen = $state(false);
  /** Bumped when the catalog switches change rules — the rules panel reloads. */
  let rulesRevision = $state(0);

  const isMcpAdmin = $derived(auth.can('mcp', 'admin'));
  const tools = $derived(status?.tools ?? []);
  // Global `mcp_require_approval_dangerous` — when off, nothing asks at all.
  const approvalsOn = $derived(status?.require_approval_dangerous ?? true);
  /** Every tool by category — the token picker in ExposePanel. */
  const allGroups = $derived(byCategory(tools));
  const gatewayNames = $derived([...new Set(gatewayTools.map((tool) => tool.name))]);
  const shownGatewayNames = $derived(gatewayNames.slice(0, 60));
  const gatewayState = $derived(gatewayView(wsId, gatewayLoading, gatewayError, gatewayNames.length));

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      status = await mcpCpApi.cpOttoServer();
    } catch (e) {
      status = null;
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  /** Resolves false when the save failed. The checkboxes here are one-way
   *  (`checked={…}`), so a failed save must put the clicked box back itself —
   *  otherwise it keeps showing a state the daemon never stored. */
  async function patch(body: Parameters<typeof mcpCpApi.cpUpdateOttoServer>[0]): Promise<boolean> {
    saving = true;
    try {
      const next = await mcpCpApi.cpUpdateOttoServer(body);
      status = { ...next, token: null };
      return true;
    } catch (e) {
      toastError('Couldn’t update the setting', e);
      return false;
    } finally {
      saving = false;
    }
  }

  async function afterRuleChange(): Promise<void> {
    rulesRevision++;
    await load();
  }

  /** One Retry for an outage: the catalog, the attach switch, the gateway
   *  tools and the rules panel reload together. */
  function retryAll(): void {
    rulesRevision++;
    void load();
    if (wsId) {
      void loadAttach(wsId);
      void loadGatewayTools(wsId);
    }
  }

  async function toggleEnabled(input: HTMLInputElement): Promise<void> {
    if (!(await patch({ enabled: !(status?.enabled ?? false) }))) input.checked = status?.enabled ?? false;
  }

  async function loadAttach(id: string): Promise<void> {
    const generation = ++attachGeneration;
    attach = null;
    attachError = null;
    try {
      const next = await mcpCpExtraApi.sessionAttach(id);
      if (wsId === id && generation === attachGeneration) attach = next;
    } catch (e) {
      if (wsId === id && generation === attachGeneration) attachError = loadErrorText(e);
    }
  }

  async function updateAttach(enabled: boolean, input: HTMLInputElement): Promise<void> {
    const id = wsId;
    if (!id || !attach || attachBusy) return;
    const reported = attach.attached; // the box goes back to it if the save fails
    const generation = ++attachGeneration;
    attachBusy = true;
    attachError = null;
    try {
      const next = await mcpCpExtraApi.setSessionAttach(id, { enabled });
      if (wsId !== id || generation !== attachGeneration) return;
      attach = next;
      toasts.success(
        enabled ? 'Attached to sessions' : 'Detached from sessions',
        'Applies to sessions started from now on.',
      );
    } catch (e) {
      if (wsId !== id || generation !== attachGeneration) return;
      // Put the box back: the daemon still has the old value.
      input.checked = reported;
      toastError(enabled ? 'Couldn’t attach to sessions' : 'Couldn’t detach from sessions', e);
    } finally {
      if (wsId === id && generation === attachGeneration) attachBusy = false;
    }
  }

  let gatewaySeq = 0;
  async function loadGatewayTools(id: string): Promise<void> {
    const seq = typeof gatewaySeq === 'undefined' ? 0 : ++gatewaySeq;
    const ok = () => wsId === id && (typeof gatewaySeq === 'undefined' || seq === gatewaySeq);
    gatewayLoading = true;
    gatewayError = null;
    try {
      const result = await mcpCpExtraApi.gatewayTools(id);
      if (ok()) gatewayTools = result.tools;
    } catch (e) {
      if (ok()) { gatewayTools = []; gatewayError = loadErrorText(e); }
    } finally {
      if (ok()) gatewayLoading = false;
    }
  }

  function copyGatewayTools(): void {
    void copyWithToast(gatewayNames.join('\n'), 'tool names');
  }

  $effect(() => {
    void load();
  });

  $effect(() => {
    const id = wsId;
    attachGeneration++; gatewaySeq++;
    attachBusy = false;
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

{#if loadError && !status}
  <!-- One outage, one line: the catalog, the attach switch, the gateway tools
       and the rules all come from the same daemon. -->
  <div data-testid="mcp-outage"><LoadState what="the MCP control plane" variant="compact" error={loadError} empty onretry={retryAll} /></div>
{:else}
<div class="otto">
  <section class="hero">
    {#if attachState === 'error'}
      <LoadState what="the session-attach setting" variant="compact" error={attachError} empty onretry={() => wsId && void loadAttach(wsId)} />
    {:else if attachState === 'loading'}
      <LoadState what="the session-attach setting" variant="compact" loading empty rows={1} />
    {:else if attachState === 'ready' && attach}
      {#key wsId}
      <label class="switchrow">
        <input
          type="checkbox"
          data-testid="mcp-session-attach"
          checked={attach.attached}
          disabled={attachBusy || !isMcpAdmin}
          onchange={(event) => void updateAttach(event.currentTarget.checked, event.currentTarget)}
        />
        <span class="switchcopy">
          <strong>Give sessions in {ws.current?.name ?? 'this workspace'} Otto’s read-only tools</strong>
          <span class="muted small">Sessions started from now on get them (ottod mcp-tools).</span>
        </span>
      </label>
      {/key}
    {:else}
      <p class="muted small">Select a workspace to manage session attachment.</p>
    {/if}

    <label class="switchrow">
      <input
        type="checkbox"
        data-testid="mcp-outward-enabled"
        checked={Boolean(status?.enabled)}
        disabled={saving || !status || !isMcpAdmin}
        onchange={(event) => void toggleEnabled(event.currentTarget)}
      />
      <span class="switchcopy">
        <strong>Let external clients use these tools</strong>
        <span class="muted small">Over stdio or HTTP with an access token. Tools that can change things start off and ask a person first; every call is audited.</span>
      </span>
    </label>
  </section>

  <AutoApprovePanel {isMcpAdmin} revision={rulesRevision} onchange={() => void load()} />

  {#if loading && !status}
    <LoadState what="Otto’s tools" loading empty rows={3} />
  {:else if status}
    <ToolCatalog {tools} {patch} reload={afterRuleChange} disabled={saving || !isMcpAdmin} {approvalsOn} />
  {/if}

  <section class="panel" data-testid="mcp-sessions-panel">
    <div class="tools-head">
      <h4 class="sec">What my sessions see</h4>
      {#if wsId}
        <div class="panel-actions">
          <button
            class="btn small"
            disabled={gatewayLoading || !gatewayNames.length}
            onclick={() => void copyGatewayTools()}
          >Copy</button>
          <button
            class="btn small"
            disabled={gatewayLoading}
            onclick={() => void loadGatewayTools(wsId!)}
          >{gatewayLoading ? 'Refreshing…' : 'Refresh'}</button>
        </div>
      {/if}
    </div>
    <p class="small" data-testid="mcp-builtin-tools">{builtInToolsLine(attachState, attach)} — Otto’s read-only tool server (<code>ottod mcp-tools</code>).</p>
    {#if gatewayState === 'no-workspace'}
      <p class="muted small">Select a workspace to inspect its gateway tools.</p>
    {:else if gatewayState === 'error'}
      <LoadState what="the gateway tools" variant="compact" error={gatewayError} empty onretry={() => wsId && void loadGatewayTools(wsId)} />
    {:else if gatewayState === 'loading'}<LoadState what="the gateway tools" variant="compact" loading empty rows={1} />
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
      <span class:open={exposeOpen}><Icon name="chevronRight" noflip size={13} /></span>
      Connect an external client
    </button>
    {#if exposeOpen}<ExposePanel groups={allGroups} {isMcpAdmin} />{/if}
  </div>
</div>
{/if}

<style>
  /* The gutter comes from the padded PageBody around it (McpPage). */
  .otto {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: var(--page-readable);
    box-sizing: border-box;
  }
  .hero,
  .panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 14px;
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
    gap: 2px;
  }
  .switchcopy strong {
    font-size: var(--fs-m);
    color: var(--text);
  }
  .panel p {
    margin: 0;
  }
  .tools-head {
    display: flex;
    align-items: center;
    gap: 12px;
    justify-content: space-between;
  }
  .panel-actions,
  .gateway-list {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .gateway-list code {
    font-size: var(--fs-xs);
    padding: 2px 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
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
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
    font-weight: 600;
    text-align: start;
  }
  .disclose[aria-expanded='true'] {
    border-radius: var(--radius-m) var(--radius-m) 0 0;
  }
  .disclose span {
    display: inline-flex;
    transition: transform var(--dur-fast) ease;
  }
  .disclose span.open {
    transform: rotate(90deg);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }

  @media (max-width: 640px) {
    .tools-head {
      align-items: stretch;
      flex-direction: column;
    }
    .panel-actions {
      align-self: flex-start;
    }
  }
</style>
