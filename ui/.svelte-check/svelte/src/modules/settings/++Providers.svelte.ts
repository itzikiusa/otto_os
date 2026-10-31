///<reference types="svelte" />
;
import { serialLatest } from '../../lib/latest';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { NO_WORKSPACE } from '../../lib/labels';
import { sectionLabel } from './sections';
import { settingsSaveErrorText } from './ownerOnly';
import PageBody from '../../lib/components/PageBody.svelte';
// Custom agent providers (root): add any CLI (opencode, kilo, …) as a
// session provider. Stored in the `providers` settings key; the daemon
// reloads its registry live on save. A non-root settings admin sees only
// what their own role reaches — Update all CLIs (ws editor) and the model
// catalog (Agents) — never the daemon-wide settings (`GET|PUT /settings`
// are root-only).
import { api } from '../../lib/api/client';
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import type { Session } from '../../lib/api/types';
import SectionIntro from './SectionIntro.svelte';
import { rel } from '../../lib/stores/now.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import SettingToggle from './SettingToggle.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { loadErrorText } from '../../lib/loadError';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
import { formToProvider, joinArgs, providerToForm, type ProviderDef } from './providerForm';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  interface CliAutoUpdate {
    enabled: boolean;
    time_of_day: string; // "HH:MM"
    use_utc: boolean;
    reload_sessions: boolean;
  }

  const AUTO_UPDATE_DEFAULTS: CliAutoUpdate = {
    enabled: true,
    time_of_day: '03:00',
    use_utc: true,
    reload_sessions: true,
  };

  // Built-in providers with their default update commands (None for shell).
  const BUILTINS: { name: string; updateCmd: string | null }[] = [
    { name: 'claude', updateCmd: 'claude update' },
    { name: 'codex',  updateCmd: 'codex update' },
    { name: 'agy',    updateCmd: 'agy update' },
    { name: 'shell',  updateCmd: null },
  ];

  let loading = $state(true);
  // A failed load shows inline with Retry. Rendering the form instead showed
  // the defaults as if they were saved — and every save here writes whole
  // objects (`providers`, `cli_auto_update`), so one click would have replaced
  // the real custom providers / update schedule with those defaults.
  let loadError = $state('');
  let saving = $state(false);
  let updating = $state(false);
  let custom: Record<string, ProviderDef> = $state({});
  // Latest full settings object (the PUT response). Saves send ONLY the keys
  // they change: PUT /settings upserts exactly the keys in the body, so
  // spreading this page-load snapshot reverted keys written since (auto-update
  // last-run, MCP/PR-review settings, another window) and wrote false
  // skip-permissions / network-listener audit entries on every save.
  let allSettings: Record<string, unknown> = $state({});
  let defaultProvider = $state('');
  /** Model for the PR / commit DRAFT turns (`pr_draft_model`). Deliberately
   *  separate from the default provider model: drafting is a short, mechanical,
   *  user-blocking turn, so it defaults to the fastest model rather than
   *  inheriting a reasoning model and making the modal wait minutes. */
  let draftModel = $state('');
  const DRAFT_MODELS = ['haiku', 'sonnet', 'opus'];
  // Providers the user has EXCLUDED — hidden from every picker (they may have the
  // CLI installed but don't want to use it). Persisted as `disabled_providers`.
  // `shell` is never excludable (plain terminals need it).
  let disabled = $state<Set<string>>(new Set());

  // Daily CLI auto-update config (settings key `cli_auto_update`).
  let autoUpdate: CliAutoUpdate = $state({ ...AUTO_UPDATE_DEFAULTS });
  let lastRun: string | null = $state(null);
  let savingAuto = $state(false);

  // Skip permission prompts (settings key `agent_skip_permissions`, default ON).
  // On = built-in agents launch with their bypass flag (unattended); off = they
  // use their own ask/auto permission mode.
  let skipPermissions = $state(true);
  let savingSkip = $state(false);

  // Dynamic model catalog: per-provider counts + freshness from
  // GET /providers/models; Refresh re-runs the daemon's source chain
  // (CLI probe / docs scrape / models.dev). Loaded independently of the
  // settings blob so a catalog hiccup never blocks this page.
  interface CatalogEntry {
    models: { id: string; label: string; source: string }[];
    fetched_at: string | null;
    stale: boolean;
    last_error?: string;
  }
  let catalog: Record<string, CatalogEntry> = $state({});
  let refreshingModels: string | null = $state(null); // provider slug, or '*' for all

  let catalogError = $state<string | null>(null);
  let catalogLoading = $state(false);
  async function loadCatalog(): Promise<void> {
    catalogLoading = true;
    catalogError = null;
    try {
      const r = await api.get<{ providers: Record<string, CatalogEntry> }>('/providers/models');
      catalog = r.providers;
    } catch (e) {
      // Non-fatal for the page, but an unread catalog must not read as "none yet".
      catalogError = loadErrorText(e);
    } finally {
      catalogLoading = false;
    }
  }

  async function refreshModels(provider?: string): Promise<void> {
    refreshingModels = provider ?? '*';
    try {
      const r = await api.post<{ providers: Record<string, CatalogEntry> }>(
        '/providers/models/refresh',
        provider ? { provider } : {},
      );
      catalog = r.providers;
      toasts.info('Model catalog refreshed');
    } catch (e) {
      toastError('Couldn’t refresh the model catalog', e);
    } finally {
      refreshingModels = null;
    }
  }

  $effect(() => {
    void loadCatalog();
  });

  // Providers offered in the default-agent picker: the live registry from
  // /meta (built-ins + custom overrides), falling back to the built-in names.
  const providers = $derived(auth.meta?.providers ?? BUILTINS.map((b) => b.name));

  // form
  let editing: string | null = $state(null);
  let name = $state('');
  let cmd = $state('');
  let args = $state('');
  let resumeArgs = $state('');
  let updateCmd = $state('');
  let modelArgs = $state('');
  let formOpen = $state(false);
  // Inline validation for the provider sheet (not a toast — components.md §2).
  let formError = $state('');
  // Inline validation for the auto-update time field.
  let timeError = $state('');

  async function updateAllCLIs(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) { toasts.error(NO_WORKSPACE); return; }
    updating = true;
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/providers/update`, {});
      ws.addSession(session); // navigates to the update session
      toasts.info('Updating CLIs…', 'Watch the Update CLIs session for progress');
    } catch (e) {
      toasts.error('Couldn’t update the provider', settingsSaveErrorText(e, loadErrorText));
    } finally {
      updating = false;
    }
  }

  $effect(() => {
    if (auth.isRoot) void loadSettings();
    else loading = false;
  });

  async function loadSettings(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      allSettings = await api.get<Record<string, unknown>>('/settings');
      custom = (allSettings['providers'] as Record<string, ProviderDef> | undefined) ?? {};
      defaultProvider = (allSettings['default_provider'] as string | undefined) ?? '';
      disabled = new Set((allSettings['disabled_providers'] as string[] | undefined) ?? []);
      autoUpdate = {
        ...AUTO_UPDATE_DEFAULTS,
        ...((allSettings['cli_auto_update'] as Partial<CliAutoUpdate> | undefined) ?? {}),
      };
      lastRun = (allSettings['cli_auto_update_last_run'] as string | undefined) ?? null;
      skipPermissions = (allSettings['agent_skip_permissions'] as boolean | undefined) ?? true;
      draftModel = (allSettings['pr_draft_model'] as string | undefined) ?? '';
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function openNew(): void {
    editing = null;
    name = '';
    cmd = '';
    args = '';
    resumeArgs = '';
    updateCmd = '';
    modelArgs = '';
    formError = '';
    formOpen = true;
  }

  function openEdit(n: string): void {
    const p = custom[n];
    editing = n;
    name = n;
    ({ cmd, args, resumeArgs, updateCmd, modelArgs } = providerToForm(p));
    formError = '';
    formOpen = true;
  }

  async function persist(next: Record<string, ProviderDef>): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        providers: next,
      });
      custom = (allSettings['providers'] as Record<string, ProviderDef>) ?? {};
      await auth.refreshMeta();
      toasts.success('Providers saved', 'Available immediately for new sessions');
      formOpen = false;
    } catch (e) {
      toasts.error('Couldn’t save providers', settingsSaveErrorText(e, loadErrorText));
    } finally {
      saving = false;
    }
  }

  async function saveDraftModel(): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        pr_draft_model: draftModel,
      });
      draftModel = (allSettings['pr_draft_model'] as string | undefined) ?? '';
      toasts.success(
        'Draft model saved',
        draftModel === ''
          ? 'PR and commit drafts use haiku (fastest)'
          : `PR and commit drafts use ${draftModel}`,
      );
    } catch (e) {
      toasts.error('Couldn’t save the draft model', settingsSaveErrorText(e, loadErrorText));
    } finally {
      saving = false;
    }
  }

  async function saveDefaultProvider(): Promise<void> {
    saving = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        default_provider: defaultProvider,
      });
      defaultProvider = (allSettings['default_provider'] as string | undefined) ?? '';
      await auth.refreshMeta();
      toasts.success(
        'Default agent saved',
        defaultProvider === ''
          ? 'New sessions use the first available CLI'
          : `New sessions and channel replies default to ${defaultProvider}`,
      );
    } catch (e) {
      toasts.error('Couldn’t save the default agent', settingsSaveErrorText(e, loadErrorText));
    } finally {
      saving = false;
    }
  }

  // Exclude / re-include a provider. Optimistic; reverts on failure.
  // Each PUT carries the WHOLE list, so the writes are SERIALIZED and each one
  // sends the latest `disabled` at send time (S17-308): ordering only the
  // responses still let two concurrent PUTs be applied out of order, leaving
  // the server on the older list. A toggle superseded while queued sends
  // nothing — the newer one carries its change.
  const saveDisabled = serialLatest(() =>
    api.put<Record<string, unknown>>('/settings', { disabled_providers: [...disabled] }),
  );
  async function toggleProvider(nameOfProvider: string, enable: boolean): Promise<void> {
    const next = new Set(disabled);
    if (enable) next.delete(nameOfProvider);
    else next.add(nameOfProvider);
    disabled = next;
    try {
      const r = await saveDisabled();
      if (!r.ok) return; // a newer toggle's write carries this change
      allSettings = r.value;
      disabled = new Set((allSettings['disabled_providers'] as string[] | undefined) ?? []);
      await auth.refreshMeta();
      toasts.success(
        enable ? `${nameOfProvider} enabled` : `${nameOfProvider} hidden`,
        enable
          ? 'It reappears in every provider picker'
          : 'Hidden from every picker; existing sessions keep working',
      );
    } catch (e) {
      // Undo THIS toggle only: a newer toggle queued behind it still sends
      // the (now reverted for this provider) latest list.
      const revert = new Set(disabled);
      if (enable) revert.add(nameOfProvider);
      else revert.delete(nameOfProvider);
      disabled = revert;
      toasts.error(`Couldn’t ${enable ? 'enable' : 'hide'} ${nameOfProvider}`, settingsSaveErrorText(e, loadErrorText));
    }
  }

  // Applies on change (like every other control on this page — no separate
  // Save for one section). The time field commits on `change` (blur / Enter),
  // so half-typed values never save; an invalid one is flagged inline.
  async function saveAutoUpdate(): Promise<void> {
    if (!/^\d{1,2}:\d{2}$/.test(autoUpdate.time_of_day)) {
      timeError = 'Use HH:MM, for example 03:00.';
      return;
    }
    timeError = '';
    savingAuto = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        cli_auto_update: { ...autoUpdate },
      });
      autoUpdate = {
        ...AUTO_UPDATE_DEFAULTS,
        ...((allSettings['cli_auto_update'] as Partial<CliAutoUpdate> | undefined) ?? {}),
      };
      toasts.success(
        autoUpdate.enabled ? 'Automatic updates on' : 'Automatic updates off',
        autoUpdate.enabled ? `Daily at ${autoUpdate.time_of_day} ${autoUpdate.use_utc ? 'UTC' : 'local time'}` : undefined,
      );
    } catch (e) {
      toasts.error('Couldn’t save automatic updates', settingsSaveErrorText(e, loadErrorText));
      // Re-read so the controls never show a schedule that isn't saved.
      autoUpdate = {
        ...AUTO_UPDATE_DEFAULTS,
        ...((allSettings['cli_auto_update'] as Partial<CliAutoUpdate> | undefined) ?? {}),
      };
    } finally {
      savingAuto = false;
    }
  }

  async function saveSkipPermissions(): Promise<void> {
    // Turning the bypass back ON removes a safety prompt from every new
    // session — ask first. Turning it off (safer) needs no confirmation.
    if (
      skipPermissions &&
      !(await confirmer.ask(
        'New agent sessions will run tools, edit files and execute commands without asking you first.',
        { title: 'Skip permission prompts?', confirmLabel: 'Skip prompts', danger: true },
      ))
    ) {
      skipPermissions = false;
      return;
    }
    savingSkip = true;
    try {
      allSettings = await api.put<Record<string, unknown>>('/settings', {
        agent_skip_permissions: skipPermissions,
      });
      skipPermissions = (allSettings['agent_skip_permissions'] as boolean | undefined) ?? true;
      await auth.refreshMeta();
      toasts.success(
        skipPermissions ? 'Permission prompts skipped' : 'Permission prompts enabled',
        'Applies to new sessions; running sessions are unchanged.',
      );
    } catch (e) {
      skipPermissions = !skipPermissions; // revert the optimistic toggle on failure
      toasts.error('Couldn’t save the permission setting', settingsSaveErrorText(e, loadErrorText));
    } finally {
      savingSkip = false;
    }
  }

  async function save(): Promise<void> {
    const n = name.trim();
    if (n === '' || cmd.trim() === '') {
      formError = 'Name and command are required.';
      return;
    }
    if (!/^[a-z0-9][a-z0-9_-]*$/.test(n)) {
      formError = 'Use lowercase letters, digits, - or _ in the name (for example opencode).';
      return;
    }
    if (editing !== n && custom[n]) {
      formError = `A custom provider named “${n}” already exists. Choose another name or edit it.`;
      return;
    }
    formError = '';
    const def = formToProvider(
      { cmd, args, resumeArgs, updateCmd, modelArgs },
      editing ? custom[editing] : undefined,
    );

    const next = { ...custom };
    if (editing && editing !== n) delete next[editing];
    next[n] = def;
    await persist(next);
  }

  async function remove(n: string): Promise<void> {
    if (
      !(await confirmer.ask(
        `Remove the custom provider “${n}”? It disappears from every picker; sessions already running on it keep working. You’d have to re-enter its command to add it back.`,
        { title: 'Remove provider?', danger: true, confirmLabel: 'Remove' },
      ))
    )
      return;
    const next = { ...custom };
    delete next[n];
    await persist(next);
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('providers'),"subtitle":`Agent CLIs Otto can spawn as sessions`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      
       { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-icon":`download`}),"onclick":updateAllCLIs,"disabled":updating || loading,"title":`Run every CLI’s update command now, in a new session`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} updating ? 'Updating…' : 'Update all CLIs';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});                     SectionIntro}
   { svelteHTML.createElement("div", { "class":`providers-body`,});

  if(!auth.isRoot){
     { svelteHTML.createElement("p", {   "class":`hint`,"role":`note`,});
                  
                       
         
     }
  } else if (loading || loadError){
     { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {         "what":`provider settings`,loading,"error":loadError || null,"empty":true,"rows":4,"onretry":() => void loadSettings(),}});}
  }else{
     const enableToggle/*Ωignore_positionΩ*/ = (pname: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("label", {     "class":`checkbox-row tgl`,"title":disabled.has(pname)
          ? `${pname} is hidden from every picker — check to show it`
          : `${pname} is shown in pickers — uncheck to hide it`,});
         { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":!disabled.has(pname),"aria-label":`Show ${pname} in pickers`,"onchange":(e) => toggleProvider(pname, e.currentTarget.checked),});}
        disabled.has(pname) ? 'Hidden' : 'Enabled';
       }
    };return __sveltets_2_any(0)}; { svelteHTML.createElement("section", { "class":`section`,});
       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
       { svelteHTML.createElement("div", { "class":`card set-card`,});
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`prov-default`,});       }
           { svelteHTML.createElement("select", {           "id":`prov-default`,"class":`input select`,"bind:value":defaultProvider,"onchange":saveDefaultProvider,"disabled":saving,});/*Ωignore_startΩ*/() => defaultProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});  }
               for(let p of __sveltets_2_ensureArray(providers)){p;
               { svelteHTML.createElement("option", { "value":p,});p; }
            }
            
            if(defaultProvider !== '' && !providers.includes(defaultProvider)){
               { svelteHTML.createElement("option", { "value":defaultProvider,});defaultProvider;  }
            }
           }
           { svelteHTML.createElement("span", { "class":`hint`,});          }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`prov-draft`,});     }
           { svelteHTML.createElement("select", {         "id":`prov-draft`,"class":`input select`,"bind:value":draftModel,"onchange":saveDraftModel,"disabled":saving,});/*Ωignore_startΩ*/() => draftModel = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});  }
               for(let m of __sveltets_2_ensureArray(DRAFT_MODELS)){m;
               { svelteHTML.createElement("option", { "value":m,});m; }
            }
           }
           { svelteHTML.createElement("span", { "class":`hint`,});
                               
                             
                 
           }
         }
       }
     }

     { svelteHTML.createElement("section", { "class":`section`,});
       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
       { svelteHTML.createElement("div", { "class":`card set-card tfirst`,});
         { const $$_elggoTgnitteS5C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS5C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"label":`Update all CLIs automatically, every day`,"checked":autoUpdate.enabled,"disabled":savingAuto,"onchange":(v) => { autoUpdate.enabled = v; return saveAutoUpdate(); },}});
                         
                      
          if(lastRun){ { svelteHTML.createElement("span", { "title":new Date(lastRun).toLocaleString(),});  rel(lastRun);  }}
         SettingToggle}
        if(autoUpdate.enabled){
           { svelteHTML.createElement("div", { "class":`prow wrap indent`,});
             { svelteHTML.createElement("label", {   "class":`inline`,"for":`prov-time`,});  }
             { svelteHTML.createElement("input", {                 "id":`prov-time`,"class":`input time`,"type":`time`,"bind:value":autoUpdate.time_of_day,"onchange":saveAutoUpdate,"disabled":savingAuto,"aria-invalid":timeError ? 'true' : undefined,"aria-describedby":timeError ? 'prov-time-err' : undefined,});/*Ωignore_startΩ*/() => autoUpdate.time_of_day = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("select", {         "class":`input tz`,"bind:value":autoUpdate.use_utc,"onchange":saveAutoUpdate,"disabled":savingAuto,"aria-label":`Time zone`,});/*Ωignore_startΩ*/() => autoUpdate.use_utc = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", { "value":true,});  }
               { svelteHTML.createElement("option", { "value":false,});  }
             }
           }
          if(timeError){ { svelteHTML.createElement("p", {     "class":`field-err indent`,"id":`prov-time-err`,"role":`alert`,});timeError; }}
           { svelteHTML.createElement("div", { "class":`indent`,});
             { const $$_elggoTgnitteS6C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS6C({ target: __sveltets_2_any(), props: {           "label":`Reload open sessions onto the new version`,"hint":`Reloaded sessions resume their conversation on the new binary.`,"checked":autoUpdate.reload_sessions,"disabled":savingAuto,"onchange":(v) => { autoUpdate.reload_sessions = v; return saveAutoUpdate(); },}});}
           }
        }
       }
     }

     { svelteHTML.createElement("section", { "class":`section`,});
       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
       { svelteHTML.createElement("div", { "class":`card set-card tfirst`,});
         { const $$_elggoTgnitteS5C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS5C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"label":`Skip permission prompts — run agents unattended`,"checked":skipPermissions,"disabled":savingSkip,"onchange":(v) => { skipPermissions = v; return saveSkipPermissions(); },}});
                  
           { svelteHTML.createElement("code", { "class":`flag`,});  } 
           { svelteHTML.createElement("code", { "class":`flag`,});  }     
                          
                      
                        
         SettingToggle}
       }
     }

    
    

     { svelteHTML.createElement("section", { "class":`section`,});
       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
       { svelteHTML.createElement("div", { "class":`list`,});
           for(let b of __sveltets_2_ensureArray(BUILTINS)){b;
           { svelteHTML.createElement("div", {  "class":`item`,});disabled.has(b.name);
             { svelteHTML.createElement("span", { "class":`mono name`,});b.name; }
            if(b.updateCmd){
               { svelteHTML.createElement("span", {   "class":`dim sm mono meta`,"title":`Update command`,});b.updateCmd; }
            }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
            if(custom[b.name]){ { svelteHTML.createElement("span", { "class":`chip`,});  }}
            if(b.name !== 'shell'){
              ;__sveltets_2_ensureSnippet(enableToggle(b.name));
            }else{
               { svelteHTML.createElement("span", {   "class":`dim sm`,"title":`Plain terminals need the shell provider`,});  }
            }
           }
        }
       }
       { svelteHTML.createElement("p", { "class":`hint`,});
          { svelteHTML.createElement("strong", {});  }           
                           
         
       }
     }

     { svelteHTML.createElement("section", { "class":`section`,});
       { svelteHTML.createElement("div", { "class":`prow between`,});
         { svelteHTML.createElement("h2", { "class":`section-title`,});  }
         { svelteHTML.createElement("button", {     "class":`btn small`,...__sveltets_2_empty({"data-icon":`plus`}),"onclick":openNew,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
       }
       { svelteHTML.createElement("div", { "class":`list`,});
           for(let [n, p] of __sveltets_2_ensureArray(Object.entries(custom))){n;
           { svelteHTML.createElement("div", {  "class":`item`,});disabled.has(n);
             { svelteHTML.createElement("span", { "class":`mono name`,});n; }
             { svelteHTML.createElement("span", {   "class":`dim sm mono meta`,"title":`${p.cmd} ${joinArgs(p.args)}`,});p.cmd; joinArgs(p.args); }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
            if(p.resume_args?.length){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Resume arguments set`,});  }}
            if(p.update_command){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Update command: ${p.update_command}`,});  }}
            if(p.model_args?.length){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Model flag: ${joinArgs(p.model_args)}`,});  }}
            ;__sveltets_2_ensureSnippet(enableToggle(n));
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => openEdit(n),});  }
             { svelteHTML.createElement("button", {     "class":`btn small danger`,"onclick":() => remove(n),"disabled":saving,});  }
           }
        }
           { svelteHTML.createElement("div", { "class":`empty`,});            }
        
       }
       { svelteHTML.createElement("p", { "class":`hint`,});
         { svelteHTML.createElement("code", {});'{sid}'; }   { svelteHTML.createElement("code", {});'{cwd}'; }     { svelteHTML.createElement("code", {});'{model}'; }  
          
       }
     }

  }
     { svelteHTML.createElement("section", { "class":`section`,});
       { svelteHTML.createElement("div", { "class":`prow between`,});
         { svelteHTML.createElement("h2", { "class":`section-title`,});  }
         { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-icon":`refresh`}),"onclick":() => refreshModels(),"disabled":refreshingModels !== null,});
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} refreshingModels === '*' ? 'Refreshing…' : 'Refresh all';
         }
       }
       { svelteHTML.createElement("div", { "class":`list`,});
           for(let [prov, cat] of __sveltets_2_ensureArray(Object.entries(catalog).sort())){prov;
           { svelteHTML.createElement("div", { "class":`item`,});
             { svelteHTML.createElement("span", { "class":`mono name`,});prov; }
             { svelteHTML.createElement("span", { "class":`dim sm`,});plural(cat.models.length, 'model'); }
            if(cat.stale){ { svelteHTML.createElement("span", {   "class":`chip warn`,"title":cat.last_error ?? 'The last refresh failed; showing the last good list',});  }}
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("span", {     "class":`dim sm meta`,"title":cat.fetched_at ? `Fetched ${new Date(cat.fetched_at).toLocaleString()}${cat.last_error ? ` · ${cat.last_error}` : ''}` : cat.last_error,});
              cat.fetched_at ? `Fetched ${rel(cat.fetched_at)}` : 'Never fetched';if(cat.last_error){  cat.last_error;}
             }
             { svelteHTML.createElement("button", {         "class":`btn small ghost`,"onclick":() => refreshModels(prov),"disabled":refreshingModels !== null,"aria-label":`Refresh ${prov} models`,});
              refreshingModels === prov ? 'Refreshing…' : 'Refresh';
             }
           }
        }
          if(catalogError){
             { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {            "what":`the model catalog`,"variant":`compact`,"loading":catalogLoading,"error":catalogError,"empty":true,"onretry":loadCatalog,}});}
          }else{
             { svelteHTML.createElement("div", { "class":`empty`,});         }
          }
        
       }
       { svelteHTML.createElement("p", { "class":`hint`,});
                        
                       
       }
     }
   }
   PageBody}
 }

if(formOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":editing ? `Edit ${editing}` : 'New provider',"width":560,"onclose":() => (formOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (formOpen = false),});  }
       { svelteHTML.createElement("button", {       "class":`btn primary`,"type":`submit`,"form":`provider-form`,"disabled":saving,});
        saving ? 'Saving…' : editing ? 'Save provider' : 'Add provider';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("form", {       "class":`grid`,"id":`provider-form`,"onsubmit":(e) => {
        e.preventDefault();
        void save();
      },});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`pv-name`,});  }
         { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`pv-name`,"class":`input mono-in`,"bind:value":name,"placeholder":`opencode`,"spellcheck":`false`,"autocomplete":`off`,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`pv-cmd`,});  }
         { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`pv-cmd`,"class":`input mono-in`,"bind:value":cmd,"placeholder":`opencode`,"spellcheck":`false`,"autocomplete":`off`,});/*Ωignore_startΩ*/() => cmd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`pv-args`,});  }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`pv-args`,"class":`input mono-in`,"bind:value":args,"placeholder":'--session {sid}',"spellcheck":`false`,});/*Ωignore_startΩ*/() => args = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`pv-resume`,});   }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`pv-resume`,"class":`input mono-in`,"bind:value":resumeArgs,"placeholder":'--resume {sid}',"spellcheck":`false`,});/*Ωignore_startΩ*/() => resumeArgs = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`pv-update`,});   }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`pv-update`,"class":`input mono-in`,"bind:value":updateCmd,"placeholder":'npm i -g opencode',"spellcheck":`false`,});/*Ωignore_startΩ*/() => updateCmd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`pv-model`,});    }
         { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`pv-model`,"class":`input mono-in`,"bind:value":modelArgs,"placeholder":'--model {model}',"spellcheck":`false`,});/*Ωignore_startΩ*/() => modelArgs = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
     }
     { svelteHTML.createElement("p", { "class":`hint`,});
       { svelteHTML.createElement("code", {});'{sid}'; }   { svelteHTML.createElement("code", {});'{cwd}'; }     { svelteHTML.createElement("code", {});'{model}'; }    
              { svelteHTML.createElement("code", {});   }    
             
     }
    if(formError){ { svelteHTML.createElement("p", {   "class":`field-err`,"role":`alert`,});formError; }}
    
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Providers__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Providers__SvelteComponent_ = ReturnType<typeof Providers__SvelteComponent_>;
/*Ωignore_endΩ*/export default Providers__SvelteComponent_;