///<reference types="svelte" />
;
import PathField from '../../lib/components/PathField.svelte';
import { toastError } from '../../lib/toastError';
// Inject tab — build/preview the inject bundle for the selected story, copy
// markdown to clipboard, and open an agent session seeded with the bundle.
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { product } from '../../lib/stores/product.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { renderMarkdown } from '../../lib/md';
import { toasts } from '../../lib/toast.svelte';
import type { InjectBundle } from './types';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import { copyText } from '../../lib/clipboard';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  // The rewrite/tests/inject run spawns an agent CLI session via the live
  // registry, so the provider must be a real registered agent (built-in or
  // custom like grok) — openai/gemini were never valid here.
  const PROVIDERS = $derived(agentProviders());

  let bundle = $state<InjectBundle | null>(null);
  let loading = $state(false);
  let copying = $state(false);
  let launching = $state(false);
  let provider = $state<string>(defaultAgentProvider());
  let cwd = $state('');

  // Collapsible state per section index.
  let collapsed = $state<Record<number, boolean>>({});

  const story = $derived(product.detail?.story ?? null);

  // Reset when story changes.
  $effect(() => {
    product.selectedId;
    bundle = null;
    collapsed = {};
    cwd = story?.cwd ?? '';
  });

  const renderedMarkdown = $derived(bundle ? renderMarkdown(bundle.markdown) : '');

  async function buildPreview(): Promise<void> {
    if (loading) return;
    loading = true;
    try {
      bundle = await product.loadInject();
    } catch (e) {
      toastError('Couldn’t build inject bundle', e);
    } finally {
      loading = false;
    }
  }

  async function copyMarkdown(): Promise<void> {
    if (!bundle || copying) return;
    copying = true;
    try {
      // A blocked clipboard says what to do next, not the cause again.
      if (await copyText(bundle.markdown)) toasts.success('Copied to clipboard', 'Inject bundle markdown copied.');
      else toasts.error('Couldn’t copy the bundle', 'Select the text and copy it manually.');
    } finally {
      copying = false;
    }
  }

  async function openInAgent(): Promise<void> {
    if (launching) return;
    launching = true;
    try {
      const session = await product.injectSession({
        provider: provider || undefined,
        cwd: cwd.trim() || undefined,
      });
      toasts.success('Agent session started', session.title ? `“${session.title}” is open in Agents.` : 'It is open in Agents.');
      // "Open in agent" means open it: land on the new session, not a toast with its id.
      ws.navigateToSession(session.id);
    } catch (e) {
      toastError('Couldn’t create agent session', e);
    } finally {
      launching = false;
    }
  }

  function toggleSection(idx: number): void {
    collapsed = { ...collapsed, [idx]: !collapsed[idx] };
  }
;
async () => {

if(!story){
   { svelteHTML.createElement("div", { "class":`muted`,});   }
}else{
   { svelteHTML.createElement("div", { "class":`inject-tab`,});

    
     { svelteHTML.createElement("div", { "class":`action-bar card`,});
       { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":buildPreview,"disabled":loading,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":13,}});}
        loading ? 'Building…' : bundle ? 'Rebuild' : 'Build / Preview';
       }

      if(bundle){
         { svelteHTML.createElement("button", {         "class":`btn`,"onclick":copyMarkdown,"disabled":copying,"title":`Copy inject bundle markdown to clipboard`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":13,}});}
          copying ? 'Copying…' : 'Copy';
         }
      }

       { svelteHTML.createElement("span", { "class":`divider`,}); }

      
       { svelteHTML.createElement("div", { "class":`agent-row`,});
         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`inject-provider`,});  }
         { svelteHTML.createElement("select", {       "id":`inject-provider`,"class":`mini-select`,"bind:value":provider,"disabled":launching,});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let p of __sveltets_2_ensureArray(PROVIDERS)){p;
             { svelteHTML.createElement("option", { "value":p,});p; }
          }
         }

         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`inject-cwd`,});  }
         { const $$_dleiFhtaP3C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP3 = new $$_dleiFhtaP3C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },value:cwd,"disabled":launching,}});/*Ωignore_startΩ*/() => cwd = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP3.$$bindings = 'value'; { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`inject-cwd`,"class":`cwd-input`,"type":`text`,"placeholder":`optional working dir`,"bind:value":cwd,"disabled":launching,});/*Ωignore_startΩ*/() => cwd = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}

         { svelteHTML.createElement("button", {         "class":`btn`,"onclick":openInAgent,"disabled":launching,"title":`Create an agent session seeded with this inject bundle`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":13,}});}
          launching ? 'Creating…' : 'Open in agent';
         }
       }
     }

    
    if(bundle){
      
      if(bundle.sections && bundle.sections.length > 0){
         { svelteHTML.createElement("div", { "class":`sections-wrap`,});
           { svelteHTML.createElement("div", { "class":`section-head-row`,});
             { svelteHTML.createElement("span", { "class":`section-label`,}); bundle.sections.length;  }
           }
              for(let sec of __sveltets_2_ensureArray(bundle.sections)){let idx = 1;idx;
             { svelteHTML.createElement("div", { "class":`section-block`,});
               { svelteHTML.createElement("button", {   "class":`sec-trigger`,"onclick":() => toggleSection(idx),});
                 { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":collapsed[idx] ? 'chevronRight' : 'chevronDown',"size":12,}});} }
                 { svelteHTML.createElement("span", { "class":`sec-heading`,});sec.heading; }
               }
              if(!collapsed[idx]){
                 { svelteHTML.createElement("div", { "class":`sec-body md-body`,}); renderMarkdown(sec.body); }
              }
             }
          }
         }
      }

      
       { svelteHTML.createElement("div", { "class":`preview-card card`,});
         { svelteHTML.createElement("div", { "class":`preview-header`,});
           { svelteHTML.createElement("span", { "class":`section-label`,});  }
           { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":copyMarkdown,"disabled":copying,"title":`Copy to clipboard`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}
            
           }
         }
        if(renderedMarkdown){
           { svelteHTML.createElement("div", { "class":`md-body`,}); renderedMarkdown; }
        }else{
           { svelteHTML.createElement("div", { "class":`muted`,});  }
        }
       }
    } else if (!loading){
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {       "icon":`zap`,"title":`No bundle built yet`,"body":`Build / Preview assembles all of this story’s context into one bundle, so an agent can start coding immediately.`,}});}
    }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const InjectTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type InjectTab__SvelteComponent_ = ReturnType<typeof InjectTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default InjectTab__SvelteComponent_;