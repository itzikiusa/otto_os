///<reference types="svelte" />
;// Brand Kit (`#/design/brand[/<id>]`): the workspace's `otto-brand/1` kits.
// With an id it is the kit editor (brand/BrandEditor.svelte); without one it
// opens the most recently edited kit (list/detail pages open on an item),
// or — when there is none — offers the three starter kits. Studios read a
// kit's tokens by name (`token:color.primary`); helpers in brand/tokens.ts.

import { untrack } from 'svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { toasts } from '../../lib/toast.svelte';
import { router } from '../../lib/router.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { designBus } from '../../lib/events.svelte';
import { library } from './library.svelte';
import BrandEditor from './brand/BrandEditor.svelte';
import NewKitModal from './brand/NewKitModal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  $effect(() => {
    void designBus.resyncTick;
    untrack(() => void library.load());
  });
  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      if (evs.some((e) => e.type === 'design_artifact_updated' && (e.format === 'otto-brand' || e.change === 'created' || e.change === 'deleted'))) {
        library.refreshSoon();
      }
    });
  });

  // `parseDesignRoute` maps every `design/brand…` to the brand view; the kit
  // id (if any) is the third route part.
  const kitId = $derived(router.parts[1] === 'brand' ? (router.parts[2] ?? null) : null);
  const kits = $derived(
    library.hits
      .filter((h) => h.artifact.format === 'otto-brand' && h.artifact.status !== 'archived')
      .sort((a, b) => b.artifact.updated_at.localeCompare(a.artifact.updated_at)),
  );
  const canEdit = $derived(auth.can('design', 'edit'));

  // No id → open the most recent kit (replace, so Back doesn't bounce here).
  $effect(() => {
    if (kitId || !library.loaded || kits.length === 0) return;
    const first = kits[0].artifact.id;
    untrack(() => router.replace(`design/brand/${encodeURIComponent(first)}`));
  });

  let creating = $state(false);
  function openNew(): void {
    if (!canEdit) {
      toasts.warn('You can view brand kits but not create them', 'Ask a workspace admin for Design Hall edit access.');
      return;
    }
    if (!ws.currentId) {
      toasts.warn('Pick a workspace first', 'A brand kit is filed under a workspace.');
      return;
    }
    creating = true;
  }
  function created(id: string): void {
    creating = false;
    void library.load();
    router.go(`design/brand/${encodeURIComponent(id)}`);
  }
;
async () => {

if(kitId){
  kitId; {
     { const $$_rotidEdnarB0C = __sveltets_2_ensureComponent(BrandEditor); new $$_rotidEdnarB0C({ target: __sveltets_2_any(), props: {     "id":kitId,kits,"onnew":openNew,}});}
  }
}else{
   { const $$_redaeHegaP0C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP0C({ target: __sveltets_2_any(), props: {      "title":`Brand Kit`,"subtitle":`Colors, type and voice every studio uses`,"crumbs":[{ label: 'Design Hall', onclick: () => router.go('design') }],}});}
   { const $$_ydoBegaP0C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP0C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
    if((library.loading && !library.loaded) || (library.loaded && kits.length > 0)){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":2,"height":200,}});}
    } else if (library.error && !library.loaded){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`brand kits`,"error":library.error,"empty":true,"onretry":() => void library.load(),}});}
    }else{
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`palette`,"title":`No brand kit yet`,"body":`A brand kit holds your colors, type scale, spacing, logos and voice as one versioned document. Every studio reads its tokens by name, and a change shows which designs it reaches before you save.`,"actionLabel":canEdit ? 'Create brand kit' : undefined,"actionIcon":`plus`,"onaction":canEdit ? openNew : undefined,}});}
    }
   PageBody}
}

if(creating && ws.currentId){
   { const $$_ladoMtiKweN0C = __sveltets_2_ensureComponent(NewKitModal); new $$_ladoMtiKweN0C({ target: __sveltets_2_any(), props: {      "workspaceId":ws.currentId,"onclose":() => (creating = false),"oncreated":created,}});}
}
};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BrandKitPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BrandKitPage__SvelteComponent_ = ReturnType<typeof BrandKitPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default BrandKitPage__SvelteComponent_;