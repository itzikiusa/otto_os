<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Personal Access Tokens (PAT) management — mint long-lived API tokens,
  // view existing ones (prefix + last-seen), and revoke them individually.
  // Routes: POST/GET/DELETE /api/v1/auth/tokens  (api.md #87-89).
  import { api, baseUrl } from '../../lib/api/client';
  import type { ApiTokenInfo, CreateApiTokenResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { copyTextOrThrow } from '../../lib/clipboard';

  let tokens: ApiTokenInfo[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
  let revoking: Set<string> = $state(new Set());
  let filter = $state<'all' | 'personal' | 'session' | 'orphaned'>('all');
  const sessionToken = (t: ApiTokenInfo) => !!(t.session_id || t.legacy_session_id);
  const sessionCount = $derived(tokens.filter(sessionToken).length);
  const orphaned = $derived(tokens.filter((t) => sessionToken(t) && t.session_exists === false));
  const visible = $derived(tokens.filter((t) => filter === 'all'
    || (filter === 'personal' && !sessionToken(t))
    || (filter === 'session' && sessionToken(t))
    || (filter === 'orphaned' && sessionToken(t) && t.session_exists === false)));

  async function revokeOrphaned(): Promise<void> {
    const candidates = [...orphaned];
    if (!candidates.length) return;
    if (!await confirmer.ask(
      `Revoke ${candidates.length} tokens referring to sessions that no longer exist? Legacy tokens are identified by their labels; review the Deleted session filter first if you named a personal token this way.`,
      { title: 'Revoke deleted-session tokens', confirmLabel: 'Revoke tokens', danger: true },
    )) return;
    let failed = 0;
    revoking = new Set([...revoking, ...candidates.map((t) => t.id)]);
    // Bounded requests; do not flood the local API for historical token lists.
    for (const t of candidates) {
      try { await api.del(`/auth/tokens/${t.id}`); tokens = tokens.filter((x) => x.id !== t.id); }
      catch { failed += 1; }
    }
    revoking = new Set([...revoking].filter((id) => !candidates.some((t) => t.id === id)));
    if (failed) toasts.error('Some tokens could not be revoked', `${failed} failed; they remain listed.`);
    else toasts.success('Tokens revoked', `${candidates.length} deleted-session tokens removed.`);
  }

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
    loadError = '';
    try {
      tokens = await api.get<ApiTokenInfo[]>('/auth/tokens');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  async function mint(): Promise<void> {
    if (minting) return;
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
      toasts.error("Couldn't create the token", loadErrorText(e));
    } finally {
      minting = false;
    }
  }

  async function revoke(t: ApiTokenInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Revoke token "${t.label ?? t.token_prefix}…"? Any script or client using it will stop working immediately.`,
      { title: 'Revoke token', confirmLabel: 'Revoke', danger: true },
    );
    if (!ok) return;
    revoking = new Set([...revoking, t.id]);
    try {
      await api.del(`/auth/tokens/${t.id}`);
      tokens = tokens.filter((x) => x.id !== t.id);
      toasts.success('Token revoked', t.label ?? t.token_prefix);
    } catch (e) {
      toasts.error("Couldn't revoke the token", loadErrorText(e));
    } finally {
      revoking = new Set([...revoking].filter((x) => x !== t.id));
    }
  }

  async function copySecret(): Promise<void> {
    if (!freshSecret) return;
    try {
      await copyTextOrThrow(freshSecret);
      toasts.success('Token copied', 'Paste it into your script or CI secret now.');
    } catch {
      toasts.error("Couldn't copy the token", 'Select it and copy it manually.');
    }
  }

  function dismissSecret(): void {
    freshSecret = null;
    freshInfo = null;
  }

  function isExpired(t: ApiTokenInfo): boolean {
    return new Date(t.expires_at).getTime() < Date.now();
  }

  /** What a row is called: its label, else its prefix. Managed session tokens
   *  carry a machine label (`otto-mcp:<session ULID>`) — shown shortened, the
   *  full value stays in the tooltip. */
  function rowName(t: ApiTokenInfo): string {
    const l = t.label ?? '';
    const m = /^(.*:)([0-9A-HJKMNP-TV-Z]{26})$/.exec(l);
    if (m) return `${m[1]}${m[2].slice(0, 6)}…`;
    return l || `${t.token_prefix}…`;
  }
  function absDate(ts: string | null | undefined): string | undefined {
    return ts ? new Date(ts).toLocaleString() : undefined;
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('tokens')} subtitle="For scripts, CI and the Otto CLI" />
  <PageBody width="readable">
  <SectionIntro>Tokens are scoped to your account and <strong>inherit your permissions</strong> — anyone holding one can do what you can. Impersonation sessions can't create tokens.</SectionIntro>

  <!-- ── One-time secret reveal ── -->
  {#if freshSecret && freshInfo}
    <div class="secret-banner" role="status">
      <div class="secret-header">
        <Icon name="key" size={14} />
        <span class="secret-title">Copy your new token now — it won't be shown again.</span>
        <button class="btn small" onclick={copySecret}><Icon name="copy" size={12} /> Copy</button>
        <button class="btn small ghost" onclick={dismissSecret}>Done</button>
      </div>
      <code class="secret-value">{freshSecret}</code>
      <div class="secret-meta">
        {freshInfo.label ?? 'No label'} · {freshInfo.token_prefix}… · expires {rel(freshInfo.expires_at)}
      </div>
    </div>
  {/if}

  <!-- ── Mint new token ── -->
  <div class="section-title">New token</div>
  <div class="card s-card">
    <label class="mint-label" for="pat-label">Label <span class="dim">(optional)</span></label>
    <div class="mint-row">
      <input
        id="pat-label"
        class="input mint-input"
        type="text"
        placeholder="CI pipeline"
        bind:value={newLabel}
        onkeydown={(e) => { if (e.key === 'Enter') void mint(); }}
        maxlength={80}
      />
      <button class="btn primary" disabled={minting} onclick={mint}>
        <Icon name="plus" size={13} /> {minting ? 'Creating…' : 'Create token'}
      </button>
    </div>
    <span class="hint">Tokens expire after 10 years unless revoked. Pass one as a <code>Bearer</code> header or in <code>OTTO_API_TOKEN</code>.</span>
  </div>

  <!-- ── Existing tokens ── -->
  <div class="tokens-head">
    <div class="section-title">Your tokens</div>
    {#if tokens.length}
      <select class="input filter" aria-label="Filter tokens" bind:value={filter}>
        <option value="all">All ({tokens.length})</option>
        <option value="personal">Personal ({tokens.length - sessionCount})</option>
        <option value="session">Session ({sessionCount})</option>
        <option value="orphaned">Deleted session ({orphaned.length})</option>
      </select>
    {/if}
  </div>

  <LoadState what="tokens" {loading} error={loadError} empty={tokens.length === 0} onretry={() => void load()} rows={3}>
    {#snippet emptyView()}
      <div class="card tok-card">
        <EmptyState icon="key" title="No tokens yet" body="Create one above for a script, a CI job or the Otto CLI." />
      </div>
    {/snippet}
    {#if orphaned.length}
      <div class="orphan-note">
        <span>{orphaned.length} token{orphaned.length === 1 ? '' : 's'} belong to sessions that no longer exist.</span>
        <button class="btn small danger" disabled={revoking.size > 0} onclick={revokeOrphaned}>Revoke deleted-session tokens…</button>
      </div>
    {/if}
    {#if visible.length === 0}
      <div class="card tok-card">
        <EmptyState icon="filter" title="No tokens in this view">
          <button class="btn ghost" onclick={() => (filter = 'all')}>Show all tokens</button>
        </EmptyState>
      </div>
    {:else}
      <div class="card tok-card token-table" role="table" aria-label="Your tokens">
        <div class="token-head" role="row">
          <span class="col-label" role="columnheader">Token</span>
          <span class="col-seen" role="columnheader">Last used</span>
          <span class="col-exp" role="columnheader">Expires</span>
          <span class="col-action" role="columnheader"><span class="sr-only">Actions</span></span>
        </div>
        {#each visible as t (t.id)}
          <div class="token-row" class:expired={isExpired(t)} role="row">
            <span class="col-label" role="cell">
              <span class="tok-label" title={t.label ?? undefined}>{rowName(t)}</span>
              <span class="tok-meta">
                {#if sessionToken(t)}{t.session_id ? 'Managed session' : 'Legacy session label'}{t.session_exists === false ? ' · session deleted' : ''} · {/if}<span class="mono">{t.token_prefix}…</span>
              </span>
            </span>
            <span class="col-seen" role="cell" title={absDate(t.last_seen_at)}>{t.last_seen_at ? rel(t.last_seen_at) : 'Never'}</span>
            <span class="col-exp" class:warn={isExpired(t)} role="cell" title={absDate(t.expires_at)}>
              {isExpired(t) ? 'Expired' : rel(t.expires_at)}
            </span>
            <span class="col-action" role="cell">
              <button
                class="btn small danger"
                disabled={revoking.has(t.id)}
                onclick={() => revoke(t)}
              >
                {revoking.has(t.id) ? 'Revoking…' : 'Revoke…'}
              </button>
            </span>
          </div>
        {/each}
      </div>
      <p class="usage-note">Otto revokes a managed session's token when the session is deleted or replaced on restart. Older label-only tokens can be reviewed and revoked here.</p>
    {/if}
  </LoadState>

  <!-- ── Usage note ── -->
  <div class="section-title">Using a token</div>
  <pre class="code-block">curl -H "Authorization: Bearer &lt;token&gt;" {baseUrl()}/api/v1/auth/me</pre>
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .s-card {
    padding: 12px 16px;
    max-width: var(--settings-col);
  }
  .tok-card {
    max-width: var(--settings-col);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  /* ── One-time secret banner ── */
  .secret-banner {
    max-width: var(--settings-col);
    margin-bottom: 16px;
    padding: 12px 14px;
    border-radius: var(--radius-m);
    background: var(--success-soft);
    border: 1px solid color-mix(in srgb, var(--success) 30%, transparent);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .secret-header {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--success);
  }
  .secret-title {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .secret-value {
    display: block;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    color: var(--text);
    word-break: break-all;
    user-select: all;
  }
  .secret-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }

  /* ── Mint row ── */
  .mint-label {
    display: block;
    margin-bottom: 4px;
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .mint-row {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-bottom: 6px;
  }
  .mint-input {
    flex: 1;
    min-width: 0;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .hint code {
    font-family: var(--font-mono);
  }

  /* ── Token table ── */
  .tokens-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    max-width: var(--settings-col);
  }
  .tokens-head .section-title {
    margin-block: 18px 8px;
  }
  .filter {
    width: auto;
    margin-top: 10px;
  }
  .orphan-note {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    max-width: var(--settings-col);
    margin-bottom: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .token-table {
    overflow: hidden;
  }
  .token-head,
  .token-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 96px 96px 84px;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    font-size: var(--fs-m);
  }
  .token-head {
    font-size: var(--fs-xs);
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
  .col-label {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .tok-label,
  .tok-meta {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tok-label {
    font-weight: 500;
  }
  .tok-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .col-seen,
  .col-exp {
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .col-action {
    display: flex;
    justify-content: flex-end;
  }
  .warn {
    color: var(--danger);
    font-weight: 500;
  }
  .usage-note {
    max-width: var(--settings-col);
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--text-dim);
    margin: 8px 0 0;
  }
  .code-block {
    max-width: var(--settings-col);
    box-sizing: border-box;
    font-size: var(--fs-s);
    font-family: var(--font-mono);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    overflow-x: auto;
    margin: 0;
    white-space: pre-wrap;
    word-break: break-all;
  }

  @media (max-width: 640px) {
    .token-head,
    .token-row {
      grid-template-columns: minmax(0, 1fr) 84px;
    }
    .col-seen,
    .col-exp {
      display: none;
    }
  }
</style>
