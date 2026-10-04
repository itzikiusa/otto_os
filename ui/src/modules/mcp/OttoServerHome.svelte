<script lang="ts">
  // Otto's built-in MCP server is the control plane's home: session attachment,
  // the outward catalog, what sessions see, and external-client setup.
  import Icon from '../../lib/components/Icon.svelte';
  import { toastError } from '../../lib/toastError';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { confirmer } from '../../lib/confirm.svelte';
  import { mcpCpExtraApi, type McpGatewayToolRow } from './cp-api';
  import ExposePanel from './ExposePanel.svelte';
  import AutoApprovePanel from './AutoApprovePanel.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
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
  let filter = $state('');
  let attach = $state<McpSessionAttach | null>(null);
  let attachBusy = $state(false);
  let attachGeneration = 0;
  let attachError = $state<string | null>(null);
  let gatewayTools = $state<McpGatewayToolRow[]>([]);
  let gatewayLoading = $state(false);
  let gatewayError = $state<string | null>(null);
  let exposeOpen = $state(false);
  /** Bumped when the catalog switches change rules — the rules panel reloads. */
  let rulesRevision = $state(0);

  const isMcpAdmin = $derived(auth.can('mcp', 'admin'));
  const tools = $derived(status?.tools ?? []);
  const enabledNames = $derived(new Set(tools.filter((tool) => tool.enabled).map((tool) => tool.name)));
  const mutatingCount = $derived(tools.filter((tool) => tool.mutating).length);
  // Global `mcp_require_approval_dangerous` — when off, nothing asks at all.
  const approvalsOn = $derived(status?.require_approval_dangerous ?? true);
  // Enabled mutating tools some auto-approve rule (any scope) covers.
  const skippedCount = $derived(
    tools.filter((tool) => tool.enabled && tool.mutating && (tool.auto_approved_by?.length ?? 0) > 0).length,
  );
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

  async function toggleEnabled(input: HTMLInputElement): Promise<void> {
    if (!(await patch({ enabled: !(status?.enabled ?? false) }))) input.checked = status?.enabled ?? false;
  }

  async function toggleTool(name: string, input: HTMLInputElement): Promise<void> {
    const next = new Set(enabledNames);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    if (!(await patch({ tools: [...next] }))) input.checked = enabledNames.has(name);
  }

  async function setCategory(categoryTools: McpOttoToolInfo[], enable: boolean): Promise<void> {
    const next = new Set(enabledNames);
    for (const tool of categoryTools) {
      if (enable) next.add(tool.name);
      else next.delete(tool.name);
    }
    await patch({ tools: [...next] });
  }

  /** Mutating tools that are enabled — the only ones an approval applies to. */
  function gatedTools(list: McpOttoToolInfo[]): McpOttoToolInfo[] {
    return list.filter((tool) => tool.mutating && tool.enabled);
  }

  /** The GLOBAL per-tool rule behind a tool's "Auto-approve everywhere" switch. */
  function globalToolRule(tool: McpOttoToolInfo): McpAutoApproveRef | undefined {
    return tool.auto_approved_by?.find((r) => r.scope === 'global' && r.target_kind === 'tool');
  }

  /** The GLOBAL category rule a category's switch reflects (read off any tool
   *  of the category it covers). */
  function globalCategoryRule(category: string, list: McpOttoToolInfo[]): McpAutoApproveRef | undefined {
    for (const tool of list) {
      const rule = tool.auto_approved_by?.find(
        (r) => r.scope === 'global' && r.target_kind === 'category' && r.target === category,
      );
      if (rule) return rule;
    }
    return undefined;
  }

  /** Rules other than the tool's own global switch — shown as badges. */
  function otherRules(tool: McpOttoToolInfo): McpAutoApproveRef[] {
    const own = globalToolRule(tool);
    return (tool.auto_approved_by ?? []).filter((r) => r.id !== own?.id);
  }

  function ruleBadge(rule: McpAutoApproveRef): string {
    const where = rule.scope === 'global' ? 'everywhere' : rule.scope === 'workspace' ? 'in a workspace' : 'for one session';
    return rule.target_kind === 'category' ? `${rule.target} writes ${where}` : `${where}`;
  }

  async function confirmAuto(what: string, irreversible: boolean): Promise<boolean> {
    const first = await confirmer.ask(
      `Auto-approve ${what} everywhere? Agents and external clients will run it without a human approval. Every call is still audited as “Auto approved”, and you can turn this off here at any time.`,
      { title: 'Auto-approve', confirmLabel: 'Auto-approve', danger: true },
    );
    if (!first || !irreversible) return first;
    // The guardrail's second explicit toggle.
    return confirmer.ask(
      `${what} is IRREVERSIBLE — nobody can undo it from Otto once it runs. Run it without asking anyway?`,
      { title: 'Irreversible tool', confirmLabel: 'Run without asking', danger: true },
    );
  }

  async function afterRuleChange(): Promise<void> {
    rulesRevision++;
    await load();
  }

  /** The per-tool "Auto-approve everywhere" switch: a global per-tool rule. A
   *  cancelled confirm or failed save restores the checkbox. */
  async function setAuto(tool: McpOttoToolInfo, on: boolean, input: HTMLInputElement): Promise<void> {
    const rule = globalToolRule(tool);
    if (on && !(await confirmAuto(tool.name, !!tool.irreversible))) {
      input.checked = false;
      return;
    }
    saving = true;
    try {
      if (on) {
        await mcpCpExtraApi.createAutoApprove({
          scope: 'global',
          target_kind: 'tool',
          target: tool.name,
          allow_irreversible: tool.irreversible ? true : undefined,
        });
      } else if (rule) {
        await mcpCpExtraApi.deleteAutoApprove(rule.id);
      }
      await afterRuleChange();
    } catch (e) {
      input.checked = !!rule;
      toastError('Couldn’t update the setting', e);
    } finally {
      saving = false;
    }
  }

  /** "Auto-approve every <category> write": one global category rule —
   *  covers future tools of the category, never an irreversible one. */
  async function setCategoryAuto(category: string, list: McpOttoToolInfo[], on: boolean, input: HTMLInputElement): Promise<void> {
    const rule = globalCategoryRule(category, list);
    if (on && !(await confirmAuto(`every ${category} write (except irreversible tools)`, false))) {
      input.checked = false;
      return;
    }
    saving = true;
    try {
      if (on) await mcpCpExtraApi.createAutoApprove({ scope: 'global', target_kind: 'category', target: category });
      else if (rule) await mcpCpExtraApi.deleteAutoApprove(rule.id);
      await afterRuleChange();
    } catch (e) {
      input.checked = !!rule;
      toastError('Couldn’t update the setting', e);
    } finally {
      saving = false;
    }
  }

  async function loadAttach(id: string): Promise<void> {
    const generation = ++attachGeneration;
    attach = null;
    attachError = null;
    try {
      const next = await mcpCpExtraApi.sessionAttach(id);
      if (wsId === id && generation === attachGeneration) attach = next;
    } catch (e) {
      if (wsId === id && generation === attachGeneration) attachError = e instanceof Error ? e.message : String(e);
    }
  }

  async function updateAttach(enabled: boolean, input: HTMLInputElement): Promise<void> {
    const id = wsId;
    if (!id || !attach || attachBusy) return;
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
      input.checked = attach?.attached ?? true;
      toasts.error(enabled ? 'Could not attach to sessions' : 'Could not detach from sessions', e instanceof Error ? e.message : String(e));
    } finally {
      if (wsId === id && generation === attachGeneration) attachBusy = false;
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
    attachGeneration++;
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

  /** Tool descriptions are markdown-ish: render `inline code` spans as code
   *  instead of showing raw backticks. */
  function descParts(text: string | null | undefined): { text: string; code: boolean }[] {
    return (text ?? '').split('`').map((t, i) => ({ text: t, code: i % 2 === 1 })).filter((p) => p.text !== '');
  }
</script>

<div class="otto">
  <section class="hero">
    <div class="hl-title">
      <Icon name="plug" size={16} />
      <span>Otto's built-in MCP server</span>
    </div>

    {#if wsId}
      {#key wsId}
      <label class="switchrow">
        <input
          type="checkbox"
          data-testid="mcp-session-attach"
          checked={attach?.attached ?? true}
          disabled={!attach || attachBusy || !isMcpAdmin}
          onchange={(event) => void updateAttach(event.currentTarget.checked, event.currentTarget)}
        />
        <span class="switchcopy">
          <strong>Attach to sessions in {ws.current?.name ?? 'this workspace'}</strong>
          <span class="muted small">
            Sessions started in this workspace get Otto's read-only tools (ottod mcp-tools). Applies to sessions started from now on.
          </span>
        </span>
      </label>
      {/key}
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
        onchange={(event) => void toggleEnabled(event.currentTarget)}
      />
      <span class="switchcopy">
        <strong>Expose to external clients</strong>
        <span class="muted small">
          Serve the otto.* tools below to external MCP clients over stdio or HTTP, authenticated by an access token. Mutating tools stay off by default; dangerous calls are approval-gated; every call is audited.
        </span>
      </span>
    </label>
    <p class="counts muted small">
      {tools.length} tools · {enabledNames.size} exposed · {mutatingCount} mutating{#if skippedCount}{' '}· <span class="noask">{skippedCount} auto-approved</span>{/if}
    </p>
  </section>

  <AutoApprovePanel {isMcpAdmin} revision={rulesRevision} onchange={() => void load()} />

  {#if (loading || loadError) && !status}
    <LoadState what="the external tool catalog" {loading} error={loadError} empty rows={3} onretry={() => void load()} />
  {/if}

  <div class="tools-head">
    <div>
      <h4 class="sec">External tool catalog</h4>
      <p class="muted small catalog-note">
        Enabled tools are served to external clients, and to Otto sessions (for tools that aren't
        built in, e.g. <code>otto_create_pr</code>) through the same gate. A mutating tool asks a
        person before each call unless an <em>Auto-approve</em> rule covers it; every call is
        audited. Policies govern registered external servers, not these tools.
      </p>
    </div>
    <input
      class="input filter"
      type="search"
      placeholder="Filter tools…"
      bind:value={filter}
      aria-label="Filter tools"
    />
  </div>
  {#if status && !approvalsOn}
    <p class="warn" data-testid="mcp-approvals-globally-off">
      Approval prompts are turned off globally (<code>mcp_require_approval_dangerous</code>), so no
      otto.* call asks for approval — the <em>Auto-approve</em> rules have no effect until that is
      turned back on.
    </p>
  {/if}
  {#each groups as group (group.cat)}
    {@const gated = gatedTools(group.tools)}
    <div class="grp">
      <div class="grp-head">
        <span class="grp-name">{group.cat}</span>
        <span class="grp-count muted">
          {group.tools.filter((tool) => tool.enabled).length}/{group.tools.length}
        </span>
        <span class="grow"></span>
        <button
          class="btn small"
          disabled={saving || !status || !isMcpAdmin}
          onclick={() => void setCategory(group.tools, true)}
        >All</button>
        <button
          class="btn small"
          disabled={saving || !status || !isMcpAdmin}
          onclick={() => void setCategory(group.tools, false)}
        >None</button>
      </div>
      {#if gated.length}
        {@const catRule = globalCategoryRule(group.cat, group.tools)}
        <label class="grp-ask" title="One rule for the category: covers its mutating tools (and future ones), never an irreversible tool.">
          <input
            type="checkbox"
            data-testid="mcp-category-auto"
            checked={!!catRule}
            disabled={saving || !status || !isMcpAdmin || !approvalsOn}
            onchange={(event) => void setCategoryAuto(group.cat, group.tools, event.currentTarget.checked, event.currentTarget)}
          />
          <span>Auto-approve every {group.cat} write</span>
          {#if group.tools.some((tool) => tool.mutating && tool.irreversible)}
            <span class="muted">(irreversible tools keep asking)</span>
          {/if}
        </label>
      {/if}
      <div class="tool-list">
        {#each group.tools as tool (tool.name)}
          <div class="tool">
            <label class="tool-main">
              <input
                type="checkbox"
                checked={tool.enabled}
                disabled={saving || !status || !isMcpAdmin}
                onchange={(event) => void toggleTool(tool.name, event.currentTarget)}
              />
              <span class="t-meta">
                <span class="t-name mono">
                  {tool.name}{#if tool.mutating}<span class="mut">Mutating</span>{/if}
                </span>
                <span class="t-desc">{#each descParts(tool.description) as part, i (i)}{#if part.code}<code>{part.text}</code>{:else}{part.text}{/if}{/each}</span>
              </span>
            </label>
            {#if tool.mutating && tool.enabled}
              {@const own = globalToolRule(tool)}
              <div class="ask">
                <label
                  class="ask-toggle"
                  title="On: calls run without asking a person — still audited. Off: a human approves each call."
                >
                  <input
                    type="checkbox"
                    data-testid={`mcp-auto-${tool.name}`}
                    checked={!!own}
                    disabled={saving || !status || !isMcpAdmin}
                    onchange={(event) => void setAuto(tool, event.currentTarget.checked, event.currentTarget)}
                  />
                  <span>Auto-approve everywhere</span>
                </label>
                {#if tool.irreversible}<span class="irr">Irreversible</span>{/if}
                {#each otherRules(tool) as rule (rule.id)}
                  <span class="noask" title={rule.name}>Auto-approved {ruleBadge(rule)}</span>
                {/each}
                {#if own}<span class="noask">runs without asking · still audited</span>{/if}
                {#if !own && !otherRules(tool).length}<span class="muted">asks before each call</span>{/if}
              </div>
            {/if}
          </div>
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
      <span class:open={exposeOpen}><Icon name="chevronRight" noflip size={13} /></span>
      Connect an external client
    </button>
    {#if exposeOpen}<ExposePanel {groups} {isMcpAdmin} />{/if}
  </div>
</div>

<style>
  .otto {
    padding: 18px 20px 40px;
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
  .hl-title {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
    font-size: var(--fs-l);
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
    font-size: var(--fs-m);
    color: var(--text);
  }
  .counts,
  .catalog-note,
  .panel p {
    margin: 0;
  }
  .sec {
    margin: 4px 0 0;
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .grp-count {
    font-size: var(--fs-xs);
  }
  .grow {
    flex: 1 1 auto;
  }
  .tool-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .tool {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .tool:last-child {
    border-bottom: none;
  }
  .tool:hover {
    background: var(--hover);
  }
  .tool-main {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    cursor: pointer;
  }
  .tool-main input {
    margin-top: 2px;
  }
  /* Indented under the tool name (checkbox width + gap). */
  .ask {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-inline-start: 23px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .noask {
    color: var(--warning);
  }
  .ask-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
  }
  .irr {
    font-size: var(--fs-xs);
    color: var(--danger);
    background: var(--danger-soft);
    border-radius: 999px;
    padding: 0 7px;
  }
  .grp-ask {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0 2px;
    font-size: var(--fs-s);
    color: var(--text);
    cursor: pointer;
  }
  .t-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .t-name {
    font-size: var(--fs-m);
    color: var(--text);
  }
  .mut {
    margin-inline-start: 8px;
    font-family: var(--font-ui);
    font-size: var(--fs-xs);
    color: var(--warning);
    background: var(--warning-soft);
    border-radius: 999px;
    padding: 0 7px;
  }
  .t-desc {
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .t-desc code {
    font-size: var(--fs-xs);
    padding: 0 4px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
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
    font-size: var(--fs-xs);
    padding: 2px 5px;
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
    padding: 11px 12px;
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
    transition: transform 120ms ease;
  }
  .disclose span.open {
    transform: rotate(90deg);
  }
  .warn {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
    border-radius: var(--radius-s);
    padding: 10px 12px;
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
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
