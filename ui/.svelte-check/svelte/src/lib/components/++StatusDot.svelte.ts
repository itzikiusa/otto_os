///<reference types="svelte" />
;// Session status dot. Every surface routes through `sessionState()` (lib/
// status.ts) so one state draws one dot everywhere: pass `state` when the
// caller already derived it (sidebar, tabs, tiles, pane header), or the raw
// `status` + `needsYou` and the dot derives it. Amber is reserved for
// "needs you"; a suspended (resumable) session is a hollow idle ring.

import type { SessionStatus } from '../api/types';
import { sessionState, type SessionStateInfo } from '../status';
;

  interface Props {
    status?: SessionStatus;
    size?: number;
    /** When true, renders an amber pulsing dot regardless of `status` — the
     *  session is blocked on operator input (distinct from plain idle). */
    needsYou?: boolean;
    /** A pre-derived state (wins over `status`/`needsYou`). */
    state?: SessionStateInfo;
  };function $$render() {

  
  
  
  
  
  
  

  let { status = 'idle', size = 7, needsYou = false, state }: Props = $props();

  const info = $derived(state ?? sessionState(null, status, needsYou));
;
async () => {

  { svelteHTML.createElement("span", {             "class":`dot ${info.key}`,"style":`width:${size}px;height:${size}px`,"role":`img`,"aria-label":info.label,"title":info.hint ?? info.label,...__sveltets_2_empty({"data-state":info.key}),});info.live; }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StatusDot__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StatusDot__SvelteComponent_ = ReturnType<typeof StatusDot__SvelteComponent_>;
/*Ωignore_endΩ*/export default StatusDot__SvelteComponent_;