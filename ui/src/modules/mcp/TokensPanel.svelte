<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpTokensApi } from '../../lib/api/mcp';
  import { api, baseUrl } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { mcpCpExtraApi } from './cp-api';
  import type { McpOttoToolInfo, McpScope, McpTokenInfo, User } from '../../lib/api/types';

  interface Props {
    groups: { cat: string; tools: McpOttoToolInfo[] }[];
  }

  let { groups }: Props = $props();

  let tokens = $state<McpTokenInfo[]>([]);
  let users = $state<User[]>([]);
  let tokensLoaded = $state(false);
  let showCreate = $state(false);
  let fOwner = $state('');
  let fLabel = $state('');
  let fAllowWrites = $state(false);
  let fRestrictTools = $state(false);
  let fTools = $state<Set<string>>(new Set());
  let fWorkspace = $state('');
  let creating = $state(false);
  let createdToken = $state<{ secret: string; info: McpTokenInfo } | null>(null);
  let rotatedToken = $state<{ secret: string; info: McpTokenInfo } | null>(null);

  async function loadTokens(): Promise<void> {
    try {
      const result = await mcpTokensApi.list();
      tokens = result.tokens;
    } catch (e) {
      toasts.error('Could not load MCP tokens', e instanceof Error ? e.message : String(e));
    } finally {
      tokensLoaded = true;
    }
  }

  async function loadUsers(): Promise<void> {
    try {
      users = await api.get<User[]>('/users');
    } catch {
      users = [];
    }
  }

  function bare(name: string): string {
    return name.replace(/^otto\./, '');
  }

  function toggleFormTool(name: string): void {
    const tool = bare(name);
    const next = new Set(fTools);
    if (next.has(tool)) next.delete(tool);
    else next.add(tool);
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
      rotatedToken = null;
      toasts.success('MCP token created', 'Copy it now — it is shown only once.');
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

  async function rotateToken(t: McpTokenInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Rotate MCP token ${t.label ? `“${t.label}” ` : ''}(${t.token_prefix}…) for ${t.username}? Only this token is replaced — every other token keeps working. The old secret stops working immediately.`,
      { title: 'Rotate token', confirmLabel: 'Rotate', danger: true },
    );
    if (!ok) return;
    try {
      const resp = await mcpCpExtraApi.rotateToken(t.id);
      rotatedToken = { secret: resp.token, info: resp.info };
      createdToken = null;
      toasts.success('Token rotated', 'Only this token was replaced.');
      await loadTokens();
    } catch (e) {
      toasts.error('Rotate failed', e instanceof Error ? e.message : String(e));
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

  function scopeSummary(scope: McpScope): string {
    const toolPart =
      scope.tools == null
        ? 'all tools'
        : `${scope.tools.length} tool${scope.tools.length === 1 ? '' : 's'}`;
    const writePart = scope.allow_writes ? 'read + write' : 'read-only';
    const wsPart = scope.workspace_id ? ' • 1 workspace' : '';
    return `${toolPart} • ${writePart}${wsPart}`;
  }

  function clientCommand(token: string): string {
    return `claude mcp add --transport http otto ${baseUrl()}/api/v1/mcp/http --header "Authorization: Bearer ${token}"`;
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`${what} copied`);
    } catch {
      toasts.error('Copy failed', 'Select and copy manually.');
    }
  }

  $effect(() => {
    void loadTokens();
    void loadUsers();
  });
</script>

<div class="tokens">
  <div class="tools-head">
    <h4 class="sec">Access tokens</h4>
    <button
      class="btn xs"
      data-testid="mcp-new-token"
      onclick={() => (showCreate = !showCreate)}
    >
      {showCreate ? 'Cancel' : 'New token'}
    </button>
  </div>
  <p class="muted small">
    Each token authenticates as a user and carries its own tool, write, and optional workspace scope.
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

  {#if rotatedToken}
    <div class="token-once" data-testid="mcp-rotated-token">
      <div class="to-head">
        <Icon name="key" size={13} />
        <strong>Rotated token for {rotatedToken.info.username} — shown once. Copy it now.</strong>
        <span class="grow"></span>
        <button class="btn xs" onclick={() => void copy(rotatedToken!.secret, 'Token')}>Copy token</button>
        <button class="btn xs" onclick={() => void copy(clientCommand(rotatedToken!.secret), 'Command')}>Copy command</button>
        <button class="btn xs" onclick={() => (rotatedToken = null)}>Dismiss</button>
      </div>
      <code class="token">{rotatedToken.secret}</code>
      <code class="token cmd">{clientCommand(rotatedToken.secret)}</code>
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
            {#each ws.workspaces as workspace (workspace.id)}
              <option value={workspace.id}>{workspace.name}</option>
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
          {#each groups as group (group.cat)}
            <div class="pick-grp">
              <span class="grp-name">{group.cat}</span>
              {#each group.tools as tool (tool.name)}
                <label class="ptool">
                  <input
                    type="checkbox"
                    checked={fTools.has(bare(tool.name))}
                    onchange={() => toggleFormTool(tool.name)}
                  />
                  <span class="mono">{bare(tool.name)}</span>{#if tool.mutating}<span class="mut">mutating</span>{/if}
                </label>
              {/each}
            </div>
          {/each}
        </div>
      {/if}
      <div class="cactions">
        <button
          class="btn primary"
          data-testid="mcp-create-token"
          disabled={creating}
          onclick={() => void createToken()}
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
      {#each tokens as tokenInfo (tokenInfo.id)}
        <div
          class="tok-row"
          data-testid="mcp-token-row"
          data-token-id={tokenInfo.id}
        >
          <div class="tok-main">
            <span class="tok-label">{tokenInfo.label || '(no label)'}</span>
            <span class="tok-prefix mono">{tokenInfo.token_prefix}…</span>
          </div>
          <span class="tok-user">{tokenInfo.username}</span>
          <span class="tok-scope muted">{scopeSummary(tokenInfo.scope)}</span>
          <span class="tok-seen muted">{rel(tokenInfo.last_seen_at)}</span>
          <button
            class="btn xs"
            data-testid="mcp-token-rotate"
            onclick={() => void rotateToken(tokenInfo)}
          >Rotate</button>
          <button
            class="btn xs danger"
            data-testid="mcp-token-revoke"
            onclick={() => void revokeToken(tokenInfo)}
          >Revoke</button>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .tokens {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .tools-head {
    display: flex;
    align-items: center;
    gap: 12px;
    justify-content: space-between;
  }
  .sec {
    margin: 4px 0 0;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
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
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    padding: 10px;
  }
  .pick-grp {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .grp-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }
  .ptool {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
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
  .cactions {
    display: flex;
    justify-content: flex-end;
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
    flex-wrap: wrap;
    font-size: 12.5px;
    color: var(--text);
  }
  .grow {
    flex: 1 1 auto;
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
  .mono {
    font-family: var(--font-mono);
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
  .btn.xs {
    font-size: 11px;
    padding: 3px 8px;
  }
  .btn.danger {
    color: var(--danger, #c0392b);
  }

  @media (max-width: 640px) {
    .tok-row {
      flex-wrap: wrap;
    }
    .tok-scope,
    .tok-seen {
      display: none;
    }
  }
</style>
