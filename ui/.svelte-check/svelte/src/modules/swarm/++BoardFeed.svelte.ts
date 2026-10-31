///<reference types="svelte" />
;
import { toastError } from '../../lib/toastError';
// The shared surface: a live feed of agent + user board posts, with a composer.
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import AgentChip from '../../lib/components/AgentChip.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { sentenceCase } from '../../lib/status';
import { toasts } from '../../lib/toast.svelte';
import type { MessageKind } from './types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let kindFilter = $state<string>('');
  let draft = $state('');
  let draftKind = $state<MessageKind>('message');
  // Target a specific agent ('' = the whole team). Lets you tag someone and tell
  // them what to do, or reply to an agent's post.
  let draftTo = $state<string>('');
  let composerEl = $state<HTMLInputElement | null>(null);

  const agents = $derived(swarm.detail?.agents ?? []);

  const KINDS: MessageKind[] = [
    'message',
    'idea',
    'review_request',
    'review',
    'decision',
    'status',
    'concern',
    'escalation',
    'handoff',
    'system',
    // Coordinator lifecycle posts.
    'worktree',
    'shared',
    'merge',
    'verify',
  ];

  const filtered = $derived(
    kindFilter ? swarm.board.filter((m) => m.kind === kindFilter) : swarm.board,
  );

  function author(m: { author_agent_id?: string | null; author_user_id?: string | null }): string {
    if (m.author_agent_id) return swarm.agentById(m.author_agent_id)?.name ?? 'agent';
    if (m.author_user_id) return 'You';
    return 'system';
  }

  // Colour carries meaning only: a concern / shared-file clash wants a look
  // (warning), an escalation is blocked (danger), a merge landed (success).
  // Every other kind is neutral — the icon + word tell them apart.
  const KIND_TONE: Record<string, 'warn' | 'bad' | 'ok'> = {
    concern: 'warn',
    shared: 'warn',
    escalation: 'bad',
    merge: 'ok',
  };

  const KIND_ICON: Record<string, IconName> = {
    message: 'comment',
    idea: 'bulb',
    review_request: 'eye',
    review: 'eye',
    decision: 'check',
    status: 'info',
    concern: 'warning',
    escalation: 'warning',
    handoff: 'send',
    system: 'gear',
    worktree: 'worktree',
    shared: 'warning',
    merge: 'merge',
    verify: 'search',
  };
  const FILTER_KINDS = ['idea', 'review', 'decision', 'concern', 'status', 'worktree', 'shared', 'merge', 'verify', 'escalation'];

  let posting = $state(false);
  async function post() {
    if (!draft.trim() || posting) return;
    const submitted = draft;
    posting = true;
    try {
      await swarm.postBoard({
        body: submitted.trim(),
        kind: draftKind,
        project_id: swarm.selectedProjectId ?? undefined,
        to_agent_id: draftTo || undefined,
      });
      if (draft === submitted) draft = '';
    } catch (e) {
      // Keep the draft so nothing typed is lost; say why it didn't post.
      toastError("Couldn’t post to the board", e);
    } finally {
      posting = false;
    }
  }

  // Reply to a message: target its author and focus the composer.
  function reply(m: { author_agent_id?: string | null }) {
    draftTo = m.author_agent_id ?? '';
    draftKind = 'message';
    composerEl?.focus();
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`board`,});
   { svelteHTML.createElement("div", {     "class":`b-filters`,"role":`group`,"aria-label":`Filter posts by kind`,});
     { svelteHTML.createElement("button", {      "class":`chip`,"aria-pressed":kindFilter === '',"onclick":() => (kindFilter = ''),});kindFilter === '';  }
       for(let k of __sveltets_2_ensureArray(FILTER_KINDS)){k;
       { svelteHTML.createElement("button", {      "class":`chip`,"aria-pressed":kindFilter === k,"onclick":() => (kindFilter = k),});kindFilter === k;
        if(KIND_ICON[k]){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":KIND_ICON[k],"size":12,}});}} sentenceCase(k);
       }
    }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => swarm.loadBoard(),"aria-label":`Refresh board`,"title":`Refresh board`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});} }
   }

   { svelteHTML.createElement("div", { "class":`feed`,});
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`board posts`,"loading":swarm.boardLoading,"error":swarm.boardError,"empty":swarm.board.length === 0,"onretry":() => void swarm.loadBoard(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`comment`,"title":`Quiet board`,"body":`Agents post ideas, reviews and decisions here as they work.`,}});}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
      
    if(filtered.length === 0){
      if(kindFilter){
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`comment`,"title":`No ${sentenceCase(kindFilter).toLowerCase()} posts`,"body":`Nothing of this kind on the board yet — pick All to see every post.`,}});}
      }else{
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`comment`,"title":`Quiet board`,"body":`Agents post ideas, reviews and decisions here as they work.`,}});}
      }
    }
       for(let m of __sveltets_2_ensureArray(filtered)){m.id;
       { svelteHTML.createElement("div", {  "class":`msg`,});!!m.author_agent_id;
         { svelteHTML.createElement("div", { "class":`msg-head`,});
           { svelteHTML.createElement("span", { "class":`chip kind-chip tone-${KIND_TONE[m.kind] ?? 'neutral'}`,});if(KIND_ICON[m.kind]){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":KIND_ICON[m.kind],"size":12,}});}} sentenceCase(m.kind); }
           { svelteHTML.createElement("span", { "class":`who`,});author(m); }if(m.author_agent_id){ { const $$_pihCtnegA5C = __sveltets_2_ensureComponent(AgentChip); new $$_pihCtnegA5C({ target: __sveltets_2_any(), props: {}});}}
          if(m.to_agent_id){ { svelteHTML.createElement("span", { "class":`dim to`,}); swarm.agentById(m.to_agent_id)?.name ?? 'an agent'; }}
           { svelteHTML.createElement("span", { "class":`grow`,}); }
          if(m.author_agent_id){
             { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => reply(m),"title":`Reply to ${author(m)}`,});  }
          }
           { svelteHTML.createElement("span", {   "class":`dim time`,"title":new Date(m.created_at).toLocaleString(),});rel(m.created_at); }
         }
         { svelteHTML.createElement("div", { "class":`msg-body`,});m.body; }
       }
    }
     LoadState}
   }

   { svelteHTML.createElement("div", { "class":`composer`,});
     { svelteHTML.createElement("select", {       "class":`input small`,"bind:value":draftTo,"aria-label":`Post to`,"title":`Who is this for?`,});/*Ωignore_startΩ*/() => draftTo = __sveltets_2_any(null);/*Ωignore_endΩ*/
       { svelteHTML.createElement("option", {"value":"",});  }
         for(let a of __sveltets_2_ensureArray(agents)){a.id; { svelteHTML.createElement("option", { "value":a.id,});a.name; }}
     }
     { svelteHTML.createElement("select", {       "class":`input small`,"bind:value":draftKind,"aria-label":`Kind of post`,"title":`Kind of post`,});/*Ωignore_startΩ*/() => draftKind = __sveltets_2_any(null);/*Ωignore_endΩ*/
         for(let k of __sveltets_2_ensureArray(KINDS)){k; { svelteHTML.createElement("option", { "value":k,});sentenceCase(k); }}
     }
     { const $$_input2 = svelteHTML.createElement("input", {             "dir":`auto`,"class":`input grow`,"aria-label":`Message`,"placeholder":draftTo ? `Tell ${swarm.agentById(draftTo)?.name ?? 'them'} what to do…` : 'Post to the team board…',"bind:value":draft,"onkeydown":(e) => e.key === 'Enter' && post(),});composerEl = $$_input2;/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":post,"disabled":!draft.trim() || posting,"title":draft.trim() ? undefined : 'Type a message to post',});
      posting ? 'Posting…' : 'Post';
     }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BoardFeed__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BoardFeed__SvelteComponent_ = ReturnType<typeof BoardFeed__SvelteComponent_>;
/*Ωignore_endΩ*/export default BoardFeed__SvelteComponent_;