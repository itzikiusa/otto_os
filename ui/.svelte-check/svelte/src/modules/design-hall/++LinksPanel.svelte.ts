///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { radioKey } from '../../lib/radioKey';
// Right panel → Links: what this design USES (outgoing) and where it is USED
// IN (incoming), each row with its relation, version policy and Open. Links a
// document makes itself (otto://design URIs) are "extracted" and change only
// by editing the document; explicit links are added and removed here.
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { toastError } from '../../lib/toastError';
import { toasts } from '../../lib/toast.svelte';
import { ApiError } from '../../lib/api/client';
import { createLink, deleteLink, search } from '../../lib/api/design';
import { openExternal } from '../../lib/external';
import { ws } from '../../lib/stores/workspace.svelte';
import type { DesignArtifact, DesignLinkDstKind, DesignLinkPolicy, DesignLinkRel, DesignSearchHit } from '../../lib/api/types';
import StudioBadge from './StudioBadge.svelte';
import { EXPLICIT_RELS, policyLabel, relLabel, type LinkRow } from './model';
import { library } from './library.svelte';
import { openArtifact, openStoryInProduct } from './nav';
;

  interface Props {
    artifact: DesignArtifact;
    uses: LinkRow[];
    usedIn: LinkRow[];
    loading: boolean;
    error: string | null;
    readonly: boolean;
    seqOf: (versionId: string) => number | null;
    onreload: () => void;
    /** Compare a pinned target's pinned version with its head. */
    oncompare: (target: DesignArtifact, pinnedVersionId: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { artifact, uses, usedIn, loading, error, readonly, seqOf, onreload, oncompare }: Props = $props();

  function iconFor(kind: DesignLinkDstKind | string): IconName {
    switch (kind) {
      case 'story':
        return 'ticket';
      case 'url':
        return 'globe';
      case 'session':
        return 'terminal';
      case 'pr':
        return 'pr';
      case 'vault_note':
        return 'book';
      default:
        return 'link';
    }
  }

  /** Pinned target whose head moved on: "pinned v9 · v10 available". */
  function newerThanPinned(r: LinkRow): number | null {
    if (r.link.policy !== 'pinned' || !r.other || !r.link.pinned_version_id) return null;
    const pinned = seqOf(r.link.pinned_version_id);
    const head = r.other.head_seq;
    return pinned != null && head != null && head > pinned ? head : null;
  }

  function pinnedSeq(r: LinkRow): number | null {
    if (!r.link.pinned_version_id) return null;
    if (r.other && r.link.pinned_version_id === r.other.head_version_id) return r.other.head_seq;
    return seqOf(r.link.pinned_version_id);
  }

  function open(r: LinkRow, incoming: boolean): void {
    if (incoming || r.link.dst_kind === 'artifact') {
      if (r.other) openArtifact(r.other.id);
      return;
    }
    if (r.link.dst_kind === 'story') openStoryInProduct(r.link.dst_id);
    else if (r.link.dst_kind === 'url') void openExternal(r.link.dst_id);
    else if (r.link.dst_kind === 'session') ws.navigateToSession(r.link.dst_id);
  }
  function canOpen(r: LinkRow, incoming: boolean): boolean {
    if (incoming || r.link.dst_kind === 'artifact') return !!r.other;
    return r.link.dst_kind === 'story' || r.link.dst_kind === 'url' || r.link.dst_kind === 'session';
  }

  async function remove(r: LinkRow): Promise<void> {
    try {
      await deleteLink(artifact.id, r.link.id);
      toasts.success('Link removed');
      onreload();
    } catch (e) {
      toastError('Couldn’t remove the link', e);
    }
  }

  // ── Add link ──────────────────────────────────────────────────────────────
  let adding = $state(false);
  let rel = $state<DesignLinkRel>('references');
  let target = $state<'artifact' | 'story' | 'url'>('artifact');
  let policy = $state<'' | DesignLinkPolicy>('');
  let q = $state('');
  let hits = $state<DesignSearchHit[]>([]);
  let picked = $state<DesignArtifact | null>(null);
  let storyId = $state('');
  let url = $state('');
  let busy = $state(false);
  let addError = $state<string | null>(null);
  let searchSeq = 0;

  function openAdd(): void {
    adding = true;
    rel = 'references';
    target = 'artifact';
    policy = '';
    q = '';
    hits = [];
    picked = null;
    storyId = '';
    url = '';
    addError = null;
    void runSearch('');
  }

  async function runSearch(term: string): Promise<void> {
    const my = ++searchSeq;
    try {
      const r = await search(term, { limit: 8 });
      if (my === searchSeq) hits = r.filter((h) => h.artifact.id !== artifact.id);
    } catch {
      if (my === searchSeq) hits = [];
    }
  }
  // The result list is one Tab stop (roving tabindex): ↑/↓/Home/End move
  // focus and the pick together, like the Outputs panel's artifact list.
  const pickedIndex = $derived(hits.findIndex((h) => h.artifact.id === picked?.id));
  function onHitKey(e: KeyboardEvent, index: number): void {
    let next: number;
    if (e.key === 'ArrowDown') next = Math.min(index + 1, hits.length - 1);
    else if (e.key === 'ArrowUp') next = Math.max(index - 1, 0);
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = hits.length - 1;
    else return;
    e.preventDefault();
    const row = e.currentTarget as HTMLButtonElement;
    row.closest('ul')?.querySelectorAll<HTMLButtonElement>('[role="option"]')[next]?.focus();
    picked = hits[next].artifact;
  }
  let t: ReturnType<typeof setTimeout> | null = null;
  function onQuery(): void {
    if (t) clearTimeout(t);
    t = setTimeout(() => void runSearch(q.trim()), 200);
  }

  const stories = $derived(Object.values(library.stories).sort((a, b) => a.source_key.localeCompare(b.source_key)));
  const valid = $derived(
    target === 'artifact' ? !!picked : target === 'story' ? !!storyId : /^https?:\/\/\S+$/i.test(url.trim()),
  );

  async function add(): Promise<void> {
    if (!valid) return;
    busy = true;
    addError = null;
    try {
      await createLink(artifact.id, {
        rel,
        dst_kind: target,
        dst_id: target === 'artifact' ? picked!.id : target === 'story' ? storyId : url.trim(),
        ...(policy ? { policy } : {}),
      });
      adding = false;
      toasts.success('Link added');
      onreload();
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        addError = /cycle/i.test(e.message)
          ? `That link would make a loop — ${picked?.title ?? 'the target'} already ${relLabel(rel)} this design, directly or through another design.`
          : `This link already exists. ${e.message}`;
      } else {
        addError = e instanceof Error ? e.message : String(e);
      }
    } finally {
      busy = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`links`,...__sveltets_2_empty({"data-testid":`design-links-panel`}),});
  if((loading || error) && !uses.length && !usedIn.length){
     { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`links`,"variant":`compact`,loading,error,"empty":true,"onretry":onreload,}});} }
  }else{
     const row/*Ωignore_positionΩ*/ = (r: LinkRow, incoming: boolean)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
      const newer = incoming ? null : newerThanPinned(r);
       { svelteHTML.createElement("li", {   "class":`row`,...__sveltets_2_empty({"data-testid":incoming ? 'design-link-in' : 'design-link-out'}),});
         { svelteHTML.createElement("span", { "class":`lead`,});
          if(r.other){ { const $$_egdaBoidutS2C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS2C({ target: __sveltets_2_any(), props: {    "studio":r.other.studio,"size":20,}});}}else{ { svelteHTML.createElement("span", { "class":`kind`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":iconFor(incoming ? 'artifact' : r.link.dst_kind),"size":12,}});} }}
         }
         { svelteHTML.createElement("span", { "class":`main`,});
           { svelteHTML.createElement("span", { "class":`t`,});
             { svelteHTML.createElement("span", {   "class":`name`,"title":r.label,});r.label; }
            if(r.other?.head_seq != null){ { svelteHTML.createElement("span", { "class":`ver`,}); r.other.head_seq; }}
           }
           { svelteHTML.createElement("span", { "class":`sub`,});
            relLabel(r.link.rel);r.link.src_node ? ` · ${r.link.src_node}` : '';
            if(r.link.origin === 'extracted'){ { svelteHTML.createElement("span", {   "class":`dim`,"title":`Found in the document (otto://design link) — edit the document to change it`,});    }}
           }
           { svelteHTML.createElement("span", { "class":`pills`,});
            if(r.link.broken){
               { svelteHTML.createElement("span", {   "class":`chip bad`,"title":`The target artifact, version or node is missing`,});  }
            } else if (newer != null){
               { svelteHTML.createElement("span", { "class":`chip warnc`,}); pinnedSeq(r);  newer;  }
            } else if (!incoming && r.link.dst_kind === 'artifact'){
               { svelteHTML.createElement("span", { "class":`chip`,});policyLabel(r.link.policy, pinnedSeq(r)); }
            }
           }
         }
         { svelteHTML.createElement("span", { "class":`acts`,});
          if(newer != null && r.other){
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => oncompare(r.other!, r.link.pinned_version_id!),});  }
          } else if (canOpen(r, incoming)){
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => open(r, incoming),});  }
          }
          if(!incoming && !readonly && r.link.origin === 'explicit'){
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void remove(r),"aria-label":`Remove link to ${r.label}`,"title":`Remove link`,});
               { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
             }
          }
         }
       }
    };return __sveltets_2_any(0)};if(error){
      
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `Couldn’t refresh the links. ${error}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`pad err`,"title":error,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}        
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":onreload,"disabled":loading,});loading ? 'Retrying…' : 'Retry'; }
       }}
    }
    

     { svelteHTML.createElement("section", {});
       { svelteHTML.createElement("header", {});
         { svelteHTML.createElement("h3", {});  }
         { svelteHTML.createElement("span", { "class":`dim`,});uses.length;  }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(!readonly){
           { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":openAdd,...__sveltets_2_empty({"data-testid":`design-add-link`}),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
        }
       }
      if(uses.length){
         { svelteHTML.createElement("ul", {});   for(let r of __sveltets_2_ensureArray(uses)){r.link.id;;__sveltets_2_ensureSnippet(row(r, false));} }
      }else{
         { svelteHTML.createElement("p", { "class":`dim empty`,});               { svelteHTML.createElement("span", { "class":`mono`,});  }       }
      }
     }
     { svelteHTML.createElement("section", {});
       { svelteHTML.createElement("header", {});
         { svelteHTML.createElement("h3", {});  }
         { svelteHTML.createElement("span", { "class":`dim`,});usedIn.length;  }
       }
      if(usedIn.length){
         { svelteHTML.createElement("ul", {});   for(let r of __sveltets_2_ensureArray(usedIn)){r.link.id;;__sveltets_2_ensureSnippet(row(r, true));} }
      }else{
         { svelteHTML.createElement("p", { "class":`dim empty`,});     }
      }
     }
     { svelteHTML.createElement("p", { "class":`foot dim`,});                         }
  }
 }

if(adding){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Add link`,"width":520,"onclose":() => (adding = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (adding = false),});  }
       { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":add,"disabled":!valid || busy,...__sveltets_2_empty({"data-testid":`design-add-link-confirm`}),});busy ? 'Adding…' : 'Add link'; }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`form`,});
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`dh-link-rel`,});  }
         { svelteHTML.createElement("select", {     "id":`dh-link-rel`,"class":`input`,"bind:value":rel,});/*Ωignore_startΩ*/() => rel = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let r of __sveltets_2_ensureArray(EXPLICIT_RELS)){r; { svelteHTML.createElement("option", { "value":r,});relLabel(r); }}
         }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", {   "class":`lbl`,"id":`dh-link-target`,});  }
         { svelteHTML.createElement("div", {     "class":`seg-row`,"role":`radiogroup`,"aria-labelledby":`dh-link-target`,});
             for(let [k, l] of __sveltets_2_ensureArray(([['artifact', 'A design'], ['story', 'A story'], ['url', 'A web page']]))){k;
             { svelteHTML.createElement("button", {              "type":`button`,"role":`radio`,"aria-checked":target === k,"tabindex":target === k ? 0 : -1,"onkeydown":radioKey,"class":`pill-toggle`,"onclick":() => (target = k as typeof target),});target === k;l; }
          }
         }
       }
      if(target === 'artifact'){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`dh-link-q`,});   }
           { svelteHTML.createElement("input", {              "dir":`auto`,"id":`dh-link-q`,"class":`input`,"bind:value":q,"oninput":onQuery,"placeholder":`Tier card`,...__sveltets_2_empty({"data-testid":`design-link-search`}),});/*Ωignore_startΩ*/() => q = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("ul", {     "class":`pick`,"role":`listbox`,"aria-label":`Designs`,});
                for(let h of __sveltets_2_ensureArray(hits)){let index = 1;h.artifact.id;
               { svelteHTML.createElement("li", { "role":`none`,});
                 { svelteHTML.createElement("button", {            "type":`button`,"role":`option`,"aria-selected":picked?.id === h.artifact.id,"tabindex":index === (pickedIndex >= 0 ? pickedIndex : 0) ? 0 : -1,"onkeydown":(e) => onHitKey(e, index),"onclick":() => (picked = h.artifact),});picked?.id === h.artifact.id;
                   { const $$_egdaBoidutS6C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS6C({ target: __sveltets_2_any(), props: {  "studio":h.artifact.studio,}});}
                   { svelteHTML.createElement("span", { "class":`name`,});h.artifact.title; }
                  if(h.artifact.head_seq != null){ { svelteHTML.createElement("span", { "class":`ver`,}); h.artifact.head_seq; }}
                 }
               }
            }
               { svelteHTML.createElement("li", {       "class":`dim`,"role":`option`,"aria-disabled":`true`,"aria-selected":`false`,});   }
            
           }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`dh-link-policy`,});  }
           { svelteHTML.createElement("select", {     "id":`dh-link-policy`,"class":`input`,"bind:value":policy,});/*Ωignore_startΩ*/() => policy = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});  relLabel(rel);  }
             { svelteHTML.createElement("option", { "value":`follow_approved`,});    }
             { svelteHTML.createElement("option", { "value":`follow_latest`,});    }
             { svelteHTML.createElement("option", { "value":`pinned`,});    }
           }
         }
      } else if (target === 'story'){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`dh-link-story`,});  }
           { svelteHTML.createElement("select", {       "id":`dh-link-story`,"class":`input`,"bind:value":storyId,"disabled":!stories.length,});/*Ωignore_startΩ*/() => storyId = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});stories.length ? 'Pick a story' : 'No product stories loaded'; }
               for(let s of __sveltets_2_ensureArray(stories)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.source_key;  s.title; }}
           }
         }
      }else{
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`dh-link-url`,});  }
           { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`dh-link-url`,"class":`input`,"bind:value":url,"placeholder":`https://example.com/inspiration`,});/*Ωignore_startΩ*/() => url = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`hint`,});       }
         }
      }
      if(addError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: addError, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`err-msg`,});addError; }}}
     }
    
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LinksPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LinksPanel__SvelteComponent_ = ReturnType<typeof LinksPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default LinksPanel__SvelteComponent_;