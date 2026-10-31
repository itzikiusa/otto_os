///<reference types="svelte" />
;
import { toastError } from '../../../lib/toastError';
// Brand Kit → Logos: one tile per logo (full / mark / mono). A logo's asset is
// an image artifact in the graph (`otto://design/<id>` — the kit `embeds`
// it); uploading a file imports it as a Graphics artifact in the kit's
// project. A slot with no asset shows a generated placeholder in the kit's
// colours so the kit is usable before the real files arrive.
import { untrack } from 'svelte';
import type { BrandDoc, BrandLogoKind } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { fetchContent } from '../../../lib/api/design';
import { importDesignFile } from '../create';
import { openArtifact } from '../nav';
import { parseOttoUri } from '../model';
import { LOGO_KINDS, brandPalette } from './tokens';
;

  interface Props {
    doc: BrandDoc;
    readonly?: boolean;
    workspaceId: string;
    projectId?: string | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { doc = $bindable(), readonly = false, workspaceId, projectId = null }: Props = $props()/*Ωignore_startΩ*/;doc;/*Ωignore_endΩ*/;

  const KIND_LABEL: Record<BrandLogoKind, string> = { full: 'Full', mark: 'Mark', mono: 'Mono' };
  const pal = $derived(brandPalette(doc));
  const initial = $derived((doc.name || 'B').trim().charAt(0).toUpperCase() || 'B');
  const word = $derived((doc.name || 'Brand').replace(/\s*brand kit\s*$/i, '').trim().slice(0, 18) || 'Brand');

  // Object URLs of uploaded logo images, by artifact id.
  let urls = $state<Record<string, string>>({});
  const wanted = $derived(
    (doc.logos ?? []).map((l) => (l.asset ? parseOttoUri(l.asset)?.artifactId : null)).filter((x): x is string => !!x),
  );
  $effect(() => {
    const ids = wanted;
    untrack(() => {
      for (const id of ids) {
        if (urls[id]) continue;
        urls[id] = '';
        fetchContent(id, { asText: false })
          .then((c) => {
            if (c.blobUrl) urls[id] = c.blobUrl;
          })
          .catch(() => {
            /* a missing / unreadable asset keeps the placeholder */
          });
      }
    });
  });
  $effect(() => () => {
    for (const u of Object.values(urls)) if (u) URL.revokeObjectURL(u);
  });

  let fileInput = $state<HTMLInputElement | null>(null);
  let uploadIndex = $state(-1);
  let uploading = $state(false);

  function pick(i: number): void {
    uploadIndex = i;
    fileInput?.click();
  }

  async function onFile(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    const i = uploadIndex;
    if (!file || i < 0 || !doc.logos[i]) return;
    uploading = true;
    try {
      const id = await importDesignFile(file, { workspaceId, projectId });
      doc.logos[i].asset = `otto://design/${id}`;
      toasts.success('Logo added', `${file.name} is now a design in this workspace. Save the kit to keep it.`);
    } catch (err) {
      toastError('Couldn’t add the logo', err);
    } finally {
      uploading = false;
    }
  }

  function add(): void {
    const used = new Set((doc.logos ?? []).map((l) => l.kind));
    const kind = LOGO_KINDS.find((k) => !used.has(k)) ?? 'full';
    doc.logos = [...(doc.logos ?? []), { name: `${KIND_LABEL[kind]} logo`, kind, asset: '' }];
  }

  function remove(i: number): void {
    doc.logos = doc.logos.filter((_, j) => j !== i);
  }
;
async () => {

 { svelteHTML.createElement("section", {     "id":`brand-logos`,"class":`bsec`,"aria-labelledby":`brand-logos-h`,});
   { svelteHTML.createElement("div", { "class":`sec-h`,});
     { svelteHTML.createElement("h2", { "id":`brand-logos-h`,});  }
     { svelteHTML.createElement("span", { "class":`meta`,});            }
   }

   { svelteHTML.createElement("div", {   "class":`logos`,...__sveltets_2_empty({"data-testid":`brand-logos`}),});
        for(let logo of __sveltets_2_ensureArray(doc.logos ?? [])){let i = 1;i;
      const aid = logo.asset ? parseOttoUri(logo.asset)?.artifactId : null;
       { svelteHTML.createElement("figure", {  "class":`ltile`,});logo.kind === 'mono';
         { svelteHTML.createElement("div", {  "class":`lt-in`,});__sveltets_2_ensureType(String, Number, logo.kind === 'mono' ? undefined : pal.surface);
          if(aid && urls[aid]){
             { svelteHTML.createElement("img", {    "src":urls[aid],"alt":logo.name,});}
          }else{
             { svelteHTML.createElement("svg", {      "viewBox":logo.kind === 'mark' ? '0 0 40 40' : '0 0 200 40',"role":`img`,"aria-label":`${logo.name} (placeholder)`,});logo.kind === 'mark';
               { svelteHTML.createElement("rect", {            "x":`0`,"y":`0`,"width":`40`,"height":`40`,"rx":`11`,"fill":logo.kind === 'mono' ? 'currentColor' : pal.primary,});}
               { svelteHTML.createElement("text", {           "x":`20`,"y":`27`,"text-anchor":`middle`,"font-size":`20`,"font-weight":`800`,"fill":logo.kind === 'mono' ? 'var(--surface)' : pal.accent,});initial; }
              if(logo.kind !== 'mark'){
                 { svelteHTML.createElement("text", {         "x":`52`,"y":`28`,"font-size":`22`,"font-weight":`700`,"fill":logo.kind === 'mono' ? 'currentColor' : pal.ink,});word; }
              }
             }
          }
         }
         { svelteHTML.createElement("figcaption", {});
           { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input name`,"value":logo.name,"oninput":(e) => (logo.name = e.currentTarget.value),"disabled":readonly,"aria-label":`Logo name`,});}
           { svelteHTML.createElement("select", {         "class":`input kind`,"value":logo.kind,"onchange":(e) => (logo.kind = e.currentTarget.value as BrandLogoKind),"disabled":readonly,"aria-label":`Logo variant`,});
               for(let k of __sveltets_2_ensureArray(LOGO_KINDS)){k; { svelteHTML.createElement("option", { "value":k,});KIND_LABEL[k]; }}
           }
           { svelteHTML.createElement("div", { "class":`acts`,});
            if(!aid){ { svelteHTML.createElement("span", { "class":`dim`,});  }}
             { svelteHTML.createElement("span", { "class":`grow`,}); }
            if(aid){
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Open the ${logo.name} design`,"title":`Open design`,"onclick":() => openArtifact(aid),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":14,}});} }
            }
            if(!readonly){
               { svelteHTML.createElement("button", {         "class":`icon-btn`,"aria-label":`${aid ? 'Replace' : 'Upload'} ${logo.name}`,"title":aid ? 'Replace file' : 'Upload file',"onclick":() => pick(i),"disabled":uploading,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":14,}});} }
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Remove ${logo.name}`,"title":`Remove`,"onclick":() => remove(i),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});} }
            }
           }
         }
       }
    }
   }
  if(!readonly && (doc.logos?.length ?? 0) < 16){
     { svelteHTML.createElement("button", {   "class":`btn small addbtn`,"onclick":add,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
  }
   { const $$_input1 = svelteHTML.createElement("input", {              "class":`file`,"type":`file`,"accept":`.svg,.png,.jpg,.jpeg,.webp`,"onchange":onFile,"tabindex":-1,"aria-hidden":`true`,});fileInput = $$_input1;}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('doc'), slots: {}, events: {} }}
const LogosSection__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LogosSection__SvelteComponent_ = ReturnType<typeof LogosSection__SvelteComponent_>;
/*Ωignore_endΩ*/export default LogosSection__SvelteComponent_;