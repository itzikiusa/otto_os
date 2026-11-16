<script lang="ts">
  // The calm start state (T3 Code's draft hero): one heading that names the
  // project, ONE composer (RuntimeStartComposer — the same one every embed's
  // draft state uses) and `NewThreadMetaRow` beneath it. This host owns the
  // project / workspace choice and what a start does in Threads.
  import { api } from '../../lib/api/client';
  import type { Repo } from '../../lib/api/types';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { checkFolder } from '../../lib/folderCheck';
  import { auth } from '../../lib/stores/auth.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { transcript } from '../../lib/stores/transcript.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import { buildFolderMenuItems, buildWorkspaceMenuItems } from './newThreadMenus';
  import NewThreadMetaRow from './NewThreadMetaRow.svelte';
  import { council } from './runtime/councilStore.svelte';
  import { isRuntimeProvider, runtimeThreads } from './runtime/runtimeThreads.svelte';
  import RuntimeStartComposer from './RuntimeStartComposer.svelte';
  import type { StartLaunch } from './startComposer';
  import { placeholderTitle } from './threadTitle';
  import { FolderPickerModal, ProjectCardView } from './threadLazy.svelte';
  import { threadRepos } from './threadRepos.svelte';
  import { harnessLabel, projectNameFromCwd } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  interface Props {
    showCard?: boolean;
  }
  let { showCard = false }: Props = $props();

  let cwd = $state('');
  let useScratch = $state(false);
  let cwdError = $state('');
  let folderPicker = $state(false);
  let openInBeside = $state(false);
  let worktree = $state<{ path: string; branch: string | null; repo: Repo } | null>(null);
  let projectEl = $state<HTMLButtonElement | null>(null);

  const homePath = $derived(ws.scratch?.root_path ?? '~');
  const scratchActive = $derived(useScratch || ws.current === null);
  const targetWs = $derived(scratchActive ? SCRATCH_WORKSPACE_ID : (ws.currentId ?? SCRATCH_WORKSPACE_ID));
  const cleanCwd = $derived(cwd.trim().replace(/\/+$/, ''));
  const stageDraftKey = $derived(`threads-stage:${cleanCwd || targetWs}`);
  const onWorktree = $derived(worktree !== null && worktree.path.replace(/\/+$/, '') === cleanCwd);
  const repo = $derived(onWorktree ? worktree!.repo : threadRepos.repoFor(cleanCwd, scratchActive ? null : ws.currentId));
  const branch = $derived(onWorktree ? worktree!.branch : repo ? (git.statusById[repo.id]?.branch ?? null) : null);
  const projectName = $derived(
    !cleanCwd || cleanCwd === homePath || cleanCwd === '~' ? null : projectNameFromCwd(cleanCwd),
  );

  $effect(() => {
    const seeded = threads.draftCwd;
    if (seeded) {
      cwd = seeded;
      useScratch = false;
    } else if (!cwd) {
      cwd = threads.lastProject ?? (scratchActive ? homePath : (ws.current?.root_path ?? homePath));
    }
  });

  $effect(() => {
    threads.stageCwd = cleanCwd || null;
  });

  $effect(() => {
    if (repo && !onWorktree) git.ensureStatus(repo.id);
  });

  function setCwd(path: string): void {
    cwd = path;
    cwdError = '';
  }

  function openProjectMenu(anchor: HTMLElement): void {
    const items = buildFolderMenuItems({
      scratchActive,
      currentWorkspace: ws.current,
      repos: git.repos,
      sessions: ws.sessions,
      cwd,
      homePath,
      onPickPath: setCwd,
      onBrowse: () => (folderPicker = true),
    });
    items.push(
      { separator: true },
      {
        label: 'Switch workspace…',
        icon: 'layers',
        hint: scratchActive ? 'Scratch' : (ws.current?.name ?? undefined),
        action: () => queueMicrotask(() => openWorkspaceMenu(anchor)),
      },
    );
    ctxMenu.showAt(anchor, items);
  }

  function openWorkspaceMenu(anchor: HTMLElement): void {
    ctxMenu.showAt(
      anchor,
      buildWorkspaceMenuItems({
        workspaces: ws.workspaces,
        currentId: ws.currentId,
        scratchActive,
        homePath,
        onSelectWorkspace: (w) => {
          useScratch = false;
          setCwd(w.root_path);
          void ws.select(w.id);
        },
        onSelectScratch: () => {
          useScratch = true;
          setCwd(homePath);
        },
      }),
    );
  }

  async function launch(req: StartLaunch): Promise<boolean> {
    const { provider } = req;
    let dir = cwd.trim();
    if (homePath !== '~' && (dir === '~' || dir.startsWith('~/'))) dir = homePath + dir.slice(1);
    if (dir !== '') {
      cwdError = '';
      const check = await checkFolder(dir, (url) => api.get(url));
      if (!check.ok) {
        cwdError = check.message;
        return false;
      }
    }
    const firstMsg = req.message;
    try {
      const meta: Record<string, unknown> = {
        surface: 'threads',
        interaction_mode: req.interactionMode,
        ...(req.model ? { model: req.model } : {}),
        ...(req.effort ? { effort: req.effort } : {}),
      };
      const title = placeholderTitle(req.titleSeed, `${harnessLabel(provider)} thread`);
      const opts = { scratch: scratchActive && ws.currentId !== SCRATCH_WORKSPACE_ID };
      const created = req.council
        ? (await council.start(targetWs, { prompt: firstMsg, title, cwd: dir || null }, req.background ? 'background' : 'panes'))[0]
        : req.runtime && isRuntimeProvider(provider, auth.meta)
          ? await runtimeThreads.start(targetWs, {
              provider,
              title,
              titleAuto: !!req.titleSeed,
              cwd: dir || null,
              model: req.model,
              effort: req.effort ?? undefined,
              plan: req.plan,
              prompt: firstMsg,
              attachments: req.attachments,
            })
          : firstMsg
            ? await ws.openSessionWithPrompt({ provider, title, cwd: dir || null, prompt: firstMsg, meta }, opts)
            : await ws.createSessionQuiet({ kind: 'agent', provider, title, cwd: dir || null, meta }, opts);
      if (firstMsg && !req.runtime && !req.council) transcript.recordOptimisticPrompt(created.id, firstMsg);
      threads.draftCwd = null;
      threads.rememberProject(dir || null);
      if (openInBeside && threads.paneIds.length > 0 && !req.council) threads.openBeside(created.id);
      else if (!req.council) threads.selectThread(created.id);
      return true;
    } catch (e) {
      toastError('Couldn’t start thread', e);
      return false;
    }
  }

  function escape(): boolean {
    if (threads.paneIds.length === 0) return false;
    threads.newDraftMode = false;
    return true;
  }
</script>

<div class="nt-stage" class:with-card={showCard} data-testid="new-thread-stage">
  {#if showCard && ProjectCardView.component}
    <div class="nt-card-slot">
      <ProjectCardView.component floating cwd={cleanCwd} onpickfolder={openProjectMenu} />
    </div>
  {/if}

  <div class="nt-inner">
    <h1 class="nt-title">
      {#if projectName}
        What should we build in <button
          bind:this={projectEl}
          type="button"
          class="nt-project"
          aria-haspopup="menu"
          title="Change project ({cleanCwd})"
          data-testid="new-thread-project"
          onclick={() => projectEl && openProjectMenu(projectEl)}>{projectName}</button>?
      {:else}
        What should we work on?
      {/if}
    </h1>

    <RuntimeStartComposer
      cwd={cleanCwd}
      pickerStart={homePath}
      draftKey={stageDraftKey}
      rememberScope="threads:{targetWs}"
      error={cwdError}
      onlaunch={launch}
      onescape={escape}
    >
      {#snippet below(m)}
        <NewThreadMetaRow
          {repo}
          {cleanCwd}
          {onWorktree}
          {scratchActive}
          {projectName}
          {openInBeside}
          {branch}
          modeSupported={m.modeSupported}
          planSupported={m.planSupported}
          interactionMode={m.interactionMode}
          onPickWorktree={(path, br, r) => {
            worktree = { path, branch: br, repo: r };
            setCwd(path);
          }}
          onOpenProjectMenu={openProjectMenu}
          onToggleBeside={() => (openInBeside = !openInBeside)}
          onOpenModeMenu={(anchor) => m.openModeMenu(anchor)}
          onTogglePlan={() => m.setMode(m.interactionMode === 'plan' ? 'full' : 'plan')}
        />
      {/snippet}
    </RuntimeStartComposer>
  </div>
</div>

{#if folderPicker && FolderPickerModal.component}
  <FolderPickerModal.component
    title="Choose project folder"
    files={false}
    start={cleanCwd || homePath || ''}
    onpick={(picked) => {
      setCwd(picked);
      folderPicker = false;
    }}
    onclose={() => (folderPicker = false)}
  />
{/if}

<style>
  .nt-stage {
    position: relative;
    flex: 1;
    min-inline-size: 0;
    min-block-size: 0;
    overflow-y: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px 16px;
    background: var(--bg);
  }
  .nt-card-slot {
    position: absolute;
    inset-block-start: 16px;
    inset-inline-end: 16px;
    inline-size: 300px;
    z-index: var(--z-sticky);
  }
  .nt-inner {
    inline-size: 100%;
    max-inline-size: 720px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .nt-title {
    margin-block: 0 6px;
    overflow-wrap: anywhere;
    font-size: var(--fs-hero);
    font-weight: 400;
    letter-spacing: -0.01em;
    line-height: 1.25;
    color: var(--text);
    text-align: center;
  }
  .nt-project {
    max-inline-size: 100%;
    padding-inline: 2px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-weight: 500;
    cursor: pointer;
  }
  .nt-project:hover {
    background: var(--hover);
  }
  @media (max-width: 640px) {
    .nt-title {
      font-size: var(--fs-2xl);
    }
  }
</style>
