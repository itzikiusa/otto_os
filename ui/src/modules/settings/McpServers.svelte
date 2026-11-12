<script lang="ts">
  // Settings → MCP servers moved to the MCP Control Plane, the one home for
  // MCP servers: its Servers list shows native servers (launched by the agent
  // CLI from `.mcp.json`) beside governed ones, with add / edit / enable /
  // remove and the switch between the two; the `otto` server toggle is
  // "Attach to sessions" on the Control Plane's Otto server page. This route
  // stays so old links, bookmarks and ⌘K land there instead of on an unknown
  // section.
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { router } from '../../lib/router.svelte';
  import { movedSectionRoute, sectionLabel } from './sections';

  const to = movedSectionRoute('mcp-servers') ?? 'mcp/servers';
  $effect(() => {
    router.replace(to);
  });
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('mcp-servers')} subtitle="Now in the MCP Control Plane" />
  <PageBody width="readable">
    <EmptyState
      icon="server"
      title="MCP servers live in the MCP Control Plane"
      body="Add, edit, enable and remove every MCP server there, native or governed."
      actionLabel="Open MCP servers"
      onaction={() => router.replace(to)}
    />
  </PageBody>
</div>

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
</style>
