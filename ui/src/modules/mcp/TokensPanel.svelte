<script lang="ts">
  import { plural } from '../../lib/plural';
  import Icon from '../../lib/components/Icon.svelte';
  import { toastError } from '../../lib/toastError';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { mcpTokensApi } from '../../lib/api/mcp';
  import { api, ApiError, baseUrl } from '../../lib/api/client';
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

  /** Failed token-list load — inline with Retry, never "No MCP tokens yet.". */
  let tokensError = $state<string | null>(null);

  async function loadTokens(): Promise<void> {
    try {
      const result = await mcpTokensApi.list();
      tokens = result.tokens;
      tokensError = null;
    } catch (e) {
      tokensError = loadErrorText(e);
    } finally {
      tokensLoaded = true;
    }
  }

  /** The owner list failed to load: the picker falls back to "Me" and says why. */
  let usersError = $state<string | null>(null);
  async function loadUsers(): Promise<void> {
    try {
      users = await api.get<User[]>('/users');
      usersError = null;
    } catch (e) {
      users = [];
      // Only the owner may list users (403 otherwise): then "Me" is the whole
      // list by design, not a failure.
      usersError = e instanceof ApiError && e.status === 403 ? null : loadErrorText(e);
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
    if (fRestrictTools && fTools.size === 0) return;
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
      toastError('Couldn’t create the token', e);
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
      toastError('Couldn’t rotate the token', e);
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
      toastError('Couldn’t revoke the token', e);
    }
  }

  function scopeSummary(scope: McpScope): string {
    const toolPart =
      scope.tools == null
        ? 'all tools'
        : `${plural(scope.tools.length, 'tool')}`;
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
      toasts.error('Couldn’t copy', 'Select and copy manually.');
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
      class="btn small"
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
        <button class="btn small" onclick={() => void copy(createdToken!.secret, 'Token')}>Copy token</button>
        <button class="btn small" onclick={() => void copy(clientCommand(createdToken!.secret), 'Command')}>Copy command</button>
        <button class="btn small" onclick={() => (createdToken = null)}>Dismiss</button>
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
        <button class="btn small" onclick={() => void copy(rotatedToken!.secret, 'Token')}>Copy token</button>
        <button class="btn small" onclick={() => void copy(clientCommand(rotatedToken!.secret), 'Command')}>Copy command</button>
        <button class="btn small" onclick={() => (rotatedToken = null)}>Dismiss</button>
      </div>
      <code class="token">{rotatedToken.secret}</code>
      <code class="token cmd">{clientCommand(rotatedToken.secret)}</code>
    </div>
  {/if}

  {#if showCreate}
    <div class="create">
      <div class="frow">
        <!-- The Retry sits beside the select, not inside its label. -->
        <div class="field tok-field">
          <label for="mcp-token-owner">Owner</label>
          <select id="mcp-token-owner" class="input" bind:value={fOwner} aria-describedby={usersError ? 'mcp-token-owner-err' : undefined}>
            <option value="">Me ({auth.me?.username ?? 'self'})</option>
            {#each users as u (u.id)}
              <option value={u.id}>{u.username}</option>
            {/each}
          </select>
          {#if usersError}
            <span class="hint tok-note" id="mcp-token-owner-err">
              Couldn’t load other users — the token can only be yours.
              <button type="button" class="btn small ghost" onclick={() => void loadUsers()}>Retry</button>
            </span>
          {/if}
        </div>
        <div class="field tok-field">
          <label for="mcp-token-label">Label</label>
          <input dir="ltr" id="mcp-token-label" class="input" placeholder="ci-readonly" bind:value={fLabel} />
        </div>
        <div class="field tok-field">
          <label for="mcp-token-workspace">Workspace pin (optional)</label>
          <select id="mcp-token-workspace" class="input" bind:value={fWorkspace}>
            <option value="">Any workspace</option>
            {#each ws.workspaces as workspace (workspace.id)}
              <option value={workspace.id}>{workspace.name}</option>
            {/each}
          </select>
        </div>
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
          disabled={creating || (fRestrictTools && fTools.size === 0)}
          title={fRestrictTools && fTools.size === 0 ? 'Pick at least one tool — a token restricted to none can call nothing' : undefined}
          onclick={() => void createToken()}
        >
          {creating ? 'Creating…' : 'Create token'}
        </button>
      </div>
    </div>
  {/if}

  <div class="tok-list" data-testid="mcp-tokens">
    {#if tokensError && !tokens.length}
      <LoadState what="MCP tokens" variant="compact" error={tokensError} empty onretry={() => void loadTokens()} />
    {:else if !tokens.length}
      <p class="muted small pad">{tokensLoaded ? 'No MCP tokens yet.' : 'Loading MCP tokens…'}</p>
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
            class="btn small"
            data-testid="mcp-token-rotate"
            onclick={() => void rotateToken(tokenInfo)}
          >Rotate</button>
          <button
            class="btn small danger"
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
    font-size: var(--fs-s);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .create {
    display: flex;
    flex-direction: column;
    gap: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 12px;
  }
  .frow {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }
  .tok-field {
    flex: 1 1 160px;
    margin-block-end: 0;
  }
  .hint.tok-note {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
    color: var(--warning);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
    color: var(--text);
  }
  .tool-pick {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 220px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px;
  }
  .pick-grp {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .grp-name {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .ptool {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .mut {
    margin-inline-start: 8px;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    color: var(--warning);
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    border-radius: var(--radius-s);
    padding: 0 4px;
  }
  .cactions {
    display: flex;
    justify-content: flex-end;
  }
  .token-once {
    border: 1px solid color-mix(in srgb, var(--warning) 45%, transparent);
    background: color-mix(in srgb, var(--warning) 10%, transparent);
    border-radius: var(--radius-m);
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
    font-size: var(--fs-m);
    color: var(--text);
  }
  .grow {
    flex: 1 1 auto;
  }
  .token {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    word-break: break-all;
    color: var(--text);
  }
  .token.cmd {
    font-size: var(--fs-xs);
  }
  .tok-list {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .tok-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
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
    font-size: var(--fs-m);
    color: var(--text);
  }
  .tok-prefix {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tok-user,
  .tok-scope,
  .tok-seen {
    font-size: var(--fs-s);
    flex: none;
  }
  .mono {
    font-family: var(--font-mono);
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
    .tok-row {
      flex-wrap: wrap;
    }
    .tok-scope,
    .tok-seen {
      display: none;
    }
  }
</style>
