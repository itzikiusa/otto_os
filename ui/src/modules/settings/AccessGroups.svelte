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
  import { announceShown } from '../../lib/announce.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import SideRow from '../../lib/components/side/SideRow.svelte';
  import { onTabKey } from '../../lib/tabKeys';
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
  // Groups and role presets are two list/detail editors; showing one at a
  // time (like Context library's Skills | Souls | Context) keeps ONE primary
  // action on screen instead of two stacked forms each with its own.
  let tab = $state<'groups' | 'presets'>('groups');
  /** Phone: the group / preset form is pushed over its list ("‹ Groups"). */
  let phoneOpen = $state(false);
  // Save stays disabled until something changed (layout.md → Settings form).
  const groupDirty = $derived(
    !selected || name.trim() !== selected.name || description.trim() !== (selected.description ?? ''),
  );
  const savedRole = $derived(roles.find((r) => r.id === roleId));
  const sameSet = (a: string[], b: string[]) => a.length === b.length && a.every((x) => b.includes(x));
  const roleDirty = $derived(
    !savedRole ||
      roleName.trim() !== savedRole.name ||
      roleDescription.trim() !== (savedRole.description ?? '') ||
      roleKind !== savedRole.kind ||
      !sameSet(operations, savedRole.operations ?? []) ||
      !sameSet(grantable, savedRole.grantable_operations ?? []),
  );
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
  // Typed-but-unsaved group or preset edits (a blank new form isn't "dirty").
  const leaveDirty = $derived(
    (selected ? groupDirty : !!(name.trim() || description.trim())) ||
      (savedRole ? roleDirty : !!(roleName.trim() || roleDescription.trim() || operations.length)),
  );
  $effect(() => guardUnsaved(() => leaveDirty, { what: 'this group or preset' }));
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
  async function chooseGroup(group?: AccessGroup) {
    if (busy) return;
    phoneOpen = true;
    if (group?.id === selected?.id && group) return;
    const dirty = selected ? groupDirty : !!(name.trim() || description.trim());
    if (dirty && !(await confirmer.ask('Switching groups discards your unsaved changes.', { title: 'Discard unsaved changes?', danger: true, confirmLabel: 'Discard', cancelLabel: 'Keep editing' }))) return;
    if (group) await selectGroup(group);
    else newGroup();
  }
  async function chooseRole(role?: AccessRole) {
    if (busy) return;
    phoneOpen = true;
    if (role?.id === roleId && role) return;
    const dirty = savedRole ? roleDirty : !!(roleName.trim() || roleDescription.trim() || operations.length);
    if (dirty && !(await confirmer.ask('Switching presets discards your unsaved changes.', { title: 'Discard unsaved changes?', danger: true, confirmLabel: 'Discard', cancelLabel: 'Keep editing' }))) return;
    editRole(role);
  }
  async function selectGroup(group: AccessGroup) {
    const generation = ++membershipGeneration;
    selected = group;
    error = '';
    name = group.name;
    description = group.description ?? '';
    members = [];
    memberId = '';
    try {
      const result = await accessApi.members(group.id);
      if (generation === membershipGeneration) members = result;
    } catch (e) {
      if (generation === membershipGeneration) error = `Couldn’t load the members of ${group.name}. ${loadErrorText(e)}`;
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
      error = `Couldn’t ${what}. ${loadErrorText(e)}`;
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
        { title: 'Remove group member', danger: true, confirmLabel: 'Remove member' },
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
      const result = roleId ? await accessApi.updateRole(roleId, input) : await accessApi.createRole(input);
      roles = await accessApi.roles();
      // Stay on the preset just saved — by id (two presets may share a name).
      const saved = roles.find((r) => r.id === result.id);
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
  >
    {#snippet tabs()}
      {#if auth.isRoot}
        <div class="segmented" role="tablist" tabindex="-1" aria-label="Show" onkeydown={onTabKey}>
          <button role="tab" tabindex={tab === 'groups' ? 0 : -1} aria-selected={tab === 'groups'} class:active={tab === 'groups'} onclick={() => { tab = 'groups'; error = ''; }}>
            Groups <span class="count">{groups.length}</span>
          </button>
          <button role="tab" tabindex={tab === 'presets' ? 0 : -1} aria-selected={tab === 'presets'} class:active={tab === 'presets'} onclick={() => { tab = 'presets'; error = ''; }}>
            Role presets <span class="count">{roles.length}</span>
          </button>
        </div>
      {/if}
    {/snippet}
  </PageHeader>
  {#if !auth.isRoot}
    <PageBody width="readable"><p class="hint">Only the root account can manage groups and role presets.</p></PageBody>
  {:else}
  <PageBody fill padded={false}>
  <LoadState what="groups and role presets" {loading} error={loadError} empty={loading || !!loadError} rows={4} onretry={() => void load()}>
    {#if tab === 'groups'}
      <!-- An empty list hides its pane: the page opens on the create form. -->
      <ListDetail module="settings" pane="access-groups" name="Groups" open={phoneOpen} onback={() => (phoneOpen = false)} empty={groups.length === 0}>
        {#snippet list()}
          <ListPane count={groups.length} primary={{ icon: 'plus', label: 'New group', onclick: () => void chooseGroup() }} testid="access-groups-list">
            {#if !selected}
              <SideRow title={name.trim() || 'New group'} meta="Not created yet" selected onclick={() => {}} />
            {/if}
            {#each groups as group (group.id)}
              <SideRow title={group.name} meta={group.description || undefined} tip={group.description ?? group.name} selected={selected?.id === group.id} onclick={() => void chooseGroup(group)} />
            {/each}
          </ListPane>
        {/snippet}
        <div class="ag-content">
          <section class="access-groups" aria-label="Groups">
            <SectionIntro>
              Groups grant access to resources, not pages: <strong>users still need page access in Settings → Users.</strong>
              Role presets are reusable sets of operations you copy into a resource rule.
            </SectionIntro>
            {#if error}<p class="error" use:announceShown={{ text: error, assertive: true }}><Icon name="warning" size={12} /> {error}</p>{/if}
        <fieldset disabled={busy} class="detail">
          <legend class="detail-title">{selected ? selected.name : 'New group'}</legend>
          <div class="field"><label for="ag-name">Group name</label><input dir="auto" id="ag-name" class="input" bind:value={name} maxlength="120" placeholder="Database readers" /></div>
          <div class="field"><label for="ag-desc">Description</label><textarea dir="auto" id="ag-desc" class="input" bind:value={description} rows="2" placeholder="Read-only access to production databases"></textarea></div>
          <div class="actions">
            {#if selected}<button class="btn small danger" onclick={removeGroup}><Icon name="trash" size={12} /> Delete group…</button>{/if}
            <span class="grow"></span>
            <button
              class="btn primary"
              disabled={!name.trim() || !groupDirty}
              title={!name.trim() ? 'Enter a group name' : !groupDirty ? 'No changes to save' : undefined}
              onclick={saveGroup}
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
          </section>
        </div>
      </ListDetail>
    {:else}
      <ListDetail module="settings" pane="role-presets" name="Role presets" open={phoneOpen} onback={() => (phoneOpen = false)} empty={roles.length === 0}>
        {#snippet list()}
          <ListPane count={roles.length} primary={{ icon: 'plus', label: 'New preset', onclick: () => void chooseRole() }} testid="role-presets-list">
            {#if !roleId}
              <SideRow title={roleName.trim() || 'New preset'} meta="Not created yet" selected onclick={() => {}} />
            {/if}
            {#each roles as role (role.id)}
              <SideRow title={role.name} meta={resourceLabels[role.kind]} selected={roleId === role.id} onclick={() => void chooseRole(role)} />
            {/each}
          </ListPane>
        {/snippet}
        <div class="ag-content">
          <section class="access-groups" aria-label="Role presets">
            <p class="hint">
              Copy a preset into a resource rule. Editing a preset does not change existing rules.
            </p>
            {#if error}<p class="error" use:announceShown={{ text: error, assertive: true }}><Icon name="warning" size={12} /> {error}</p>{/if}
        <fieldset disabled={busy} class="detail">
          <legend class="detail-title">{roleId ? roleName || 'Preset' : 'New preset'}</legend>
          <div class="field"><label for="rp-name">Preset name</label><input dir="auto" id="rp-name" class="input" bind:value={roleName} maxlength="120" placeholder="Read-only analyst" /></div>
          <div class="field"><label for="rp-desc">Preset description</label><textarea dir="auto" id="rp-desc" class="input" bind:value={roleDescription} rows="2"></textarea></div>
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
              disabled={!roleName.trim() || !operations.length || !roleDirty}
              title={!roleName.trim() ? 'Enter a preset name' : !operations.length ? 'Pick at least one operation' : !roleDirty ? 'No changes to save' : undefined}
              onclick={saveRole}>{roleId ? 'Save preset' : 'Create preset'}</button
            >
          </div>
        </fieldset>
          </section>
        </div>
      </ListDetail>
    {/if}
  </LoadState>
  </PageBody>
  {/if}
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .ag-content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 20px 24px 32px;
  }
  .access-groups {
    display: flex;
    flex-direction: column;
    gap: 12px;
    color: var(--text);
    max-width: var(--settings-col);
  }
  .count {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  p {
    margin: 0;
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
  /* Floated so the legend is an ordinary flex item inside the fieldset box
     (not drawn on its border); inline-start keeps it right in RTL. */
  .detail-title {
    float: inline-start;
    inline-size: 100%;
    padding: 0;
    margin: 0;
    margin-block-end: 2px;
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
    padding-block: 2px; padding-inline: 10px 6px;
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
  @container (max-width: 640px) {
    .ag-content {
      padding: 14px 16px 24px;
    }
  }
</style>
