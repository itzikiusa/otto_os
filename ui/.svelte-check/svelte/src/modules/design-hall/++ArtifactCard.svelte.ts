///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
// One design as a card (Continue strip, project and studio pages): preview,
// title + head version, studio + status, when it last changed, and link chips
// (the stories it implements, how often it is referenced). The whole card is
// a link to the artifact view.
import Icon from '../../lib/components/Icon.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { router } from '../../lib/router.svelte';
import { canPopout, popoutItems } from '../../lib/popoutMenu';
import { rel } from '../../lib/stores/now.svelte';
import type { DesignArtifact } from '../../lib/api/types';
import ArtifactThumb from './ArtifactThumb.svelte';
import StudioBadge from './StudioBadge.svelte';
import StatusPill from './StatusPill.svelte';
;

  interface Props {
    artifact: DesignArtifact;
    /** Story keys this artifact implements ("LOY-142"). */
    stories?: string[];
    /** Incoming references (`DesignSearchHit.reference_count`). */
    referenceCount?: number;
    live?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { artifact, stories = [], referenceCount = 0, live = false }: Props = $props();
  const by = $derived(artifact.created_by_kind === 'agent' ? ' · drafted by Otto' : '');
;
async () => {

  {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('a')));{ svelteHTML.createElement("a", __sveltets_2_union($$action_0), {         "class":`acard`,"href":`#/design/a/${encodeURIComponent(artifact.id)}`,...__sveltets_2_empty({"data-testid":`design-artifact-card`}),"oncontextmenu":canPopout
    ? (e) =>
        ctxMenu.show(e, [
          { label: 'Open', icon: 'chevronRight', action: () => router.go(`design/a/${encodeURIComponent(artifact.id)}`) },
          ...popoutItems(`design/a/${encodeURIComponent(artifact.id)}`, artifact.title),
        ])
    : undefined,});
   { svelteHTML.createElement("div", { "class":`pic`,}); { const $$_bmuhTtcafitrA2C = __sveltets_2_ensureComponent(ArtifactThumb); new $$_bmuhTtcafitrA2C({ target: __sveltets_2_any(), props: {  artifact,live,}});} }
   { svelteHTML.createElement("div", { "class":`body`,});
     { svelteHTML.createElement("div", { "class":`line1`,});
       { svelteHTML.createElement("span", {   "class":`title`,"title":artifact.title,});artifact.title; }
      if(artifact.head_seq != null){ { svelteHTML.createElement("span", { "class":`ver`,}); artifact.head_seq; }}
     }
     { svelteHTML.createElement("div", { "class":`line2`,});
        { const $$_egdaBoidutS3C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS3C({ target: __sveltets_2_any(), props: {  "studio":artifact.studio,"showName":true,}});}
       { const $$_lliPsutatS3C = __sveltets_2_ensureComponent(StatusPill); new $$_lliPsutatS3C({ target: __sveltets_2_any(), props: {  "status":artifact.status,}});}
     }
     { svelteHTML.createElement("div", {   "class":`meta`,"title":new Date(artifact.updated_at).toLocaleString(),});
       rel(artifact.updated_at);by;
     }
    if(stories.length || referenceCount){
       { svelteHTML.createElement("div", { "class":`chips`,});
        if(referenceCount){
           { svelteHTML.createElement("span", { "class":`chip`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});}   referenceCount; }
        }
            for(let key of __sveltets_2_ensureArray((stories.slice(0, 2)))){let i = 1;i;
           { svelteHTML.createElement("span", { "class":`chip mono-chip`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":12,}});} key; }
        }
       }
    }
   }
 }}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ArtifactCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ArtifactCard__SvelteComponent_ = ReturnType<typeof ArtifactCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default ArtifactCard__SvelteComponent_;