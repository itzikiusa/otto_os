<script lang="ts">
  // Site credentials for the in-app browser: list/add/edit/delete + a
  // confirm-gated reveal. The password is NEVER in the list/edit payload —
  // `BrowserCredential` has no password field at all (mirrors
  // `otto_state::browser_credentials::BrowserCredential`); the only place a
  // plaintext password ever appears client-side is the one-shot reveal
  // banner below, and it is never logged or persisted (no localStorage, no
  // console.log — just the in-memory `revealed` state, cleared on dismiss).
  //
  // Standalone panel (not wired into BrowserView by this task) — a caller
  // renders `<CredentialsPanel workspaceId={ws.currentId} />` wherever the
  // browser module wants a credentials tab/section.

  import * as browserApi from '../../lib/api/browser';
  import type { BrowserCredential } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  let { workspaceId }: { workspaceId: string } = $props();

  let creds: BrowserCredential[] = $state([]);
  let loading = $state(true);

  // ── Add/Edit form (shared modal) ────────────────────────────────────────
  let formOpen = $state(false);
  let editing: BrowserCredential | null = $state(null);
  let fDomain = $state('');
  let fUsername = $state('');
  let fPassword = $state('');
  let fAllowAgentUse = $state(false);
  let fNotes = $state('');
  let saving = $state(false);

  // ── Reveal ───────────────────────────────────────────────────────────────
  /** The one credential whose password is currently shown, plus the value
   *  itself — cleared on dismiss so it doesn't linger in memory/DOM. */
  let revealed: { id: string; password: string } | null = $state(null);
  let revealingId: string | null = $state(null);

  $effect(() => {
    if (workspaceId) void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      creds = await browserApi.listCredentials(workspaceId);
    } catch (e) {
      toasts.error('Could not load credentials', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  function openAdd(): void {
    editing = null;
    fDomain = '';
    fUsername = '';
    fPassword = '';
    fAllowAgentUse = false;
    fNotes = '';
    formOpen = true;
  }

  function openEdit(c: BrowserCredential): void {
    editing = c;
    fDomain = c.domain;
    fUsername = c.username;
    fPassword = '';
    fAllowAgentUse = c.allow_agent_use;
    fNotes = c.notes;
    formOpen = true;
  }

  function closeForm(): void {
    formOpen = false;
    fPassword = '';
  }

  async function save(): Promise<void> {
    if (!editing && (!fDomain.trim() || !fUsername.trim() || !fPassword)) {
      toasts.error('Missing fields', 'Domain, username, and password are required.');
      return;
    }
    saving = true;
    try {
      if (editing) {
        const updated = await browserApi.updateCredential(editing.id, {
          username: fUsername.trim(),
          allow_agent_use: fAllowAgentUse,
          notes: fNotes,
          ...(fPassword ? { password: fPassword } : {}),
        });
        creds = creds.map((c) => (c.id === updated.id ? updated : c));
        toasts.success('Credential updated', `${updated.domain} · ${updated.username}`);
      } else {
        const created = await browserApi.createCredential(workspaceId, {
          domain: fDomain.trim(),
          username: fUsername.trim(),
          password: fPassword,
          allow_agent_use: fAllowAgentUse,
          notes: fNotes,
        });
        creds = [...creds, created].sort(
          (a, b) => a.domain.localeCompare(b.domain) || a.username.localeCompare(b.username),
        );
        toasts.success('Credential saved', `${created.domain} · ${created.username}`);
      }
      closeForm();
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function remove(c: BrowserCredential): Promise<void> {
    const ok = await confirmer.ask(
      `Delete the credential for "${c.username}" on ${c.domain}? This also removes it from the Keychain.`,
      { title: 'Delete Credential', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    try {
      await browserApi.deleteCredential(c.id);
      creds = creds.filter((x) => x.id !== c.id);
      if (revealed?.id === c.id) revealed = null;
      toasts.success('Credential deleted', `${c.domain} · ${c.username}`);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function reveal(c: BrowserCredential): Promise<void> {
    const ok = await confirmer.ask(
      `Show the password for "${c.username}" on ${c.domain}? Make sure no one else can see your screen.`,
      { title: 'Reveal Password', confirmLabel: 'Reveal', danger: true },
    );
    if (!ok) return;
    revealingId = c.id;
    try {
      const resp = await browserApi.revealCredential(c.id);
      revealed = { id: c.id, password: resp.password };
    } catch (e) {
      toasts.error('Reveal failed', e instanceof Error ? e.message : String(e));
    } finally {
      revealingId = null;
    }
  }

  function dismissReveal(): void {
    revealed = null;
  }

  async function copyRevealed(): Promise<void> {
    if (!revealed) return;
    try {
      await copyTextOrThrow(revealed.password);
      toasts.success('Copied', 'Password copied to clipboard.');
    } catch {
      toasts.error('Copy failed', 'Could not write to clipboard.');
    }
  }
</script>

<div class="panel">
  <div class="panel-head">
    <div>
      <h2>Site credentials</h2>
      <div class="sub">
        Stored in the macOS Keychain — Otto never keeps the password itself in
        its database.
      </div>
    </div>
    <button class="btn primary" onclick={openAdd}>+ Add credential</button>
  </div>

  {#if loading}
    <p class="empty">Loading…</p>
  {:else if creds.length === 0}
    <p class="empty">No saved credentials yet.</p>
  {:else}
    <ul class="list">
      {#each creds as c (c.id)}
        <li class="row">
          <div class="body">
            <div class="domain-line">
              <span class="domain">{c.domain}</span>
              {#if c.allow_agent_use}
                <span class="badge" title="Agent sessions may autofill this credential">
                  agent-use
                </span>
              {/if}
            </div>
            <div class="username">{c.username}</div>
            {#if c.notes}<div class="notes">{c.notes}</div>{/if}
            {#if revealed?.id === c.id}
              <div class="reveal-banner">
                <code class="reveal-value">{revealed.password}</code>
                <button class="btn small" onclick={copyRevealed}>Copy</button>
                <button class="btn small" onclick={dismissReveal}>Hide</button>
              </div>
            {/if}
          </div>
          <div class="actions">
            <button
              class="icon-btn"
              title="Reveal password"
              disabled={revealingId === c.id}
              onclick={() => reveal(c)}
            >
              <Icon name="eye" size={13} />
            </button>
            <button class="icon-btn" title="Edit" onclick={() => openEdit(c)}>
              <Icon name="edit" size={13} />
            </button>
            <button class="icon-btn" title="Delete" onclick={() => remove(c)}>
              <Icon name="trash" size={13} />
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

{#if formOpen}
  <Modal title={editing ? 'Edit credential' : 'Add credential'} onclose={closeForm}>
    {#snippet children()}
      <div class="form">
        {#if !editing}
          <label>
            <span>Domain</span>
            <input type="text" bind:value={fDomain} placeholder="example.com" spellcheck="false" />
          </label>
        {:else}
          <label>
            <span>Domain</span>
            <input type="text" value={editing.domain} disabled />
          </label>
        {/if}
        <label>
          <span>Username</span>
          <input type="text" bind:value={fUsername} spellcheck="false" />
        </label>
        <label>
          <span>{editing ? 'New password (leave blank to keep current)' : 'Password'}</span>
          <input type="password" bind:value={fPassword} autocomplete="new-password" />
        </label>
        <label>
          <span>Notes</span>
          <textarea bind:value={fNotes} rows="2"></textarea>
        </label>
        <label class="checkbox">
          <input type="checkbox" bind:checked={fAllowAgentUse} />
          <span>Allow agent sessions to autofill this credential</span>
        </label>
        {#if fAllowAgentUse}
          <p class="warning">
            An unattended agent session running in this workspace will be able to use this
            credential to sign in on your behalf. Only enable this for accounts you're
            comfortable an agent acting autonomously could access.
          </p>
        {/if}
      </div>
    {/snippet}
    {#snippet footer()}
      <button class="btn" onclick={closeForm} disabled={saving}>Cancel</button>
      <button class="btn primary" onclick={save} disabled={saving}>
        {saving ? 'Saving…' : 'Save'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
    padding: 0.8rem;
  }
  .panel-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.6rem;
  }
  .panel-head h2 {
    margin: 0;
    font-size: 0.95rem;
  }
  .sub {
    color: var(--text-dim);
    font-size: 0.78rem;
    margin-top: 0.2rem;
  }
  .empty {
    color: var(--text-dim);
    font-size: 0.82rem;
    padding: 0.5rem 0.1rem;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .row {
    display: flex;
    gap: 0.5rem;
    padding: 0.6rem 0.7rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    align-items: flex-start;
  }
  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  .domain-line {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .domain {
    font-weight: 600;
    font-size: 0.86rem;
    color: var(--text);
  }
  .badge {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 0.1rem 0.35rem;
    border-radius: var(--radius-s);
    background: var(--accent);
    color: var(--accent-contrast, #fff);
  }
  .username {
    font-size: 0.8rem;
    color: var(--text-dim);
  }
  .notes {
    font-size: 0.76rem;
    color: var(--text-dim);
    white-space: pre-wrap;
  }
  .reveal-banner {
    margin-top: 0.35rem;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.35rem 0.5rem;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .reveal-value {
    font-family: var(--font-mono, monospace);
    font-size: 0.8rem;
    user-select: all;
    word-break: break-all;
  }
  .actions {
    display: flex;
    gap: 0.2rem;
    flex-shrink: 0;
  }
  .icon-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text);
  }
  .icon-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 0.78rem;
    padding: 0.3rem 0.65rem;
    cursor: pointer;
  }
  .btn.small {
    padding: 0.2rem 0.5rem;
    font-size: 0.72rem;
  }
  .btn.primary {
    background: var(--accent);
    color: var(--accent-contrast, #fff);
    border-color: var(--accent);
  }
  .btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .form label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }
  .form label.checkbox {
    flex-direction: row;
    align-items: center;
    gap: 0.4rem;
    color: var(--text);
  }
  .form input[type='text'],
  .form input[type='password'],
  .form textarea {
    width: 100%;
    box-sizing: border-box;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 0.4rem 0.55rem;
    font: inherit;
    font-size: 0.82rem;
  }
  .form input:disabled {
    opacity: 0.65;
  }
  .warning {
    margin: 0;
    font-size: 0.76rem;
    color: var(--warning, #b45309);
    background: var(--warning-bg, rgba(180, 83, 9, 0.1));
    border-radius: var(--radius-s);
    padding: 0.4rem 0.55rem;
  }
</style>
