///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// A project in the lobby grid: a 2×2 mosaic of its newest artifacts, the
// bound epic, counts, and a one-line review summary. Links to the project.
import Icon from '../../lib/components/Icon.svelte';
import { rel } from '../../lib/stores/now.svelte';
import type { DesignArtifact, DesignProject } from '../../lib/api/types';
import ArtifactThumb from './ArtifactThumb.svelte';
import { projectStats, projectSummary } from './model';
;

  interface Props {
    project: DesignProject;
    artifacts: DesignArtifact[];
    /** "LOY-120 · Loyalty relaunch" when the epic is known. */
    epicLabel?: string | null;
    /** Render the mosaic's designs live when they have no stored thumbnail. */
    live?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { project, artifacts, epicLabel = null, live = false }: Props = $props();
  const stats = $derived(projectStats(project, artifacts));
  const summary = $derived(projectSummary(stats));
;
async () => {

 { svelteHTML.createElement("a", {     "class":`pcard`,"href":`#/design/p/${encodeURIComponent(project.id)}`,...__sveltets_2_empty({"data-testid":`design-project-card`}),});
   { svelteHTML.createElement("div", {   "class":`mosaic`,"aria-hidden":`true`,});
       for(let i of __sveltets_2_ensureArray(([0, 1, 2, 3]))){i;
       { svelteHTML.createElement("div", { "class":`cell`,});
        if(stats.mosaic[i]){  { const $$_bmuhTtcafitrA3C = __sveltets_2_ensureComponent(ArtifactThumb); new $$_bmuhTtcafitrA3C({ target: __sveltets_2_any(), props: {   "artifact":stats.mosaic[i],live,"compact":true,}});}}
       }
    }
   }
   { svelteHTML.createElement("div", { "class":`body`,});
     { svelteHTML.createElement("div", {   "class":`name`,"title":project.name,});project.name; }
    if(epicLabel){
       { svelteHTML.createElement("span", {   "class":`chip epic`,"title":epicLabel,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`epic-t`,});epicLabel; } }
    }
     { svelteHTML.createElement("div", { "class":`meta`,});
      plural(stats.artifacts, 'artifact');  plural(stats.studios.length, 'studio');   rel(stats.updatedAt);
     }
     { svelteHTML.createElement("div", { "class":`summary tone-${summary.tone}`,});summary.text; }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProjectCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProjectCard__SvelteComponent_ = ReturnType<typeof ProjectCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProjectCard__SvelteComponent_;