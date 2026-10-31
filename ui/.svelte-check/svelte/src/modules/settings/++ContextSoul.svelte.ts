///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { NO_WORKSPACE } from '../../lib/labels';
import { toastError } from '../../lib/toastError';
import { sectionLabel } from './sections';
import { guardUnsaved } from '../../lib/leaveGuard';
import SectionIntro from './SectionIntro.svelte';
import SettingToggle from './SettingToggle.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
// Context & Soul settings page: per-workspace context provisioning. Pick which
// library skills are active, which soul (persona) to use, free-form extra
// context, and whether to inline the workspace MEMORY.md. A "Materialize now"
// button per provider (claude / codex / agy) writes the resolved context into
// the workspace's native CLI files on demand.
import { onDestroy } from 'svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { contextApi } from '../../lib/api/context';
import type {
    ContextPreviewReq,
    LibrarySkill,
    LibrarySoul,
    UpdateWorkspaceContextReq,
    WorkspaceContextConfig,
  } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import EmptyState from '../../lib/components/EmptyState.svelte';
import ContextPreview from '../agents/ContextPreview.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { loadErrorText } from '../../lib/loadError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let cfg: WorkspaceContextConfig | null = $state(null);
  let skills: LibrarySkill[] = $state([]);
  let souls: LibrarySoul[] = $state([]);
  let loading = $state(false);
  let saving = $state(false);
  let error = $state('');
  let references = $state('');
  let artifacts = $state('');
  let generation = 0;
  const editing = $derived(auth.me?.is_root || ws.current?.my_role === 'admin');
  const canMaterialize = $derived(auth.me?.is_root || ['admin', 'editor'].includes(ws.current?.my_role ?? ''));
  const lines = (text: string) => text.split('\n').map((line) => line.trim()).filter(Boolean);
  onDestroy(() => { generation++; });
  let materializing: string | null = $state(null);

  // "All active" vs explicit selection. When true, cfg.skills is null (every
  // library skill is active). When false, cfg.skills is the explicit list.
  let allSkills = $state(true);
  // Explicit selection (a Set of skill names) — only meaningful when !allSkills.
  let selectedSkills: Set<string> = $state(new Set());

  const wsId = $derived(ws.currentId);

  // The agent CLIs this feature can materialize into. The backend only knows how
  // to write a context bundle for these three (anything else is skipped), so the
  // set is a genuine capability limit — NOT the full provider registry.
  const MATERIALIZE_PROVIDERS = ['claude', 'codex', 'agy'] as const;

  // Which provider the dry-run preview targets. Seed from the configured default
  // agent, constrained to the materializable set (fall back to the first one).
  let previewProvider = $state(
    (MATERIALIZE_PROVIDERS as readonly string[]).includes(defaultAgentProvider())
      ? defaultAgentProvider()
      : MATERIALIZE_PROVIDERS[0],
  );
  // The not-yet-saved selection, fed to the preview so it reflects in-flight
  // edits (skills/soul/extra/memory) rather than only what's persisted.
  function buildPreviewOverrides(c: WorkspaceContextConfig): ContextPreviewReq {
    return {
      skills: allSkills ? null : [...selectedSkills],
      soul: c.soul,
      extra_context_md: c.extra_context_md,
      goal_md: c.goal_md,
      memory_md: c.memory_md,
      decisions_md: c.decisions_md,
      references: lines(references),
      artifacts: lines(artifacts),
      include_memory: c.include_memory,
      include_repo_map: c.include_repo_map ?? false,
    };
  }

  // Agent CLIs offered for materialization: the live registry, restricted to the
  // providers whose context bundle the backend can actually write.
  const providers = $derived(
    agentProviders().filter((p) => (MATERIALIZE_PROVIDERS as readonly string[]).includes(p)),
  );

  // ---------------------------------------------------------------------------
  // Load on workspace change
  // ---------------------------------------------------------------------------

  $effect(() => {
    const id = wsId;
    if (id) void load(id);
    else { generation++; cfg = null; loading = false; error = ''; }
  });

  // What Save would send, as a string — compared against the last loaded /
  // saved value so Save stays disabled until something actually changed.
  function draftKey(c: WorkspaceContextConfig): string {
    return JSON.stringify({
      skills: allSkills ? null : [...selectedSkills].sort(),
      soul: c.soul ?? null,
      extra: c.extra_context_md ?? '',
      goal: c.goal_md ?? '',
      memory: c.memory_md ?? '',
      decisions: c.decisions_md ?? '',
      references: lines(references),
      artifacts: lines(artifacts),
      include_memory: !!c.include_memory,
      include_repo_map: !!c.include_repo_map,
    });
  }
  let savedKey = $state('');
  const dirty = $derived(!!cfg && draftKey(cfg) !== savedKey);
  // Leaving Settings (or this page) with unsaved edits asks first.
  $effect(() => guardUnsaved(() => dirty && !!editing, { what: 'the workspace context' }));
  let loadError = $state('');

  function accept(value: WorkspaceContextConfig) {
    cfg = { ...value, goal_md: value.goal_md ?? '', memory_md: value.memory_md ?? '', decisions_md: value.decisions_md ?? '', references: value.references ?? [], artifacts: value.artifacts ?? [], context_version: value.context_version ?? 0 };
    references = cfg.references.join('\n');
    artifacts = cfg.artifacts.join('\n');
    allSkills = cfg.skills === null;
    selectedSkills = new Set(cfg.skills ?? []);
    savedKey = draftKey(cfg);
  }

  async function load(id: string): Promise<void> {
    const request = ++generation;
    loading = true; saving = false; cfg = null; error = ''; loadError = '';
    try {
      const [next, nextSkills, nextSouls] = await Promise.all([
        contextApi.getWorkspaceContext(id), contextApi.listSkills(), contextApi.listSouls(),
      ]);
      if (request !== generation || wsId !== id) return;
      accept(next); skills = nextSkills; souls = nextSouls;
    } catch (e) {
      if (request === generation && wsId === id) loadError = loadErrorText(e);
    } finally {
      if (request === generation && wsId === id) loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Skill selection helpers
  // ---------------------------------------------------------------------------

  function toggleAllSkills(checked: boolean): void {
    allSkills = checked;
    if (!checked && selectedSkills.size === 0) {
      // Switching to explicit selection: pre-select everything so the user
      // starts from "all" and trims down rather than from an empty set.
      selectedSkills = new Set(skills.map((s) => s.name));
    }
  }

  function toggleSkill(name: string, checked: boolean): void {
    const next = new Set(selectedSkills);
    if (checked) next.add(name);
    else next.delete(name);
    selectedSkills = next;
  }

  // ---------------------------------------------------------------------------
  // Save config
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    const id = wsId; const current = cfg; const request = generation;
    if (!id || !current || !editing || saving || loading || !dirty) return;
    saving = true; error = '';
    try {
      const body: UpdateWorkspaceContextReq = {
        skills: allSkills ? null : [...selectedSkills], soul: current.soul,
        extra_context_md: current.extra_context_md,
        goal_md: current.goal_md, memory_md: current.memory_md, decisions_md: current.decisions_md,
        references: lines(references), artifacts: lines(artifacts), context_version: current.context_version,
        include_memory: current.include_memory, include_repo_map: current.include_repo_map ?? false,
      };
      const saved = await contextApi.updateWorkspaceContext(id, body);
      if (request !== generation || wsId !== id) return;
      accept(saved);
      toasts.success('Workspace context saved', 'Used when agent sessions start or restart.');
    } catch (e) {
      if (request === generation && wsId === id) {
        error = e instanceof Error ? e.message : String(e);
        toasts.error('Couldn’t save workspace context', error);
      }
    } finally {
      if (request === generation && wsId === id) saving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Materialize now
  // ---------------------------------------------------------------------------

  async function materialize(provider: string): Promise<void> {
    if (!wsId || !canMaterialize) return;
    materializing = provider;
    try {
      const resp = await contextApi.materialize(wsId, provider);
      const result = resp.provider_results.find((r) => r.provider === provider);
      if (!result || result.skipped) {
        toasts.info(`${provider}: nothing to write`, 'Its context files were already up to date.');
      } else if (result.files_written.length === 0) {
        toasts.info(`Materialized ${provider}`, 'No files written.');
      } else {
        toasts.success(
          `Materialized ${provider}`,
          result.files_written.join(', '),
        );
      }
    } catch (e) {
      toastError(`Couldn’t materialize ${provider}`, e);
    } finally {
      materializing = null;
    }
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => { if (dirty && editing && !error) announce('Unsaved changes — Save to apply them.'); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('context-soul'),"subtitle":`Shared goals, instructions and memory for sessions`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(cfg && editing){
         { svelteHTML.createElement("button", {         "class":`btn small primary`,"disabled":saving || loading || !dirty,"title":dirty ? 'Save workspace context' : 'No unsaved changes',"onclick":save,});
          saving ? 'Saving…' : 'Save';
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}}); { svelteHTML.createElement("strong", {});ws.current?.name ?? 'Your workspace'; }                            SectionIntro}

  if(!wsId){
    
     { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`folder`,"title":NO_WORKSPACE,"body":`Workspace context belongs to a workspace. Pick one from the workspace menu at the top of the sidebar to edit its shared project context.`,}});}
  } else if ((loading && !cfg) || (!cfg && loadError)){
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`workspace context`,"loading":loading,"error":loadError,"empty":true,"rows":4,"onretry":() => wsId && load(wsId),}});}
  } else if (cfg){
     { svelteHTML.createElement("p", { "class":`meta`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
       { svelteHTML.createElement("span", {   "class":`path mono`,"title":ws.current?.root_path,});ws.current?.root_path; }
     }
    if(!editing){ { svelteHTML.createElement("p", { "class":`readonly`,}); { svelteHTML.createElement("strong", {});  }       }}
    if(dirty && editing && !error){ { svelteHTML.createElement("p", { "class":`form-note unsaved`,});       }}
    if(error){
       { svelteHTML.createElement("div", {   "class":`save-err`,"role":`alert`,});
         { svelteHTML.createElement("p", {}); { svelteHTML.createElement("strong", {});    }  { svelteHTML.createElement("span", { "class":`dim`,});error; } }
         { svelteHTML.createElement("p", { "class":`dim`,});                }
         { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":saving,"onclick":() => wsId && load(wsId),});   }
       }
    }
    
     { svelteHTML.createElement("fieldset", {   "class":`cs-form`,"disabled":!editing || saving || loading,});
       { svelteHTML.createElement("legend", { "class":`sr-only`,});  }
       { svelteHTML.createElement("h2", { "class":`section-title`,});   }
       { svelteHTML.createElement("div", { "class":`card form`,});
      
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", {   "class":`lbl`,"id":`cs-skills-lbl`,});  }
         { const $$_elggoTgnitteS5C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS5C({ target: __sveltets_2_any(), props: {        "label":`All library skills active`,"hint":`Uncheck to pick the skills this workspace’s agents get.`,"checked":allSkills,"onchange":(v) => toggleAllSkills(v),}});}
        if(!allSkills){
          if(skills.length === 0){
             { svelteHTML.createElement("span", { "class":`hint`,});             }
          }else{
             { svelteHTML.createElement("div", {     "class":`skill-grid`,"role":`group`,"aria-labelledby":`cs-skills-lbl`,});
                 for(let s of __sveltets_2_ensureArray(skills)){s.name;
                 { svelteHTML.createElement("label", {   "class":`skill-row`,"title":s.description ? `${s.name} — ${s.description}` : s.name,});
                   { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":selectedSkills.has(s.name),"onchange":(e) => toggleSkill(s.name, e.currentTarget.checked),});}
                   { svelteHTML.createElement("span", { "class":`skill-name mono`,});s.name; }
                  if(s.description){
                     { svelteHTML.createElement("span", { "class":`skill-desc dim`,});s.description; }
                  }
                 }
              }
             }
          }
           { svelteHTML.createElement("span", { "class":`hint`,});
            selectedSkills.size;  skills.length;           
           }
        }
       }

      
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cs-soul`,});  }
         { svelteHTML.createElement("select", {         "id":`cs-soul`,"class":`input soul`,"value":cfg.soul ?? '',"onchange":(e) => cfg && (cfg.soul = e.currentTarget.value === '' ? null : e.currentTarget.value),});
           { svelteHTML.createElement("option", {"value":"",});  }
             for(let s of __sveltets_2_ensureArray(souls)){s.name;
             { svelteHTML.createElement("option", { "value":s.name,});s.name; }
          }
         }
         { svelteHTML.createElement("span", { "class":`hint`,});
                        
             
         }
       }
       }

       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
       { svelteHTML.createElement("div", { "class":`card form`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cs-goal`,});  }
         { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`cs-goal`,"class":`input`,"rows":2,"bind:value":cfg.goal_md,"maxlength":32000,"placeholder":`Ship the v2 billing API by March`,});/*Ωignore_startΩ*/() => cfg.goal_md = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("span", { "class":`hint`,});      }
       }
      
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cs-extra`,});  }
         { svelteHTML.createElement("textarea", {              "dir":`auto`,"id":`cs-extra`,"class":`input mono`,"rows":6,"bind:value":cfg.extra_context_md,"spellcheck":`false`,"placeholder":`Always run the tests before committing.`,});/*Ωignore_startΩ*/() => cfg.extra_context_md = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("span", { "class":`hint`,});                        }
       }

       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cs-shared-memory`,});  }
         { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`cs-shared-memory`,"class":`input`,"rows":4,"bind:value":cfg.memory_md,"maxlength":32000,"placeholder":`The staging database is read-only.`,});/*Ωignore_startΩ*/() => cfg.memory_md = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("span", { "class":`hint`,});      }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`cs-decisions`,});  }
         { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`cs-decisions`,"class":`input`,"rows":3,"bind:value":cfg.decisions_md,"maxlength":32000,"placeholder":`Use Postgres, not MySQL — the team already runs it.`,});/*Ωignore_startΩ*/() => cfg.decisions_md = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("span", { "class":`hint`,});     }
       }
       { svelteHTML.createElement("div", { "class":`grid2`,});
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`cs-references`,});  }
           { svelteHTML.createElement("textarea", {             "dir":`ltr`,"id":`cs-references`,"class":`input mono`,"rows":3,"bind:value":references,"spellcheck":`false`,"placeholder":`docs/architecture.md`,});/*Ωignore_startΩ*/() => references = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           { svelteHTML.createElement("span", { "class":`hint`,});                    }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`cs-artifacts`,});  }
           { svelteHTML.createElement("textarea", {             "dir":`ltr`,"id":`cs-artifacts`,"class":`input mono`,"rows":3,"bind:value":artifacts,"spellcheck":`false`,"placeholder":`https://example.com/design-doc`,});/*Ωignore_startΩ*/() => artifacts = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           { svelteHTML.createElement("span", { "class":`hint`,});         }
         }
       }
       }

       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
      
       { svelteHTML.createElement("div", { "class":`card toggles`,});
         { const $$_elggoTgnitteS4C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS4C({ target: __sveltets_2_any(), props: {         "label":`Inline the workspace MEMORY.md`,"hint":`Adds the workspace’s MEMORY.md file to the context, not just the curated memory above.`,"checked":cfg.include_memory,"onchange":(v) => { if (cfg) cfg.include_memory = v; },}});}
         { const $$_elggoTgnitteS4C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS4C({ target: __sveltets_2_any(), props: {           "label":`Inject a repo map (tree-sitter)`,"hint":`A map of the repo’s most-referenced symbols (tree-sitter + PageRank), so agents find their way faster.`,"checked":!!cfg.include_repo_map,"testid":`context-repomap`,"onchange":(v) => { if (cfg) cfg.include_repo_map = v; },}});}
       }
     }

    
     { svelteHTML.createElement("h2", { "class":`section-title`,});  }
     { svelteHTML.createElement("div", { "class":`card form`,});
     { svelteHTML.createElement("p", { "class":`card-info dim`,});
                 
                   
     }
     { svelteHTML.createElement("div", { "class":`actions`,});
         for(let p of __sveltets_2_ensureArray(providers)){p;
         { svelteHTML.createElement("button", {         "class":`btn small`,"disabled":!canMaterialize || materializing !== null,"title":canMaterialize ? `Write ${p}'s context files now` : 'Workspace editors and admins can materialize',"onclick":() => materialize(p),});
          materializing === p ? 'Materializing…' : `Materialize ${p}`;
         }
      }
      if(providers.length === 0){
         { svelteHTML.createElement("span", { "class":`dim`,});          }
      }
     }
     }

    
    if(providers.length > 0 && wsId){
       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
       { svelteHTML.createElement("p", { "class":`card-info dim intro-line`,});
                    
                  
           
       }
       { svelteHTML.createElement("div", { "class":`field preview-prov`,});
         { svelteHTML.createElement("label", { "for":`cs-preview-prov`,});  }
         { svelteHTML.createElement("select", {     "id":`cs-preview-prov`,"class":`input`,"bind:value":previewProvider,});/*Ωignore_startΩ*/() => previewProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let p of __sveltets_2_ensureArray(providers)){p;
             { svelteHTML.createElement("option", { "value":p,});p; }
          }
         }
       }
       { svelteHTML.createElement("div", { "class":`preview-box`,});
        if(cfg){
           { const $$_weiverPtxetnoC3C = __sveltets_2_ensureComponent(ContextPreview); new $$_weiverPtxetnoC3C({ target: __sveltets_2_any(), props: {      wsId,"provider":previewProvider,"overrides":buildPreviewOverrides(cfg),}});}
        }
       }
    }
  }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ContextSoul__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ContextSoul__SvelteComponent_ = ReturnType<typeof ContextSoul__SvelteComponent_>;
/*Ωignore_endΩ*/export default ContextSoul__SvelteComponent_;