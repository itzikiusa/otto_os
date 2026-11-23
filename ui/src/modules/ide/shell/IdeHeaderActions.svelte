<script lang="ts">
  // The IDE header's actions — ONE toolbar row, one primary action (Run), the
  // rest under ⋯ (AGENTS.md "Less is more"; review G13). Everything here is
  // `data-keep`, so PageHeader never adds a second "⋯" of its own:
  //   workbench: [launch cluster: config ▾ · Run · Debug · Stop · ⚙] [Agent] [⋯]
  //   stack:     [Run] [⋯]
  // Search, Git, Maven/Gradle, Share, colour scheme, Settings and Coverage
  // live in the ⋯ menu (ideShellMenus.ts).
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { ide } from '../ideStore.svelte';
  import { ideAgent } from '../store/ideAgentController.svelte';
  import type { IdeStackPane } from './ideLayout.svelte';
  import type { QuickSchemePrefs } from './ideShellCommands';

  const IdeRunToolbarLazy = lazyComponent(() => import('./IdeRunToolbar.svelte'));
  const loadShellMenus = () => import('./ideShellMenus');

  interface Props {
    stack: boolean;
    quickPrefs: QuickSchemePrefs;
    explorerVisible: boolean;
    onToggleExplorer: () => void;
    onTriggerAgent: () => void;
    onAttachFolder: () => void;
    onShowPane: (pane: IdeStackPane) => void;
    onOpenScratches: () => void;
    onToggleAgentDock: () => void;
  }
  let {
    stack,
    quickPrefs = $bindable(),
    explorerVisible,
    onToggleExplorer,
    onTriggerAgent,
    onAttachFolder,
    onShowPane,
    onOpenScratches,
    onToggleAgentDock,
  }: Props = $props();

  function openMore(e: MouseEvent): void {
    const anchor = e.currentTarget instanceof HTMLElement ? e.currentTarget : e;
    void loadShellMenus().then((m) =>
      m.showIdeMoreMenu(anchor, {
        explorerVisible,
        onToggleExplorer,
        onTriggerAgent,
        onAttachFolder,
        stack,
        onShowPane,
        onOpenScratches,
        scheme: { cur: quickPrefs, onChange: (next) => (quickPrefs = next) },
      }),
    );
  }
</script>

{#if ide.hasTarget}
  <LazyMount lazy={IdeRunToolbarLazy} what="launch controls" quiet props={{ compact: stack }} />

  {#if !stack}
    <button
      type="button"
      class="btn small ghost ide-tool-toggle"
      data-keep
      aria-pressed={ideAgent.panelOpen}
      title="Toggle AI Agent (⇧⌘L)"
      onclick={onToggleAgentDock}
      data-testid="ide-toggle-agent"
    >
      <Icon name="assistant" size={13} />
      Agent
      {#if ideAgent.contexts.length > 0}
        <Badge tone="accent" label={String(ideAgent.contexts.length)} />
      {/if}
    </button>
  {/if}
{/if}

<button
  type="button"
  class="icon-btn ide-more-btn"
  data-keep
  aria-label="More actions"
  title="More actions"
  aria-haspopup="menu"
  onclick={openMore}
  data-testid="ide-more-btn"
>
  <Icon name="more" size={14} />
</button>

<style>
  .ide-tool-toggle[aria-pressed='true'] { background: var(--accent-soft); border-color: var(--accent-line); color: var(--accent-text); }
  /* A finger-sized ⋯ on touch screens (the desktop icon button is 28 px). */
  @media (pointer: coarse) {
    .ide-more-btn { min-inline-size: var(--hit-min); min-block-size: var(--hit-min); }
  }
</style>
