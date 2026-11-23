<!--
  The daemon's notice about subagents of this thread (runtime/children.rs).
  "You asked Codex: …" is the turn's subagent row (RuntimeTimelineRows groups
  it with the agent's own delegations; this is the fallback outside a
  timeline). "Codex finished: …" draws nothing: that row already says how
  each subagent ended, and its answer reached the agent as "Reply from".
-->
<script lang="ts">
  import type { ThreadNotice } from './childThreads';
  import RuntimeChildGroup from './RuntimeChildGroup.svelte';

  let { notice }: { notice: ThreadNotice } = $props();
</script>

{#if notice.event === 'started'}
  <RuntimeChildGroup kids={notice.ids.map((id) => ({ kind: 'thread' as const, id }))} />
{/if}
