///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
// Session name themes: pick the theme new agent sessions are auto-named from
// (e.g. "Ronaldo", "Messi") and manage your own custom name lists (family
// names, …). Per-user; backed by /name-themes.
import { api } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import SectionIntro from './SectionIntro.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { guardUnsaved } from '../../lib/leaveGuard';
import type {
    NameThemesResp,
    NameThemeInfo,
    CustomThemeResp,
  } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let resp = $state<NameThemesResp | null>(null);
  let loading = $state(true);
  // A failed load renders inline with Retry (it used to toast and leave the
  // page blank under the intro).
  let loadError = $state('');
  let saving = $state(false);

  // New custom-theme form.
  let newLabel = $state('');
  let newNames = $state('');
  let creating = $state(false);

  const NONE_ID = 'none';

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      resp = await api.get<NameThemesResp>('/name-themes');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  async function setActive(id: string): Promise<void> {
    if (saving || resp?.active === id) return;
    saving = true;
    const prev = resp?.active;
    if (resp) resp.active = id; // optimistic
    try {
      resp = await api.put<NameThemesResp>('/name-themes/active', { theme_id: id });
    } catch (e) {
      if (resp && prev) resp.active = prev; // revert
      toasts.error("Couldn’t switch the name theme", loadErrorText(e));
    } finally {
      saving = false;
    }
  }

  async function createTheme(): Promise<void> {
    const label = newLabel.trim();
    const names = newNames
      .split('\n')
      .map((n) => n.trim())
      .filter((n) => n.length > 0);
    if (!label || names.length === 0) return;
    creating = true;
    try {
      await api.post<CustomThemeResp>('/name-themes', { label, names });
      newLabel = '';
      newNames = '';
      await load();
      toasts.success('Custom theme created', `${label} · ${names.length} names`);
    } catch (e) {
      toasts.error('Couldn’t create the theme', loadErrorText(e));
    } finally {
      creating = false;
    }
  }

  /** Custom themes whose delete is in flight, by id — that row's button disables. */
  let deleting = $state<Record<string, boolean>>({});

  async function deleteTheme(t: NameThemeInfo): Promise<void> {
    if (deleting[t.id]) return;
    const active = resp?.active === t.id;
    if (
      !(await confirmer.ask(
        `Delete custom theme “${t.label}” and its ${plural(t.capacity, 'name')}?${active ? ' New sessions go back to numbered names.' : ''} Sessions already named from it keep their names.`,
        { title: 'Delete theme?', danger: true, confirmLabel: 'Delete theme' },
      ))
    )
      return;
    deleting = { ...deleting, [t.id]: true };
    try {
      await api.del(`/name-themes/${t.id}`);
      await load();
      toasts.success('Theme deleted', t.label);
    } catch (e) {
      toasts.error('Couldn’t delete the theme', loadErrorText(e));
    } finally {
      const next = { ...deleting };
      delete next[t.id];
      deleting = next;
    }
  }

  const builtins = $derived((resp?.themes ?? []).filter((t) => t.kind === 'builtin'));
  const customs = $derived((resp?.themes ?? []).filter((t) => t.kind === 'custom'));
  const newNameCount = $derived(newNames.split('\n').filter((n) => n.trim()).length);
  const canCreate = $derived(newLabel.trim() !== '' && newNameCount > 0);
  // A half-typed custom theme isn't lost to a stray sidebar click.
  $effect(() => guardUnsaved(() => !creating && (newLabel.trim() !== '' || newNames.trim() !== ''), { what: 'the new custom theme' }));
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('session-names'),"subtitle":`Auto-names for new agent sessions`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});           { svelteHTML.createElement("strong", {});  }    { svelteHTML.createElement("code", {});  }               { svelteHTML.createElement("code", {});    }  SectionIntro}

   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`name themes`,loading,"error":loadError,"empty":!resp,"onretry":() => void load(),"rows":3,}});
  if(resp){
     const themeCard/*Ωignore_positionΩ*/ = (id: string, label: string, sample: string, cap: string, custom: boolean)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
      const on = resp?.active === id;
       { svelteHTML.createElement("button", {            "type":`button`,"class":`theme-card`,"aria-pressed":on,"disabled":saving,"onclick":() => setActive(id),});on;
         { svelteHTML.createElement("span", { "class":`theme-head`,});
           { svelteHTML.createElement("span", {   "class":`theme-label`,"title":label,});label; }
          if(custom){ { svelteHTML.createElement("span", { "class":`chip`,});  }}
          if(on){ { svelteHTML.createElement("span", {   "class":`on-mark`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} }}
         }
         { svelteHTML.createElement("span", {   "class":`theme-sample`,"title":sample,});sample; }
        if(cap){ { svelteHTML.createElement("span", { "class":`theme-cap`,});cap; }}
       }
    };return __sveltets_2_any(0)};

     { svelteHTML.createElement("div", { "class":`section-title`,});  }
     { svelteHTML.createElement("div", {     "class":`theme-grid`,"role":`group`,"aria-label":`Name theme`,});
      ;__sveltets_2_ensureSnippet(themeCard(NONE_ID, 'Numbered', 'claude #1 · codex #2 · shell #3', '', false));
         for(let t of __sveltets_2_ensureArray(builtins)){t.id;
        ;__sveltets_2_ensureSnippet(themeCard(t.id, t.label, t.sample.join(' · '), `${t.capacity.toLocaleString()} names`, false));
      }
         for(let t of __sveltets_2_ensureArray(customs)){t.id;
        ;__sveltets_2_ensureSnippet(themeCard(
          t.id,
          t.label,
          t.sample.join(' · ') || 'No names yet',
          `${plural(t.capacity, 'name')} · then #2, #3…`,
          true,
        ));
      }
     }

     { svelteHTML.createElement("div", { "class":`section-title`,});  }
    if(customs.length > 0){
       { svelteHTML.createElement("ul", { "class":`custom-list card`,});
           for(let t of __sveltets_2_ensureArray(customs)){t.id;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("span", {   "class":`cl-label`,"title":t.label,});t.label; }
             { svelteHTML.createElement("span", {   "class":`cl-names`,"title":t.sample.join(', '),});t.sample.join(', ');t.capacity > t.sample.length ? '…' : ''; }
             { svelteHTML.createElement("button", {             "class":`icon-btn`,"title":deleting[t.id] ? `Deleting ${t.label}…` : `Delete ${t.label}`,"aria-label":deleting[t.id] ? `Deleting ${t.label}…` : `Delete ${t.label}`,"aria-busy":deleting[t.id] ? 'true' : undefined,"disabled":deleting[t.id],"onclick":() => deleteTheme(t),});
              if(deleting[t.id]){ { svelteHTML.createElement("span", {   "class":`spinner`,"aria-hidden":`true`,}); }}else{ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}}
             }
           }
        }
       }
    }

     { svelteHTML.createElement("div", { "class":`card s-card new-form`,});
       { svelteHTML.createElement("div", { "class":`new-title`,});   }
       { svelteHTML.createElement("p", { "class":`new-sub`,});                }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`nt-label`,});  }
         { svelteHTML.createElement("input", {          "dir":`auto`,"id":`nt-label`,"class":`input`,"placeholder":`Family`,"bind:value":newLabel,});/*Ωignore_startΩ*/() => newLabel = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`nt-names`,});  { svelteHTML.createElement("span", { "class":`dim`,});     } }
         { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`nt-names`,"class":`input names-area`,"rows":5,"placeholder":'Dad\nMom\nSister\nBrother',"bind:value":newNames,});/*Ωignore_startΩ*/() => newNames = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("span", { "class":`hint`,});plural(newNameCount, 'name'); }
       }
       { svelteHTML.createElement("div", { "class":`form-actions`,});
         { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":creating || !canCreate,"title":canCreate ? undefined : 'Enter a theme name and at least one name',"onclick":createTheme,});
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});} creating ? 'Creating…' : 'Create theme';
         }
       }
     }
  }
   LoadState}
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SessionNames__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SessionNames__SvelteComponent_ = ReturnType<typeof SessionNames__SvelteComponent_>;
/*Ωignore_endΩ*/export default SessionNames__SvelteComponent_;