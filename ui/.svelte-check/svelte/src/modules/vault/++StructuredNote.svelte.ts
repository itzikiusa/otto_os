///<reference types="svelte" />
;// Structured reading view for typed OKF notes: metadata header (type,
// owners, status, tags, resource), key-field cards (endpoints, environments,
// dependencies, data stores… from frontmatter or the matching body
// section), the kind's required-content checklist, outgoing links +
// backlinks with hover previews, and LIVE CONTEXT from the rest of Otto.
// Sits above the unchanged markdown body; plain notes never mount it.

import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { sentenceCase } from '../../lib/labels';
import type { VaultNote } from '../../lib/api/types';
import { vault } from './vault.svelte';
import LiveContext from './LiveContext.svelte';
import { linkPreview } from './previewStore.svelte';
import type { StructuredModel } from './structuredNote';

;type $$ComponentProps =  { model: StructuredModel; note: VaultNote };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { model, note }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const outgoing = $derived.by(() => {
    const seen = new Set<string>();
    return note.outgoing.filter((o) => {
      const key = o.dst_path ?? `?${o.raw_target}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  });
  const missing = $derived(model.sections.filter((s) => !s.present).length);
  let showSections = $state(false);

  const showPreview = (e: Event, path: string | null | undefined): void => linkPreview.show(e.currentTarget as Element, path);
  const hidePreview = (): void => linkPreview.hide();

  function open(path: string | null | undefined): void {
    hidePreview();
    if (path && /\.md$/i.test(path)) void vault.open(path);
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`structured`,...__sveltets_2_empty({"data-testid":`vault-structured`}),});
   { svelteHTML.createElement("div", { "class":`meta-head`,});
     { svelteHTML.createElement("span", { "class":`kind`,});model.label; }
    if(model.status){ { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {  "label":sentenceCase(model.status),}});}}
    if(model.operation){
       { svelteHTML.createElement("code", { "class":`op`,}); { svelteHTML.createElement("span", { "class":`method m-${model.operation.method.toLowerCase()}`,});model.operation.method; } model.operation.path; }
    }
   }
   { svelteHTML.createElement("dl", { "class":`facts`,});
    if(model.owners.length){ { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});model.owners.join(', '); } }}
    if(model.resource && !model.operation){ { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {   "class":`mono`,"title":model.resource,});model.resource; } }}
    if(model.tags.length){
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`tags`,});    for(let t of __sveltets_2_ensureArray(model.tags)){let i = 1;i; { svelteHTML.createElement("button", {     "class":`chip tag`,"onclick":() => vault.searchTag(t),"title":`Search #${t}`,}); t; }} } }
    }
   }

  if(model.cards.length){
     { svelteHTML.createElement("div", { "class":`cards`,});
         for(let c of __sveltets_2_ensureArray(model.cards)){c.key;
         { svelteHTML.createElement("div", { "class":`sn-card`,});
           { svelteHTML.createElement("h4", {});c.title;  { svelteHTML.createElement("span", { "class":`n`,});c.items.length; } }
           { svelteHTML.createElement("ul", {});
                for(let it of __sveltets_2_ensureArray(c.items)){let i = 1;i;
               { svelteHTML.createElement("li", {});
                if(it.path){
                   { svelteHTML.createElement("button", {           "class":`lnk`,"onclick":() => open(it.path),"onmouseenter":(e) => showPreview(e, it.path),"onmouseleave":hidePreview,"onfocus":(e) => showPreview(e, it.path),"onblur":hidePreview,});it.value; }
                }else{
                   { svelteHTML.createElement("span", {});it.value; }
                }
               }
            }
           }
         }
      }
     }
  }

   { svelteHTML.createElement("div", { "class":`checklist`,});
     { svelteHTML.createElement("button", {     "class":`toggle`,"aria-expanded":showSections,"onclick":() => (showSections = !showSections),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":missing ? 'warning' : 'check',"size":12,}});}
      missing ? `${missing} of ${model.sections.length} expected sections missing` : `All ${model.sections.length} expected sections present`;
     }
    if(showSections){
       { svelteHTML.createElement("ul", { "class":`secs`,});
           for(let s of __sveltets_2_ensureArray(model.sections)){s.label; { svelteHTML.createElement("li", { });!s.present; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":s.present ? 'check' : 'x',"size":12,}});}s.label; }}
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`links`,});
     { svelteHTML.createElement("div", {   "class":`col`,...__sveltets_2_empty({"data-testid":`vault-structured-outgoing`}),});
       { svelteHTML.createElement("h4", {});   { svelteHTML.createElement("span", { "class":`n`,});outgoing.length; } }
      if(outgoing.length === 0){ { svelteHTML.createElement("p", { "class":`dim`,});   }}
       { svelteHTML.createElement("div", { "class":`chips`,});
           for(let o of __sveltets_2_ensureArray((outgoing.slice(0, 40)))){o.dst_path ?? `?${o.raw_target}`;
           { svelteHTML.createElement("button", {                  "class":`chip lnkchip`,"disabled":!o.dst_path || !/\.md$/i.test(o.dst_path),"title":o.dst_path ?? `${o.raw_target} (unresolved)`,"onclick":() => open(o.dst_path),"onmouseenter":(e) => showPreview(e, o.dst_path),"onmouseleave":hidePreview,"onfocus":(e) => showPreview(e, o.dst_path),"onblur":hidePreview,});!o.dst_path;o.alias ?? o.raw_target.replace(/\.md$/i, '').split('/').pop(); }
        }
        if(outgoing.length > 40){ { svelteHTML.createElement("span", { "class":`dim`,}); outgoing.length - 40;      }}
       }
     }
     { svelteHTML.createElement("div", {   "class":`col`,...__sveltets_2_empty({"data-testid":`vault-structured-backlinks`}),});
       { svelteHTML.createElement("h4", {});   { svelteHTML.createElement("span", { "class":`n`,});vault.backlinks.length; } }
      
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL3 = new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`backlinks`,"variant":`compact`,"loading":vault.backlinksLoading,"error":vault.backlinksError || null,"empty":vault.backlinks.length === 0,"onretry":() => void vault.reloadBacklinks(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", { "class":`dim`,});   }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL3.$$prop_def;/*Ωignore_endΩ*/
        
       { svelteHTML.createElement("div", { "class":`chips`,});
           for(let b of __sveltets_2_ensureArray((vault.backlinks.slice(0, 40)))){b.path + b.kind;
           { svelteHTML.createElement("button", {               "class":`chip lnkchip`,"title":b.path,"onclick":() => open(b.path),"onmouseenter":(e) => showPreview(e, b.path),"onmouseleave":hidePreview,"onfocus":(e) => showPreview(e, b.path),"onblur":hidePreview,});b.title; }
        }
        if(vault.backlinks.length > 40){ { svelteHTML.createElement("span", { "class":`dim`,}); vault.backlinks.length - 40;      }}
       }
       LoadState}
     }
   }

   { const $$_txetnoCeviL1C = __sveltets_2_ensureComponent(LiveContext); new $$_txetnoCeviL1C({ target: __sveltets_2_any(), props: {    "hints":model.hints,"wsId":vault.wsId,}});}
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StructuredNote__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StructuredNote__SvelteComponent_ = ReturnType<typeof StructuredNote__SvelteComponent_>;
/*Ωignore_endΩ*/export default StructuredNote__SvelteComponent_;