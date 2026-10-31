///<reference types="svelte" />
;// Outputs — the artifacts an agent session produced (files written, PRs,
// images, reports, URLs) folded from its transcript, with an inline preview.
// Rendering discipline (docs/design/conversation-view.md §5.6/§6):
//   • HTML  → <iframe sandbox=""> via srcdoc (MockupViewer's approach: no
//     scripts, forms, popups or same-origin — the frame can't reach the daemon)
//   • Markdown → lib/render/markdown (marked + allowlist sanitizer), never lib/md
//   • images → <img> from an authed blob URL; PDF → sandboxed <iframe>
//   • anything else → an authed download link
// Bytes come ONLY from `GET /sessions/{id}/artifacts/{artifact_id}` (opaque id;
// the server maps it back to the path it folded itself) — so an `on_disk`
// History entry, which has no session, lists artifacts without a preview.
//
// Hosted two ways: as the right-panel **Outputs** tab (no props → the focused
// agent session) and embedded under the History conversation (`embedded`).

import { copyText } from '../../lib/clipboard';
import { untrack } from 'svelte';
import { loadErrorText } from '../../lib/loadError';
import { ws } from '../../lib/stores/workspace.svelte';
import { activity } from '../../lib/stores/activity.svelte';
import { authedBlobUrl, authedText } from '../../lib/api/client';
import { renderNote } from '../../lib/render/markdown';
import { toasts } from '../../lib/toast.svelte';
import { rel } from '../../lib/stores/now.svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import type { Artifact } from '../../lib/api/types';
;

  interface Props {
    /** Session whose artifacts to show; defaults to the focused agent session. */
    sessionId?: string | null;
    /** Pre-folded list (History `on_disk` entries) — no session ⇒ no preview. */
    artifacts?: Artifact[] | null;
    /** Compact chrome when embedded under a conversation. */
    embedded?: boolean;
  };;

  type PreviewKind = 'html' | 'md' | 'image' | 'pdf' | 'text' | 'link' | 'download' | 'none';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { sessionId = null, artifacts = null, embedded = false }: Props = $props();

  // Resolve the session: explicit prop, else the focused agent session (the
  // right-panel tab is gated on one by the shell, but stay defensive).
  const focused = $derived(ws.activeSession?.kind === 'agent' ? ws.activeSession : null);
  const sid = $derived(sessionId ?? focused?.id ?? null);
  const list = $derived<Artifact[]>(artifacts ?? activity.artifacts(sid));

  const listLoading = $derived(!artifacts && sid ? activity.artifactsLoadingBySession[sid] : false);
  const listError = $derived(!artifacts && sid ? activity.artifactsErrorBySession[sid] : null);
  $effect(() => {
    const session = sid;
    if (!artifacts && session) return untrack(() => activity.watchArtifacts(session));
  });

  // ── Selection + preview ──────────────────────────────────────────────────────
  let selectedId = $state<string | null>(null);
  const selected = $derived(list.find((a) => a.id === selectedId) ?? null);

  let preview = $state<{ kind: PreviewKind; url?: string; text?: string; html?: string } | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let createdUrls: string[] = [];
  let previewRequest = 0;
  let previewIdentity = '';
  let dismissed = $state(false);

  function revokeCreated(): void {
    for (const u of createdUrls.splice(0)) URL.revokeObjectURL(u);
  }
  $effect(() => () => {
    // A fetch can resolve after this panel was unmounted. Invalidate it before
    // releasing the current preview so late blob URLs are revoked too.
    previewRequest++;
    revokeCreated();
  });

  /** Extension of the artifact's path/url (lowercase, no dot). */
  function ext(a: Artifact): string {
    const p = a.path ?? a.url ?? '';
    const base = p.split(/[?#]/)[0].split('/').pop() ?? '';
    const i = base.lastIndexOf('.');
    return i >= 0 ? base.slice(i + 1).toLowerCase() : '';
  }

  /** How to render an artifact — mime first, extension as the fallback. */
  function classify(a: Artifact): PreviewKind {
    if (a.kind === 'pr' || a.kind === 'url') return 'link';
    const m = (a.mime ?? '').toLowerCase();
    const e = ext(a);
    if (m.startsWith('image/') || ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(e)) return 'image';
    if (m === 'text/html' || e === 'html' || e === 'htm') return 'html';
    if (m === 'text/markdown' || e === 'md' || e === 'markdown') return 'md';
    if (m === 'application/pdf' || e === 'pdf') return 'pdf';
    if (m.startsWith('text/') || m === 'application/json' || ['csv', 'json', 'txt', 'log', 'yaml', 'yml', 'toml'].includes(e))
      return 'text';
    return 'download';
  }

  const KIND_ICON: Record<Artifact['kind'], IconName> = {
    file: 'file',
    pr: 'pr',
    image: 'image',
    report: 'note',
    url: 'link',
  };

  /** Cap for inline text previews — the server already caps at 25 MB; the
   *  DOM shouldn't hold more than a couple hundred KB of <pre>. */
  const TEXT_CAP = 200 * 1024;

  async function select(a: Artifact): Promise<void> {
    const request = ++previewRequest;
    previewIdentity = JSON.stringify(a);
    selectedId = a.id;
    dismissed = false;
    preview = null;
    error = null;
    loading = false;
    revokeCreated();
    const kind = classify(a);
    if (kind === 'link') {
      preview = { kind, url: a.url ?? undefined };
      return;
    }
    if (!sid) {
      // on_disk History entry: the bytes route is per-session, so no preview.
      preview = { kind: 'none' };
      return;
    }
    // Two quick clicks race: every await below re-checks that THIS artifact is
    // still the selection, else the earlier fetch would land its bytes (and
    // the Download link) under the later label.
    const mine = a.id;
    const stale = (): boolean => selectedId !== mine || request !== previewRequest;
    const route = `/sessions/${sid}/artifacts/${encodeURIComponent(a.id)}`;
    loading = true;
    try {
      if (kind === 'image' || kind === 'pdf' || kind === 'download') {
        const url = await authedBlobUrl(route);
        if (stale()) {
          URL.revokeObjectURL(url);
          return;
        }
        createdUrls.push(url);
        preview = { kind, url };
      } else if (kind === 'html') {
        const html = await authedText(route);
        if (stale()) return;
        preview = { kind, html };
      } else if (kind === 'md') {
        const md = await authedText(route);
        if (stale()) return;
        preview = { kind, html: renderNote(md, { resolve: () => null, assetUrl: () => null }) };
      } else {
        let text = await authedText(route);
        if (stale()) return;
        if (text.length > TEXT_CAP) text = text.slice(0, TEXT_CAP) + `\n… (truncated at ${TEXT_CAP / 1024} KB)`;
        preview = { kind: 'text', text };
      }
    } catch (e) {
      if (stale()) return;
      error = loadErrorText(e);
    } finally {
      if (!stale()) loading = false;
    }
  }

  // Drop the selection when the session changes underneath us.
  $effect(() => {
    void sid;
    previewIdentity = '';
    previewRequest++;
    loading = false;
    selectedId = null;
    dismissed = false;
    preview = null;
    error = null;
    revokeCreated();
  });

  // Open on the first available artifact, including an asynchronously loaded
  // list. Explicitly closing a preview is respected until the session changes.
  $effect(() => {
    const first = list[0];
    if (first && !selected && !dismissed) untrack(() => void select(first));
  });

  // A producing turn may replace the same path/id while the preview is open.
  // Keep the selection, but load the current revision's bytes and invalidate
  // any older fetch. Unrelated list updates do not disturb the preview.
  $effect(() => {
    const current = selected;
    if (current && JSON.stringify(current) !== previewIdentity) untrack(() => void select(current));
    else if (!current) untrack(() => {
      previewRequest++;
      preview = null;
      loading = false;
      revokeCreated();
    });
  });

  function closePreview(): void {
    previewRequest++;
    dismissed = true;
    selectedId = null;
    preview = null;
    loading = false;
    revokeCreated();
  }

  function onArtifactKey(e: KeyboardEvent, index: number): void {
    let next: number;
    if (e.key === 'ArrowDown') next = Math.min(index + 1, list.length - 1);
    else if (e.key === 'ArrowUp') next = Math.max(index - 1, 0);
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = list.length - 1;
    else return;
    e.preventDefault();
    const row = e.currentTarget as HTMLButtonElement;
    row.closest('ul')?.querySelectorAll<HTMLButtonElement>('[role="option"]')[next]?.focus();
    void select(list[next]);
  }

  async function copyPath(a: Artifact): Promise<void> {
    const v = a.path ?? a.url ?? '';
    if (await copyText(v)) toasts.info('Copied', v);
    else toasts.error('Couldn’t copy', `The clipboard isn’t available here. Select the path and copy it manually. ${v}`);
  }

  /** Absolute time for the hover title next to the relative `rel()` label. */
  function absTime(iso: string | null): string {
    if (!iso) return '';
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleString();
  }

  // Long identities remain reachable on touch and by keyboard. Ordinary
  // short names/paths do not add redundant stops to the Tab sequence.
  function scrollableIdentity(node: HTMLElement) {
    const update = () => {
      if (node.scrollHeight > node.clientHeight + 1) node.tabIndex = 0;
      else node.removeAttribute('tabindex');
    };
    // Give focused regions explicit, consistent scrolling across webviews.
    const onKey = (event: KeyboardEvent) => {
      if (event.target !== node || node.scrollHeight <= node.clientHeight) return;
      if (event.key === 'ArrowDown') node.scrollTop += 40;
      else if (event.key === 'ArrowUp') node.scrollTop -= 40;
      else if (event.key === 'PageDown') node.scrollTop += node.clientHeight;
      else if (event.key === 'PageUp') node.scrollTop -= node.clientHeight;
      else if (event.key === 'Home') node.scrollTop = 0;
      else if (event.key === 'End') node.scrollTop = node.scrollHeight;
      else return;
      event.preventDefault();
      event.stopPropagation();
    };
    node.addEventListener('keydown', onKey);
    const resize = new ResizeObserver(update);
    const content = new MutationObserver(update);
    resize.observe(node);
    content.observe(node, { childList: true, subtree: true, characterData: true });
    update();
    return { destroy() { resize.disconnect(); content.disconnect(); node.removeEventListener('keydown', onKey); } };
  }

  function downloadName(a: Artifact): string {
    return (a.path ?? a.url ?? a.label).split('/').pop() || a.label;
  }
;
async () => {

if(!sid && !artifacts){
   { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":`layers`,"title":`No session selected`,"body":`Open or focus an agent session to see the files, PRs and images it produced.`,}});}
}else{
   { svelteHTML.createElement("div", {   "class":`outputs`,...__sveltets_2_empty({"data-testid":`outputs-panel`}),});embedded;
     { svelteHTML.createElement("div", { "class":`list-head`,});
       { svelteHTML.createElement("span", { "class":`section-title`,});  }
      if(list.length > 0){ { svelteHTML.createElement("span", { "class":`count`,});list.length; }}
     }

     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`outputs`,"variant":`compact`,"loading":listLoading,"error":listError,"empty":list.length === 0,"onretry":() => sid && void activity.loadArtifacts(sid, true),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("p", { "class":`empty-line dim`,});
                          
         }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("ul", {     "class":`alist`,"role":`listbox`,"aria-label":`Artifacts`,});
            for(let a of __sveltets_2_ensureArray(list)){let index = 1;a.id;
           { svelteHTML.createElement("li", { "role":`none`,});
             { svelteHTML.createElement("button", {                "class":`arow`,"role":`option`,"aria-selected":selectedId === a.id,"tabindex":selectedId === a.id || (!selected && index === 0) ? 0 : -1,"onkeydown":(e) => onArtifactKey(e, index),"onclick":() => selectedId !== a.id && void select(a),"title":a.path ?? a.url ?? a.label,});selectedId === a.id;
               { svelteHTML.createElement("span", { "class":`aicon`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":KIND_ICON[a.kind] ?? 'file',"size":12,}});} }
               { svelteHTML.createElement("span", { "class":`alabel`,});a.label; }
               { svelteHTML.createElement("span", { "class":`ameta mono`,});ext(a) || a.kind; }
               { svelteHTML.createElement("span", {   "class":`ameta mono`,"title":absTime(a.produced_at),});a.produced_at ? rel(a.produced_at) : ''; }
             }
           }
        }
       }
     LoadState}

    if(selected){
       { svelteHTML.createElement("div", {   "class":`preview`,...__sveltets_2_empty({"data-testid":`outputs-preview`}),});
         { svelteHTML.createElement("div", { "class":`phead`,});
           {const $$action_0 = __sveltets_2_ensureAction(scrollableIdentity(svelteHTML.mapElementTag('span')));{ svelteHTML.createElement("span", __sveltets_2_union($$action_0), {       "class":`ptitle`,"role":`region`,"aria-label":`Output name`,"title":selected.path ?? selected.url ?? '',});selected.label; }}
           { svelteHTML.createElement("span", { "class":`pactions`,});
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Copy path`,"aria-label":`Copy path`,"onclick":() => void copyPath(selected!),});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}
             }
            if(preview?.kind === 'download' || preview?.kind === 'image' || preview?.kind === 'pdf'){
               { svelteHTML.createElement("a", {         "class":`icon-btn`,"href":preview.url,"download":downloadName(selected),"title":`Download`,"aria-label":`Download`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}
               }
            }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Close preview`,"aria-label":`Close preview`,"onclick":closePreview,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }
         }
        if(selected.path ?? selected.url){
           {const $$action_0 = __sveltets_2_ensureAction(scrollableIdentity(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {        "class":`pidentity mono`,"dir":`ltr`,"role":`region`,"aria-label":`Output path`,});selected.path ?? selected.url; }}
        }
        if(loading || error){
           { svelteHTML.createElement("div", {  "class":`pbody`,});!!error;
             { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "what":`the preview`,"variant":`compact`,loading,error,"empty":true,"onretry":() => void select(selected!),}});}
           }
        } else if (preview?.kind === 'link'){
           { svelteHTML.createElement("div", { "class":`pbody`,});
             { svelteHTML.createElement("a", {       "class":`ext-link`,"href":preview.url,"target":`_blank`,"rel":`noopener noreferrer`,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});} preview.url;
             }
           }
        } else if (preview?.kind === 'none'){
           { svelteHTML.createElement("div", { "class":`pbody dim`,});
             { svelteHTML.createElement("div", { "class":`mono path`,});selected.path ?? selected.url; }
                     
           }
        } else if (preview?.kind === 'image'){
           { svelteHTML.createElement("div", { "class":`pbody media`,}); { svelteHTML.createElement("img", {    "src":preview.url,"alt":selected.label,});} }
        } else if (preview?.kind === 'html'){
          
           { svelteHTML.createElement("iframe", {     "class":`frame`,"title":selected.label,"sandbox":"","srcdoc":preview.html,}); }
        } else if (preview?.kind === 'pdf'){
           { svelteHTML.createElement("iframe", {     "class":`frame`,"title":selected.label,"sandbox":"","src":preview.url,}); }
        } else if (preview?.kind === 'md'){
          
           { svelteHTML.createElement("div", { "class":`pbody md`,}); preview.html; }
        } else if (preview?.kind === 'text'){
           { svelteHTML.createElement("pre", { "class":`pbody mono text`,});preview.text; }
        } else if (preview?.kind === 'download'){
           { svelteHTML.createElement("div", { "class":`pbody dim`,});
                 
             { svelteHTML.createElement("a", {   "href":preview.url,"download":downloadName(selected),}); downloadName(selected); }
           }
        }
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const OutputsPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type OutputsPanel__SvelteComponent_ = ReturnType<typeof OutputsPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default OutputsPanel__SvelteComponent_;