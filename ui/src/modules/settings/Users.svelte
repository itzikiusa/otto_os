<script lang="ts">
  // Users admin (root): create/disable users + per-workspace role matrix.
  import { api } from '../../lib/api/client';
  import type { MemberEntry, User, WorkspaceRole } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Modal from '../../lib/components/Modal.svelte';

  let users: User[] = $state([]);
  let loading = $state(true);
  let createOpen = $state(false);
  let newUsername = $state('');
  let newDisplay = $state('');
  let newPassword = $state('');
  let busy = $state(false);

  // role matrix
  const roleOptions: (WorkspaceRole | 'none')[] = ['none', 'viewer', 'editor', 'admin'];

  let matrixWs: string = $state('');
  let members: MemberEntry[] = $state([]);
  let matrixLoading = $state(false);

  $effect(() => {
    void loadUsers();
  });

  $effect(() => {
    if (matrixWs === '' && ws.workspaces.length > 0) matrixWs = ws.workspaces[0].id;
  });

  $effect(() => {
    const id = matrixWs;
    if (id === '') return;
    matrixLoading = true;
    void api
      .get<MemberEntry[]>(`/workspaces/${id}/members`)
      .then((m) => (members = m))
      .catch(() => (members = []))
      .finally(() => (matrixLoading = false));
  });

  async function loadUsers(): Promise<void> {
    loading = true;
    try {
      users = await api.get<User[]>('/users');
    } catch (e) {
      toasts.error('Could not load users', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  async function createUser(): Promise<void> {
    busy = true;
    try {
      const u = await api.post<User>('/users', {
        username: newUsername.trim(),
        password: newPassword,
        display_name: newDisplay.trim() === '' ? null : newDisplay.trim(),
      });
      users = [...users, u];
      createOpen = false;
      newUsername = newDisplay = newPassword = '';
      toasts.success('User created', u.username);
    } catch (e) {
      toasts.error('Create failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  async function toggleDisabled(u: User): Promise<void> {
    try {
      const updated = await api.patch<User>(`/users/${u.id}`, { disabled: !u.disabled });
      users = users.map((x) => (x.id === u.id ? updated : x));
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    }
  }

  function roleOf(userId: string): WorkspaceRole | 'none' {
    return members.find((m) => m.user_id === userId)?.role ?? 'none';
  }

  async function setRole(userId: string, role: WorkspaceRole | 'none'): Promise<void> {
    const next = members.filter((m) => m.user_id !== userId);
    if (role !== 'none') {
      const u = users.find((x) => x.id === userId);
      next.push({
        user_id: userId,
        username: u?.username ?? '?',
        display_name: u?.display_name ?? '?',
        role,
      });
    }
    try {
      members = await api.put<MemberEntry[]>(`/workspaces/${matrixWs}/members`, {
        members: next.map((m) => ({ user_id: m.user_id, role: m.role })),
      });
      toasts.success('Membership updated');
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Users</h1>
      <div class="sub">Root manages accounts and per-workspace roles.</div>
    </div>
    <button class="btn primary" onclick={() => (createOpen = true)}>New User</button>
  </div>

  {#if loading}
    <Skeleton rows={3} height={40} />
  {:else}
    <div class="card user-table">
      {#each users as u (u.id)}
        <div class="user-row" class:disabled={u.disabled}>
          <span class="avatar">{u.display_name.slice(0, 1).toUpperCase()}</span>
          <div class="grow">
            <div class="u-name">
              {u.display_name}
              {#if u.is_root}<span class="chip accent">root</span>{/if}
              {#if u.disabled}<span class="chip bad">disabled</span>{/if}
            </div>
            <div class="u-sub">@{u.username}</div>
          </div>
          {#if !u.is_root}
            <button class="btn small {u.disabled ? '' : 'danger'}" onclick={() => toggleDisabled(u)}>
              {u.disabled ? 'Enable' : 'Disable'}
            </button>
          {/if}
        </div>
      {/each}
    </div>

    <div class="section-title">Workspace roles</div>
    <div class="row" style="margin-bottom: 10px">
      <select class="input" bind:value={matrixWs} style="max-width: 220px">
        {#each ws.workspaces as w (w.id)}
          <option value={w.id}>{w.name}</option>
        {/each}
      </select>
    </div>

    {#if matrixLoading}
      <Skeleton rows={3} height={32} />
    {:else}
      <div class="card matrix">
        <div class="matrix-head">
          <span>User</span>
          <span>Role in workspace</span>
        </div>
        {#each users.filter((u) => !u.is_root) as u (u.id)}
          <div class="matrix-row">
            <span class:dim={u.disabled}>{u.display_name} <span class="dim">@{u.username}</span></span>
            <div class="segmented">
              {#each roleOptions as r (r)}
                <button
                  class:active={roleOf(u.id) === r}
                  disabled={u.disabled}
                  onclick={() => setRole(u.id, r)}
                >
                  {r}
                </button>
              {/each}
            </div>
          </div>
        {:else}
          <div class="matrix-empty dim">No non-root users yet.</div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

{#if createOpen}
  <Modal title="New User" onclose={() => (createOpen = false)}>
    <div class="field">
      <label for="nu-user">Username</label>
      <input id="nu-user" class="input" bind:value={newUsername} spellcheck="false" />
    </div>
    <div class="field">
      <label for="nu-display">Display name <span class="dim">(optional)</span></label>
      <input id="nu-display" class="input" bind:value={newDisplay} />
    </div>
    <div class="field">
      <label for="nu-pass">Password</label>
      <input id="nu-pass" class="input" type="password" bind:value={newPassword} autocomplete="new-password" />
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => (createOpen = false)}>Cancel</button>
      <button
        class="btn primary"
        disabled={busy || newUsername.trim() === '' || newPassword.length < 6}
        onclick={createUser}
      >
        {busy ? 'Creating…' : 'Create User'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .user-table {
    max-width: 560px;
    overflow: hidden;
  }
  .user-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
  }
  .user-row + .user-row {
    border-top: 1px solid var(--border);
  }
  .user-row.disabled {
    opacity: 0.6;
  }
  .avatar {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 25%, transparent);
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
    display: grid;
    place-items: center;
  }
  .u-name {
    font-size: 13px;
    font-weight: 500;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .u-sub {
    font-size: 11px;
    color: var(--text-dim);
  }
  .matrix {
    max-width: 640px;
  }
  .matrix-head {
    display: flex;
    justify-content: space-between;
    padding: 8px 14px;
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .matrix-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 14px;
    font-size: 12.5px;
  }
  .matrix-row + .matrix-row {
    border-top: 1px solid var(--border);
  }
  .matrix-empty {
    padding: 16px;
    text-align: center;
  }
</style>
