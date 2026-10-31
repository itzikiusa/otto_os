///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { toastError } from '../../lib/toastError';
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
import { COPY_FAILED_BODY, copyTextOrThrow } from '../../lib/clipboard';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { loadErrorText } from '../../lib/loadError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let users: User[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
  let createOpen = $state(false);
  let createError = $state('');
  let newUsername = $state('');
  let newDisplay = $state('');
  let newPassword = $state('');
  let busy = $state(false);
  // The daemon's floor (the same one Onboarding shows): the sheet used to
  // say 6 and enable Create at 6, so every 6–9 character password failed.
  const MIN_PASSWORD = 10;

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
      toasts.error('Couldn’t copy the username', COPY_FAILED_BODY);
    }
  }

  async function copyUserJson(u: User): Promise<void> {
    try {
      await copyAsJson(u);
      toasts.success('User JSON copied');
    } catch {
      toasts.error('Couldn’t copy the user', COPY_FAILED_BODY);
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
  let allMembersError = $state('');
  /** Workspace ids currently mid-save (disables that row's buttons). */
  let savingWs: string[] = $state([]);

  /** Load every workspace's member list (by-user view needs them all). */
  async function loadAllMembers(): Promise<void> {
    allMembersLoading = true;
    allMembersError = '';
    try {
      const pairs = await Promise.all(
        ws.workspaces.map(async (w) =>
          [w.id, await api.get<MemberEntry[]>(`/workspaces/${w.id}/members`)] as const,
        ),
      );
      allMembers = Object.fromEntries(pairs);
    } catch (e) {
      // Membership updates replace the full list. Never substitute an empty
      // list for an unreadable workspace, or the next edit removes its users.
      allMembersError = loadErrorText(e);
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
  ): Promise<boolean> {
    if (allMembersLoading || allMembersError || !allMembers[wsId] || savingWs.includes(wsId)) return false;
    if (roleIn(wsId, userId) === role) return true;
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
      flashSaved('roles');
      return true;
    } catch (e) {
      toastError('Couldn’t change the workspace role', e);
      return false;
    } finally {
      savingWs = savingWs.filter((id) => id !== wsId);
    }
  }

  /** Give the selected user the same role in EVERY workspace (or remove them). */
  async function setRoleEverywhere(role: WorkspaceRole | 'none'): Promise<void> {
    const userId = memberUserId;
    if (!userId || savingWs.length || allMembersLoading || allMembersError) return;
    const targets = ws.workspaces.filter((w) => roleIn(w.id, userId) !== role);
    if (targets.length === 0) return;
    // Removing revokes access everywhere at once and drops the role history:
    // irreversible from here, so it asks first (patterns §6).
    if (role === 'none') {
      const name = users.find((u) => u.id === userId)?.username ?? 'this user';
      const ok = await confirmer.ask(
        `Remove @${name} from ${plural(targets.length, 'workspace')}? They lose access to each one immediately; their roles there are not kept.`,
        { title: 'Remove from all workspaces', confirmLabel: 'Remove', danger: true },
      );
      if (!ok || memberUserId !== userId) return;
    }
    const results = await Promise.all(targets.map((w) => setRoleIn(w.id, userId, role)));
    const updated = results.filter(Boolean).length;
    if (updated !== targets.length) {
      toasts.error('Some workspace roles were not updated', `${updated} of ${targets.length} workspaces updated. Retry to apply the remaining changes.`);
      return;
    }
    toasts.success(
      role === 'none' ? 'Removed from all workspaces' : `Set to ${role} in all workspaces`,
      `${plural(targets.length, 'workspace')} updated`,
    );
  }

  const memberWsCount = $derived(
    memberUserId ? ws.workspaces.filter((w) => roleIn(w.id, memberUserId) !== 'none').length : 0,
  );

  // ---- feature grant matrix -----------------------------------------------
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
  // Derived, never hand-listed: the grants PUT is a full replace, so a
  // `Feature` missing from a hand-kept list was wiped on the next save. The
  // `Record<Feature, …>` type makes a new feature a compile error here instead.
  const ALL_FEATURES = Object.keys(FEATURE_LABELS) as Feature[];
  const CAP_OPTIONS: Capability[] = ['none', 'view', 'edit', 'admin'];

  /** Selected user for the grant matrix (non-root only). */
  let grantUserId: string = $state('');
  /** Working copy of grants for the selected user: feature → capability. */
  let grantMap: Record<string, Capability> = $state({});
  let grantLoading = $state(false);
  // Grants apply on click, like the workspace roles above (one saving model
  // per page — this used to be the one matrix with its own Save button, and
  // an unsaved grant was silently lost on navigation). One PUT at a time: the
  // endpoint replaces the whole grant list, so overlapping writes could
  // interleave.
  let grantSaving = $state(false);
  // A failed grants load must never show every feature as "None" — the next
  // click would then write that over the user's real grants.
  let grantError = $state('');

  // Quiet inline "Saved" beside the section that just applied a change.
  let savedIn = $state<'roles' | 'grants' | null>(null);
  let savedTimer: ReturnType<typeof setTimeout> | undefined;
  function flashSaved(which: 'roles' | 'grants'): void {
    savedIn = which;
    // a11y-6-01: announced at the state edge (the visible tick is inserted
    // with its text, which VoiceOver/WebKit doesn't read).
    announce('Saved');
    clearTimeout(savedTimer);
    savedTimer = setTimeout(() => (savedIn = null), 1800);
  }

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
  // Root only: a Users:Admin sees the section, but the grants routes stay root.
  $effect(() => {
    if (auth.isRoot && grantUserId === '' && nonRootUsers.length > 0) {
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
    if (membershipAxis === 'user' && ws.workspaces.length > 0 && Object.keys(allMembers).length === 0) {
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
    if (busy || newUsername.trim() === '' || newPassword.length < MIN_PASSWORD) return;
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
      toastError(`Couldn’t ${u.disabled ? 'enable' : 'disable'} @${u.username}`, e);
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
      toastError(`Couldn’t impersonate @${u.username}`, e);
    } finally {
      impersonatingId = null;
    }
  }

  function roleOf(userId: string): WorkspaceRole | 'none' {
    return members.find((m) => m.user_id === userId)?.role ?? 'none';
  }

  /** A role write in flight (members PUT is a full replace: a second click
   *  racing the first would undo it), with the workspace it was made for. */
  let roleSaving = $state(false);
  async function setRole(userId: string, role: WorkspaceRole | 'none'): Promise<void> {
    if (roleSaving || matrixLoading || matrixError) return;
    // Capture the workspace: the picker can change while the PUT is in flight,
    // and A's reply must never be cached or shown as B's (the next click
    // would then PUT A's members into B, dropping B's users).
    const wsId = matrixWs;
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
    roleSaving = true;
    try {
      const saved = await api.put<MemberEntry[]>(`/workspaces/${wsId}/members`, {
        members: next.map((m) => ({ user_id: m.user_id, role: m.role })),
      });
      if (matrixWs === wsId) members = saved;
      // Keep the by-user view's cache honest too.
      if (allMembers[wsId]) allMembers = { ...allMembers, [wsId]: saved };
      flashSaved('roles');
    } catch (e) {
      toastError('Couldn’t change the workspace role', e);
    } finally {
      roleSaving = false;
    }
  }

  function grantCapOf(feature: Feature): Capability {
    return grantMap[feature] ?? 'none';
  }

  /** Apply one feature's capability now; revert the cell if the write fails. */
  async function setGrantCap(feature: Feature, cap: Capability): Promise<void> {
    const userId = grantUserId;
    if (!userId || grantError || grantSaving || grantCapOf(feature) === cap) return;
    const before = grantMap;
    const next = { ...grantMap, [feature]: cap };
    grantMap = next;
    grantSaving = true;
    try {
      const grants: GrantEntry[] = ALL_FEATURES
        .filter((f) => next[f] && next[f] !== 'none')
        .map((f) => ({ feature: f, capability: next[f] }));
      await api.put(`/users/${userId}/grants`, { grants });
      flashSaved('grants');
    } catch (e) {
      if (grantUserId === userId) grantMap = before;
      toastError(`Couldn’t change access to ${FEATURE_LABELS[feature]}`, e);
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
      // Account changes (`PATCH /users/{id}`) are root-only on the daemon; a
      // Users:Admin keeps the list, impersonation and the Sessions section.
      if (auth.isRoot) items.push({ separator: true }, {
        label: u.disabled ? 'Enable account' : 'Disable account',
        icon: u.disabled ? 'unlock' : 'lock',
        danger: !u.disabled,
        action: () => void toggleDisabled(u),
      });
    }
    ctxMenu.show(e, items);
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('users'),"subtitle":auth.isRoot ? 'Root manages accounts and per-workspace roles' : 'Accounts and per-workspace roles; only root creates, disables or grants',actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(auth.isRoot){
         { svelteHTML.createElement("button", {     "class":`btn small primary`,...__sveltets_2_empty({"data-icon":`plus`}),"onclick":openCreate,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});

   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`users`,loading,"error":loadError,"empty":users.length === 0,"rows":3,"onretry":() => void loadUsers(),}});
     { svelteHTML.createElement("h2", { "class":`section-title first`,});  }
    if(users.length > 5){
       { svelteHTML.createElement("div", { "class":`user-filter-row`,});
         { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input filter`,"type":`search`,"placeholder":`Filter users…`,"aria-label":`Filter users`,"bind:value":userFilter,});/*Ωignore_startΩ*/() => userFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`dim count`,});filteredUsers.length;  users.length; }
       }
    }
     { svelteHTML.createElement("div", { "class":`card user-table`,});
         for(let u of __sveltets_2_ensureArray(filteredUsers)){u.id;
         { svelteHTML.createElement("div", {  "class":`user-row`,});u.disabled;
           { svelteHTML.createElement("span", {   "class":`avatar`,"aria-hidden":`true`,});u.display_name.slice(0, 1).toUpperCase(); }
           { svelteHTML.createElement("div", { "class":`grow`,});
             { svelteHTML.createElement("div", { "class":`u-name`,});
               { svelteHTML.createElement("span", {   "class":`u-text`,"title":u.display_name,});u.display_name; }
              if(u.is_root){ { svelteHTML.createElement("span", { "class":`chip`,});  }}
              if(u.disabled){ { svelteHTML.createElement("span", { "class":`chip bad`,});  }}
              if(u.id === auth.realUser?.id){ { svelteHTML.createElement("span", { "class":`dim you`,});  }}
             }
             { svelteHTML.createElement("div", { "class":`u-sub`,}); u.username; }
           }
          if(auth.isRoot && !u.is_root && u.disabled){
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void toggleDisabled(u),});  }
          }
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"title":`Actions for @${u.username}`,"aria-label":`Actions for @${u.username}`,"onclick":(e) => userMenu(e, u),});
             { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
           }
         }
      }
         { svelteHTML.createElement("div", { "class":`matrix-empty dim`,});   userFilter.trim();  }
      
     }

    if(nonRootUsers.length === 0){
       { svelteHTML.createElement("div", { "class":`no-members`,});
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {       "icon":`user`,"title":`Only the root account so far`,"body":`Add a user to give them their own sign-in, then choose which workspaces they reach and which features they can use.`,}});}
       }
    }else{
     { svelteHTML.createElement("div", { "class":`title-row`,});
       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
      if(savedIn === 'roles'){ { svelteHTML.createElement("span", { "class":`saved`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }}
     }
     { svelteHTML.createElement("p", { "class":`sub`,});
                   { svelteHTML.createElement("b", {});  }   
          
     }
     { svelteHTML.createElement("div", { "class":`urow controls`,});
       { svelteHTML.createElement("div", {     "class":`segmented axis`,"role":`group`,"aria-label":`View roles`,});
         { svelteHTML.createElement("button", {        "aria-pressed":membershipAxis === 'workspace',"onclick":() => (membershipAxis = 'workspace'),...__sveltets_2_empty({"data-testid":`membership-axis-workspace`}),});membershipAxis === 'workspace';
           
         }
         { svelteHTML.createElement("button", {        "aria-pressed":membershipAxis === 'user',"onclick":() => (membershipAxis = 'user'),...__sveltets_2_empty({"data-testid":`membership-axis-user`}),});membershipAxis === 'user';
           
         }
       }
      if(membershipAxis === 'workspace'){
         { svelteHTML.createElement("select", {       "class":`input picker`,"bind:value":matrixWs,"aria-label":`Workspace`,"disabled":roleSaving,});/*Ωignore_startΩ*/() => matrixWs = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let w of __sveltets_2_ensureArray(ws.workspaces)){w.id;
             { svelteHTML.createElement("option", { "value":w.id,});w.name; }
          }
         }
      }else{
         { svelteHTML.createElement("select", {     "class":`input picker`,"bind:value":memberUserId,"aria-label":`User`,});/*Ωignore_startΩ*/() => memberUserId = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let u of __sveltets_2_ensureArray(nonRootUsers)){u.id;
             { svelteHTML.createElement("option", { "value":u.id,});u.display_name; u.username;  }
          }
         }
      }
     }

    if(membershipAxis === 'workspace'){
      if(matrixLoading){
         { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":3,"height":32,}});}
      } else if (matrixError){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`this workspace’s members`,"error":matrixError,"empty":true,"onretry":() => void loadMatrix(matrixWs),}});}
      }else{
         { svelteHTML.createElement("div", { "class":`card matrix`,});
           { svelteHTML.createElement("div", { "class":`matrix-head`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("span", {});   }
           }
             for(let u of __sveltets_2_ensureArray(nonRootUsers)){u.id;
             { svelteHTML.createElement("div", { "class":`matrix-row`,});
               { svelteHTML.createElement("span", {    "class":`ws-label`,"title":`${u.display_name} (@${u.username})`,});u.disabled;u.display_name;  { svelteHTML.createElement("span", { "class":`dim`,}); u.username; } }
               { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Role for @${u.username}`,});
                   for(let r of __sveltets_2_ensureArray(roleOptions)){r;
                   { svelteHTML.createElement("button", {          "aria-pressed":roleOf(u.id) === r,"disabled":u.disabled || roleSaving,"title":u.disabled ? 'Enable the account to change its role' : undefined,"onclick":() => setRole(u.id, r),});roleOf(u.id) === r;
                    ROLE_LABEL[r];
                   }
                }
               }
             }
          }
         }
      }
    } else if (allMembersLoading){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":4,"height":32,}});}
    } else if (allMembersError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`workspace memberships`,"error":allMembersError,"empty":true,"onretry":() => void loadAllMembers(),}});}
    }else{
       { svelteHTML.createElement("div", { "class":`urow controls`,});
         { svelteHTML.createElement("span", { "class":`dim`,});
            memberWsCount;  plural(ws.workspaces.length, 'workspace');
         }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("span", { "class":`dim`,});   }
         { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Set the role in every workspace`,});
             for(let r of __sveltets_2_ensureArray(roleOptions.filter((r) => r !== 'none'))){r;
             { svelteHTML.createElement("button", {     "disabled":savingWs.length > 0,"onclick":() => void setRoleEverywhere(r),});
              ROLE_LABEL[r];
             }
          }
         }
        
         { svelteHTML.createElement("button", {       "class":`btn small danger`,"disabled":savingWs.length > 0 || memberWsCount === 0,"onclick":() => void setRoleEverywhere('none'),});
            
         }
       }
       { svelteHTML.createElement("div", { "class":`card matrix`,});
         { svelteHTML.createElement("div", { "class":`matrix-head`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", {});  }
         }
           for(let w of __sveltets_2_ensureArray(ws.workspaces)){w.id;
           { svelteHTML.createElement("div", { "class":`matrix-row`,});
             { svelteHTML.createElement("span", {   "class":`ws-label`,"title":w.root_path,});
              w.name;  { svelteHTML.createElement("span", { "class":`dim`,});w.root_path; }
             }
             { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Role in ${w.name}`,});
                 for(let r of __sveltets_2_ensureArray(roleOptions)){r;
                 { svelteHTML.createElement("button", {        "aria-pressed":roleIn(w.id, memberUserId) === r,"disabled":savingWs.includes(w.id),"onclick":() => void setRoleIn(w.id, memberUserId, r),});roleIn(w.id, memberUserId) === r;
                  ROLE_LABEL[r];
                 }
              }
             }
           }
        }
           { svelteHTML.createElement("div", { "class":`matrix-empty dim`,});   }
        
       }
    }

    
    if(auth.isRoot){
     { svelteHTML.createElement("div", { "class":`title-row`,});
       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
      if(savedIn === 'grants'){ { svelteHTML.createElement("span", { "class":`saved`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }}
     }
     { svelteHTML.createElement("p", { "class":`sub`,});
                           
         
     }

     { svelteHTML.createElement("div", { "class":`urow controls`,});
       { svelteHTML.createElement("select", {     "class":`input picker`,"bind:value":grantUserId,"aria-label":`User to grant features to`,});/*Ωignore_startΩ*/() => grantUserId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let u of __sveltets_2_ensureArray(nonRootUsers)){u.id;
           { svelteHTML.createElement("option", { "value":u.id,});u.display_name; u.username;  }
        }
       }
     }

    if(grantLoading){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":5,"height":32,}});}
    } else if (grantError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`this user’s feature grants`,"error":grantError,"empty":true,"onretry":() => void loadGrants(grantUserId),}});}
    }else{
       { svelteHTML.createElement("div", { "class":`card grant-matrix`,});
         { svelteHTML.createElement("div", { "class":`grant-head`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", {});  }
         }
           for(let feat of __sveltets_2_ensureArray(ALL_FEATURES)){feat;
           { svelteHTML.createElement("div", { "class":`grant-row`,});
             { svelteHTML.createElement("span", { "class":`grant-label`,});FEATURE_LABELS[feat]; }
             { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Access to ${FEATURE_LABELS[feat]}`,});
                 for(let c of __sveltets_2_ensureArray(CAP_OPTIONS)){c;
                 { svelteHTML.createElement("button", {        "aria-pressed":grantCapOf(feat) === c,"disabled":grantSaving,"onclick":() => void setGrantCap(feat, c),});grantCapOf(feat) === c;CAP_LABEL[c]; }
              }
             }
           }
        }
       }
    }
    }
    }
   LoadState}
   PageBody}
 }

if(createOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"title":`New user`,"onclose":() => (createOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (createOpen = false),});  }
       { svelteHTML.createElement("button", {           "class":`btn primary`,"type":`submit`,"form":`new-user-form`,"disabled":busy || newUsername.trim() === '' || newPassword.length < MIN_PASSWORD,"title":newUsername.trim() === '' ? 'Enter a username' : newPassword.length < MIN_PASSWORD ? `Enter a password of at least ${MIN_PASSWORD} characters` : undefined,});
        busy ? 'Creating…' : 'Create user';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("form", {     "id":`new-user-form`,"onsubmit":(e) => {
        e.preventDefault();
        void createUser();
      },});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`nu-user`,});  }
         { svelteHTML.createElement("input", {              "dir":`auto`,"id":`nu-user`,"class":`input`,"bind:value":newUsername,"spellcheck":`false`,"autocomplete":`off`,"placeholder":`dana`,});/*Ωignore_startΩ*/() => newUsername = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`hint`,});        }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`nu-display`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
         { svelteHTML.createElement("input", {          "dir":`auto`,"id":`nu-display`,"class":`input`,"bind:value":newDisplay,"placeholder":`Dana Cohen`,});/*Ωignore_startΩ*/() => newDisplay = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`nu-pass`,});  }
         { svelteHTML.createElement("input", {            "id":`nu-pass`,"class":`input`,"type":`password`,"bind:value":newPassword,"autocomplete":`new-password`,"aria-describedby":`nu-pass-hint`,});/*Ωignore_startΩ*/() => newPassword = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", {    "class":`hint`,"id":`nu-pass-hint`,});newPassword.length > 0 && newPassword.length < MIN_PASSWORD;  MIN_PASSWORD; if(newPassword.length > 0 && newPassword.length < MIN_PASSWORD){' '; newPassword.length; MIN_PASSWORD; }      }
       }
      if(createError){ { svelteHTML.createElement("p", {   "class":`form-err`,"role":`alert`,}); { svelteHTML.createElement("strong", {});    } createError; }}
     }
    
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Users__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Users__SvelteComponent_ = ReturnType<typeof Users__SvelteComponent_>;
/*Ωignore_endΩ*/export default Users__SvelteComponent_;