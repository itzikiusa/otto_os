///<reference types="svelte" />
;// One transcript image: authed fetch → blob URL (revoked on unmount), click
// → lightbox. On-disk history transcripts have no session to serve images
// from, so they get a labelled placeholder instead.

import { getContext } from 'svelte';
import Icon from '../../../lib/components/Icon.svelte';
import Lightbox from './Lightbox.svelte';
import { fetchImageUrl } from './api';
import type { ConvContext } from './context';
;

  interface Props {
    id: string;
    alt: string | null;
    mediaType?: string | null;
    /** Thumbnail height for images inside tool results. */
    small?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { id, alt, mediaType = null, small = false }: Props = $props();

  const ctx = getContext<ConvContext>('conv');

  let url = $state<string | null>(null);
  let failed = $state(false);
  let open = $state(false);

  $effect(() => {
    const sid = ctx.sessionId;
    if (!sid) return;
    const load = ctx.imageUrl ?? ((img: string) => fetchImageUrl(sid, img));
    let alive = true;
    let got: string | null = null;
    url = null;
    failed = false;
    load(id)
      .then((u) => {
        if (!alive) {
          URL.revokeObjectURL(u);
          return;
        }
        got = u;
        url = u;
      })
      .catch(() => {
        if (alive) failed = true;
      });
    return () => {
      alive = false;
      if (got) URL.revokeObjectURL(got);
    };
  });
;
async () => {

if(!ctx.sessionId || failed){
   { svelteHTML.createElement("span", {   "class":`img-ph`,"title":mediaType ?? 'image',});small;
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":12,}});} alt ?? 'Image';failed ? ' (unavailable)' : ' (not served for on-disk transcripts)';
   }
} else if (url){
   { svelteHTML.createElement("button", {     "class":`img-btn`,"onclick":() => (open = true),"title":`Open image`,});small;
     { svelteHTML.createElement("img", {      "src":url,"alt":alt ?? 'Image',"loading":`lazy`,});}
   }
  if(open){
     { const $$_xobthgiL0C = __sveltets_2_ensureComponent(Lightbox); new $$_xobthgiL0C({ target: __sveltets_2_any(), props: {      "src":url,"alt":alt ?? 'Image',"onclose":() => (open = false),}});}
  }
}else{
   { svelteHTML.createElement("span", {  "class":`img-ph loading`,});small; { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":12,}});}  }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ImageBlock__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ImageBlock__SvelteComponent_ = ReturnType<typeof ImageBlock__SvelteComponent_>;
/*Ωignore_endΩ*/export default ImageBlock__SvelteComponent_;