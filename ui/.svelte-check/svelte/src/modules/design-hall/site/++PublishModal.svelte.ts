///<reference types="svelte" />
;
import { announceShown } from '../../../lib/announce.svelte';
import { copyTextOrThrow } from '../../../lib/clipboard';
import { plural } from '../../../lib/plural';
// Publish sheets for a site. Both targets stay on this Mac:
//   zip   — the static site (HTML per page + ONE site.css whose first block is
//           the brand tokens as --brand-* CSS variables + assets) as a download;
//   local — a loopback preview served by the daemon (bearer-authenticated).
// Each publish is recorded server-side with the exact versions it rendered
// (the site version, the brand kit, every 3D embed and image), so a later
// change to a linked artifact never silently alters what was shipped.
// Nothing here uploads anywhere; "Publish as claude.ai artifact" is not built.
import Modal from '../../../lib/components/Modal.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { rel } from '../../../lib/stores/now.svelte';
import * as design from '../../../lib/api/design';
import type { DesignArtifact, DesignPublish, DesignSiteLocalResp } from '../../../lib/api/types';
import type { AuditFinding } from './engine/audit';
import type { SiteDoc } from './engine/types';
import { exportZip, listPublishes, pinnedEmbedPoster, previewHtml, publishLocal } from './siteApi';
import { baseUrl } from '../../../lib/api/client';
import { auth } from '../../../lib/stores/auth.svelte';
;

  interface Props {
    mode: 'zip' | 'local';
    artifact: DesignArtifact;
    doc: SiteDoc;
    /** The working copy as the studio would save it (dirty check vs the head). */
    source: string;
    /** The editor's own dirty flag. When given it wins: comparing raw bytes
     *  with this serializer's output flagged any other formatting as unsaved. */
    unsaved?: boolean;
    findings: AuditFinding[];
    embedCount: number;
    onclose: () => void;
    onpreview: (served: Record<string, string>, url: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Name the REAL audience (S18-10): the daemon's actual host:port, and the
  // reach the listener setting gives — not a hardcoded "127.0.0.1, this Mac".
  const daemonHost = (() => {
    try {
      return new URL(baseUrl()).host;
    } catch {
      return 'your Otto daemon';
    }
  })();
  const onNetwork = $derived(!!auth.meta?.network_listener);

  let { mode, artifact, doc, source, unsaved = undefined, findings, embedCount, onclose, onpreview }: Props = $props();

  let fetchedDirty = $state<boolean | null>(null);
  const dirty = $derived(unsaved ?? fetchedDirty);
  let history = $state<DesignPublish[]>([]);
  let busy = $state(false);
  /** What failed (the headline, never the raw exception) + the dim cause. */
  let error = $state<{ what: string; detail: string } | null>(null);
  let local = $state<DesignSiteLocalResp | null>(null);

  $effect(() => {
    const id = artifact.id;
    if (unsaved === undefined) {
      void design
        .fetchContent(id, { asText: true })
        .then((c) => (fetchedDirty = (c.text ?? '') !== source))
        .catch(() => (fetchedDirty = null));
    }
    void listPublishes(id)
      .then((h) => (history = h.slice(0, 5)))
      .catch(() => (history = []));
  });

  const pages = $derived(doc.pages.length);
  const errors = $derived(findings.filter((f) => f.level === 'error'));
  const seq = $derived(artifact.head_seq);

  async function doZip(): Promise<void> {
    busy = true;
    error = null;
    try {
      const r = await exportZip(artifact.id);
      const url = URL.createObjectURL(r.blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = r.filename;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
      toasts.success(`Saved ${r.filename}`, `v${r.seq ?? seq} · every linked artifact pinned in the publish record.`);
      onclose();
    } catch (e) {
      error = { what: 'Couldn’t export the site.', detail: loadErrorText(e) };
    } finally {
      busy = false;
    }
  }

  async function doLocal(): Promise<void> {
    busy = true;
    error = null;
    try {
      local = await publishLocal(artifact.id);
      history = [local.publish, ...history].slice(0, 5);
    } catch (e) {
      error = { what: 'Couldn’t publish the preview.', detail: loadErrorText(e) };
    } finally {
      busy = false;
    }
  }

  async function openLocal(): Promise<void> {
    if (!local) return;
    busy = true;
    error = null;
    try {
      const served: Record<string, string> = {};
      for (const [i, p] of local.pages.entries()) {
        served[p.id] = await previewHtml(artifact.id, { page: i === 0 ? undefined : p.slug || p.id, publish: local.publish.id });
      }
      onpreview(served, local.url);
    } catch (e) {
      error = { what: 'Couldn’t open the preview.', detail: loadErrorText(e) };
    } finally {
      busy = false;
    }
  }

  async function copyUrl(): Promise<void> {
    if (!local) return;
    try {
      await copyTextOrThrow(local.url);
      toasts.success('Preview address copied', onNetwork ? 'It needs your Otto sign-in and design access; devices on your network can reach it.' : 'It needs your Otto sign-in and only works on this Mac.');
    } catch {
      toasts.warn('Couldn’t copy the address', `The clipboard isn’t available here. Select the address and copy it manually: ${local.url}`);
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":mode === 'zip' ? 'Export static site' : 'Publish preview (local)',"width":540,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
    if(mode === 'zip'){
       { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || !artifact.head_version_id,"onclick":() => void doZip(),...__sveltets_2_empty({"data-testid":`site-export-zip`}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} busy ? 'Exporting…' : 'Export .zip';
       }
    } else if (!local){
       { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || !artifact.head_version_id,"onclick":() => void doLocal(),...__sveltets_2_empty({"data-testid":`site-publish-local`}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});} busy ? 'Publishing…' : 'Publish preview';
       }
    }else{
       { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy,"onclick":() => void openLocal(),...__sveltets_2_empty({"data-testid":`site-open-local`}),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}   }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", {   "class":`body`,...__sveltets_2_empty({"data-testid":`site-publish-modal`}),});
    if(dirty){
       { svelteHTML.createElement("p", { "class":`banner warn`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}     mode === 'zip' ? 'The export' : 'The preview';      seq;         }
    }
    if(mode === 'zip'){
       { svelteHTML.createElement("p", { "class":`lead`,});  { svelteHTML.createElement("strong", {});artifact.title; } seq;         }
       { svelteHTML.createElement("ul", { "class":`checks`,});
         { svelteHTML.createElement("li", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} plural(pages, 'page');           }
         { svelteHTML.createElement("li", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}   { svelteHTML.createElement("span", { "class":`mono`,});  }        { svelteHTML.createElement("span", { "class":`mono`,});  }   }
         { svelteHTML.createElement("li", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}        { svelteHTML.createElement("span", { "class":`mono`,});  } }
         { svelteHTML.createElement("li", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} embedCount ? `${plural(embedCount, '3D embed')} as poster images (the interactive runtime lands later)` : 'Responsive: desktop, tablet and mobile from the same files'; }
         { svelteHTML.createElement("li", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`shield`,"size":12,}});}             }
       }
    }else{
       { svelteHTML.createElement("p", { "class":`lead`,});  { svelteHTML.createElement("strong", {});artifact.title; } seq;       { svelteHTML.createElement("span", { "class":`mono`,});daemonHost; }  }
       { svelteHTML.createElement("ul", { "class":`checks`,});
        if(onNetwork){
           { svelteHTML.createElement("li", { ...__sveltets_2_empty({"data-testid":`site-local-reach`}),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}                            }
        }else{
           { svelteHTML.createElement("li", { ...__sveltets_2_empty({"data-testid":`site-local-reach`}),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`shield`,"size":12,}});}                          }
        }
         { svelteHTML.createElement("li", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}              }
       }
      if(local){
         { svelteHTML.createElement("div", {   "class":`served`,...__sveltets_2_empty({"data-testid":`site-local-url`}),});
           { svelteHTML.createElement("span", { "class":`mono url`,});local.url; }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void copyUrl(),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}  }
         }
        if(local.warnings.length){
           { svelteHTML.createElement("ul", { "class":`warns`,});   for(let w of __sveltets_2_ensureArray(local.warnings)){w; { svelteHTML.createElement("li", {}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} w; }} }
        }
      }
    }

    if(errors.length){
       { svelteHTML.createElement("p", { "class":`banner`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});} plural(errors.length, 'accessibility issue');              }
    }
    if(error){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: `${error.what} ${error.detail ?? ''}`.trim(), assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`banner bad`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`err-text`,}); { svelteHTML.createElement("strong", {});error.what; }if(error.detail){ { svelteHTML.createElement("span", { "class":`dim err-detail`,});error.detail; }} }
       }}
    }

    if(history.length){
       { svelteHTML.createElement("div", { "class":`history`,});
         { svelteHTML.createElement("span", { "class":`k`,});  }
         { svelteHTML.createElement("ul", {});
             for(let h of __sveltets_2_ensureArray(history)){h.id;
            const set = Array.isArray(h.pinned_set) ? h.pinned_set : [];
            const pinnedRefs = set.filter((r) => r.role !== 'site').length;
            const standIns = set.filter((r) => pinnedEmbedPoster(r).standIn).length;
             { svelteHTML.createElement("li", {});
               { svelteHTML.createElement("span", { "class":`chip`,});h.target === 'zip' ? '.zip' : 'local'; }
              set.find((r) => r.role === 'site')?.seq ?? '?';  rel(h.created_at);
              if(pinnedRefs){ { svelteHTML.createElement("span", { "class":`dim`,});  pinnedRefs;  }}
              if(standIns){ { svelteHTML.createElement("span", { "class":`dim`,});  plural(standIns, 'poster stand-in'); }}
             }
          }
         }
       }
    }
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PublishModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PublishModal__SvelteComponent_ = ReturnType<typeof PublishModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default PublishModal__SvelteComponent_;