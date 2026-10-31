///<reference types="svelte" />
;// Otto School — the box's toolbar row (ClassroomsBox): where you are in
// the school as breadcrumbs (3D) or the head count (List), the 3D / List
// switch, and in 3D the reset-view and fill-the-page buttons.

import Icon from '../../../../lib/components/Icon.svelte';
import { plural } from '../../../../lib/plural';
import { home } from '../../home.svelte';
import type { School } from '../../school/model';
import type { SchoolHandle } from '../../school/scene';
;

  interface Props {
    boxId: string;
    mode: '3d' | 'list';
    /** WebGL is available (else List is the only view). */
    gl: boolean;
    zoomed: boolean;
    school: School;
    handle: SchoolHandle | null;
    crumbs: { label: string; go?: () => void }[];
    setMode: (m: '3d' | 'list') => void;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { boxId, mode, gl, zoomed, school, handle, crumbs, setMode }: Props = $props();
;
async () => {

 { svelteHTML.createElement("div", { "class":`bar`,});
  if(mode === '3d'){
     { svelteHTML.createElement("nav", {   "class":`crumbs`,"aria-label":`Where you are in the school`,});
          for(let c of __sveltets_2_ensureArray(crumbs)){let i = 1;i;
        if(i > 0){ { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}}
        if(c.go){
           { svelteHTML.createElement("button", {   "class":`crumb link`,"onclick":c.go,});c.label; }
        }else{
           { svelteHTML.createElement("span", {   "class":`crumb ellipsis`,"aria-current":`location`,});c.label; }
        }
      }
     }
  }else{
     { svelteHTML.createElement("span", { "class":`sum`,});plural(school.rooms.length, 'classroom');  plural(school.total, 'kid'); }
  }
  if(gl){
     { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`School view`,});
       { svelteHTML.createElement("button", {      "class":`seg`,"aria-pressed":mode === '3d',"onclick":() => setMode('3d'),});mode === '3d';  }
       { svelteHTML.createElement("button", {      "class":`seg`,"aria-pressed":mode === 'list',"onclick":() => setMode('list'),});mode === 'list';  }
     }
  }
  if(mode === '3d'){
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => handle?.resetView(),"disabled":!handle,"title":`Reset the school view`,"aria-label":`Reset the school view`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => home.toggleZoom(boxId),"title":zoomed ? 'Exit full page' : 'Fill the page',"aria-label":zoomed ? 'Exit full page' : 'Fill the page',});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":zoomed ? 'minimize' : 'maximize',"size":12,}});}
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SchoolBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SchoolBar__SvelteComponent_ = ReturnType<typeof SchoolBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default SchoolBar__SvelteComponent_;