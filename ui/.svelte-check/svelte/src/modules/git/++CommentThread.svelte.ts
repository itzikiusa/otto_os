///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// One PR comment + its replies (recursive). Optional reply affordance.
import CommentThread from './CommentThread.svelte';
import type { PrComment } from '../../lib/api/types';
import { guardUnsaved } from '../../lib/leaveGuard';
import { renderMarkdown } from '../../lib/md';
;

  interface Props {
    comment: PrComment;
    onreply?: (parentId: string, body: string) => Promise<void>;
    /** Resolve/reopen the thread on the provider. Only offered on thread heads
     *  that carry a provider thread_id. */
    onresolve?: (threadId: string, resolved: boolean) => Promise<void>;
    depth?: number;
  };function $$render() {

  
  
  
  
  
  

  let { comment, onreply, onresolve, depth = 0 }: Props = $props();

  let replying = $state(false);
  let replyText = $state('');
  let busy = $state(false);
  let replyError = $state<string | null>(null);
  $effect(() => guardUnsaved(() => replying && replyText.trim() !== '', { what: 'comment reply' }));
  let resolveBusy = $state(false);

  // Thread status lives on the head comment; replies inherit the container.
  const isHead = $derived(depth === 0);
  const canResolve = $derived(isHead && onresolve != null && comment.thread_id != null);

  // On a phone the desktop 18px-per-level indent quickly eats the narrow width
  // for deep reply chains; halve it under ≤1024 so nested threads stay readable
  // without pushing content off-screen. Tracked via matchMedia (same convention
  // as the other git views) so desktop nesting is unchanged.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  const indent = $derived(Math.min(depth, 4) * (isMobile ? 9 : 18));

  function fmtDate(iso: string): string {
    const d = new Date(iso);
    return d.toLocaleDateString([], { month: 'short', day: 'numeric' }) +
      ' ' +
      d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  async function submitReply(): Promise<void> {
    if (busy || !onreply || replyText.trim() === '') return;
    busy = true;
    replyError = null;
    try {
      await onreply(comment.id, replyText.trim());
      replying = false;
      replyText = '';
    } catch (e) {
      replyError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function toggleResolved(): Promise<void> {
    if (!onresolve || comment.thread_id == null) return;
    resolveBusy = true;
    try {
      await onresolve(comment.thread_id, !comment.resolved);
    } finally {
      resolveBusy = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`cmt`,"style":`margin-inline-start: ${indent}px`,});
   { svelteHTML.createElement("div", { "class":`cmt-head`,});
     { svelteHTML.createElement("span", { "class":`cmt-avatar`,});comment.author.slice(0, 1).toUpperCase(); }
     { svelteHTML.createElement("span", { "class":`cmt-author`,});comment.author; }
    if(comment.path && comment.line !== null && depth === 0){
       { svelteHTML.createElement("span", { "class":`chip mono cmt-loc`,});comment.path; comment.line; }
    }
    if(isHead && comment.resolved){
       { svelteHTML.createElement("span", {   "class":`chip ok cmt-status`,"title":`This thread is resolved on the provider`,});  }
    }
     { svelteHTML.createElement("span", { "class":`cmt-date dim`,});fmtDate(comment.created_at); }
   }
   { svelteHTML.createElement("div", { "class":`cmt-body md-body`,});
    
     renderMarkdown(comment.body);
   }
  if(onreply || canResolve){
     { svelteHTML.createElement("div", { "class":`cmt-actions`,});
      if(replying){
         { svelteHTML.createElement("textarea", {             "dir":`auto`,"class":`input`,"rows":2,"bind:value":replyText,"disabled":busy,"aria-label":`Reply`,"placeholder":`Reply…`,});/*Ωignore_startΩ*/() => replyText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
        if(replyError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: `Couldn’t post reply. ${replyError}`, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`reply-error`,});   replyError;    }}}
         { svelteHTML.createElement("div", {   "class":`row`,"style":`justify-content: flex-end`,});
           { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy,"onclick":() => (replying = false),});  }
           { svelteHTML.createElement("button", {     "class":`btn small primary`,"disabled":busy || replyText.trim() === '',"onclick":submitReply,});
            busy ? 'Posting…' : 'Reply';
           }
         }
      }else{
         { svelteHTML.createElement("div", { "class":`row cmt-action-row`,});
          if(onreply){
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (replying = true),});  }
          }
          if(canResolve){
             { svelteHTML.createElement("button", {         "class":`btn small ghost`,"disabled":resolveBusy,"title":comment.resolved ? 'Reopen this thread on the provider' : 'Mark this thread resolved on the provider',"onclick":toggleResolved,});resolveBusy ? '…' : comment.resolved ? 'Reopen' : 'Resolve'; }
          }
         }
      }
     }
  }
     for(let r of __sveltets_2_ensureArray(comment.replies)){r.id;
     { const $$_daerhTtnemmoC1C = __sveltets_2_ensureComponent(CommentThread); new $$_daerhTtnemmoC1C({ target: __sveltets_2_any(), props: {     "comment":r,onreply,"depth":depth + 1,}});}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CommentThread__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CommentThread__SvelteComponent_ = ReturnType<typeof CommentThread__SvelteComponent_>;
/*Ωignore_endΩ*/export default CommentThread__SvelteComponent_;