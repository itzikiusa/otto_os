<script lang="ts">
  // Register a governed MCP server. Two transports:
  //   stdio — Otto spawns `command args` (with env + secret env) as the daemon.
  //           This runs an ARBITRARY local command and requires MCP Admin.
  //   http  — Otto POSTs JSON-RPC to `url` with headers (+ secret headers).
  // Secret env/header values are write-only: they go to the macOS Keychain and
  // are never returned — responses only carry their key names + has_secret.
  import Modal from '../../lib/components/Modal.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type {
    CreateMcpControlServerReq,
    McpInjectionRisk,
    McpToolAccess,
    McpTransport,
  } from '../../lib/api/types';

  interface Props {
    wsId: string;
    onclose: () => void;
    onsaved: () => void;
  }
  let { wsId, onclose, onsaved }: Props = $props();

  let name = $state('');
  let transport = $state<McpTransport>('stdio');
  let description = $state('');
  // stdio
  let command = $state('');
  let argsText = $state('');
  let envText = $state('');
  let secretEnvText = $state('');
  // http
  let url = $state('');
  let headersText = $state('');
  let secretHeadersText = $state('');
  // governance
  let injectionRisk = $state<McpInjectionRisk>('medium');
  let defaultToolAccess = $state<McpToolAccess>('allow');
  let enabled = $state(false);

  let saving = $state(false);

  /** One value per non-empty line. */
  function parseLines(text: string): string[] {
    return text
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean);
  }

  /** `KEY=value` per non-empty line → a record (value may be empty). */
  function parseKv(text: string): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of text.split('\n')) {
      const t = line.trim();
      if (!t) continue;
      const eq = t.indexOf('=');
      if (eq < 0) out[t] = '';
      else out[t.slice(0, eq).trim()] = t.slice(eq + 1).trim();
    }
    return out;
  }

  async function save(): Promise<void> {
    if (!name.trim()) {
      toasts.error('A server name is required');
      return;
    }
    if (transport === 'stdio' && !command.trim()) {
      toasts.error('A stdio server needs a command');
      return;
    }
    if (transport === 'http' && !url.trim()) {
      toasts.error('An http server needs a URL');
      return;
    }
    const body: CreateMcpControlServerReq = {
      name: name.trim(),
      transport,
      description: description.trim() || null,
      injection_risk: injectionRisk,
      default_tool_access: defaultToolAccess,
      enabled,
    };
    if (transport === 'stdio') {
      body.command = command.trim();
      body.args = parseLines(argsText);
      body.env = parseKv(envText);
      body.secret_env = parseKv(secretEnvText);
    } else {
      body.url = url.trim();
      body.headers = parseKv(headersText);
      body.secret_headers = parseKv(secretHeadersText);
    }
    saving = true;
    try {
      await mcpCpApi.cpCreate(wsId, body);
      toasts.success('MCP server added', name.trim());
      onsaved();
      onclose();
    } catch (e) {
      toasts.error('Could not add server', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }
</script>

<Modal title="Add MCP server" width={560} {onclose}>
  <div class="form">
    <label class="field">
      <span>Name</span>
      <input bind:value={name} placeholder="e.g. linear, github, web-fetch" />
    </label>

    <label class="field">
      <span>Transport</span>
      <select bind:value={transport}>
        <option value="stdio">stdio (spawn a local command)</option>
        <option value="http">http (Streamable HTTP / JSON-RPC)</option>
      </select>
    </label>

    {#if transport === 'stdio'}
      <p class="warn">
        ⚠ A stdio server runs an arbitrary command <strong>as the Otto daemon</strong>. Registering
        one requires MCP Admin and is audited.
      </p>
      <label class="field">
        <span>Command</span>
        <input bind:value={command} placeholder="npx" class="mono" />
      </label>
      <label class="field">
        <span>Arguments <em>(one per line)</em></span>
        <textarea bind:value={argsText} rows="3" class="mono" placeholder={'-y\n@modelcontextprotocol/server-github'}></textarea>
      </label>
      <label class="field">
        <span>Env <em>(KEY=value, one per line)</em></span>
        <textarea bind:value={envText} rows="2" class="mono" placeholder="LOG_LEVEL=info"></textarea>
      </label>
      <label class="field">
        <span>Secret env <em>(KEY=value — stored in Keychain, never shown again)</em></span>
        <textarea bind:value={secretEnvText} rows="2" class="mono" placeholder="GITHUB_TOKEN=ghp_…"></textarea>
      </label>
    {:else}
      <label class="field">
        <span>URL</span>
        <input bind:value={url} placeholder="https://mcp.example.com/rpc" class="mono" />
      </label>
      <label class="field">
        <span>Headers <em>(Name=value, one per line)</em></span>
        <textarea bind:value={headersText} rows="2" class="mono" placeholder="X-Client=otto"></textarea>
      </label>
      <label class="field">
        <span>Secret headers <em>(stored in Keychain, never shown again)</em></span>
        <textarea bind:value={secretHeadersText} rows="2" class="mono" placeholder="Authorization=Bearer …"></textarea>
      </label>
    {/if}

    <label class="field">
      <span>Description</span>
      <input bind:value={description} placeholder="What this server provides (optional)" />
    </label>

    <div class="row2">
      <label class="field">
        <span>Injection risk</span>
        <select bind:value={injectionRisk}>
          <option value="low">low</option>
          <option value="medium">medium</option>
          <option value="high">high</option>
        </select>
      </label>
      <label class="field">
        <span>Default tool access</span>
        <select bind:value={defaultToolAccess}>
          <option value="allow">allow</option>
          <option value="deny">deny</option>
        </select>
      </label>
    </div>

    <label class="check">
      <input type="checkbox" bind:checked={enabled} />
      <span>Enable now (governed by the control plane)</span>
    </label>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose} disabled={saving}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={saving}>
      {saving ? 'Adding…' : 'Add server'}
    </button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
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
  .field em {
    font-style: normal;
    opacity: 0.7;
  }
  .row2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  input,
  select,
  textarea {
    width: 100%;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    padding: 7px 9px;
    font-size: 13px;
  }
  textarea {
    resize: vertical;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text);
  }
  .check input {
    width: auto;
  }
  .warn {
    margin: 0;
    font-size: 12px;
    color: #e0a000;
    background: color-mix(in srgb, #e0a000 12%, transparent);
    border: 1px solid color-mix(in srgb, #e0a000 35%, transparent);
    border-radius: var(--radius-s, 6px);
    padding: 8px 10px;
  }
</style>
