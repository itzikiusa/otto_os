///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// History tab — sectioned event timeline with section filter for the selected story.
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import { product } from '../../lib/stores/product.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { ProductEvent } from './types';
function $$render() {

  
  
  
  
  
  
  
  

  type SectionFilter =
    | 'all'
    | 'source'
    | 'analysis'
    | 'questions'
    | 'notes'
    | 'rewrite'
    | 'tests'
    | 'publish'
    | 'inject'
    | 'watch';

  const SECTIONS: { id: SectionFilter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'source', label: 'Source' },
    { id: 'analysis', label: 'Analysis' },
    { id: 'questions', label: 'Questions' },
    { id: 'notes', label: 'Notes' },
    { id: 'rewrite', label: 'Rewrite' },
    { id: 'tests', label: 'Tests' },
    { id: 'publish', label: 'Publish' },
    { id: 'inject', label: 'Inject' },
    { id: 'watch', label: 'Watch' },
  ];

  let selectedSection = $state<SectionFilter>('all');
  let loaded = $state(false);

  const story = $derived(product.detail?.story ?? null);

  // Reset and reload when story changes.
  $effect(() => {
    product.selectedId;
    loaded = false;
    loadError = null;
    void doLoad('all');
    selectedSection = 'all';
  });

  /** A failed load — inline with Retry, never as "No events yet". */
  let loadError = $state<string | null>(null);
  async function doLoad(section: SectionFilter): Promise<void> {
    try {
      await product.loadEvents(section === 'all' ? undefined : section);
      loaded = true;
      loadError = null;
    } catch (e) {
      loadError = loadErrorText(e);
    }
  }

  /** Section id → its label ("tests" → "Tests"). */
  function sectionLabel(id: string): string {
    return SECTIONS.find((x) => x.id === id)?.label ?? id;
  }
  /** Event kind enum → words ("question_posted" → "Question posted"). */
  function kindLabel(kind: string): string {
    const w = kind.replace(/[_-]+/g, ' ').trim();
    return w ? w[0].toUpperCase() + w.slice(1) : kind;
  }

  async function onSectionChange(e: Event): Promise<void> {
    const val = (e.target as HTMLSelectElement).value as SectionFilter;
    selectedSection = val;
    await doLoad(val);
  }

  // Newest first (already from API but sort defensively)
  const events = $derived(
    [...product.events].sort(
      (a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
    ),
  );

  // Sections are kinds, not states: every section reads as a neutral chip with
  // its kind icon (components.md §4 — no per-source hues).
  function sectionIcon(section: string): IconName {
    switch (section) {
      case 'source': return 'file';
      case 'analysis': return 'gauge';
      case 'questions': return 'comment';
      case 'notes': return 'note';
      case 'rewrite': return 'edit';
      case 'tests': return 'check';
      case 'publish': return 'send';
      case 'inject': return 'zap';
      case 'watch': return 'eye';
      default: return 'dot';
    }
  }

;
async () => {

if(!story){
   { svelteHTML.createElement("div", { "class":`muted`,});   }
}else{
   { svelteHTML.createElement("div", { "class":`history-tab`,});
    
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":13,}});}
       { svelteHTML.createElement("span", { "class":`label`,});  }
       { svelteHTML.createElement("select", {         "class":`input htab-select`,"aria-label":`Section`,"value":selectedSection,"onchange":onSectionChange,"disabled":product.loadingEvents,});
           for(let s of __sveltets_2_ensureArray(SECTIONS)){s.id;
           { svelteHTML.createElement("option", { "value":s.id,});s.label; }
        }
       }
      if(product.loadingEvents){
         { svelteHTML.createElement("span", {       "class":`spinner`,"role":`img`,"aria-label":`Loading events`,"title":`Loading events`,}); }
      }else{
         { svelteHTML.createElement("span", { "class":`dim`,});plural(events.length, 'event'); }
      }
     }

    
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`history`,"loading":product.loadingEvents && !loaded,"error":loadError,"empty":!loaded || events.length === 0,"onretry":() => void doLoad(selectedSection),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("div", { "class":`muted`,});  selectedSection !== 'all' ? ` in ${sectionLabel(selectedSection)}` : '';  }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
         { svelteHTML.createElement("div", { "class":`timeline`,});
             for(let ev of __sveltets_2_ensureArray(events)){ev.id;
             { svelteHTML.createElement("div", { "class":`event-row`,});
              
               { svelteHTML.createElement("div", { "class":`ev-line-col`,});
                 { svelteHTML.createElement("div", { "class":`ev-dot`,}); }
                 { svelteHTML.createElement("div", { "class":`ev-connector`,}); }
               }

              
               { svelteHTML.createElement("div", { "class":`ev-content`,});
                 { svelteHTML.createElement("div", { "class":`ev-header`,});
                   { svelteHTML.createElement("span", { "class":`chip hist-sec-chip`,});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":sectionIcon(ev.section),"size":12,}});}
                    sectionLabel(ev.section);
                   }
                   { svelteHTML.createElement("span", {   "class":`ev-kind`,"title":ev.actor_id ? `${ev.kind} · by ${ev.actor_id}` : ev.kind,});kindLabel(ev.kind); }
                   { svelteHTML.createElement("span", { "class":`spacer`,}); }
                   { const $$_emiTleR6C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR6C({ target: __sveltets_2_any(), props: {    "iso":ev.created_at,"class":`ev-time`,}});}
                 }
                 { svelteHTML.createElement("p", { "class":`ev-summary`,});ev.summary; }
               }
             }
          }
         }
    
     LoadState}
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HistoryTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HistoryTab__SvelteComponent_ = ReturnType<typeof HistoryTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default HistoryTab__SvelteComponent_;