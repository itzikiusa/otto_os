<script lang="ts">
  // The "working" dot for surfaces that only know "an agent is busy right now"
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

  interface Props {
    size?: number;
    /** Text shown beside the dot while live; replaced by "Reconnecting…" when stale. */
    label?: string;
    /** Connection of a window with its OWN events socket (the tray's
     *  TopicSocket); omit to follow the main window's events client. */
    live?: boolean;
  }
  let { size = 7, label, live }: Props = $props();

  const info = $derived(
    sessionState(null, 'working', false, { stale: live === undefined ? events.state !== 'connected' : !live }),
  );
</script>

<StatusDot state={info} {size} />{#if label}{' '}{info.key === 'stale' ? info.label : label}{/if}
