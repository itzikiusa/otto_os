///<reference types="svelte" />
;
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
import { guardUnsaved } from '../../lib/leaveGuard';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
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
    if (group?.id === selected?.id && group) return;
    const dirty = selected ? groupDirty : !!(name.trim() || description.trim());
    if (dirty && !(await confirmer.ask('Switching groups discards your unsaved changes.', { title: 'Discard unsaved changes?', danger: true, confirmLabel: 'Discard', cancelLabel: 'Keep editing' }))) return;
    if (group) await selectGroup(group);
    else newGroup();
  }
  async function chooseRole(role?: AccessRole) {
    if (role?.id === roleId) return;
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
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {     "title":sectionLabel('access-groups'),"subtitle":`Reusable access rules across resources`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
            { svelteHTML.createElement("strong", {});         }
                
   SectionIntro}
 { svelteHTML.createElement("section", { "class":`access-groups`,});
  if(!auth.isRoot){ { svelteHTML.createElement("p", { "class":`hint`,});          }
  }else{
   { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`groups and role presets`,loading,"error":loadError,"empty":loading || !!loadError,"rows":4,"onretry":() => void load(),}});
     { svelteHTML.createElement("div", {     "class":`segmented tabs`,"role":`group`,"aria-label":`Show`,});
       { svelteHTML.createElement("button", {     "aria-pressed":tab === 'groups',"onclick":() => { tab = 'groups'; error = ''; },});tab === 'groups';
          { svelteHTML.createElement("span", { "class":`count`,});groups.length; }
       }
       { svelteHTML.createElement("button", {     "aria-pressed":tab === 'presets',"onclick":() => { tab = 'presets'; error = ''; },});tab === 'presets';
           { svelteHTML.createElement("span", { "class":`count`,});roles.length; }
       }
     }
    if(error){ { svelteHTML.createElement("p", {   "role":`alert`,"class":`error`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} error; }}
    if(tab === 'groups'){
     { svelteHTML.createElement("section", { "aria-label":`Groups`,});
      
       { svelteHTML.createElement("div", {  "class":`layout`,});groups.length === 0;
        if(groups.length){
         { svelteHTML.createElement("div", { "class":`list-pane`,});
           { svelteHTML.createElement("div", { "class":`list-head`,});
             { svelteHTML.createElement("span", { "class":`list-label`,});groups.length ? 'All groups' : 'No groups yet'; }
             { svelteHTML.createElement("button", {         "class":`icon-btn`,"aria-label":`New group`,"title":`New group`,"disabled":busy,"onclick":() => chooseGroup(),}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
           }
           { svelteHTML.createElement("nav", {   "class":`list`,"aria-label":`Access groups`,});
            if(!selected){
               { svelteHTML.createElement("div", {   "class":`lrow active new`,"aria-current":`true`,}); { svelteHTML.createElement("span", { "class":`row-name`,});name.trim() || 'New group'; } { svelteHTML.createElement("span", { "class":`row-meta`,});   } }
            }
               for(let group of __sveltets_2_ensureArray(groups)){group.id;
               { svelteHTML.createElement("button", {              "class":`lrow`,"aria-current":selected?.id === group.id ? 'true' : undefined,"aria-label":group.name,"title":group.description ?? group.name,"disabled":busy,"onclick":() => chooseGroup(group),});selected?.id === group.id;
                 { svelteHTML.createElement("span", { "class":`row-name`,});group.name; }
                if(group.description){ { svelteHTML.createElement("span", { "class":`row-meta`,});group.description; }}
               }
            }
           }
         }
        }
         { svelteHTML.createElement("fieldset", {   "disabled":busy,"class":`detail`,});
           { svelteHTML.createElement("legend", { "class":`detail-title`,});selected ? selected.name : 'New group'; }
           { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`ag-name`,});  } { svelteHTML.createElement("input", {            "dir":`auto`,"id":`ag-name`,"class":`input`,"bind:value":name,"maxlength":120,"placeholder":`Database readers`,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
           { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`ag-desc`,});  } { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`ag-desc`,"class":`input`,"bind:value":description,"rows":2,"placeholder":`Read-only access to production databases`,});/*Ωignore_startΩ*/() => description = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
           { svelteHTML.createElement("div", { "class":`actions`,});
            if(selected){ { svelteHTML.createElement("button", {   "class":`btn small danger`,"onclick":removeGroup,}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}   }}
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":!name.trim() || !groupDirty,"title":!name.trim() ? 'Enter a group name' : !groupDirty ? 'No changes to save' : undefined,"onclick":saveGroup,});selected ? 'Save group' : 'Create group'; }
           }
          if(selected){
             { svelteHTML.createElement("h3", { "class":`sub-title`,});  { svelteHTML.createElement("span", { "class":`count`,});members.length; } }
             { svelteHTML.createElement("p", { "class":`hint`,});
                        
                   
             }
            if(members.length){
               { svelteHTML.createElement("div", { "class":`members`,});
                   for(let id of __sveltets_2_ensureArray(members)){id;
                  const u = users.find((x) => x.id === id);
                   { svelteHTML.createElement("div", { "class":`member`,});
                     { svelteHTML.createElement("span", {   "class":`row-name`,"title":u ? `${u.display_name} (@${u.username})` : id,});u?.display_name ?? id;if(u){ { svelteHTML.createElement("span", { "class":`dim`,}); u.username; }} }
                     { svelteHTML.createElement("button", {      "class":`btn small ghost`,"aria-label":`Remove ${u?.display_name ?? id}`,"onclick":() => membership(id, false),});  }
                   }
                }
               }
            }else{
               { svelteHTML.createElement("p", { "class":`hint`,});        }
            }
             { svelteHTML.createElement("div", { "class":`actions add`,});
               { svelteHTML.createElement("div", { "class":`field grow`,});
                 { svelteHTML.createElement("label", { "for":`ag-add`,});  }
                 { svelteHTML.createElement("select", {      "id":`ag-add`,"class":`input`,"bind:value":memberId,});/*Ωignore_startΩ*/() => memberId = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", {"value":"",});   }   for(let user of __sveltets_2_ensureArray(users.filter((u) => !u.disabled && !members.includes(u.id)))){user.id; { svelteHTML.createElement("option", {  "value":user.id,});user.display_name; user.username;  }} }
               }
               { svelteHTML.createElement("button", {      "class":`btn`,"disabled":!memberId,"onclick":() => membership(memberId, true),});  }
             }
          }
         }
       }
     }
    }else{
     { svelteHTML.createElement("section", { "aria-label":`Role presets`,});
       { svelteHTML.createElement("p", { "class":`hint`,});
                      
       }
       { svelteHTML.createElement("div", {  "class":`layout`,});roles.length === 0;
        if(roles.length){
         { svelteHTML.createElement("div", { "class":`list-pane`,});
           { svelteHTML.createElement("div", { "class":`list-head`,});
             { svelteHTML.createElement("span", { "class":`list-label`,});roles.length ? 'All presets' : 'No presets yet'; }
             { svelteHTML.createElement("button", {         "class":`icon-btn`,"aria-label":`New preset`,"title":`New preset`,"disabled":busy,"onclick":() => chooseRole(),}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
           }
           { svelteHTML.createElement("nav", {   "class":`list`,"aria-label":`Role presets`,});
            if(!roleId){
               { svelteHTML.createElement("div", {   "class":`lrow active new`,"aria-current":`true`,}); { svelteHTML.createElement("span", { "class":`row-name`,});roleName.trim() || 'New preset'; } { svelteHTML.createElement("span", { "class":`row-meta`,});   } }
            }
               for(let role of __sveltets_2_ensureArray(roles)){role.id;
               { svelteHTML.createElement("button", {            "class":`lrow`,"aria-current":roleId === role.id ? 'true' : undefined,"aria-label":role.name,"disabled":busy,"onclick":() => chooseRole(role),});roleId === role.id;
                 { svelteHTML.createElement("span", { "class":`row-name`,});role.name; }
                 { svelteHTML.createElement("span", { "class":`row-meta`,});resourceLabels[role.kind]; }
               }
            }
           }
         }
        }
         { svelteHTML.createElement("fieldset", {   "disabled":busy,"class":`detail`,});
           { svelteHTML.createElement("legend", { "class":`detail-title`,});roleId ? roleName || 'Preset' : 'New preset'; }
           { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`rp-name`,});  } { svelteHTML.createElement("input", {            "dir":`auto`,"id":`rp-name`,"class":`input`,"bind:value":roleName,"maxlength":120,"placeholder":`Read-only analyst`,});/*Ωignore_startΩ*/() => roleName = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
           { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`rp-desc`,});  } { svelteHTML.createElement("textarea", {         "dir":`auto`,"id":`rp-desc`,"class":`input`,"bind:value":roleDescription,"rows":2,});/*Ωignore_startΩ*/() => roleDescription = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
           { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`rp-kind`,});  } { svelteHTML.createElement("select", {         "id":`rp-kind`,"class":`input`,"bind:value":roleKind,"onchange":() => {
                operations = [];
                grantable = [];
              },});/*Ωignore_startΩ*/() => roleKind = __sveltets_2_any(null);/*Ωignore_endΩ*/   for(let [kind, label] of __sveltets_2_ensureArray(Object.entries(resourceLabels))){kind; { svelteHTML.createElement("option", {  "value":kind,});label; }} } }
           { svelteHTML.createElement("h3", { "class":`sub-title`,});  }
           { svelteHTML.createElement("div", { "class":`operations`,});
               for(let op of __sveltets_2_ensureArray(accessOperations[roleKind])){op; { svelteHTML.createElement("label", {  "class":`checkbox-row`,}); { svelteHTML.createElement("input", {     "type":`checkbox`,"value":op,});operations = __sveltets_2_any(null);}operationLabel(
                  op,
                ); }}
           }
           { svelteHTML.createElement("details", {});
             { svelteHTML.createElement("summary", {});  }
             { svelteHTML.createElement("p", { "class":`hint`,});       }
             { svelteHTML.createElement("div", { "class":`operations`,});
                 for(let op of __sveltets_2_ensureArray(accessOperations[roleKind])){op; { svelteHTML.createElement("label", {  "class":`checkbox-row`,}); { svelteHTML.createElement("input", {     "type":`checkbox`,"value":op,});grantable = __sveltets_2_any(null);}operationLabel(
                    op,
                  ); }}
             }
           }
           { svelteHTML.createElement("div", { "class":`actions`,});
            if(roleId){ { svelteHTML.createElement("button", {   "class":`btn small danger`,"onclick":removeRole,}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}   }}
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {        "class":`btn primary`,"disabled":!roleName.trim() || !operations.length || !roleDirty,"title":!roleName.trim() ? 'Enter a preset name' : !operations.length ? 'Pick at least one operation' : !roleDirty ? 'No changes to save' : undefined,"onclick":saveRole,});roleId ? 'Save preset' : 'Create preset'; }
           }
         }
       }
     }
    }
   LoadState}
  }
 }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AccessGroups__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AccessGroups__SvelteComponent_ = ReturnType<typeof AccessGroups__SvelteComponent_>;
/*Ωignore_endΩ*/export default AccessGroups__SvelteComponent_;