<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { onMount } from 'svelte';
  import { accessApi } from '../../lib/api/access';
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { accessOperations, operationLabel, resourceLabels } from '../../lib/access-options';
  import type { AccessGroup, AccessRole, User, ResourceKind } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { loadErrorText } from '../../lib/loadError';
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
  let loadError = $state('');
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
      if (generation === loadGeneration) {
        [groups, roles, users] = loaded;
        loadError = '';
        // Open on an item, not on a blank form: the first group / preset.
        if (!selected && groups[0]) void selectGroup(groups[0]);
        if (!roleId && roles[0]) editRole(roles[0]);
      }
    } catch (e) {
      if (generation === loadGeneration) loadError = loadErrorText(e);
    } finally {
      if (generation === loadGeneration) loading = false;
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
      if (generation === membershipGeneration) error = `Couldn’t load the members of ${group.name}. ${e instanceof Error ? e.message : String(e)}`;
    }
  }
  function newGroup() {
    membershipGeneration++;
    error = '';
    selected = null;
    name = '';
    description = '';
    members = [];
  }
  async function mutate(what: string, action: () => Promise<unknown>, done?: string) {
    busy = true;
    error = '';
    try {
      await action();
      resourceAccess.invalidate();
      if (done) toasts.success(done);
    } catch (e) {
      // Inline, next to the form — the fix is usually in the fields above.
      error = `Couldn’t ${what}. ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      busy = false;
    }
  }
  async function saveGroup() {
    const creating = !selected;
    await mutate(creating ? 'create the group' : 'save the group', async () => {
      const g = selected
        ? await accessApi.updateGroup(selected.id, name.trim(), description.trim() || undefined)
        : await accessApi.createGroup(name.trim(), description.trim() || undefined);
      groups = await accessApi.groups();
      await selectGroup(g);
    }, creating ? 'Group created' : 'Group saved');
  }
  async function removeGroup() {
    const g = selected;
    if (!g) return;
    const ok = await confirmer.ask(
      `Delete ${g.name}? Its memberships and rules stop applying, including Deny rules.`,
      { title: 'Delete access group', confirmLabel: 'Delete group', danger: true },
    );
    if (ok)
      await mutate('delete the group', async () => {
        await accessApi.deleteGroup(g.id);
        newGroup();
        groups = await accessApi.groups();
        if (groups[0]) void selectGroup(groups[0]);
      }, `Deleted ${g.name}`);
  }
  async function membership(userId: string, add: boolean) {
    const g = selected;
    if (!g) return;
    if (
      !add &&
      !(await confirmer.ask(
        `Remove ${users.find((u) => u.id === userId)?.display_name ?? 'this member'} from ${g.name}? Both Allow and Deny rules from this group stop applying to them.`,
        { title: 'Remove group member', confirmLabel: 'Remove member' },
      ))
    )
      return;
    await mutate(add ? 'add the member' : 'remove the member', async () => {
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
    const creating = !roleId;
    await mutate(creating ? 'create the preset' : 'save the preset', async () => {
      const input = {
        name: roleName.trim(),
        description: roleDescription.trim() || null,
        kind: roleKind,
        operations,
        grantable_operations: grantable,
      };
      const name = input.name;
      if (roleId) await accessApi.updateRole(roleId, input);
      else await accessApi.createRole(input);
      roles = await accessApi.roles();
      // Stay on the preset just saved (it used to reset to a blank form).
      const saved = roles.find((r) => r.name === name);
      editRole(saved);
    }, creating ? 'Preset created' : 'Preset saved');
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
    await mutate('delete the preset', async () => {
      await accessApi.deleteRole(roleId);
      roles = await accessApi.roles();
      editRole(roles[0]);
    }, 'Preset deleted');
  }
</script>

<div class="settings-section">
  <PageHeader
    title={sectionLabel('access-groups')}
    subtitle="Reusable access rules across resources"
  />
  <PageBody width="readable">
  <SectionIntro>Groups grant access to resources, not pages: <strong>users still need page access in Settings → Users.</strong></SectionIntro>
<section class="access-groups">
  {#if !auth.isRoot}<p class="hint">Only the root account can manage groups and role presets.</p>
  {:else}
  <LoadState what="groups and role presets" {loading} error={loadError} empty={loading || !!loadError} rows={4} onretry={() => void load()}>
    {#if error}<p role="alert" class="error"><Icon name="warning" size={12} /> {error}</p>{/if}
    <section>
      <h2 class="section-title">Groups <span class="count">{groups.length}</span></h2>
      <div class="layout">
        <div class="list-pane">
          <div class="list-head">
            <span class="list-label">{groups.length ? 'All groups' : 'No groups yet'}</span>
            <button class="icon-btn" aria-label="New group" title="New group" disabled={busy} onclick={newGroup}><Icon name="plus" size={14} /></button>
          </div>
          <nav class="list" aria-label="Access groups">
            {#if !selected}
              <div class="lrow active new" aria-current="true"><span class="row-name">{name.trim() || 'New group'}</span><span class="row-meta">Not created yet</span></div>
            {/if}
            {#each groups as group (group.id)}
              <button
                class="lrow"
                class:active={selected?.id === group.id}
                aria-current={selected?.id === group.id ? 'true' : undefined}
                aria-label={group.name}
                title={group.description ?? group.name}
                disabled={busy}
                onclick={() => selectGroup(group)}
              >
                <span class="row-name">{group.name}</span>
                {#if group.description}<span class="row-meta">{group.description}</span>{/if}
              </button>
            {/each}
          </nav>
        </div>
        <fieldset disabled={busy} class="detail">
          <legend class="detail-title">{selected ? selected.name : 'New group'}</legend>
          <div class="field"><label for="ag-name">Group name</label><input id="ag-name" class="input" bind:value={name} maxlength="120" placeholder="Database readers" /></div>
          <div class="field"><label for="ag-desc">Description</label><textarea id="ag-desc" class="input" bind:value={description} rows="2" placeholder="Read-only access to production databases"></textarea></div>
          <div class="actions">
            {#if selected}<button class="btn small danger" onclick={removeGroup}><Icon name="trash" size={12} /> Delete group…</button>{/if}
            <span class="grow"></span>
            <button class="btn primary" disabled={!name.trim()} title={name.trim() ? undefined : 'Enter a group name'} onclick={saveGroup}
              >{selected ? 'Save group' : 'Create group'}</button
            >
          </div>
          {#if selected}
            <h3 class="sub-title">Members <span class="count">{members.length}</span></h3>
            <p class="hint">
              Membership changes affect all resource rules for this group immediately. Removing
              membership also removes this group’s restrictions.
            </p>
            {#if members.length}
              <div class="members">
                {#each members as id (id)}
                  {@const u = users.find((x) => x.id === id)}
                  <div class="member">
                    <span class="row-name" title={u ? `${u.display_name} (@${u.username})` : id}>{u?.display_name ?? id}{#if u}<span class="dim"> @{u.username}</span>{/if}</span>
                    <button
                      class="btn small ghost"
                      aria-label={`Remove ${u?.display_name ?? id}`}
                      onclick={() => membership(id, false)}>Remove…</button
                    >
                  </div>
                {/each}
              </div>
            {:else}
              <p class="hint">No members yet — add a user below.</p>
            {/if}
            <div class="actions add">
              <div class="field grow">
                <label for="ag-add">Add user</label>
                <select id="ag-add" class="input" bind:value={memberId}
                  ><option value="">Choose a user…</option
                  >{#each users.filter((u) => !u.disabled && !members.includes(u.id)) as user (user.id)}<option
                      value={user.id}>{user.display_name} (@{user.username})</option
                    >{/each}</select
                >
              </div>
              <button class="btn" disabled={!memberId} onclick={() => membership(memberId, true)}
                >Add member</button
              >
            </div>
          {/if}
        </fieldset>
      </div>
    </section>
    <section>
      <h2 class="section-title">Role presets <span class="count">{roles.length}</span></h2>
      <p class="hint">
        Copy a preset into a resource rule. Editing a preset does not change existing rules.
      </p>
      <div class="layout">
        <div class="list-pane">
          <div class="list-head">
            <span class="list-label">{roles.length ? 'All presets' : 'No presets yet'}</span>
            <button class="icon-btn" aria-label="New preset" title="New preset" disabled={busy} onclick={() => editRole()}><Icon name="plus" size={14} /></button>
          </div>
          <nav class="list" aria-label="Role presets">
            {#if !roleId}
              <div class="lrow active new" aria-current="true"><span class="row-name">{roleName.trim() || 'New preset'}</span><span class="row-meta">Not created yet</span></div>
            {/if}
            {#each roles as role (role.id)}
              <button
                class="lrow"
                class:active={roleId === role.id}
                aria-current={roleId === role.id ? 'true' : undefined}
                aria-label={role.name}
                disabled={busy}
                onclick={() => editRole(role)}
              >
                <span class="row-name">{role.name}</span>
                <span class="row-meta">{resourceLabels[role.kind]}</span>
              </button>
            {/each}
          </nav>
        </div>
        <fieldset disabled={busy} class="detail">
          <legend class="detail-title">{roleId ? roleName || 'Preset' : 'New preset'}</legend>
          <div class="field"><label for="rp-name">Preset name</label><input id="rp-name" class="input" bind:value={roleName} maxlength="120" placeholder="Read-only analyst" /></div>
          <div class="field"><label for="rp-desc">Preset description</label><textarea id="rp-desc" class="input" bind:value={roleDescription} rows="2"></textarea></div>
          <div class="field"><label for="rp-kind">Resource type</label><select
              id="rp-kind"
              class="input"
              bind:value={roleKind}
              onchange={() => {
                operations = [];
                grantable = [];
              }}
              >{#each Object.entries(resourceLabels) as [kind, label] (kind)}<option value={kind}
                  >{label}</option
                >{/each}</select
            ></div>
          <h3 class="sub-title">Operations</h3>
          <div class="operations">
            {#each accessOperations[roleKind] as op (op)}<label class="checkbox-row"
                ><input type="checkbox" value={op} bind:group={operations} />{operationLabel(
                  op,
                )}</label
              >{/each}
          </div>
          <details>
            <summary>Grantable operations</summary>
            <p class="hint">Operations the subject can delegate to others.</p>
            <div class="operations">
              {#each accessOperations[roleKind] as op (op)}<label class="checkbox-row"
                  ><input type="checkbox" value={op} bind:group={grantable} />{operationLabel(
                    op,
                  )}</label
                >{/each}
            </div>
          </details>
          <div class="actions">
            {#if roleId}<button class="btn small danger" onclick={removeRole}><Icon name="trash" size={12} /> Delete preset…</button>{/if}
            <span class="grow"></span>
            <button
              class="btn primary"
              disabled={!roleName.trim() || !operations.length}
              title={!roleName.trim() ? 'Enter a preset name' : !operations.length ? 'Pick at least one operation' : undefined}
              onclick={saveRole}>{roleId ? 'Save preset' : 'Create preset'}</button
            >
          </div>
        </fieldset>
      </div>
    </section>
  </LoadState>
  {/if}
</section>
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
  .access-groups {
    display: flex;
    flex-direction: column;
    gap: 24px;
    color: var(--text);
    max-width: 960px;
  }
  .access-groups :global(section) {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  p {
    margin: 0;
  }
  .section-title {
    margin: 0;
  }
  .count {
    font-weight: 500;
  }
  .sub-title {
    margin: 8px 0 0;
    padding-top: 12px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .hint {
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.5;
    max-width: 78ch;
  }
  .dim {
    color: var(--text-dim);
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(180px, 240px) minmax(0, 1fr);
    gap: 16px;
    align-items: start;
  }
  .list-pane {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 24px;
  }
  .list-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .list {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    overflow: hidden auto;
    max-height: 400px;
  }
  .lrow {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px 10px;
    text-align: start;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
    min-width: 0;
  }
  .lrow + .lrow {
    border-top: 1px solid var(--border);
  }
  .lrow:hover:not(:disabled):not(.active) {
    background: var(--hover);
  }
  .lrow.active {
    background: var(--accent-soft);
  }
  .row-name {
    font-size: var(--fs-m);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    padding: 14px 16px;
    margin: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .detail-title {
    float: left;
    width: 100%;
    padding: 0;
    margin: 0 0 2px;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .detail-title + * {
    clear: both;
  }
  .detail .field {
    margin: 0;
  }
  textarea {
    resize: vertical;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: flex-end;
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .members {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .member {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 32px;
    padding: 2px 6px 2px 10px;
    font-size: var(--fs-s);
  }
  .member + .member {
    border-top: 1px solid var(--border);
  }
  .operations {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(185px, 1fr));
    gap: 8px;
  }
  .operations .checkbox-row {
    font-size: var(--fs-s);
  }
  .error {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  details summary {
    cursor: pointer;
    font-size: var(--fs-s);
  }
  details[open] .operations {
    margin-top: 8px;
  }
  details .hint {
    margin-top: 6px;
  }
  @media (max-width: 640px) {
    .layout {
      grid-template-columns: 1fr;
    }
    .list {
      max-height: 180px;
    }
  }
</style>
