<script lang="ts">
  import { onMount } from 'svelte';
  import { accessApi } from '../../lib/api/access';
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { accessOperations, operationLabel, resourceLabels } from '../../lib/access-options';
  import type { AccessGroup, AccessRole, User, ResourceKind } from '../../lib/api/types';
  let groups = $state<AccessGroup[]>([]),
    roles = $state<AccessRole[]>([]),
    users = $state<User[]>([]);
  let selected = $state<AccessGroup | null>(null),
    members = $state<string[]>([]);
  let name = $state(''),
    description = $state(''),
    memberId = $state(''),
    error = $state('');
  let busy = $state(false),
    loading = $state(true);
  let roleId = $state(''),
    roleName = $state(''),
    roleDescription = $state('');
  let roleKind = $state<ResourceKind>('connection');
  let operations = $state<string[]>([]),
    grantable = $state<string[]>([]);
  let membershipGeneration = 0;
  let loadGeneration = 0;
  async function load() {
    const generation = ++loadGeneration;
    loading = true;
    error = '';
    try {
      const loaded = await Promise.all([
        accessApi.groups(),
        accessApi.roles(),
        api.get<User[]>('/users'),
      ]);
      if (generation === loadGeneration) [groups, roles, users] = loaded;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }
  onMount(() => {
    if (auth.isRoot) void load();
  });
  $effect(() =>
    resourceAccess.subscribe((change) => {
      if (change.type === 'reset' && change.identity) {
        loadGeneration++;
        membershipGeneration++;
        groups = [];
        roles = [];
        users = [];
        selected = null;
        members = [];
        name = '';
        description = '';
        editRole();
        void load();
      }
    }),
  );
  async function selectGroup(group: AccessGroup) {
    const generation = ++membershipGeneration;
    selected = group;
    name = group.name;
    description = group.description ?? '';
    members = [];
    memberId = '';
    try {
      const result = await accessApi.members(group.id);
      if (generation === membershipGeneration) members = result;
    } catch (e) {
      if (generation === membershipGeneration) error = String(e);
    }
  }
  function newGroup() {
    membershipGeneration++;
    selected = null;
    name = '';
    description = '';
    members = [];
  }
  async function mutate(action: () => Promise<unknown>) {
    busy = true;
    error = '';
    try {
      await action();
      resourceAccess.invalidate();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function saveGroup() {
    await mutate(async () => {
      const g = selected
        ? await accessApi.updateGroup(selected.id, name.trim(), description.trim() || undefined)
        : await accessApi.createGroup(name.trim(), description.trim() || undefined);
      groups = await accessApi.groups();
      await selectGroup(g);
    });
  }
  async function removeGroup() {
    const g = selected;
    if (!g) return;
    const ok = await confirmer.ask(
      `Delete ${g.name}? Its memberships and rules stop applying, including Deny rules.`,
      { title: 'Delete access group', confirmLabel: 'Delete group', danger: true },
    );
    if (ok)
      await mutate(async () => {
        await accessApi.deleteGroup(g.id);
        newGroup();
        groups = await accessApi.groups();
      });
  }
  async function membership(userId: string, add: boolean) {
    const g = selected;
    if (!g) return;
    if (
      !add &&
      !(await confirmer.ask(
        'Remove this member? Both Allow and Deny rules from this group stop applying.',
        { title: 'Remove group member', confirmLabel: 'Remove member' },
      ))
    )
      return;
    await mutate(async () => {
      if (add) await accessApi.addMember(g.id, userId);
      else await accessApi.removeMember(g.id, userId);
      if (selected?.id === g.id) members = await accessApi.members(g.id);
      memberId = '';
    });
  }
  function editRole(role?: AccessRole) {
    roleId = role?.id ?? '';
    roleName = role?.name ?? '';
    roleDescription = role?.description ?? '';
    roleKind = role?.kind ?? 'connection';
    operations = [...(role?.operations ?? [])];
    grantable = [...(role?.grantable_operations ?? [])];
  }
  async function saveRole() {
    await mutate(async () => {
      const input = {
        name: roleName.trim(),
        description: roleDescription.trim() || null,
        kind: roleKind,
        operations,
        grantable_operations: grantable,
      };
      if (roleId) await accessApi.updateRole(roleId, input);
      else await accessApi.createRole(input);
      roles = await accessApi.roles();
      editRole();
    });
  }
  async function removeRole() {
    if (!roleId) return;
    if (
      !(await confirmer.ask(
        'Delete this preset? Rules that already copied it keep their operations.',
        { title: 'Delete role preset', confirmLabel: 'Delete preset', danger: true },
      ))
    )
      return;
    await mutate(async () => {
      await accessApi.deleteRole(roleId);
      roles = await accessApi.roles();
      editRole();
    });
  }
</script>

<section class="access-groups">
  <h2>Groups & access roles</h2>
  <p class="hint">
    Groups apply reusable access rules across resources. Users still need page access in Settings →
    Users.
  </p>
  {#if !auth.isRoot}<p>Only root can manage groups and role presets.</p>
  {:else if loading}<p>Loading groups…</p>
  {:else}
    {#if error}<p role="alert" class="error">{error}</p>{/if}
    <section>
      <h3>Groups</h3>
      <div class="layout">
        <nav aria-label="Access groups">
          <button class="btn" onclick={newGroup}>New group</button>{#each groups as group}<button
              class="btn"
              class:primary={selected?.id === group.id}
              disabled={busy}
              onclick={() => selectGroup(group)}>{group.name}</button
            >{/each}
        </nav>
        <fieldset disabled={busy}>
          <label>Group name<input bind:value={name} maxlength="120" /></label><label
            >Description<textarea bind:value={description} rows="2"></textarea></label
          >
          <div class="actions">
            <button class="btn primary" disabled={!name.trim()} onclick={saveGroup}
              >{selected ? 'Save group' : 'Create group'}</button
            >{#if selected}<button class="btn danger" onclick={removeGroup}>Delete group</button
              >{/if}
          </div>
          {#if selected}<h4>Members</h4>
            <p class="hint">
              Membership changes affect all resource rules for this group immediately. Removing
              membership also removes this group’s restrictions.
            </p>
            {#each members as id}<div class="member">
                <span>{users.find((u) => u.id === id)?.display_name ?? id}</span><button
                  class="btn small"
                  aria-label={`Remove ${users.find((u) => u.id === id)?.display_name ?? id}`}
                  onclick={() => membership(id, false)}>Remove</button
                >
              </div>{/each}
            {#if !members.length}<p class="hint">No members yet.</p>{/if}
            <div class="actions">
              <label
                >Add user<select bind:value={memberId}
                  ><option value="">Choose user</option
                  >{#each users.filter((u) => !u.disabled && !members.includes(u.id)) as user}<option
                      value={user.id}>{user.display_name} (@{user.username})</option
                    >{/each}</select
                ></label
              ><button class="btn" disabled={!memberId} onclick={() => membership(memberId, true)}
                >Add member</button
              >
            </div>
          {/if}
        </fieldset>
      </div>
    </section>
    <section>
      <h3>Role presets</h3>
      <p class="hint">
        Copy a preset into a resource rule. Editing a preset does not change existing rules.
      </p>
      <div class="layout">
        <nav aria-label="Role presets">
          <button class="btn" onclick={() => editRole()}>New preset</button
          >{#each roles as role}<button
              class="btn"
              class:primary={roleId === role.id}
              disabled={busy}
              onclick={() => editRole(role)}
              >{role.name}<small>{resourceLabels[role.kind]}</small></button
            >{/each}
        </nav>
        <fieldset disabled={busy}>
          <label>Preset name<input bind:value={roleName} maxlength="120" /></label><label
            >Preset description<textarea bind:value={roleDescription} rows="2"></textarea></label
          ><label
            >Resource type<select
              bind:value={roleKind}
              onchange={() => {
                operations = [];
                grantable = [];
              }}
              >{#each Object.entries(resourceLabels) as [kind, label]}<option value={kind}
                  >{label}</option
                >{/each}</select
            ></label
          >
          <h4>Operations</h4>
          <div class="operations">
            {#each accessOperations[roleKind] as op}<label
                ><input type="checkbox" value={op} bind:group={operations} />{operationLabel(
                  op,
                )}</label
              >{/each}
          </div>
          <details>
            <summary>Grantable operations</summary>
            <p class="hint">Operations the subject can delegate to others.</p>
            <div class="operations">
              {#each accessOperations[roleKind] as op}<label
                  ><input type="checkbox" value={op} bind:group={grantable} />{operationLabel(
                    op,
                  )}</label
                >{/each}
            </div>
          </details>
          <div class="actions">
            <button
              class="btn primary"
              disabled={!roleName.trim() || !operations.length}
              onclick={saveRole}>{roleId ? 'Save preset' : 'Create preset'}</button
            >{#if roleId}<button class="btn danger" onclick={removeRole}>Delete preset</button>{/if}
          </div>
        </fieldset>
      </div>
    </section>
  {/if}
</section>

<style>
  .access-groups {
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    max-width: 1050px;
    color: var(--text);
  }
  h2,
  h3,
  h4,
  p {
    margin: 0;
  }
  h2 {
    font-size: 20px;
  }
  h3 {
    font-size: 16px;
  }
  h4 {
    font-size: 13px;
  }
  .hint {
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.5;
    max-width: 78ch;
  }
  section > section {
    display: flex;
    flex-direction: column;
    gap: 12px;
    border-block-start: 1px solid var(--border);
    padding-top: 18px;
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(150px, 220px) 1fr;
    gap: 20px;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 400px;
    overflow: auto;
  }
  nav button {
    text-align: start;
    overflow-wrap: anywhere;
    white-space: normal;
  }
  nav small {
    display: block;
    color: var(--text-dim);
  }
  fieldset {
    display: flex;
    flex-direction: column;
    gap: 12px;
    border: 0;
    min-width: 0;
    padding: 0;
    margin: 0;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
  }
  input,
  textarea,
  select {
    min-width: 0;
    max-width: 100%;
    padding: 7px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
  }
  textarea {
    resize: vertical;
  }
  .actions,
  .member {
    display: flex;
    gap: 8px;
    align-items: end;
    flex-wrap: wrap;
  }
  .member {
    justify-content: space-between;
    font-size: 12px;
  }
  .operations {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(185px, 1fr));
    gap: 8px;
  }
  .operations label {
    flex-direction: row;
    align-items: center;
  }
  .error,
  .danger {
    color: var(--status-exited, #ed635c);
  }
  details summary {
    cursor: pointer;
    font-size: 12px;
  }
  details[open] .operations {
    margin-top: 12px;
  }
  @media (max-width: 640px) {
    .access-groups {
      padding: 14px;
    }
    .layout {
      grid-template-columns: 1fr;
    }
    nav {
      max-height: 180px;
    }
  }
</style>
