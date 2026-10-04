<script lang="ts">
  // The one "Agent" marker (patterns.md §2): sits beside any text an agent
  // wrote so it never passes for a person's. A neutral `.chip` — agent
  // identity is never the accent colour. AgentByline renders it too.
  interface Props {
    /** Override the tooltip. */
    title?: string;
  }
  let { title = 'Written by an agent — review before you apply it' }: Props = $props();
</script>

<span class="chip agent-chip" {title}>Agent</span>

<style>
  .agent-chip {
    height: 16px;
    padding: 0 6px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
  }
</style>
