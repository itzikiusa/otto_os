///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
// Right panel → References: search the team library (shipped work first) and
// borrow from it — Add as reference (an explicit, pinned `references` link),
// Start from this (a new draft forked from that version, `derived_from`),
// Compare, Open. Below: the provenance of this design as a lineage list.
import { untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import Icon from '../../lib/components/Icon.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { isAbortError } from '../../lib/api/client';
import { captureSignal, createArtifact, createLink, search } from '../../lib/api/design';
import type { DesignArtifact, DesignArtifactFormat, DesignSearchHit, DesignStatus, DesignStudio } from '../../lib/api/types';
import ArtifactThumb from './ArtifactThumb.svelte';
import StudioBadge from './StudioBadge.svelte';
import StatusPill from './StatusPill.svelte';
import { STUDIOS, buildLineage, statusLabel, type LinkRow } from './model';
import { library } from './library.svelte';
import { openArtifact } from './nav';
;

  interface Props {
    artifact: DesignArtifact;
    uses: LinkRow[];
    readonly: boolean;
    seqOf: (versionId: string) => number | null;
    onreload: () => void;
    oncompare: (ref: DesignArtifact) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { artifact, uses, readonly, seqOf, onreload, oncompare }: Props = $props();

  let q = $state('');
  let studio = $state<'' | DesignStudio>('');
  let status = $state<'' | DesignStatus>('');
  let storyId = $state('');
  let since = $state<'' | '30' | '90' | '365'>('');
  let hits = $state<DesignSearchHit[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let ctl: AbortController | null = null;

  const stories = $derived(Object.values(library.stories).sort((a, b) => a.source_key.localeCompare(b.source_key)));

  async function run(): Promise<void> {
    ctl?.abort();
    const mine = new AbortController();
    ctl = mine;
    loading = true;
    try {
      const sinceIso = since ? new Date(Date.now() - Number(since) * 86_400_000).toISOString() : undefined;
      const r = await search(
        q.trim(),
        { studio: studio || undefined, status: status || undefined, story_id: storyId || undefined, since: sinceIso, limit: 30 },
        mine.signal,
      );
      if (ctl !== mine) return;
      hits = r.filter((h) => h.artifact.id !== artifact.id);
      error = null;
    } catch (e) {
      if (isAbortError(e) || ctl !== mine) return;
      error = loadErrorText(e);
    } finally {
      if (ctl === mine) loading = false;
    }
  }

  let timer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    void q;
    void studio;
    void status;
    void storyId;
    void since;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => untrack(() => void run()), 200);
    return () => {
      if (timer) clearTimeout(timer);
    };
  });

  const lineage = $derived(buildLineage(uses, seqOf));
  /** artifact id → its citation label (R1…) when this design already borrows from it. */
  const cited = $derived(new Map(lineage.filter((l) => l.cite && l.artifactId).map((l) => [l.artifactId!, l.cite!])));
  const referenced = (id: string) =>
    cited.has(id) ||
    uses.some((u) => u.other?.id === id && (u.link.rel === 'references' || u.link.rel === 'derived_from'));

  let busyId = $state<string | null>(null);

  async function addReference(ref: DesignArtifact): Promise<void> {
    busyId = ref.id;
    try {
      await createLink(artifact.id, { rel: 'references', dst_kind: 'artifact', dst_id: ref.id });
      captureSignal({ artifact_id: artifact.id, kind: 'reference_added', payload: { target_artifact_id: ref.id, target_seq: ref.head_seq } });
      toasts.success('Added as reference', `${ref.title} is pinned at v${ref.head_seq ?? '?'}.`);
      onreload();
    } catch (e) {
      toastError('Couldn’t add the reference', e);
    } finally {
      busyId = null;
    }
  }

  async function startFrom(ref: DesignArtifact): Promise<void> {
    busyId = ref.id;
    try {
      const res = await createArtifact({
        workspace_id: ws.currentId ?? artifact.workspace_id,
        format: ref.format as DesignArtifactFormat,
        studio: ref.studio,
        title: `${artifact.title} (from ${ref.title})`,
        project_id: artifact.project_id ?? undefined,
        derived_from: { artifact_id: ref.id },
        message: `Started from ${ref.title}`,
      });
      captureSignal({
        artifact_id: res.artifact.id,
        kind: 'forked',
        version_id: res.version.id,
        payload: { source_artifact_id: ref.id, source: 'start_from_this', from_artifact_id: artifact.id },
      });
      toasts.success('Draft created', `“${res.artifact.title}” keeps a pinned link to ${ref.title}.`);
      openArtifact(res.artifact.id);
    } catch (e) {
      toastError('Couldn’t start from this design', e);
    } finally {
      busyId = null;
    }
  }

  function more(e: MouseEvent, ref: DesignArtifact): void {
    ctxMenu.show(e, [
      { label: 'Compare with this design', icon: 'columns', action: () => oncompare(ref) },
      { label: 'Open', icon: 'external', action: () => openArtifact(ref.id) },
    ]);
  }

  const monthYear = (iso: string) => new Date(iso).toLocaleDateString(undefined, { month: 'short', year: 'numeric' });
;
async () => {

 { svelteHTML.createElement("div", {   "class":`refs`,...__sveltets_2_empty({"data-testid":`design-references`}),});
   { svelteHTML.createElement("div", { "class":`search`,});
     { svelteHTML.createElement("label", { "class":`box`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":14,}});}
       { svelteHTML.createElement("input", {            "dir":`auto`,"type":`search`,"bind:value":q,"placeholder":`Search the team library…`,"aria-label":`Search references`,...__sveltets_2_empty({"data-testid":`design-ref-search`}),});/*Ωignore_startΩ*/() => q = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`filters`,});
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":studio,"aria-label":`Studio`,});/*Ωignore_startΩ*/() => studio = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let s of __sveltets_2_ensureArray(STUDIOS)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.name; }}
       }
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":status,"aria-label":`Status`,});/*Ωignore_startΩ*/() => status = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});   }
           for(let s of __sveltets_2_ensureArray((['shipped', 'approved', 'review', 'draft']))){s; { svelteHTML.createElement("option", { "value":s,});statusLabel(s); }}
       }
       { svelteHTML.createElement("select", {       "class":`input`,"bind:value":storyId,"aria-label":`Linked story`,"disabled":!stories.length,});/*Ωignore_startΩ*/() => storyId = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let s of __sveltets_2_ensureArray(stories)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.source_key; }}
       }
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":since,"aria-label":`Date`,});/*Ωignore_startΩ*/() => since = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
         { svelteHTML.createElement("option", { "value":`30`,});   }
         { svelteHTML.createElement("option", { "value":`90`,});   }
         { svelteHTML.createElement("option", { "value":`365`,});  }
       }
     }
   }

   { svelteHTML.createElement("div", { "class":`results`,});
    if(error){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: 'Couldn’t search the references.', assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`err`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}      { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => void run(),});  } }}
    } else if (loading && !hits.length){
      
       { svelteHTML.createElement("p", { "class":`dim`,});   }
    } else if (!hits.length){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),('No designs match.')));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`dim`,});          }}
    }else{
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),(plural(hits.length, 'result'))));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`count dim`,});plural(hits.length, 'result');        }}
       { svelteHTML.createElement("ul", {});
           for(let h of __sveltets_2_ensureArray(hits)){h.artifact.id;
          const a = h.artifact;
          const isRef = referenced(a.id);
           { svelteHTML.createElement("li", {    "class":`hit`,...__sveltets_2_empty({"data-testid":`design-ref-hit`}),});isRef;
             { svelteHTML.createElement("div", { "class":`top`,});
               { svelteHTML.createElement("a", {       "class":`pic`,"href":`#/design/a/${encodeURIComponent(a.id)}`,"tabindex":-1,"aria-hidden":`true`,}); { const $$_bmuhTtcafitrA6C = __sveltets_2_ensureComponent(ArtifactThumb); new $$_bmuhTtcafitrA6C({ target: __sveltets_2_any(), props: {  "artifact":a,}});} }
               { svelteHTML.createElement("div", { "class":`info`,});
                 { svelteHTML.createElement("div", { "class":`t`,});
                   { svelteHTML.createElement("span", {   "class":`name`,"title":a.title,});a.title; }
                  if(cited.get(a.id)){ { svelteHTML.createElement("span", { "class":`cite`,});cited.get(a.id); }}
                 }
                 { svelteHTML.createElement("div", { "class":`meta`,});  { const $$_egdaBoidutS7C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS7C({ target: __sveltets_2_any(), props: {  "studio":a.studio,"showName":true,}});}  { svelteHTML.createElement("span", { "class":`dim`,}); monthYear(a.updated_at); } }
                 { svelteHTML.createElement("div", { "class":`meta`,});
                      for(let sid of __sveltets_2_ensureArray(([...new Set(h.story_ids)].slice(0, 2)))){let i = 1;i;
                    if(library.stories[sid]){ { svelteHTML.createElement("span", { "class":`chip mono-chip`,});library.stories[sid].source_key; }}
                  }
                   { const $$_lliPsutatS7C = __sveltets_2_ensureComponent(StatusPill); new $$_lliPsutatS7C({ target: __sveltets_2_any(), props: {  "status":a.status,}});}
                 }
                if(h.reference_count){ { svelteHTML.createElement("div", { "class":`dim small`,});    plural(h.reference_count, 'design'); }}
                if(h.snippet){ { svelteHTML.createElement("div", { "class":`snip dim small`,});h.snippet.replace(/<\/?[^>]+>/g, ''); }}
               }
             }
             { svelteHTML.createElement("div", { "class":`acts`,});
              if(isRef){
                 { svelteHTML.createElement("span", { "class":`done`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
              } else if (!readonly){
                 { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":busyId === a.id,"onclick":() => void addReference(a),...__sveltets_2_empty({"data-testid":`design-ref-add`}),});   }
              }
               { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":busyId === a.id,"onclick":() => void startFrom(a),});   }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":(e) => more(e, a),"aria-label":`More actions for ${a.title}`,"title":`More actions`,"aria-haspopup":`menu`,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
               }
             }
           }
        }
       }
    }
   }

   { svelteHTML.createElement("section", {   "class":`prov`,"aria-labelledby":`dh-prov-h`,});
     { svelteHTML.createElement("h3", { "id":`dh-prov-h`,});    }
     { svelteHTML.createElement("div", { "class":`prov-title`,});artifact.title;artifact.head_seq != null ? ` v${artifact.head_seq}` : ''; }
    if(lineage.length){
       { svelteHTML.createElement("ul", { "class":`lineage`,});
            for(let l of __sveltets_2_ensureArray(lineage)){let i = 1;i;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("span", { "class":`rel`,});l.relText; }
            if(l.artifactId){
               { svelteHTML.createElement("a", {   "href":`#/design/a/${encodeURIComponent(l.artifactId)}`,"class":`lt`,});l.title; }
            }else{
               { svelteHTML.createElement("span", { "class":`lt`,});l.title; }
            }
             { svelteHTML.createElement("span", { "class":`dim small`,});l.broken ? 'broken' : l.version; }
            if(l.cite){ { svelteHTML.createElement("span", { "class":`cite`,});l.cite; }}
           }
        }
       }
    }else{
       { svelteHTML.createElement("p", { "class":`dim small`,});          }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReferencesPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReferencesPanel__SvelteComponent_ = ReturnType<typeof ReferencesPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReferencesPanel__SvelteComponent_;