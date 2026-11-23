<script lang="ts">
  // The calm start state (T3 Code's draft hero): one heading that names the
  // project, ONE composer (RuntimeStartComposer — the same one every embed's
  // draft state uses) and `NewThreadMetaRow` beneath it. This host owns the
  // project / workspace choice and what a start does in Threads.
  import { onMount, untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { Repo, RuntimeAccessMode } from '../../lib/api/types';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { checkFolder } from '../../lib/folderCheck';
  import { auth } from '../../lib/stores/auth.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { transcript } from '../../lib/stores/transcript.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import { accessCaution, canonicalPath, defaultStageFolder, folderLabel, stageAccess, threadsInCheckout } from './newThreadDefaults';
  import { buildFolderMenuItems, buildWorkspaceMenuItems } from './newThreadMenus';
  import NewThreadMetaRow from './NewThreadMetaRow.svelte';
  import { council } from './runtime/councilStore.svelte';
  import { isRuntimeProvider, runtimeThreads } from './runtime/runtimeThreads.svelte';
  import RuntimeStartComposer from './RuntimeStartComposer.svelte';
  import type { StartLaunch } from './startComposer';
  import { placeholderTitle } from './threadTitle';
  import { FolderPickerModal } from './threadLazy.svelte';
  import { threadAccessDefault } from './threadAccessDefault.svelte';
  import { BRANCH_NAME_OK, createWorktree, runWorktreeSetup, takenBranches, worktreeBase } from './threadGitMenus';
  import { preparedNote, setupFailure } from './worktreeReady';
  import { toasts } from '../../lib/toast.svelte';
  import { worktreeBranch } from './worktreeBranch';
  import { confirmer } from '../../lib/confirm.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { announceShown } from '../../lib/announce.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { saveTransientComposerDraft } from '../../lib/stores/runtimeDraftCursor';
  import { threadRepos } from './threadRepos.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
  import { harnessLabel, isThreadSession } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  // Null until the person picks: the folder then follows the workspace as it
  // loads (newThreadDefaults.ts), so a late workspace never leaves the stage
  // silently in the home folder.
  let pickedCwd = $state<string | null>(null);
  let pickedAccess = $state<RuntimeAccessMode | null>(null);
  let useScratch = $state(false);
  let cwdError = $state('');
  let folderPicker = $state(false);
  let openInBeside = $state(false);
  let worktree = $state<{ path: string; branch: string | null; repo: Repo } | null>(null);
  let projectEl = $state<HTMLButtonElement | null>(null);
  // "New worktree": only the intent until the first message is sent — then
  // the branch is named from it (worktreeBranch.ts) and the worktree made,
  // with progress here and, on failure, the draft kept and Retry beside the error.
  let newWt = $state<{ repo: Repo; base: string | null; branch: string | null } | null>(null);
  let wtProgress = $state<string | null>(null);
  let wtError = $state<string | null>(null);
  let composer = $state<{ retry(): Promise<void>; draftText(): string } | null>(null);

  const homePath = $derived(ws.scratch?.root_path ?? '~');
  const scratchActive = $derived(useScratch || ws.current === null);
  const targetWs = $derived(scratchActive ? SCRATCH_WORKSPACE_ID : (ws.currentId ?? SCRATCH_WORKSPACE_ID));
  const wsRepos = $derived(scratchActive ? [] : git.repos.filter((r) => r.workspace_id === ws.currentId));
  const recentCwds = $derived(
    [...Object.values(runtimeThreads.byId), ...ws.sessions.filter((s) => isThreadSession(s))]
      .sort((a, b) => b.last_active_at.localeCompare(a.last_active_at))
      .map((s) => s.cwd ?? '')
      .filter(Boolean),
  );
  const fallback = $derived(
    defaultStageFolder({
      workspace: scratchActive ? null : ws.current,
      repos: wsRepos,
      recentCwds,
      lastProject: threadsChrome.lastProjectByWs[targetWs] ?? (scratchActive ? null : threads.lastProject),
      homePath,
    }),
  );
  const cwd = $derived(pickedCwd ?? fallback.path);
  const cleanCwd = $derived(cwd.trim().replace(/\/+$/, ''));
  const stageDraftKey = $derived(`threads-stage:${cleanCwd || targetWs}`);
  const onWorktree = $derived(worktree !== null && worktree.path.replace(/\/+$/, '') === cleanCwd);
  const repo = $derived(onWorktree ? worktree!.repo : threadRepos.repoFor(cleanCwd, scratchActive ? null : ws.currentId));
  const branch = $derived(onWorktree ? worktree!.branch : repo ? (git.statusById[repo.id]?.branch ?? null) : null);
  const isHome = $derived(!cleanCwd || cleanCwd === '~' || canonicalPath(cleanCwd) === canonicalPath(homePath));
  const folderName = $derived(folderLabel(cleanCwd, { homePath, repos: [...wsRepos, ...(repo ? [repo] : [])], workspaces: ws.workspaces }));
  const projectName = $derived(isHome ? null : folderName);
  // What the first turn runs with, shown on the chip and sent with the start.
  const accessMode = $derived(stageAccess(pickedAccess, threadAccessDefault.skipPermissions));
  const caution = $derived(accessCaution(accessMode, { home: isHome, repo: repo ? true : threadRepos.checked(cleanCwd) ? false : null }));
  // Another live, writable thread in this checkout (T2-17).
  const checkoutRoot = $derived(onWorktree ? worktree!.path : (repo?.path ?? cleanCwd));
  const busyHere = $derived(isHome ? [] : threadsInCheckout(runtimeThreads.live(), checkoutRoot));
  const threadsHere = (path: string): number => threadsInCheckout(runtimeThreads.live(), path).length;

  onMount(() => threadAccessDefault.ensure());

  $effect(() => {
    const seeded = threads.draftCwd;
    if (seeded) {
      pickedCwd = seeded;
      useScratch = false;
    }
  });

  // Another workspace opened (the shell's switcher): its own default applies.
  let seenWs: string | null = null;
  $effect(() => {
    const id = ws.currentId;
    if (seenWs !== null && id !== seenWs) untrack(() => (pickedCwd = threads.draftCwd));
    seenWs = id;
  });

  $effect(() => {
    threads.stageCwd = cleanCwd || null;
  });

  $effect(() => {
    if (repo && !onWorktree) git.ensureStatus(repo.id);
  });

  function setCwd(path: string): void {
    pickedCwd = path;
    cwdError = '';
    newWt = null;
    wtError = null;
  }

  function chooseNewWorktree(r: Repo, base: string | null): void {
    newWt = { repo: r, base, branch: null };
    wtError = null;
  }

  async function nameBranch(): Promise<void> {
    const wt = newWt;
    if (!wt) return;
    let initial = wt.branch;
    if (!initial) {
      const taken = await takenBranches(wt.repo).catch(() => ({ branches: [] as string[], base: null }));
      initial = worktreeBranch({ prompt: composer?.draftText() ?? '', branches: taken.branches, defaultBranch: taken.base ?? wt.base, now: new Date() });
    }
    const name = await confirmer.promptText(`The new worktree’s branch${wt.base ? `, from ${wt.base}` : ''}. It is made when you send.`, {
      title: 'Name the branch',
      confirmLabel: 'Use this name',
      initial,
      validate: BRANCH_NAME_OK,
    });
    if (name && newWt === wt) newWt = { ...wt, branch: name.trim() };
  }

  /** A worktree made whose setup command has not succeeded yet: Retry runs
   *  only the setup ("Start without setup" skips it); the message stays. */
  let pendingSetup = $state<{ of: NonNullable<typeof newWt>; path: string; branch: string | null; name: string; skip?: boolean } | null>(null);

  /** Make the chosen new worktree (and run the repo's setup in it); its path,
   *  or null (the error is shown). */
  async function makeWorktree(req: StartLaunch): Promise<string | null> {
    const wt = newWt!;
    wtError = null;
    try {
      if (pendingSetup?.of !== wt) {
        wtProgress = 'Naming the branch…';
        const taken = await takenBranches(wt.repo);
        const name = wt.branch ?? worktreeBranch({ prompt: req.withPrompt ? req.titleSeed : '', branches: taken.branches, defaultBranch: taken.base ?? wt.base, now: new Date() });
        wtProgress = `Creating worktree ${name}${wt.base ? ` from ${wt.base}` : ''}…`;
        const w = await createWorktree(wt.repo, name, wt.base);
        const note = preparedNote(w.prepared);
        if (note) toasts.info('Worktree made', note);
        pendingSetup = { of: wt, path: w.path, branch: w.branch, name: w.prepared?.setup ?? '' };
      }
      const ps = pendingSetup!;
      if (ps.name && !ps.skip) {
        wtProgress = `Setting up: ${ps.name}…`;
        const r = await runWorktreeSetup(wt.repo, ps.path);
        if (!r.ok) throw new Error(setupFailure(r));
      }
      pendingSetup = null;
      pickWorktree(ps.path, ps.branch, wt.repo);
      return ps.path;
    } catch (e) {
      wtError = loadErrorText(e);
      return null;
    } finally {
      wtProgress = null;
    }
  }

  function pickWorktree(path: string, br: string | null, r: Repo): void {
    worktree = { path, branch: br, repo: r };
    setCwd(path);
  }

  function openProjectMenu(anchor: HTMLElement): void {
    const items = buildFolderMenuItems({
      scratchActive,
      currentWorkspace: ws.current,
      repos: wsRepos,
      sessions: ws.sessions,
      cwd,
      homePath,
      recentCwds,
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
        // The chosen workspace's default (its last project or repo) applies.
        onSelectWorkspace: (w) => {
          useScratch = false;
          pickedCwd = null;
          cwdError = '';
          void ws.select(w.id);
        },
        onSelectScratch: () => {
          useScratch = true;
          pickedCwd = null;
          cwdError = '';
        },
      }),
    );
  }

  async function launch(req: StartLaunch): Promise<boolean> {
    const { provider } = req;
    // A new worktree moves the stage (and its draft key) to the worktree's folder.
    const draftKeyBefore = stageDraftKey;
    let dir = cwd.trim();
    if (homePath !== '~' && (dir === '~' || dir.startsWith('~/'))) dir = homePath + dir.slice(1);
    // Every provider starts after this: the worktree comes first.
    if (newWt) {
      const made = await makeWorktree(req);
      if (!made) return false;
      dir = made;
    }
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
              accessMode: req.accessMode ?? undefined,
              prompt: firstMsg,
              attachments: req.attachments,
            })
          : firstMsg
            ? await ws.openSessionWithPrompt({ provider, title, cwd: dir || null, prompt: firstMsg, meta }, opts)
            : await ws.createSessionQuiet({ kind: 'agent', provider, title, cwd: dir || null, meta }, opts);
      if (firstMsg && !req.runtime && !req.council) transcript.recordOptimisticPrompt(created.id, firstMsg);
      threads.draftCwd = null;
      threads.rememberProject(dir || null);
      threadsChrome.rememberProject(targetWs, dir || null);
      if (openInBeside && threads.paneIds.length > 0 && !req.council) threads.openBeside(created.id);
      else if (!req.council) threads.selectThread(created.id);
      if (stageDraftKey !== draftKeyBefore) saveTransientComposerDraft(draftKeyBefore, '');
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

<div class="nt-stage" data-testid="new-thread-stage">
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
      bind:this={composer}
      cwd={cleanCwd}
      pickerStart={homePath}
      draftKey={stageDraftKey}
      rememberScope="threads:{targetWs}"
      error={cwdError}
      {accessMode}
      onaccess={(m) => (pickedAccess = m)}
      onlaunch={launch}
      onescape={escape}
      prefill={threads.draftText}
      onprefilled={() => (threads.draftText = null)}
    >
      {#snippet below(m)}
        <NewThreadMetaRow
          {repo}
          {cleanCwd}
          {onWorktree}
          {folderName}
          {isHome}
          {threadsHere}
          {openInBeside}
          {branch}
          modeSupported={m.modeSupported}
          planSupported={m.planSupported}
          interactionMode={m.interactionMode}
          onPickWorktree={pickWorktree}
          newWorktree={newWt ? { base: newWt.base, branch: newWt.branch } : null}
          onNewWorktree={(base) => repo && chooseNewWorktree(repo, base)}
          onNameBranch={() => void nameBranch()}
          onOpenProjectMenu={openProjectMenu}
          onToggleBeside={() => (openInBeside = !openInBeside)}
          onOpenModeMenu={(anchor) => m.openModeMenu(anchor)}
          onTogglePlan={() => m.setMode(m.interactionMode === 'plan' ? 'full' : 'plan')}
        />
        {#if caution && m.accessShown}
          <p class="nt-note" data-testid="new-thread-access-caution">{caution}</p>
        {/if}
        {#if wtProgress}
          <p class="nt-note nt-progress" use:announceShown={wtProgress} data-testid="new-thread-worktree-progress"><span class="spinner" aria-hidden="true"></span>{wtProgress}</p>
        {:else if wtError}
          <div data-testid="new-thread-worktree-failed">
            <LoadState what="the worktree" errorTitle={pendingSetup ? 'The worktree’s setup failed — your message is still here' : 'Couldn’t create the worktree — your message is still here'} variant="compact" error={wtError} empty onretry={() => void composer?.retry()} />
            {#if pendingSetup}<button type="button" class="btn small ghost" data-testid="new-thread-skip-setup" onclick={() => { pendingSetup!.skip = true; void composer?.retry(); }}>Start without setup</button>{/if}
          </div>
        {/if}
        {#if busyHere.length > 0 && !newWt}
          <p class="nt-note" data-testid="new-thread-same-checkout">
            {busyHere.length === 1 ? 'Another thread is' : `${busyHere.length} threads are`} working in this {repo ? 'checkout' : 'folder'}{#if repo}{' · '}<button type="button" class="nt-link" onclick={() => { const r = repo; if (r) void worktreeBase(r).then((b) => chooseNewWorktree(r, b)); }}>Use a new worktree</button>{/if}
          </p>
        {/if}
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
  .nt-inner {
    inline-size: 100%;
    max-inline-size: 760px;
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
  .nt-note {
    margin: -8px 0 0;
    padding-inline: 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .nt-progress {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }
  .nt-link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .nt-link:hover {
    text-decoration: underline;
  }
  @media (max-width: 640px) {
    .nt-title {
      font-size: var(--fs-2xl);
    }
  }
</style>
