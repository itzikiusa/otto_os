<script lang="ts">
  // Bottom checkout / split / branch / access-mode strip beneath the composer
  // in `NewThreadStage.svelte`.
  import type { Repo } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import type { InteractionMode } from '../../lib/providerModels';
  import { openBranchMenu, openCheckoutMenu } from './threadGitMenus';
  import { modeLabel } from './threadPickerMenus';
  import { projectNameFromCwd } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  interface Props {
    repo: Repo | null;
    cleanCwd: string;
    onWorktree: boolean;
    /** The chosen folder's name ("Home" for the home folder). */
    folderName: string;
    isHome: boolean;
    /** Open threads already working in a checkout (the checkout menu's hint). */
    threadsHere?: (path: string) => number;
    openInBeside: boolean;
    branch: string | null;
    modeSupported: boolean;
    /** A runtime agent that takes plan mode (`capabilities.plan`). */
    planSupported?: boolean;
    interactionMode: InteractionMode;
    onPickWorktree: (path: string, branch: string | null, repo: Repo) => void;
    /** "New worktree" chosen: made when the first message is sent (its base branch, and the name given, if any). */
    newWorktree?: { base: string | null; branch: string | null } | null;
    onNewWorktree?: (base: string | null) => void;
    onNameBranch?: () => void;
    onOpenProjectMenu: (anchor: HTMLElement) => void;
    onToggleBeside: () => void;
    onOpenModeMenu: (anchor: HTMLButtonElement) => void;
    onTogglePlan?: () => void;
  }

  let {
    repo,
    cleanCwd,
    onWorktree,
    folderName,
    isHome,
    threadsHere,
    openInBeside,
    branch,
    modeSupported,
    planSupported = false,
    interactionMode,
    onPickWorktree,
    newWorktree = null,
    onNewWorktree,
    onNameBranch,
    onOpenProjectMenu,
    onToggleBeside,
    onOpenModeMenu,
    onTogglePlan,
  }: Props = $props();
</script>

<div class="nt-meta">
  {#if repo}
    <button
      type="button"
      class="nt-meta-btn"
      aria-haspopup="menu"
      title={newWorktree ? 'A new worktree and branch, made when you send' : 'Run in the current checkout or another worktree'}
      data-testid="new-thread-checkout"
      onclick={(e) =>
        void openCheckoutMenu(e.currentTarget, repo, cleanCwd, (path, br) => onPickWorktree(path, br, repo), threadsHere, {
          pending: !!newWorktree,
          onNew: (base) => onNewWorktree?.(base),
          onName: () => onNameBranch?.(),
        })}
    >
      {#if newWorktree}
        <Icon name="worktree" size={13} />
        <span class="ellipsis">New worktree</span>
        <span class="nt-meta-hint mono ellipsis" dir="ltr">{newWorktree.branch ?? (newWorktree.base ? `from ${newWorktree.base}` : '')}</span>
      {:else}
        <Icon name={onWorktree && !cleanCwd.startsWith(repo.path) ? 'worktree' : 'folder'} size={13} />
        <span class="ellipsis">{onWorktree && !cleanCwd.startsWith(repo.path) ? projectNameFromCwd(cleanCwd) : 'Current checkout'}</span>
      {/if}
    </button>
  {:else}
    <button
      type="button"
      class="nt-meta-btn"
      aria-haspopup="menu"
      title={isHome ? `Your home folder (${cleanCwd}), not a project. Choose a folder` : cleanCwd || 'Choose a folder'}
      data-testid="new-thread-folder"
      onclick={(e) => onOpenProjectMenu(e.currentTarget)}
    >
      <Icon name={isHome ? 'home' : 'folder'} size={13} />
      <span class="ellipsis">{folderName || 'Choose folder'}</span>
    </button>
  {/if}

  <span class="nt-spacer"></span>

  {#if threads.paneIds.length > 0}
    <button
      type="button"
      class="nt-meta-btn"
      class:on={openInBeside}
      aria-pressed={openInBeside}
      title="Open the new thread beside the current one (Split)"
      onclick={onToggleBeside}
    >
      <Icon name="split" size={13} />
      <span>Beside</span>
    </button>
  {/if}

  {#if repo && branch && !newWorktree}
    <button
      type="button"
      class="nt-meta-btn mono"
      dir="ltr"
      aria-haspopup="menu"
      title="Git branch: {branch}"
      data-testid="new-thread-branch"
      disabled={onWorktree}
      onclick={(e) => void openBranchMenu(e.currentTarget, repo)}
    >
      <Icon name="branch" size={13} />
      <span class="ellipsis">{branch}</span>
    </button>
  {/if}

  {#if modeSupported}
    <button
      type="button"
      class="nt-meta-btn"
      class:warn={interactionMode === 'plan'}
      aria-haspopup="menu"
      aria-label="Access mode: {modeLabel(interactionMode)}"
      title="Access mode (/mode · ⇧⇥ toggles Plan)"
      data-testid="picker-mode"
      onclick={(e) => onOpenModeMenu(e.currentTarget)}
    >
      <Icon name={interactionMode === 'plan' ? 'compass' : interactionMode === 'accept-edits' ? 'shield' : 'unlock'} size={13} />
      <span>{modeLabel(interactionMode)}</span>
    </button>
  {:else if planSupported}
    <!-- A runtime thread's access is the daemon default; Plan is the one
         mode its agent adds (⇧⇥ toggles it). -->
    <button
      type="button"
      class="nt-meta-btn"
      class:warn={interactionMode === 'plan'}
      aria-pressed={interactionMode === 'plan'}
      title="Plan first: the agent researches and proposes, and changes nothing (⇧⇥)"
      data-testid="picker-plan"
      onclick={() => onTogglePlan?.()}
    >
      <Icon name="compass" size={13} />
      <span>Plan</span>
    </button>
  {/if}
</div>

<style>
  .nt-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    padding-inline: 6px;
    min-inline-size: 0;
  }
  .nt-spacer {
    flex: 1;
  }
  .nt-meta-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    max-inline-size: 240px;
    block-size: 26px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  @media (pointer: coarse) {
    .nt-meta-btn {
      block-size: 44px;
    }
  }
  .nt-meta-btn:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .nt-meta-btn:disabled {
    cursor: default;
  }
  .nt-meta-hint {
    color: var(--text-dim);
    opacity: 0.8;
  }
  .nt-meta-btn.on {
    color: var(--accent-text);
  }
  .nt-meta-btn.warn {
    color: var(--warning);
  }
</style>
