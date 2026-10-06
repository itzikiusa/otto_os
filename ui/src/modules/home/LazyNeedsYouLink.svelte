<script lang="ts">
  // NeedsYouLink, loaded after its host page (Mission Control, MCP approvals,
  // the Assistant rail, Agents → Work Queue). The link pulls the Today store,
  // which joins every "needs you" source (assistant, MCP, scheduled tasks,
  // design and mission-control APIs); imported statically it put all of that
  // into each host page's chunk — Mission Control +17 %, MCP +9 % gzip vs main
  // (bundle budget). It is a small secondary affordance, so nothing renders
  // while its chunk loads (no skeleton flash); a failed load leaves it out —
  // Home → Needs you stays one sidebar click away.
  interface Props {
    /** Extra class for placement tweaks by the host surface. */
    class?: string;
  }
  let { class: cls = '' }: Props = $props();

  const link = import('./NeedsYouLink.svelte');
</script>

{#await link then m}
  <m.default class={cls} />
{:catch}
  <!-- left out on a failed chunk load (see above) -->
{/await}
