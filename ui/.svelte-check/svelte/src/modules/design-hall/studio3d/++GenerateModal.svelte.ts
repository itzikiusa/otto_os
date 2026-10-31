///<reference types="svelte" />
;
import { announceShown } from '../../../lib/announce.svelte';
// ✨ Generate → "Text → 3D model" / "Image → 3D model". A provider picker
// (proposal §5.4): the local path runs one Otto agent turn on this scene
// (Blender MCP tool calls ask first); Tripo / Meshy are opt-in cloud
// providers, disabled until a Keychain key exists, each showing where the
// prompt goes and what it costs. Nothing is sent before the person presses
// Generate, and cloud providers never run without a key.
import { untrack } from 'svelte';
import Badge from '../../../lib/components/Badge.svelte';
import Modal from '../../../lib/components/Modal.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { ApiError } from '../../../lib/api/client';
import { createArtifact } from '../../../lib/api/design';
import { startAssist } from '../assist/api';
import type { DesignArtifact, DesignArtifactFormat, DesignAssistTurn } from '../../../lib/api/types';
import {
    GEN3D_PROVIDERS,
    defaultProvider,
    localPrompt,
    unavailableReason,
    type Gen3dContext,
    type Gen3dKind,
    type Gen3dProviderId,
    type Gen3dQuality,
  } from './providers';
;

  interface Props {
    kind: Gen3dKind;
    artifact: DesignArtifact;
    ctx: Gen3dContext;
    onclose: () => void;
    /** The agent turn started (the studio shows it until it lands). */
    onstarted: (turn: DesignAssistTurn, label: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { kind, artifact, ctx, onclose, onstarted }: Props = $props();

  let prompt = $state('');
  let quality = $state<Gen3dQuality>('standard');
  // Preselect once (the picker is the person's choice after that).
  let providerId = $state<Gen3dProviderId>(untrack(() => defaultProvider(kind, ctx).id));
  let image = $state<File | null>(null);
  let imageUrl = $state<string | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  const provider = $derived(GEN3D_PROVIDERS.find((p) => p.id === providerId) ?? GEN3D_PROVIDERS[0]);
  const blocked = $derived(unavailableReason(provider, kind, ctx));
  const ready = $derived(!blocked && !busy && (kind === 'text' ? prompt.trim().length > 2 : !!image));

  $effect(() => () => {
    if (imageUrl) URL.revokeObjectURL(imageUrl);
  });

  function pickImage(e: Event): void {
    const f = (e.currentTarget as HTMLInputElement).files?.[0] ?? null;
    if (f && !/^image\/(png|jpeg|webp)$/.test(f.type)) {
      error = 'Pick a PNG, JPEG or WebP image.';
      return;
    }
    if (f && f.size > 10 * 1024 * 1024) {
      error = 'That image is over 10 MB.';
      return;
    }
    error = null;
    if (imageUrl) URL.revokeObjectURL(imageUrl);
    image = f;
    imageUrl = f ? URL.createObjectURL(f) : null;
  }

  function b64(f: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const fr = new FileReader();
      fr.onerror = () => reject(fr.error);
      fr.onload = () => resolve(String(fr.result).split(',')[1] ?? '');
      fr.readAsDataURL(f);
    });
  }

  async function generate(): Promise<void> {
    if (!ready || provider.where !== 'local') return;
    busy = true;
    error = null;
    try {
      // Image → 3D: the image becomes a reference design in the project so the
      // agent's brief carries it as [R1] (refs/R1.png) — nothing leaves the Mac.
      let refId: string | null = null;
      if (kind === 'image' && image) {
        const fmt = (image.type === 'image/jpeg' ? 'jpeg' : image.type === 'image/webp' ? 'webp' : 'png') as DesignArtifactFormat;
        const res = await createArtifact({
          workspace_id: artifact.workspace_id,
          project_id: artifact.project_id ?? undefined,
          studio: 'graphics',
          format: fmt,
          title: `${image.name.replace(/\.[a-z0-9]+$/i, '') || 'Reference'} (3D reference)`,
          content_b64: await b64(image),
          tags: ['3d-reference'],
          message: `Reference image for ${artifact.title}`,
        });
        refId = res.artifact.id;
      }
      const turn = await startAssist(artifact.id, {
        prompt: localPrompt(kind, prompt, quality, refId ? 'R1' : null),
        mode: 'refine',
        references: refId ? [refId] : undefined,
      });
      onstarted(turn, kind === 'text' ? `Modelling “${prompt.trim().slice(0, 40)}”` : 'Modelling from the image');
      onclose();
    } catch (e) {
      error =
        e instanceof ApiError && e.status === 409
          ? 'Otto is already working on this design — wait for that turn to finish.'
          : e instanceof Error
            ? e.message
            : String(e);
      toasts.error('Couldn’t start the generation', error);
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":kind === 'text' ? 'Text → 3D model' : 'Image → 3D model',"width":560,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":!ready,"onclick":() => void generate(),...__sveltets_2_empty({"data-testid":`gen3d-go`}),});
      busy ? 'Starting…' : 'Generate';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", {   "class":`gen`,...__sveltets_2_empty({"data-testid":`gen3d-modal`}),});
    if(kind === 'text'){
       { svelteHTML.createElement("label", { "class":`gfield`,});
         { svelteHTML.createElement("span", { "class":`lbl`,});   }
         { svelteHTML.createElement("textarea", {           "dir":`auto`,"rows":3,"bind:value":prompt,"placeholder":`A small gift box with a violet ribbon`,"maxlength":1200,...__sveltets_2_empty({"data-testid":`gen3d-prompt`}),});/*Ωignore_startΩ*/() => prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
    }else{
       { svelteHTML.createElement("div", { "class":`gfield`,});
         { svelteHTML.createElement("span", { "class":`lbl`,});  }
         { svelteHTML.createElement("div", { "class":`img-row`,});
          if(imageUrl){
             { svelteHTML.createElement("img", {      "class":`preview`,"src":imageUrl,"alt":`The reference you picked`,});}
          }
           { svelteHTML.createElement("label", { "class":`btn small`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":12,}});} image ? 'Change image…' : 'Choose image…';
             { svelteHTML.createElement("input", {        "class":`file`,"type":`file`,"accept":`image/png,image/jpeg,image/webp`,"onchange":pickImage,});}
           }
         }
         { svelteHTML.createElement("label", { "class":`gfield`,});
           { svelteHTML.createElement("span", { "class":`lbl`,});  }
           { svelteHTML.createElement("input", {          "dir":`auto`,"type":`text`,"bind:value":prompt,"placeholder":`Keep the proportions; use Brand violet`,"maxlength":600,});/*Ωignore_startΩ*/() => prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
       }
    }

     { svelteHTML.createElement("fieldset", { "class":`providers`,});
       { svelteHTML.createElement("legend", { "class":`lbl`,});  }
         for(let p of __sveltets_2_ensureArray(GEN3D_PROVIDERS)){p.id;
        const why = unavailableReason(p, kind, ctx);
         { svelteHTML.createElement("label", {   "class":`prov`,});!!why && p.where === 'cloud';providerId === p.id;
           { svelteHTML.createElement("input", {         "type":`radio`,"name":`gen3d-provider`,"value":p.id,"disabled":p.where === 'cloud' && !!why,});providerId = __sveltets_2_any(null);}
           { svelteHTML.createElement("span", { "class":`prov-main`,});
             { svelteHTML.createElement("span", { "class":`prov-name`,});
              p.label;
              if(p.where === 'cloud'){ { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {  "label":`Opt-in`,}});}}
             }
             { svelteHTML.createElement("span", { "class":`prov-note`,});p.privacy; }
            if(p.cost){ { svelteHTML.createElement("span", { "class":`prov-note`,}); p.cost; }}
            if(why && p.where === 'cloud'){ { svelteHTML.createElement("span", { "class":`prov-why`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});} why; }}
           }
         }
      }
     }

     { svelteHTML.createElement("label", { "class":`gfield inline`,});
       { svelteHTML.createElement("span", { "class":`lbl`,});  }
       { svelteHTML.createElement("select", { "bind:value":quality,});/*Ωignore_startΩ*/() => quality = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`draft`,});    }
         { svelteHTML.createElement("option", { "value":`standard`,});    }
       }
     }

    if(blocked && provider.where === 'local'){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),(blocked)));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`note warn`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}  { svelteHTML.createElement("span", {});blocked; } }}
    }else{
       { svelteHTML.createElement("p", { "class":`note`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}  { svelteHTML.createElement("span", {});       { svelteHTML.createElement("strong", {});  }               } }
    }
    if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`note err`,});error; }}}
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GenerateModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GenerateModal__SvelteComponent_ = ReturnType<typeof GenerateModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default GenerateModal__SvelteComponent_;