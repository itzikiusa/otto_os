<script lang="ts">
  // Users admin (root): create/disable users + per-workspace role matrix + feature grants.
  import { api } from '../../lib/api/client';
  import type { GrantEntry, MemberEntry, User, UserGrantsResp, WorkspaceRole } from '../../lib/api/types';
  import type { Capability, Feature } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { copyAsJson } from '../../lib/components/exporters';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Modal from '../../lib/components/Modal.svelte';

  let users: User[] = $state([]);
  let loading = $state(true);
  let createOpen = $state(false);
  let newUsername = $state('');
  let newDisplay = $state('');
  let newPassword = $state('');
  let busy = $state(false);

  /** Filter text for the user list. */
  let userFilter = $state('');

  const filteredUsers = $derived.by(() => {
    const q = userFilter.trim().toLowerCase();
    if (!q) return users;
    return users.filter(
      (u) =>
        u.username.toLowerCase().includes(q) ||
        u.display_name.toLowerCase().includes(q),
    );
  });

  async function copyUsername(u: User): Promise<void> {
    try {
      await navigator.clipboard.writeText(`@${u.username}`);
      toasts.success('Copied', `@${u.username}`);
    } catch {
      toasts.error('Copy failed', 'Could not write to clipboard.');
    }
  }

  async function copyUserJson(u: User): Promise<void> {
    await copyAsJson(u);
    toasts.success('Copied', 'User JSON copied to clipboard.');
  }

  // role matrix
  const roleOptions: (WorkspaceRole | 'none')[] = ['none', 'viewer', 'editor', 'admin'];

  let matrixWs: string = $state('');
  let members: MemberEntry[] = $state([]);
  let matrixLoading = $state(false);

  // ---- feature grant matrix -----------------------------------------------
  const ALL_FEATURES: Feature[] = [
    'agents', 'mission_control', 'connections', 'database', 'git', 'issues', 'product', 'swarm',
    'api_client', 'workflows', 'channels', 'skill_eval', 'skills', 'insights',
    'usage', 'self_improvement', 'context', 'settings', 'users', 'canvas',
    'proof_pack', 'mcp', 'scheduled_tasks', 'run_with_otto',
  ];
  const FEATURE_LABELS: Record<Feature, string> = {
    agents: 'Agents', mission_control: 'Mission Control', connections: 'Connections', database: 'Database',
    git: 'Git', issues: 'Issues', product: 'Product', swarm: 'Swarm',
    api_client: 'API Client', workflows: 'Workflows', channels: 'Channels',
    skill_eval: 'Skills Evaluator', skills: 'Skills', insights: 'Insights',
    usage: 'Usage', self_improvement: 'Self-Improvement', context: 'Context',
    settings: 'Settings', users: 'Users', canvas: 'Canvas',
    proof_pack: 'Proof Packs', mcp: 'MCP Control Plane', scheduled_tasks: 'Scheduled Tasks',
    run_with_otto: 'Run with Otto',
  };
  const CAP_OPTIONS: Capability[] = ['none', 'view', 'edit', 'admin'];

  /** Selected user for the grant matrix (non-root only). */
  let grantUserId: string = $state('');
  /** Working copy of grants for the selected user: feature → capability. */
  let grantMap: Record<string, Capability> = $state({});
  let grantLoading = $state(false);
  let grantSaving = $state(false);

  /** The non-root users available to manage grants for. */
  const nonRootUsers = $derived(users.filter((u) => !u.is_root));

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

  // Auto-select first non-root user for the grant matrix when the list loads.
  $effect(() => {
    if (grantUserId === '' && nonRootUsers.length > 0) {
      grantUserId = nonRootUsers[0].id;
    }
  });

  // Reload the grant map whenever the selected grant user changes.
  $effect(() => {
    const id = grantUserId;
    if (id === '') return;
    grantLoading = true;
    grantMap = {};
    void api
      .get<UserGrantsResp>(`/users/${id}/grants`)
      .then((resp) => {
        const m: Record<string, Capability> = {};
        for (const g of resp.grants) m[g.feature] = g.capability as Capability;
        grantMap = m;
      })
      .catch(() => (grantMap = {}))
      .finally(() => (grantLoading = false));
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

  let impersonatingId: string | null = $state(null);

  async function doImpersonate(u: User): Promise<void> {
    const ok = await confirmer.ask(
      `Act as "${u.display_name}" (@${u.username})? You will see their sessions and data until you stop impersonating.`,
      { title: 'Impersonate User', confirmLabel: 'Impersonate', danger: false },
    );
    if (!ok) return;
    impersonatingId = u.id;
    try {
      await auth.impersonate(u.id);
      toasts.success('Now acting as', u.username);
    } catch (e) {
      toasts.error('Impersonate failed', e instanceof Error ? e.message : String(e));
    } finally {
      impersonatingId = null;
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

  function grantCapOf(feature: Feature): Capability {
    return grantMap[feature] ?? 'none';
  }

  function setGrantCap(feature: Feature, cap: Capability): void {
    grantMap = { ...grantMap, [feature]: cap };
  }

  async function saveGrants(): Promise<void> {
    if (!grantUserId) return;
    grantSaving = true;
    try {
      const grants: GrantEntry[] = ALL_FEATURES
        .filter((f) => grantMap[f] && grantMap[f] !== 'none')
        .map((f) => ({ feature: f, capability: grantMap[f] }));
      await api.put(`/users/${grantUserId}/grants`, { grants });
      toasts.success('Feature grants saved');
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      grantSaving = false;
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
    <div class="user-filter-row">
      <input
        class="input"
        type="search"
        placeholder="Filter users…"
        bind:value={userFilter}
        style="max-width: 260px"
      />
      <span class="dim" style="font-size: 11.5px">
        {filteredUsers.length} of {users.length}
      </span>
    </div>
    <div class="card user-table">
      {#each filteredUsers as u (u.id)}
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
          <button
            class="btn small icon-only"
            title="Copy @username"
            onclick={() => copyUsername(u)}
            aria-label="Copy username"
          >@</button>
          <button
            class="btn small icon-only"
            title="Copy as JSON"
            onclick={() => copyUserJson(u)}
            aria-label="Copy as JSON"
          >{'{}'}</button>
          {#if !u.is_root}
            <button class="btn small {u.disabled ? '' : 'danger'}" onclick={() => toggleDisabled(u)}>
              {u.disabled ? 'Enable' : 'Disable'}
            </button>
            {#if !u.is_root && u.id !== auth.realUser?.id}
              <button
                class="btn small"
                disabled={!!impersonatingId || u.disabled}
                onclick={() => doImpersonate(u)}
              >
                {impersonatingId === u.id ? 'Acting as…' : 'Impersonate'}
              </button>
            {/if}
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

    <!-- Feature grant matrix -->
    <div class="section-title" style="margin-top: 24px">Feature grants</div>
    <div class="sub" style="margin-bottom: 10px">Per-feature capability for non-root users. None = no access; root always has admin everywhere.</div>

    {#if nonRootUsers.length === 0}
      <div class="dim" style="padding: 8px 0">No non-root users yet.</div>
    {:else}
      <div class="row" style="margin-bottom: 10px">
        <select class="input" bind:value={grantUserId} style="max-width: 220px">
          {#each nonRootUsers as u (u.id)}
            <option value={u.id}>{u.display_name} (@{u.username})</option>
          {/each}
        </select>
      </div>

      {#if grantLoading}
        <Skeleton rows={5} height={32} />
      {:else}
        <div class="card grant-matrix">
          <div class="grant-head">
            <span>Feature</span>
            <div class="grant-caps-head">
              {#each CAP_OPTIONS as c (c)}
                <span>{c}</span>
              {/each}
            </div>
          </div>
          {#each ALL_FEATURES as feat (feat)}
            <div class="grant-row">
              <span class="grant-label">{FEATURE_LABELS[feat]}</span>
              <div class="segmented">
                {#each CAP_OPTIONS as c (c)}
                  <button
                    class:active={grantCapOf(feat) === c}
                    onclick={() => setGrantCap(feat, c)}
                  >{c}</button>
                {/each}
              </div>
            </div>
          {/each}
        </div>
        <div style="margin-top: 10px">
          <button class="btn primary" disabled={grantSaving} onclick={saveGrants}>
            {grantSaving ? 'Saving…' : 'Save grants'}
          </button>
        </div>
      {/if}
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
  .user-filter-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 10px;
  }

  .user-table {
    max-width: 600px;
    overflow: hidden;
  }

  .icon-only {
    font-size: 11px;
    font-family: monospace;
    padding: 0 7px;
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
    max-width: min(640px, 92vw);
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
  /* ---- feature grant matrix ---- */
  .grant-matrix {
    max-width: min(700px, 92vw);
  }
  .grant-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 14px;
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .grant-caps-head {
    display: flex;
    gap: 2px;
    /* align with the segmented buttons below */
    min-width: 280px;
    justify-content: space-between;
    padding: 0 2px;
  }
  .grant-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 7px 14px;
    font-size: 12.5px;
  }
  .grant-row + .grant-row {
    border-top: 1px solid var(--border);
  }
  .grant-label {
    min-width: 130px;
  }
</style>
