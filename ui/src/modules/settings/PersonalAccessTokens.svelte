<script lang="ts">
  // Personal Access Tokens (PAT) management — mint long-lived API tokens,
  // view existing ones (prefix + last-seen), and revoke them individually.
  // Routes: POST/GET/DELETE /api/v1/auth/tokens  (api.md #87-89).
  import { api } from '../../lib/api/client';
  import type { ApiTokenInfo, CreateApiTokenResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  let tokens: ApiTokenInfo[] = $state([]);
  let loading = $state(true);
  let revoking: Set<string> = $state(new Set());

  // ---- create form ----
  let newLabel = $state('');
  let minting = $state(false);
  /** Raw secret returned once on creation — cleared when the user dismisses. */
  let freshSecret: string | null = $state(null);
  let freshInfo: ApiTokenInfo | null = $state(null);

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      tokens = await api.get<ApiTokenInfo[]>('/auth/tokens');
    } catch (e) {
      toasts.error('Could not load tokens', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  async function mint(): Promise<void> {
    minting = true;
    try {
      const resp = await api.post<CreateApiTokenResp>('/auth/tokens', {
        label: newLabel.trim() || null,
      });
      freshSecret = resp.token;
      freshInfo = resp.info;
      tokens = [resp.info, ...tokens];
      newLabel = '';
    } catch (e) {
      toasts.error('Could not create token', e instanceof Error ? e.message : String(e));
    } finally {
      minting = false;
    }
  }

  async function revoke(t: ApiTokenInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Revoke token "${t.label ?? t.token_prefix}…"? Any script or client using it will stop working immediately.`,
      { title: 'Revoke Token', confirmLabel: 'Revoke', danger: true },
    );
    if (!ok) return;
    revoking = new Set([...revoking, t.id]);
    try {
      await api.del(`/auth/tokens/${t.id}`);
      tokens = tokens.filter((x) => x.id !== t.id);
      toasts.success('Token revoked', t.label ?? t.token_prefix);
    } catch (e) {
      toasts.error('Revoke failed', e instanceof Error ? e.message : String(e));
    } finally {
      revoking = new Set([...revoking].filter((x) => x !== t.id));
    }
  }

  async function copySecret(): Promise<void> {
    if (!freshSecret) return;
    try {
      await navigator.clipboard.writeText(freshSecret);
      toasts.success('Copied', 'Token secret copied to clipboard.');
    } catch {
      toasts.error('Copy failed', 'Could not write to clipboard.');
    }
  }

  function dismissSecret(): void {
    freshSecret = null;
    freshInfo = null;
  }

  function isExpired(t: ApiTokenInfo): boolean {
    return new Date(t.expires_at).getTime() < Date.now();
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Personal Access Tokens</h1>
      <div class="sub">
        Long-lived tokens for scripts, CI, and the Otto CLI API.
        Tokens are scoped to your account and inherit your permissions.
        Impersonation sessions cannot mint PATs.
      </div>
    </div>
  </div>

  <!-- ── One-time secret reveal ── -->
  {#if freshSecret && freshInfo}
    <div class="secret-banner">
      <div class="secret-header">
        <span class="secret-title">Token created — copy it now. It will not be shown again.</span>
        <button class="btn small" onclick={copySecret}>Copy</button>
        <button class="btn small" onclick={dismissSecret}>Dismiss</button>
      </div>
      <div class="secret-body">
        <code class="secret-value">{freshSecret}</code>
      </div>
      <div class="secret-meta dim">
        Label: {freshInfo.label ?? '(none)'}  ·
        Prefix: {freshInfo.token_prefix}…  ·
        Expires: {rel(freshInfo.expires_at)}
      </div>
    </div>
  {/if}

  <!-- ── Mint new token ── -->
  <div class="section-title">New token</div>
  <div class="card pad">
    <div class="mint-row">
      <input
        class="input"
        type="text"
        placeholder="Label (optional — e.g. 'CI pipeline')"
        bind:value={newLabel}
        onkeydown={(e) => { if (e.key === 'Enter') void mint(); }}
        style="flex: 1; min-width: 0"
        maxlength={80}
      />
      <button class="btn primary" disabled={minting} onclick={mint}>
        {minting ? 'Creating…' : 'Create token'}
      </button>
    </div>
  </div>

  <!-- ── Existing tokens ── -->
  <div class="section-title">Your tokens</div>

  {#if loading}
    <Skeleton rows={3} height={44} />
  {:else if tokens.length === 0}
    <div class="empty dim">No personal access tokens yet.</div>
  {:else}
    <div class="card token-table">
      <div class="token-head">
        <span class="col-label">Label / Prefix</span>
        <span class="col-seen">Last used</span>
        <span class="col-exp">Expires</span>
        <span class="col-action"></span>
      </div>
      {#each tokens as t (t.id)}
        <div class="token-row" class:expired={isExpired(t)}>
          <span class="col-label">
            {#if t.label}
              <span class="tok-label">{t.label}</span>
              <span class="tok-prefix dim">{t.token_prefix}…</span>
            {:else}
              <span class="tok-prefix">{t.token_prefix}…</span>
            {/if}
          </span>
          <span class="col-seen dim">{rel(t.last_seen_at)}</span>
          <span class="col-exp" class:dim={!isExpired(t)} class:warn={isExpired(t)}>
            {isExpired(t) ? 'Expired' : rel(t.expires_at)}
          </span>
          <span class="col-action">
            <button
              class="btn small danger"
              disabled={revoking.has(t.id)}
              onclick={() => revoke(t)}
            >
              {revoking.has(t.id) ? 'Revoking…' : 'Revoke'}
            </button>
          </span>
        </div>
      {/each}
    </div>
  {/if}

  <!-- ── Usage note ── -->
  <div class="section-title" style="margin-top: 24px">Using a token</div>
  <div class="card pad">
    <p class="usage-note dim">
      Pass the token as a <code>Bearer</code> header or via
      <code>OTTO_API_TOKEN</code> in the environment. Example:
    </p>
    <pre class="code-block">curl -H "Authorization: Bearer &lt;token&gt;" http://127.0.0.1:7700/api/v1/auth/me</pre>
  </div>
</div>

<style>
  .empty {
    padding: 24px 0;
    text-align: center;
    font-size: 13px;
  }

  /* ── One-time secret banner ── */
  .secret-banner {
    margin-bottom: 18px;
    padding: 12px 14px;
    border-radius: var(--radius-m);
    background: color-mix(in srgb, #22c55e 10%, var(--surface));
    border: 1px solid color-mix(in srgb, #22c55e 30%, transparent);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .secret-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .secret-title {
    flex: 1;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
  }

  .secret-body {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    overflow-x: auto;
  }

  .secret-value {
    font-size: 12.5px;
    font-family: monospace;
    color: var(--text);
    word-break: break-all;
    user-select: all;
  }

  .secret-meta {
    font-size: 11.5px;
  }

  /* ── Mint row ── */
  .mint-row {
    display: flex;
    gap: 10px;
    align-items: center;
  }

  /* ── Token table ── */
  .token-table {
    overflow: hidden;
    max-width: 640px;
  }

  .token-head,
  .token-row {
    display: grid;
    grid-template-columns: 1fr 110px 110px 90px;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    font-size: 12.5px;
  }

  .token-head {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }

  .token-row + .token-row {
    border-top: 1px solid var(--border);
  }

  .token-row.expired {
    opacity: 0.6;
  }

  .tok-label {
    font-weight: 500;
    margin-inline-end: 6px;
  }

  .tok-prefix {
    font-family: monospace;
    font-size: 11.5px;
  }

  .col-action {
    display: flex;
    justify-content: flex-end;
  }

  .warn {
    color: #ef4444;
    font-weight: 500;
  }

  /* ── Usage note ── */
  .usage-note {
    font-size: 12.5px;
    line-height: 1.55;
    margin: 0 0 10px;
  }

  .usage-note code {
    font-family: monospace;
    background: var(--surface-2);
    padding: 1px 4px;
    border-radius: 3px;
  }

  .code-block {
    font-size: 11.5px;
    font-family: monospace;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    overflow-x: auto;
    margin: 0;
    white-space: pre-wrap;
    word-break: break-all;
  }
</style>
