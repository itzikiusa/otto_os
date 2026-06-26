<script lang="ts">
  // Per-server discovered tools with their governance controls (enable,
  // require-approval, risk-label override) plus a governed tool tester: run a
  // tool with JSON arguments through the full invoke pipeline (optionally
  // dry-run) and see the decision + preview / content / pending-approval id.
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type {
    McpInvokeResp,
    McpRiskLabel,
    McpServerDetail,
    McpToolView,
  } from '../../lib/api/types';
  import McpPill from './McpPill.svelte';

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
    selectedServerId: string | null;
    onSelect: (id: string) => void;
  }
  let { wsId, servers, selectedServerId, onSelect }: Props = $props();

  let tools = $state<McpToolView[]>([]);
  let loading = $state(false);
  let busyTool = $state<Record<string, boolean>>({});

  const server = $derived(servers.find((s) => s.id === selectedServerId) ?? null);

  async function loadTools(): Promise<void> {
    const id = selectedServerId;
    if (!id) {
      tools = [];
      return;
    }
    loading = true;
    try {
      tools = await mcpCpApi.cpTools(id);
    } catch (e) {
      toasts.error('Failed to load tools', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void selectedServerId;
    void loadTools();
  });

  function replaceTool(t: McpToolView): void {
    tools = tools.map((x) => (x.id === t.id ? t : x));
  }

  async function patchTool(
    t: McpToolView,
    patch: { enabled?: boolean; require_approval?: boolean; risk_label?: McpRiskLabel },
  ): Promise<void> {
    busyTool = { ...busyTool, [t.id]: true };
    try {
      const updated = await mcpCpApi.cpPatchTool(t.id, patch);
      replaceTool(updated);
    } catch (e) {
      toasts.error('Could not update tool', e instanceof Error ? e.message : String(e));
    } finally {
      const n = { ...busyTool };
      delete n[t.id];
      busyTool = n;
    }
  }

  async function discover(): Promise<void> {
    const id = selectedServerId;
    if (!id) return;
    loading = true;
    try {
      tools = await mcpCpApi.cpDiscover(id);
      toasts.success('Discovered tools', `${tools.length} found`);
    } catch (e) {
      toasts.error('Discovery failed', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // ---- tester ----
  let testToolName = $state('');
  let argsText = $state('{}');
  let dryRun = $state(true);
  let running = $state(false);
  let result = $state<McpInvokeResp | null>(null);
  let argError = $state<string | null>(null);

  // Default the tester to the first tool whenever the tool set changes.
  $effect(() => {
    if (tools.length && !tools.some((t) => t.name === testToolName)) {
      testToolName = tools[0].name;
    }
  });

  function pretty(v: unknown): string {
    try {
      return JSON.stringify(v, null, 2);
    } catch {
      return String(v);
    }
  }

  async function run(): Promise<void> {
    const id = selectedServerId;
    if (!id || !testToolName) return;
    let args: unknown = {};
    argError = null;
    if (argsText.trim()) {
      try {
        args = JSON.parse(argsText);
      } catch (e) {
        argError = `Arguments must be valid JSON: ${e instanceof Error ? e.message : String(e)}`;
        return;
      }
    }
    running = true;
    result = null;
    try {
      result = await mcpCpApi.cpInvoke(id, testToolName, {
        arguments: args,
        dry_run: dryRun,
        workspace_id: wsId,
      });
    } catch (e) {
      toasts.error('Invoke failed', e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }
</script>

<div class="tools">
  <div class="bar">
    <label class="picker">
      <span>Server</span>
      <select
        value={selectedServerId ?? ''}
        onchange={(e) => onSelect((e.currentTarget as HTMLSelectElement).value)}
      >
        {#if servers.length === 0}
          <option value="">No servers</option>
        {/if}
        {#each servers as s (s.id)}
          <option value={s.id}>{s.name}</option>
        {/each}
      </select>
    </label>
    {#if server}
      <McpPill kind="injection" value={server.injection_risk} />
      <span class="grow"></span>
      <button class="btn small" onclick={() => void discover()} disabled={loading}>
        {loading ? 'Discovering…' : 'Discover'}
      </button>
    {/if}
  </div>

  {#if !server}
    <p class="muted pad">Pick a server above (or add one on the Servers tab) to manage its tools.</p>
  {:else if loading && tools.length === 0}
    <p class="muted pad">Loading tools…</p>
  {:else if tools.length === 0}
    <div class="empty">
      <Icon name="zap" size={24} />
      <p>No tools discovered yet for <strong>{server.name}</strong>.</p>
      <button class="btn primary" onclick={() => void discover()}>Discover tools</button>
    </div>
  {:else}
    <div class="grid">
      <div class="thead">
        <span>Tool</span>
        <span>Risk</span>
        <span>Injection</span>
        <span>Enabled</span>
        <span>Approval</span>
        <span>Override risk</span>
      </div>
      {#each tools as t (t.id)}
        <div class="trow">
          <div class="tname">
            <span class="nm">{t.title || t.name}</span>
            <code class="code">{t.name}</code>
            {#if t.description}<span class="desc">{t.description}</span>{/if}
          </div>
          <span class="cell">
            <McpPill kind="risk" value={t.risk_label} small />
            {#if t.risk_overridden}<span class="pinned" title="Human-pinned override (survives re-discovery)"><Icon name="key" size={10} /></span>{/if}
          </span>
          <span class="cell"><McpPill kind="injection" value={t.injection_risk} small /></span>
          <span class="cell">
            <button
              class="switch"
              class:on={t.enabled}
              role="switch"
              aria-checked={t.enabled}
              disabled={busyTool[t.id]}
              onclick={() => void patchTool(t, { enabled: !t.enabled })}
              title={t.enabled ? 'Enabled' : 'Disabled'}
            ><span class="knob"></span></button>
          </span>
          <span class="cell">
            <button
              class="switch"
              class:on={t.require_approval}
              role="switch"
              aria-checked={t.require_approval}
              disabled={busyTool[t.id]}
              onclick={() => void patchTool(t, { require_approval: !t.require_approval })}
              title={t.require_approval ? 'Requires approval' : 'No approval required'}
            ><span class="knob"></span></button>
          </span>
          <span class="cell">
            <select
              value={t.risk_label}
              disabled={busyTool[t.id]}
              onchange={(e) =>
                void patchTool(t, {
                  risk_label: (e.currentTarget as HTMLSelectElement).value as McpRiskLabel,
                })}
            >
              <option value="read">read</option>
              <option value="write">write</option>
              <option value="dangerous">dangerous</option>
              <option value="unknown">unknown</option>
            </select>
          </span>
        </div>
      {/each}
    </div>

    <!-- Tool tester -->
    <div class="tester">
      <div class="tester-head">
        <Icon name="play" size={13} />
        <span class="th">Tool tester</span>
        <span class="muted small">Runs through the governance pipeline (allowlist → policy → approval → execute).</span>
      </div>
      <div class="tester-body">
        <div class="tcontrols">
          <label class="picker">
            <span>Tool</span>
            <select bind:value={testToolName}>
              {#each tools as t (t.id)}
                <option value={t.name}>{t.name}</option>
              {/each}
            </select>
          </label>
          <label class="check">
            <input type="checkbox" bind:checked={dryRun} />
            <span>Dry run (preview only)</span>
          </label>
          <span class="grow"></span>
          <button class="btn primary small" onclick={() => void run()} disabled={running || !testToolName}>
            {running ? 'Running…' : 'Run'}
          </button>
        </div>
        <label class="field">
          <span>Arguments (JSON)</span>
          <textarea bind:value={argsText} rows="4" class="mono" spellcheck="false"></textarea>
        </label>
        {#if argError}<p class="warn">{argError}</p>{/if}

        {#if result}
          <div class="result">
            <div class="rhead">
              <McpPill kind="decision" value={result.decision} />
              {#if result.dry_run}<span class="tag">dry run</span>{/if}
              {#if result.is_error}<span class="tag bad">tool error</span>{/if}
              {#if result.reason}<span class="reason">{result.reason}</span>{/if}
            </div>
            {#if result.decision === 'pending_approval'}
              <p class="pending">
                ⏳ Pending approval — review it on the <strong>Approvals</strong> tab.
                {#if result.approval_id}<br />Approval id: <code class="code">{result.approval_id}</code>{/if}
              </p>
            {:else if result.preview !== undefined && result.preview !== null}
              <pre class="json">{pretty(result.preview)}</pre>
            {:else if result.content !== undefined && result.content !== null}
              <pre class="json">{pretty(result.content)}</pre>
            {:else}
              <p class="muted small">No content returned.</p>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .tools {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .grow {
    flex: 1;
  }
  .picker {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .grid {
    overflow: auto;
  }
  .thead,
  .trow {
    display: grid;
    grid-template-columns: minmax(220px, 2fr) 110px 90px 70px 80px 130px;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
  }
  .thead {
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
  }
  .trow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .trow:hover {
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .tname {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .tname .nm {
    font-size: 13px;
    font-weight: 600;
  }
  .code {
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .desc {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cell {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .pinned {
    color: var(--text-dim);
  }
  select,
  textarea {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    padding: 5px 8px;
    font-size: 12.5px;
  }
  .switch {
    width: 30px;
    height: 17px;
    border-radius: 9px;
    border: none;
    background: color-mix(in srgb, var(--text-dim) 30%, transparent);
    position: relative;
    cursor: pointer;
    padding: 0;
    flex: none;
  }
  .switch.on {
    background: var(--status-working, #28c840);
  }
  .switch .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: #fff;
    transition: left 120ms ease;
  }
  .switch.on .knob {
    left: 15px;
  }
  .tester {
    margin: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background: var(--surface);
  }
  .tester-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
  }
  .tester-head .th {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .tester-body {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .tcontrols {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--text);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field > span {
    font-size: 12px;
    color: var(--text-dim);
  }
  textarea {
    width: 100%;
    resize: vertical;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .result {
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    background: var(--bg);
    padding: 10px;
  }
  .rhead {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  .reason {
    font-size: 12px;
    color: var(--text-dim);
  }
  .tag {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    border-radius: 4px;
    padding: 1px 6px;
  }
  .tag.bad {
    color: var(--status-exited, #ff5f57);
    background: color-mix(in srgb, var(--status-exited, #ff5f57) 18%, transparent);
  }
  .json {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 11.5px;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 320px;
    overflow: auto;
    color: var(--text);
  }
  .pending {
    margin: 0;
    font-size: 13px;
    color: #e0a000;
  }
  .warn {
    margin: 0;
    font-size: 12px;
    color: var(--status-exited, #ff5f57);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: 11px;
  }
  .pad {
    padding: 16px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
    padding: 36px 24px;
  }

  @media (max-width: 760px) {
    .thead {
      display: none;
    }
    .trow {
      grid-template-columns: 1fr 1fr;
      gap: 6px;
    }
    .tname {
      grid-column: 1 / -1;
    }
  }
</style>
