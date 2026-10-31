///<reference types="svelte" />
;// The "working" dot for surfaces that only know "an agent is busy right now"
// (assistant, tray, design assist). It is a claim from the live events
// stream, so while that socket is down it must not keep pulsing as if true:
// it flips to the stale "Reconnecting…" ring via sessionState(..., {stale})
// like every other session dot (patterns.md §1).
//
// Pass `label` when the dot is followed by its own words ("Otto is working…")
// and they should flip to "Reconnecting…" together with it.

import StatusDot from './StatusDot.svelte';
import { sessionState } from '../status';
import { events } from '../events.svelte';
;

  interface Props {
    size?: number;
    /** Text shown beside the dot while live; replaced by "Reconnecting…" when stale. */
    label?: string;
    /** Connection of a window with its OWN events socket (the tray's
     *  TopicSocket); omit to follow the main window's events client. */
    live?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { size = 7, label, live }: Props = $props();

  const info = $derived(
    sessionState(null, 'working', false, { stale: live === undefined ? events.state !== 'connected' : !live }),
  );
;
async () => {

 { const $$_toDsutatS0C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS0C({ target: __sveltets_2_any(), props: {   "state":info,size,}});}if(label){' ';info.key === 'stale' ? info.label : label;}
};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LiveWorkingDot__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LiveWorkingDot__SvelteComponent_ = ReturnType<typeof LiveWorkingDot__SvelteComponent_>;
/*Ωignore_endΩ*/export default LiveWorkingDot__SvelteComponent_;