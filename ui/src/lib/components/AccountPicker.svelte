<script lang="ts">
  import { api } from '../api/client';
  import type { Session, ProviderAccount } from '../api/types';
  import { ws } from '../stores/workspace.svelte';
  import Terminal from './Terminal.svelte';
  import { loadErrorText } from '../loadError';
  let { provider, value = '', workspaceId, onchange }: {
    provider: string; value?: string; workspaceId: string; onchange: (id: string) => void;
  } = $props();
  let accounts = $state<ProviderAccount[]>([]);
  let adding = $state(false);
  let label = $state('');
  let busy = $state(false);
  let error = $state('');
  let status = $state('');
  let loginSession = $state<Session | null>(null);
  $effect(() => {
    const currentProvider = provider;
    let alive = true;
    void api.get<ProviderAccount[]>('/auth/provider-accounts').then((all) => {
      if (alive) accounts = all.filter((a) => a.provider === currentProvider);
    }).catch(() => { if (alive) error = 'Could not load accounts. Reopen this form to retry.'; });
    return () => { alive = false; };
  });
  async function add() {
    busy = true; error = '';
    try {
      const account = await api.post<ProviderAccount>('/auth/provider-accounts', { provider, label });
      accounts = [...accounts, account]; onchange(account.id); label = ''; adding = false;
      status = 'Profile created. Sign in to connect its subscription.';
    } catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
  async function login() {
    busy = true; error = ''; status = '';
    try {
      loginSession = await api.post<Session>(`/auth/provider-accounts/${value}/login`, { workspace_id: workspaceId });
      ws.addSession(loginSession);
    } catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
  async function check() {
    busy = true; error = '';
    try {
      const result = await api.get<{signed_in: boolean}>(`/auth/provider-accounts/${value}/status`);
      status = result.signed_in ? 'Signed in' : 'Sign-in required';
    } catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
</script>

<div class="account-picker">
  <!-- One row: the picker and its actions side by side (they used to stack,
       leaving "Add account" stranded under the select). -->
  <div class="actions">
    <label>
      <span>{provider} account</span>
      <select class="input" value={value} disabled={busy} onchange={(e) => { onchange(e.currentTarget.value); status = ''; loginSession = null; }}>
        <option value="">Default CLI account</option>
        {#each accounts as account (account.id)}<option value={account.id}>{account.label}</option>{/each}
      </select>
    </label>
    <button class="btn small" type="button" disabled={busy} aria-expanded={adding} onclick={() => (adding = !adding)}>Add account…</button>
    {#if value}
      <button class="btn small" type="button" disabled={busy} onclick={login}>Sign in</button>
      <button class="btn small" type="button" disabled={busy} onclick={check}>Check sign-in</button>
    {/if}
  </div>
  {#if adding}
    <div class="actions">
      <input class="input" aria-label="Account label" placeholder="Personal, Work…" maxlength="80" bind:value={label} />
      <button class="btn small" type="button" disabled={busy || !label.trim()} onclick={add}>Create profile</button>
    </div>
    <p>Each profile has its own subscription login. Your default CLI account stays available.</p>
  {/if}
  {#if status}<p role="status">{status}</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if loginSession}
    <p>Complete the provider’s sign-in below, then check sign-in.</p>
    <div class="login-terminal"><Terminal sessionId={loginSession.id} /></div>
  {/if}
</div>

<style>
  .account-picker { display: flex; flex-direction: column; gap: 6px; margin-block: 10px; min-width: 0; }
  label, .actions { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  label span { font-size: var(--fs-s); }
  select, input { width: auto; min-width: 0; max-width: 100%; }
  p { margin: 0; font-size: var(--fs-s); color: var(--text-dim); }
  .error { color: var(--danger); }
  .login-terminal { height: 260px; max-height: 45vh; min-width: 0; overflow: hidden; }
</style>
