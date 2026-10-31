///<reference types="svelte" />
;// Brand Kit → Used in: every design with a `uses_tokens` link to this kit
// (from `POST …/brand/impact` without content), grouped by studio, with how
// each one follows the kit (approved / latest / pinned) and which tokens it
// names. Opening a row opens that design.

import type { BrandConsumer, BrandImpactResp } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import StudioBadge from '../StudioBadge.svelte';
import StatusPill from '../StatusPill.svelte';
import { openArtifact } from '../nav';
import { studioInfo } from '../model';
;

  interface Props {
    usage: BrandImpactResp | null;
    loading: boolean;
    error: string | null;
    kitId: string;
    onretry: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { usage, loading, error, kitId, onretry }: Props = $props();

  function follows(c: BrandConsumer): string {
    if (c.policy === 'pinned') return 'pinned — stays until updated';
    if (c.policy === 'follow_latest') return 'follows every save';
    return 'follows the approved kit';
  }
  function tokens(c: BrandConsumer): string {
    if (c.whole_kit) return c.scanned ? 'the whole kit' : 'the whole kit (not scanned)';
    return c.tokens.length === 1 ? c.tokens[0] : `${c.tokens.length} tokens`;
  }
;
async () => {

 { svelteHTML.createElement("section", {     "id":`brand-used`,"class":`bsec`,"aria-labelledby":`brand-used-h`,});
   { svelteHTML.createElement("div", { "class":`sec-h`,});
     { svelteHTML.createElement("h2", { "id":`brand-used-h`,});  }
     { svelteHTML.createElement("span", { "class":`meta`,});     }
   }

   { svelteHTML.createElement("div", {   "class":`card`,...__sveltets_2_empty({"data-testid":`brand-used-in`}),});
    if((loading || error) && !usage){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {           "what":`where this kit is used`,"variant":`compact`,"rows":2,loading,error,"empty":true,onretry,}});}
    } else if (usage && usage.artifact_count === 0){
       { svelteHTML.createElement("p", { "class":`empty`,});
                          { svelteHTML.createElement("span", { "class":`mono`,});  }   
         { svelteHTML.createElement("span", { "class":`mono`,}); kitId; }    { svelteHTML.createElement("em", {});  }          
         { svelteHTML.createElement("span", { "class":`mono`,});  }
       }
    } else if (usage){
       { svelteHTML.createElement("div", { "class":`sum`,});
         { svelteHTML.createElement("span", { "class":`badges`,});   for(let s of __sveltets_2_ensureArray(usage.by_studio)){s.studio; { const $$_egdaBoidutS4C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS4C({ target: __sveltets_2_any(), props: {  "studio":s.studio,}});}} }
         { svelteHTML.createElement("b", {});usage.artifact_count; usage.artifact_count === 1 ? 'design' : 'designs'; }
         { svelteHTML.createElement("span", { "class":`dim`,}); usage.studio_count; usage.studio_count === 1 ? 'studio' : 'studios';usage.hidden_count ? ` · ${usage.hidden_count} more in workspaces you can’t see` : ''; }
       }
       { svelteHTML.createElement("ul", { "class":`list`,});
           for(let c of __sveltets_2_ensureArray(usage.consumers)){c.artifact.id;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {       "class":`row`,"onclick":() => openArtifact(c.artifact.id),...__sveltets_2_empty({"data-testid":`brand-used-row`}),"title":`Open ${c.artifact.title}`,});
               { const $$_egdaBoidutS5C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS5C({ target: __sveltets_2_any(), props: {  "studio":c.artifact.studio,}});}
               { svelteHTML.createElement("span", { "class":`t`,});c.artifact.title; }
               { svelteHTML.createElement("span", { "class":`dim studio`,});studioInfo(c.artifact.studio).name; }
               { const $$_lliPsutatS5C = __sveltets_2_ensureComponent(StatusPill); new $$_lliPsutatS5C({ target: __sveltets_2_any(), props: {  "status":c.artifact.status,}});}
               { svelteHTML.createElement("span", { "class":`chip`,});follows(c); }
               { svelteHTML.createElement("span", { "class":`mono dim tok`,});tokens(c); }
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":14,}});}
             }
           }
        }
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const UsedIn__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type UsedIn__SvelteComponent_ = ReturnType<typeof UsedIn__SvelteComponent_>;
/*Ωignore_endΩ*/export default UsedIn__SvelteComponent_;