<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
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
  import { copyTextOrThrow } from '../../lib/clipboard';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { loadErrorText } from '../../lib/loadError';

  let users: User[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
  let createOpen = $state(false);
  let createError = $state('');
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
      await copyTextOrThrow(`@${u.username}`);
      toasts.success('Username copied', `@${u.username}`);
    } catch {
      toasts.error('Couldn’t copy the username', 'The clipboard write was blocked.');
    }
  }

  async function copyUserJson(u: User): Promise<void> {
    try {
      await copyAsJson(u);
      toasts.success('User JSON copied');
    } catch {
      toasts.error('Couldn’t copy the user', 'The clipboard write was blocked.');
    }
  }

  // role matrix
  const roleOptions: (WorkspaceRole | 'none')[] = ['none', 'viewer', 'editor', 'admin'];
  const ROLE_LABEL: Record<WorkspaceRole | 'none', string> = { none: 'None', viewer: 'Viewer', editor: 'Editor', admin: 'Admin' };
  const CAP_LABEL: Record<Capability, string> = { none: 'None', view: 'View', edit: 'Edit', admin: 'Admin' };
  let matrixError = $state('');

  let matrixWs: string = $state('');
  let members: MemberEntry[] = $state([]);
  let matrixLoading = $state(false);

  // ---- membership axis ----------------------------------------------------
  //
  // "By workspace" answers *who is in this workspace*; "By user" answers *which
  // workspaces does this user reach* — the question you actually have when
  // onboarding a shared account, which otherwise means re-picking the workspace
  // dropdown once per workspace. Both drive the same
  // `PUT /workspaces/{id}/members` (a full replace, so we keep every
  // workspace's member list in hand).
  let membershipAxis: 'workspace' | 'user' = $state('workspace');
  /** Selected user for the by-user view. */
  let memberUserId: string = $state('');
  /** workspace id → its full member list. Loaded for the by-user view. */
  let allMembers: Record<string, MemberEntry[]> = $state({});
  let allMembersLoading = $state(false);
  /** Workspace ids currently mid-save (disables that row's buttons). */
  let savingWs: string[] = $state([]);

  /** Load every workspace's member list (by-user view needs them all). */
  async function loadAllMembers(): Promise<void> {
    allMembersLoading = true;
    try {
      const pairs = await Promise.all(
        ws.workspaces.map(async (w) => {
          try {
            return [w.id, await api.get<MemberEntry[]>(`/workspaces/${w.id}/members`)] as const;
          } catch {
            return [w.id, [] as MemberEntry[]] as const;
          }
        }),
      );
      allMembers = Object.fromEntries(pairs);
    } finally {
      allMembersLoading = false;
    }
  }

  function roleIn(wsId: string, userId: string): WorkspaceRole | 'none' {
    return allMembers[wsId]?.find((m) => m.user_id === userId)?.role ?? 'none';
  }

  /** Set (or clear) `userId`'s role in one workspace. */
  async function setRoleIn(
    wsId: string,
    userId: string,
    role: WorkspaceRole | 'none',
  ): Promise<void> {
    if (roleIn(wsId, userId) === role) return;
    const current = allMembers[wsId] ?? [];
    const next = current.filter((m) => m.user_id !== userId);
    if (role !== 'none') next.push({ user_id: userId, username: '', display_name: '', role });
    savingWs = [...savingWs, wsId];
    try {
      const saved = await api.put<MemberEntry[]>(`/workspaces/${wsId}/members`, {
        members: next.map((m) => ({ user_id: m.user_id, role: m.role })),
      });
      allMembers = { ...allMembers, [wsId]: saved };
      // Keep the by-workspace view honest when it's showing the same workspace.
      if (matrixWs === wsId) members = saved;
    } catch (e) {
      toasts.error('Couldn’t change the workspace role', e instanceof Error ? e.message : String(e));
    } finally {
      savingWs = savingWs.filter((id) => id !== wsId);
    }
  }

  /** Give the selected user the same role in EVERY workspace (or remove them). */
  async function setRoleEverywhere(role: WorkspaceRole | 'none'): Promise<void> {
    const userId = memberUserId;
    if (!userId) return;
    const targets = ws.workspaces.filter((w) => roleIn(w.id, userId) !== role);
    if (targets.length === 0) return;
    await Promise.all(targets.map((w) => setRoleIn(w.id, userId, role)));
    toasts.success(
      role === 'none' ? 'Removed from all workspaces' : `Set to ${role} in all workspaces`,
      `${targets.length} workspace${targets.length === 1 ? '' : 's'} updated`,
    );
  }

  const memberWsCount = $derived(
    memberUserId ? ws.workspaces.filter((w) => roleIn(w.id, memberUserId) !== 'none').length : 0,
  );

  // ---- feature grant matrix -----------------------------------------------
  const ALL_FEATURES: Feature[] = [
    'agents', 'mission_control', 'connections', 'database', 'git', 'issues', 'product', 'swarm',
    'api_client', 'workflows', 'channels', 'skill_eval', 'skills', 'insights',
    'usage', 'self_improvement', 'context', 'settings', 'users', 'canvas', 'design',
    'proof_pack', 'mcp', 'scheduled_tasks', 'run_with_otto', 'browser',
    'aws', 'aws_s3', 'aws_sqs', 'aws_ec2', 'aws_athena', 'aws_eks', 'aws_rds', 'kubernetes',
  ];
  const FEATURE_LABELS: Record<Feature, string> = {
    agents: 'Agents', mission_control: 'Mission Control', connections: 'Connections', database: 'Database',
    git: 'Git', issues: 'Issues', product: 'Product', swarm: 'Swarm',
    api_client: 'API Client', workflows: 'Workflows', channels: 'Channels',
    skill_eval: 'Skills Evaluator', skills: 'Skills', insights: 'Insights',
    usage: 'Usage', self_improvement: 'Self-Improvement', context: 'Context',
    settings: 'Settings', users: 'Users', canvas: 'Canvas', design: 'Design Hall',
    proof_pack: 'Proof Packs', mcp: 'MCP Control Plane', scheduled_tasks: 'Scheduled Tasks',
    run_with_otto: 'Run with Otto', browser: 'Browser',
    aws: 'AWS — accounts', aws_s3: 'AWS — S3', aws_sqs: 'AWS — SQS', aws_ec2: 'AWS — EC2',
    aws_athena: 'AWS — Athena', aws_eks: 'AWS — EKS', aws_rds: 'AWS — RDS', kubernetes: 'Kubernetes',
  };
  const CAP_OPTIONS: Capability[] = ['none', 'view', 'edit', 'admin'];

  /** Selected user for the grant matrix (non-root only). */
  let grantUserId: string = $state('');
  /** Working copy of grants for the selected user: feature → capability. */
  let grantMap: Record<string, Capability> = $state({});
  let grantLoading = $state(false);
  let grantSaving = $state(false);
  // A failed grants load must never show every feature as "None" — one Save
  // would then wipe the user's real grants.
  let grantError = $state('');
  let grantSavedKey = $state('');
  function grantKey(m: Record<string, Capability>): string {
    return JSON.stringify(ALL_FEATURES.map((f) => m[f] ?? 'none'));
  }
  const grantsDirty = $derived(!grantLoading && !grantError && grantKey(grantMap) !== grantSavedKey);

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
    void loadMatrix(id);
  });

  async function loadMatrix(id: string): Promise<void> {
    matrixLoading = true;
    matrixError = '';
    try {
      const m = await api.get<MemberEntry[]>(`/workspaces/${id}/members`);
      if (matrixWs === id) members = m;
    } catch (e) {
      if (matrixWs === id) {
        members = [];
        matrixError = loadErrorText(e);
      }
    } finally {
      if (matrixWs === id) matrixLoading = false;
    }
  }

  // Auto-select first non-root user for the grant matrix when the list loads.
  $effect(() => {
    if (grantUserId === '' && nonRootUsers.length > 0) {
      grantUserId = nonRootUsers[0].id;
    }
  });

  // Auto-select the same user for the by-user membership view.
  $effect(() => {
    if (memberUserId === '' && nonRootUsers.length > 0) {
      memberUserId = nonRootUsers[0].id;
    }
  });

  // Load every workspace's members the first time the by-user view is opened.
  $effect(() => {
    if (membershipAxis === 'user' && Object.keys(allMembers).length === 0) {
      void loadAllMembers();
    }
  });

  // Reload the grant map whenever the selected grant user changes.
  $effect(() => {
    const id = grantUserId;
    if (id === '') return;
    void loadGrants(id);
  });

  async function loadGrants(id: string): Promise<void> {
    grantLoading = true;
    grantError = '';
    grantMap = {};
    try {
      const resp = await api.get<UserGrantsResp>(`/users/${id}/grants`);
      if (grantUserId !== id) return;
      const m: Record<string, Capability> = {};
      for (const g of resp.grants) m[g.feature] = g.capability as Capability;
      grantMap = m;
      grantSavedKey = grantKey(m);
    } catch (e) {
      if (grantUserId === id) grantError = loadErrorText(e);
    } finally {
      if (grantUserId === id) grantLoading = false;
    }
  }

  async function loadUsers(): Promise<void> {
    loading = true;
    try {
      users = await api.get<User[]>('/users');
      loadError = '';
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function openCreate(): void {
    newUsername = newDisplay = newPassword = '';
    createError = '';
    createOpen = true;
  }

  async function createUser(): Promise<void> {
    if (busy || newUsername.trim() === '' || newPassword.length < 6) return;
    busy = true;
    createError = '';
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
      // Inline, in the sheet — the fix (another username) happens there.
      createError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function toggleDisabled(u: User): Promise<void> {
    try {
      const updated = await api.patch<User>(`/users/${u.id}`, { disabled: !u.disabled });
      users = users.map((x) => (x.id === u.id ? updated : x));
      toasts.success(updated.disabled ? `Disabled @${u.username}` : `Enabled @${u.username}`, updated.disabled ? 'They can no longer sign in.' : 'They can sign in again.');
    } catch (e) {
      toasts.error(`Couldn’t ${u.disabled ? 'enable' : 'disable'} @${u.username}`, e instanceof Error ? e.message : String(e));
    }
  }

  let impersonatingId: string | null = $state(null);

  async function doImpersonate(u: User): Promise<void> {
    const ok = await confirmer.ask(
      `Act as "${u.display_name}" (@${u.username})? You will see their sessions and data until you stop impersonating.`,
      { title: 'Impersonate user', confirmLabel: 'Impersonate', danger: false },
    );
    if (!ok) return;
    impersonatingId = u.id;
    try {
      await auth.impersonate(u.id);
      toasts.success(`Now acting as @${u.username}`);
    } catch (e) {
      toasts.error(`Couldn’t impersonate @${u.username}`, e instanceof Error ? e.message : String(e));
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
      // Keep the by-user view's cache honest too.
      if (allMembers[matrixWs]) allMembers = { ...allMembers, [matrixWs]: members };
    } catch (e) {
      toasts.error('Couldn’t change the workspace role', e instanceof Error ? e.message : String(e));
    }
  }

  function grantCapOf(feature: Feature): Capability {
    return grantMap[feature] ?? 'none';
  }

  function setGrantCap(feature: Feature, cap: Capability): void {
    grantMap = { ...grantMap, [feature]: cap };
  }

  async function saveGrants(): Promise<void> {
    if (!grantUserId || grantError || !grantsDirty) return;
    grantSaving = true;
    try {
      const grants: GrantEntry[] = ALL_FEATURES
        .filter((f) => grantMap[f] && grantMap[f] !== 'none')
        .map((f) => ({ feature: f, capability: grantMap[f] }));
      await api.put(`/users/${grantUserId}/grants`, { grants });
      grantSavedKey = grantKey(grantMap);
      const who = users.find((u) => u.id === grantUserId);
      toasts.success('Feature grants saved', who ? `@${who.username}` : undefined);
    } catch (e) {
      toasts.error('Couldn’t save feature grants', e instanceof Error ? e.message : String(e));
    } finally {
      grantSaving = false;
    }
  }

  function userMenu(e: MouseEvent, u: User): void {
    const items: MenuItem[] = [
      { label: 'Copy @username', icon: 'copy', action: () => void copyUsername(u) },
      { label: 'Copy as JSON', icon: 'copy', action: () => void copyUserJson(u) },
    ];
    if (!u.is_root) {
      if (u.id !== auth.realUser?.id && !u.disabled) {
        items.push({ separator: true }, { label: 'Impersonate…', icon: 'user', disabled: !!impersonatingId, action: () => void doImpersonate(u) });
      }
      items.push({ separator: true }, {
        label: u.disabled ? 'Enable account' : 'Disable account',
        icon: u.disabled ? 'unlock' : 'lock',
        danger: !u.disabled,
        action: () => void toggleDisabled(u),
      });
    }
    ctxMenu.show(e, items);
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('users')} subtitle="Root manages accounts and per-workspace roles">
    {#snippet actions()}
      <button class="btn small primary" data-icon="plus" onclick={openCreate}><Icon name="plus" size={12} /> New user</button>
    {/snippet}
  </PageHeader>
  <PageBody width="readable">

  <LoadState what="users" {loading} error={loadError} empty={users.length === 0} rows={3} onretry={() => void loadUsers()}>
    <h2 class="section-title first">Accounts</h2>
    {#if users.length > 5}
      <div class="user-filter-row">
        <input
          class="input filter"
          type="search"
          placeholder="Filter users…"
          aria-label="Filter users"
          bind:value={userFilter}
        />
        <span class="dim count">{filteredUsers.length} of {users.length}</span>
      </div>
    {/if}
    <div class="card user-table">
      {#each filteredUsers as u (u.id)}
        <div class="user-row" class:disabled={u.disabled}>
          <span class="avatar" aria-hidden="true">{u.display_name.slice(0, 1).toUpperCase()}</span>
          <div class="grow">
            <div class="u-name">
              <span class="u-text" title={u.display_name}>{u.display_name}</span>
              {#if u.is_root}<span class="chip">Root</span>{/if}
              {#if u.disabled}<span class="chip bad">Disabled</span>{/if}
              {#if u.id === auth.realUser?.id}<span class="dim you">you</span>{/if}
            </div>
            <div class="u-sub">@{u.username}</div>
          </div>
          {#if !u.is_root && u.disabled}
            <button class="btn small" onclick={() => void toggleDisabled(u)}>Enable</button>
          {/if}
          <button
            class="icon-btn"
            title={`Actions for @${u.username}`}
            aria-label={`Actions for @${u.username}`}
            onclick={(e) => userMenu(e, u)}
          >
            <Icon name="more" size={14} />
          </button>
        </div>
      {:else}
        <div class="matrix-empty dim">No users match “{userFilter.trim()}”.</div>
      {/each}
    </div>

    {#if nonRootUsers.length === 0}
      <div class="no-members">
        <EmptyState
          icon="user"
          title="Only the root account so far"
          body="Add a user to give them their own sign-in, then choose which workspaces they reach and which features they can use."
        />
      </div>
    {:else}
    <h2 class="section-title">Workspace roles</h2>
    <p class="sub">
      A user only reaches the workspaces they're a member of. Switch to <b>By user</b> to grant one
      account several workspaces at once.
    </p>
    <div class="urow controls">
      <div class="segmented axis" role="group" aria-label="View roles">
        <button
          class:active={membershipAxis === 'workspace'}
          aria-pressed={membershipAxis === 'workspace'}
          onclick={() => (membershipAxis = 'workspace')}
          data-testid="membership-axis-workspace"
        >
          By workspace
        </button>
        <button
          class:active={membershipAxis === 'user'}
          aria-pressed={membershipAxis === 'user'}
          onclick={() => (membershipAxis = 'user')}
          data-testid="membership-axis-user"
        >
          By user
        </button>
      </div>
      {#if membershipAxis === 'workspace'}
        <select class="input picker" bind:value={matrixWs} aria-label="Workspace">
          {#each ws.workspaces as w (w.id)}
            <option value={w.id}>{w.name}</option>
          {/each}
        </select>
      {:else}
        <select class="input picker" bind:value={memberUserId} aria-label="User">
          {#each nonRootUsers as u (u.id)}
            <option value={u.id}>{u.display_name} (@{u.username})</option>
          {/each}
        </select>
      {/if}
    </div>

    {#if membershipAxis === 'workspace'}
      {#if matrixLoading}
        <Skeleton rows={3} height={32} />
      {:else if matrixError}
        <LoadState what="this workspace's members" error={matrixError} empty onretry={() => void loadMatrix(matrixWs)} />
      {:else}
        <div class="card matrix">
          <div class="matrix-head">
            <span>User</span>
            <span>Role in workspace</span>
          </div>
          {#each nonRootUsers as u (u.id)}
            <div class="matrix-row">
              <span class="ws-label" class:dim={u.disabled} title={`${u.display_name} (@${u.username})`}>{u.display_name} <span class="dim">@{u.username}</span></span>
              <div class="segmented" role="group" aria-label={`Role for @${u.username}`}>
                {#each roleOptions as r (r)}
                  <button
                    class:active={roleOf(u.id) === r}
                    aria-pressed={roleOf(u.id) === r}
                    disabled={u.disabled}
                    title={u.disabled ? 'Enable the account to change its role' : undefined}
                    onclick={() => setRole(u.id, r)}
                  >
                    {ROLE_LABEL[r]}
                  </button>
                {/each}
              </div>
            </div>
          {/each}
        </div>
      {/if}
    {:else if allMembersLoading}
      <Skeleton rows={4} height={32} />
    {:else}
      <div class="urow controls">
        <span class="dim">
          Member of {memberWsCount} of {ws.workspaces.length} workspace{ws.workspaces.length === 1
            ? ''
            : 's'}
        </span>
        <span class="grow"></span>
        <span class="dim">Set all to</span>
        <div class="segmented" role="group" aria-label="Set the role in every workspace">
          {#each roleOptions as r (r)}
            <button
              disabled={savingWs.length > 0}
              onclick={() => void setRoleEverywhere(r)}
            >
              {r === 'none' ? 'Remove' : ROLE_LABEL[r]}
            </button>
          {/each}
        </div>
      </div>
      <div class="card matrix">
        <div class="matrix-head">
          <span>Workspace</span>
          <span>Role</span>
        </div>
        {#each ws.workspaces as w (w.id)}
          <div class="matrix-row">
            <span class="ws-label" title={w.root_path}>
              {w.name} <span class="dim">{w.root_path}</span>
            </span>
            <div class="segmented" role="group" aria-label={`Role in ${w.name}`}>
              {#each roleOptions as r (r)}
                <button
                  class:active={roleIn(w.id, memberUserId) === r}
                  aria-pressed={roleIn(w.id, memberUserId) === r}
                  disabled={savingWs.includes(w.id)}
                  onclick={() => void setRoleIn(w.id, memberUserId, r)}
                >
                  {ROLE_LABEL[r]}
                </button>
              {/each}
            </div>
          </div>
        {:else}
          <div class="matrix-empty dim">No workspaces yet.</div>
        {/each}
      </div>
    {/if}

    <!-- Feature grant matrix -->
    <div class="grant-title-row">
      <h2 class="section-title">Feature grants</h2>
      <span class="grow"></span>
      {#if grantsDirty}<span class="unsaved">Unsaved changes</span>{/if}
      <button class="btn small" disabled={grantSaving || !grantsDirty} title={grantsDirty ? 'Save these grants' : 'No unsaved changes'} onclick={saveGrants}>
        {grantSaving ? 'Saving…' : 'Save grants'}
      </button>
    </div>
    <p class="sub">Per-feature capability for non-root users. None = no access; root always has admin everywhere.</p>

    <div class="urow controls">
      <select class="input picker" bind:value={grantUserId} aria-label="User to grant features to">
        {#each nonRootUsers as u (u.id)}
          <option value={u.id}>{u.display_name} (@{u.username})</option>
        {/each}
      </select>
    </div>

    {#if grantLoading}
      <Skeleton rows={5} height={32} />
    {:else if grantError}
      <LoadState what="this user's feature grants" error={grantError} empty onretry={() => void loadGrants(grantUserId)} />
    {:else}
      <div class="card grant-matrix">
        <div class="grant-head">
          <span>Feature</span>
          <span>Access</span>
        </div>
        {#each ALL_FEATURES as feat (feat)}
          <div class="grant-row">
            <span class="grant-label">{FEATURE_LABELS[feat]}</span>
            <div class="segmented" role="group" aria-label={`Access to ${FEATURE_LABELS[feat]}`}>
              {#each CAP_OPTIONS as c (c)}
                <button
                  class:active={grantCapOf(feat) === c}
                  aria-pressed={grantCapOf(feat) === c}
                  onclick={() => setGrantCap(feat, c)}
                >{CAP_LABEL[c]}</button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    {/if}
    {/if}
  </LoadState>
  </PageBody>
</div>

{#if createOpen}
  <Modal title="New user" onclose={() => (createOpen = false)}>
    <form
      id="new-user-form"
      onsubmit={(e) => {
        e.preventDefault();
        void createUser();
      }}
    >
      <div class="field">
        <label for="nu-user">Username</label>
        <input id="nu-user" class="input" bind:value={newUsername} spellcheck="false" autocomplete="off" placeholder="dana" />
        <span class="hint">They sign in with this; shown as @username.</span>
      </div>
      <div class="field">
        <label for="nu-display">Display name <span class="dim">(optional)</span></label>
        <input id="nu-display" class="input" bind:value={newDisplay} placeholder="Dana Cohen" />
      </div>
      <div class="field">
        <label for="nu-pass">Password</label>
        <input id="nu-pass" class="input" type="password" bind:value={newPassword} autocomplete="new-password" aria-describedby="nu-pass-hint" />
        <span class="hint" id="nu-pass-hint" class:bad={newPassword.length > 0 && newPassword.length < 6}>At least 6 characters. Share it with them privately.</span>
      </div>
      {#if createError}<p class="form-err" role="alert"><strong>Couldn’t create the user.</strong> {createError}</p>{/if}
    </form>
    {#snippet footer()}
      <button class="btn" onclick={() => (createOpen = false)}>Cancel</button>
      <button
        class="btn primary"
        type="submit"
        form="new-user-form"
        disabled={busy || newUsername.trim() === '' || newPassword.length < 6}
        title={newUsername.trim() === '' ? 'Enter a username' : newPassword.length < 6 ? 'Enter a password of at least 6 characters' : undefined}
      >
        {busy ? 'Creating…' : 'Create user'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .section-title {
    margin: 24px 0 6px;
  }
  .section-title.first {
    margin-top: 0;
  }
  .sub {
    margin: 0 0 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    max-width: 78ch;
  }
  .dim {
    color: var(--text-dim);
  }
  .urow {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .controls {
    flex-wrap: wrap;
    margin-bottom: 10px;
    font-size: var(--fs-s);
  }
  .picker {
    max-width: 260px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .user-filter-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 10px;
  }
  .filter {
    max-width: 260px;
  }
  .count {
    font-size: var(--fs-xs);
  }
  .user-table,
  .matrix,
  .grant-matrix {
    max-width: 760px;
    overflow: hidden;
  }
  .user-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px 8px 14px;
    min-width: 0;
  }
  .user-row + .user-row {
    border-top: 1px solid var(--border);
  }
  .user-row.disabled .avatar,
  .user-row.disabled .u-name .u-text {
    opacity: 0.6;
  }
  .avatar {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
    font-size: var(--fs-s);
    font-weight: 600;
    display: grid;
    place-items: center;
  }
  .u-name {
    font-size: var(--fs-m);
    font-weight: 500;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .u-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .you {
    font-size: var(--fs-xs);
    font-weight: 400;
  }
  .u-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .no-members {
    max-width: 760px;
    margin-top: 16px;
  }
  .matrix-head,
  .grant-head {
    display: flex;
    justify-content: space-between;
    padding: 8px 14px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .matrix-row,
  .grant-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 14px;
    min-height: 36px;
    font-size: var(--fs-s);
  }
  .matrix-row + .matrix-row,
  .grant-row + .grant-row {
    border-top: 1px solid var(--border);
  }
  .matrix-row .segmented,
  .grant-row .segmented {
    flex-shrink: 0;
  }
  .matrix-empty {
    padding: 16px;
    text-align: center;
    font-size: var(--fs-s);
  }
  .segmented.axis {
    flex: 0 0 auto;
  }
  /* Workspace name + its root path on one line; the path is what disambiguates
     two same-named repos, so keep it visible but let it clip rather than push
     the role buttons out of the card. */
  .ws-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grant-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: 760px;
    margin-top: 24px;
  }
  .grant-title-row .section-title {
    margin: 0;
  }
  .grant-title-row + .sub {
    margin-top: 6px;
  }
  .unsaved {
    font-size: var(--fs-s);
    color: var(--warning);
  }
  .grant-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint.bad {
    color: var(--danger);
  }
  .form-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  @media (max-width: 640px) {
    .matrix-row,
    .grant-row {
      flex-wrap: wrap;
    }
  }
</style>
