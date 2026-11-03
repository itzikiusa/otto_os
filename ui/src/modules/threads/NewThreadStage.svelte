<script lang="ts">
  // The calm start state (T3 Code's draft hero): one heading that names the
  // project, ONE composer — attach on the start, a single agent chip (harness ·
  // model · effort, Panda-style) and a round send on the end — and a quiet meta
  // row under it: checkout on the start, branch + access mode on the end. Every
  // other launch option (empty thread, open beside) lives in the agent chip's
  // menu; the project card floats top-right on wide screens.
  import { announceShown } from '../../lib/announce.svelte';
  import { api } from '../../lib/api/client';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { checkFolder } from '../../lib/folderCheck';
  import {
    formatEffortShort,
    formatInteractionModeLabel,
    formatModelLabel,
    nextInteractionModeToggle,
    supportsEffortSelection,
    supportsInteractionMode,
    type EffortLevel,
    type InteractionMode,
  } from '../../lib/providerModels';
  import { agentProviders, defaultAgentProvider, providerReadiness } from '../../lib/providers';
  import {
    isImmediateSlashCommand,
    parseInteractiveSlashInput,
    type ComposerSlashCommand,
  } from '../../lib/slashActions';
  import { auth } from '../../lib/stores/auth.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { transcript } from '../../lib/stores/transcript.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import type { Repo } from '../../lib/api/types';
  import { buildFolderMenuItems, buildWorkspaceMenuItems, STAGE_SLASH_COMMANDS } from './newThreadMenus';
  import { openBranchMenu, openCheckoutMenu } from './threadGitMenus';
  import { buildAgentMenu, buildEffortMenu, buildModeMenu, buildModelMenu } from './threadPickerMenus';
  import StageSlashPopup from './StageSlashPopup.svelte';
  import ThreadProjectCard from './ThreadProjectCard.svelte';
  import { harnessLabel, projectNameFromCwd, relToRoot } from './threadsHelpers';
  import { threadRepos } from './threadRepos.svelte';
  import { threads } from './threadsStore.svelte';

  interface Props {
    /** Float the project card top-right (wide screen, right panel closed). */
    showCard?: boolean;
  }
  let { showCard = false }: Props = $props();

  const harnesses = $derived.by(() => {
    const merged = ['claude', 'codex', 'agy'];
    for (const p of agentProviders()) if (!merged.includes(p)) merged.push(p);
    return merged;
  });

  let provider = $state('');
  let selectedModel = $state<string | null>(null);
  let selectedEffort = $state<EffortLevel | null>(null);
  let interactionMode = $state<InteractionMode>('full');
  let cwd = $state('');
  let useScratch = $state(false);
  let prompt = $state('');
  let cwdError = $state('');
  let busy = $state(false);
  let picker = $state<'none' | 'folder' | 'file'>('none');
  let openInBeside = $state(false);
  let worktree = $state<{ path: string; branch: string | null; repo: Repo } | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let agentEl = $state<HTMLButtonElement | null>(null);
  let modeEl = $state<HTMLButtonElement | null>(null);
  let projectEl = $state<HTMLButtonElement | null>(null);

  const homePath = $derived(ws.scratch?.root_path ?? '~');
  const scratchActive = $derived(useScratch || ws.current === null);
  const cleanCwd = $derived(cwd.trim().replace(/\/+$/, ''));
  const onWorktree = $derived(worktree !== null && worktree.path.replace(/\/+$/, '') === cleanCwd);
  const repo = $derived(onWorktree ? worktree!.repo : threadRepos.repoFor(cleanCwd, scratchActive ? null : ws.currentId));
  const branch = $derived(onWorktree ? worktree!.branch : repo ? (git.statusById[repo.id]?.branch ?? null) : null);
  const projectName = $derived(
    !cleanCwd || cleanCwd === homePath || cleanCwd === '~' ? null : projectNameFromCwd(cleanCwd),
  );
  const agentLabel = $derived.by(() => {
    const meta = auth.meta;
    const model = selectedModel ? formatModelLabel(provider, selectedModel, meta) : harnessLabel(provider);
    return supportsEffortSelection(provider) && selectedEffort ? `${model} · ${formatEffortShort(selectedEffort)}` : model;
  });

  $effect(() => {
    if (!provider) {
      const def = defaultAgentProvider();
      const ready = harnesses.find((h) => providerReadiness(h).available);
      provider = providerReadiness(def).available ? def : (ready ?? harnesses[0] ?? 'claude');
    }
  });

  $effect(() => {
    const seeded = threads.draftCwd;
    if (seeded) {
      cwd = seeded;
      useScratch = false;
    } else if (!cwd) {
      // T3's "use last project": a fresh draft opens where the last thread started.
      cwd = threads.lastProject ?? (scratchActive ? homePath : (ws.current?.root_path ?? homePath));
    }
  });

  // The inspector / project card follow the folder this draft targets.
  $effect(() => {
    threads.stageCwd = cleanCwd || null;
  });

  $effect(() => {
    if (repo && !onWorktree) git.ensureStatus(repo.id);
  });

  function selectProvider(next: string): void {
    if (provider === next) return;
    provider = next;
    selectedModel = null;
    selectedEffort = null;
  }

  function setCwd(path: string): void {
    cwd = path;
    cwdError = '';
  }

  function autoGrow(el: HTMLTextAreaElement): void {
    el.style.height = 'auto';
    el.style.height = `${Math.min(240, Math.max(56, el.scrollHeight))}px`;
  }

  function focusComposer(caretToEnd = false): void {
    queueMicrotask(() => {
      if (!composerEl) return;
      autoGrow(composerEl);
      composerEl.focus();
      if (caretToEnd) composerEl.setSelectionRange(prompt.length, prompt.length);
    });
  }

  // ── Menus ────────────────────────────────────────────────────────────────

  function openAgentMenu(): void {
    if (!agentEl) return;
    const meta = auth.meta;
    ctxMenu.showAt(
      agentEl,
      buildAgentMenu({
        harnesses,
        provider,
        modelLabel: formatModelLabel(provider, selectedModel, meta),
        effort: selectedEffort,
        meta,
        openBeside: threads.paneIds.length > 0 ? openInBeside : null,
        onProvider: selectProvider,
        onOpenModel: () => queueMicrotask(openModelMenu),
        onOpenEffort: () => queueMicrotask(openEffortMenu),
        onToggleBeside: () => (openInBeside = !openInBeside),
        onEmpty: () => void launch(false),
      }),
      { align: 'end' },
    );
  }

  function openModelMenu(): void {
    if (!agentEl) return;
    ctxMenu.showAt(
      agentEl,
      buildModelMenu({ provider, pinnedModel: selectedModel, meta: auth.meta, onModel: (m) => (selectedModel = m) }),
      { align: 'end', filter: true, filterPlaceholder: 'Filter models…' },
    );
  }

  function openEffortMenu(): void {
    if (!agentEl || !supportsEffortSelection(provider)) return;
    ctxMenu.showAt(
      agentEl,
      buildEffortMenu({
        provider,
        model: selectedModel,
        effort: selectedEffort,
        meta: auth.meta,
        onEffort: (e) => (selectedEffort = e),
      }),
      { align: 'end' },
    );
  }

  function openModeMenu(): void {
    if (!modeEl || !supportsInteractionMode(provider)) return;
    ctxMenu.showAt(modeEl, buildModeMenu(interactionMode, (m) => (interactionMode = m)), { align: 'end' });
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
      onBrowse: () => (picker = 'folder'),
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

  // ── Launch ───────────────────────────────────────────────────────────────

  async function launch(withPrompt: boolean, overridePrompt?: string): Promise<void> {
    if (busy) return;
    const readiness = providerReadiness(provider);
    if (!readiness.available) {
      toasts.error(`Couldn’t start ${harnessLabel(provider)}`, readiness.message);
      return;
    }
    const rawPrompt = overridePrompt ?? (withPrompt ? prompt.trim() : '');
    if (withPrompt && overridePrompt === undefined) {
      const slash = parseInteractiveSlashInput(rawPrompt);
      if (slash) {
        prompt = '';
        switch (slash.kind) {
          case 'open-model-picker':
            queueMicrotask(openModelMenu);
            return;
          case 'set-model':
            selectedModel = slash.model;
            return;
          case 'open-effort-picker':
            queueMicrotask(openEffortMenu);
            return;
          case 'set-effort':
            selectedEffort = slash.effort;
            return;
          case 'open-mode-picker':
            queueMicrotask(openModeMenu);
            return;
          case 'set-mode':
            interactionMode = slash.mode;
            if (slash.prompt) await launch(true, slash.prompt);
            return;
          case 'send-prompt':
            await launch(true, slash.prompt);
            return;
          default:
            break;
        }
      }
    }
    const firstMsg = rawPrompt.trim();
    if (withPrompt && !firstMsg) {
      composerEl?.focus();
      return;
    }
    let dir = cwd.trim();
    if (homePath !== '~' && (dir === '~' || dir.startsWith('~/'))) dir = homePath + dir.slice(1);
    if (dir !== '') {
      cwdError = '';
      const check = await checkFolder(dir, (url) => api.get(url));
      if (!check.ok) {
        cwdError = check.message;
        return;
      }
    }
    busy = true;
    try {
      const meta: Record<string, unknown> = {
        surface: 'threads',
        interaction_mode: interactionMode,
        ...(selectedModel ? { model: selectedModel } : {}),
        ...(selectedEffort ? { effort: selectedEffort } : {}),
      };
      const title = firstMsg ? firstMsg.replace(/\s+/g, ' ').slice(0, 56) : `${harnessLabel(provider)} thread`;
      const opts = { scratch: scratchActive && ws.currentId !== SCRATCH_WORKSPACE_ID };
      const created = firstMsg
        ? await ws.openSessionWithPrompt({ provider, title, cwd: dir || null, prompt: firstMsg, meta }, opts)
        : await ws.createSessionQuiet({ kind: 'agent', provider, title, cwd: dir || null, meta }, opts);
      if (firstMsg) transcript.recordOptimisticPrompt(created.id, firstMsg);
      prompt = '';
      threads.draftCwd = null;
      threads.rememberProject(dir || null);
      if (openInBeside && threads.paneIds.length > 0) threads.openBeside(created.id);
      else threads.selectThread(created.id);
    } catch (e) {
      toastError('Couldn’t start thread', e);
    } finally {
      busy = false;
    }
  }

  // ── Slash popup ──────────────────────────────────────────────────────────

  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
  const slashPopId = $props.id();
  const cmdPrefix = $derived.by(() => {
    const m = /^\/([\w:-]*)$/.exec(prompt);
    return m ? m[1].toLowerCase() : null;
  });
  const slashSuggestions = $derived.by(() => {
    if (cmdPrefix == null || cmdDismissed) return [] as ComposerSlashCommand[];
    return STAGE_SLASH_COMMANDS.filter((c) => c.name.toLowerCase().includes(cmdPrefix));
  });
  const cmdOpen = $derived(slashSuggestions.length > 0);
  $effect(() => {
    void slashSuggestions.length;
    cmdIdx = 0;
  });
  $effect(() => {
    void prompt;
    cmdDismissed = false;
  });

  function acceptStageCmd(c: ComposerSlashCommand): void {
    if (isImmediateSlashCommand(c.name)) {
      prompt = `/${c.name}`;
      cmdDismissed = true;
      void launch(true);
      return;
    }
    prompt = `/${c.name} `;
    cmdDismissed = true;
    focusComposer(true);
  }

  function onComposerKeydown(e: KeyboardEvent): void {
    if (e.isComposing) return;
    if (e.key === 'Tab' && e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && !cmdOpen) {
      if (supportsInteractionMode(provider)) {
        e.preventDefault();
        interactionMode = nextInteractionModeToggle(interactionMode);
        return;
      }
    }
    if (cmdOpen) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const n = slashSuggestions.length;
        cmdIdx = (cmdIdx + (e.key === 'ArrowDown' ? 1 : n - 1)) % n;
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        cmdDismissed = true;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        const pick = slashSuggestions[cmdIdx];
        if (pick) {
          e.preventDefault();
          acceptStageCmd(pick);
          return;
        }
      }
    }
    if (e.key === 'Escape' && threads.paneIds.length > 0 && !prompt) {
      e.preventDefault();
      threads.newDraftMode = false;
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void launch(true);
    }
  }

  function attachPath(path: string): void {
    const rel = relToRoot(path, cleanCwd);
    const token = `@${/\s/.test(rel) ? `"${rel}"` : rel} `;
    prompt = prompt && !/\s$/.test(prompt) ? `${prompt} ${token}` : `${prompt}${token}`;
    focusComposer(true);
  }
</script>

<div class="nt-stage" class:with-card={showCard} data-testid="new-thread-stage">
  {#if showCard}
    <div class="nt-card-slot">
      <ThreadProjectCard floating cwd={cleanCwd} onpickfolder={openProjectMenu} />
    </div>
  {/if}

  <div class="nt-inner">
    <h1 class="nt-title">
      {#if projectName}
        <!-- One text run: no flex gap or block-edge whitespace around the name. -->
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

    <div class="nt-composer" class:plan={interactionMode === 'plan'}>
      {#if cmdOpen}
        <StageSlashPopup
          id={slashPopId}
          items={slashSuggestions}
          active={cmdIdx}
          onaccept={acceptStageCmd}
          onhover={(i) => (cmdIdx = i)}
        />
      {/if}
      <textarea
        bind:this={composerEl}
        bind:value={prompt}
        dir="auto"
        rows={2}
        placeholder={interactionMode === 'plan'
          ? 'Describe what to explore and plan. / commands, ⇧⇥ leaves Plan'
          : 'Ask anything. @ files, / commands'}
        aria-label="Opening prompt for new thread"
        aria-autocomplete="list"
        aria-controls={cmdOpen ? `${slashPopId}-list` : undefined}
        aria-activedescendant={cmdOpen ? `${slashPopId}-opt-${cmdIdx}` : undefined}
        data-testid="new-thread-input"
        disabled={busy}
        oninput={(e) => autoGrow(e.currentTarget)}
        onkeydown={onComposerKeydown}
      ></textarea>

      {#if cwdError}
        <p class="nt-error" use:announceShown={{ text: cwdError, assertive: true }}>{cwdError}</p>
      {/if}

      <div class="nt-foot">
        <button
          type="button"
          class="icon-btn"
          aria-label="Attach a file from the project"
          title="Attach a file (@ mention)"
          data-testid="new-thread-attach"
          disabled={busy}
          onclick={() => (picker = 'file')}
        >
          <Icon name="link" size={16} />
        </button>
        <span class="nt-spacer"></span>
        <button
          bind:this={agentEl}
          type="button"
          class="nt-agent"
          aria-haspopup="menu"
          aria-label="Agent: {harnessLabel(provider)}, {agentLabel}"
          title="Harness, model and effort (/model · /effort)"
          data-testid="picker-agent"
          disabled={busy}
          onclick={openAgentMenu}
        >
          {#if hasProviderIcon(provider)}
            <ProviderIcon {provider} size={14} />
          {:else}
            <Icon name="terminal" size={14} />
          {/if}
          <span class="ellipsis">{agentLabel}</span>
        </button>
        <button
          type="button"
          class="nt-send"
          aria-label={busy ? 'Starting thread' : 'Start thread'}
          title="Start thread (↵)"
          data-testid="new-thread-send-btn"
          disabled={busy || !prompt.trim()}
          onclick={() => void launch(true)}
        >
          {#if busy}
            <span class="spinner" aria-hidden="true"></span>
          {:else}
            <Icon name="arrowUp" size={16} />
          {/if}
        </button>
      </div>
    </div>

    <div class="nt-meta">
      {#if repo}
        <button
          type="button"
          class="nt-meta-btn"
          aria-haspopup="menu"
          title="Run in the current checkout or another worktree"
          data-testid="new-thread-checkout"
          onclick={(e) =>
            void openCheckoutMenu(e.currentTarget, repo!, cleanCwd, (path, br) => {
              worktree = { path, branch: br, repo: repo! };
              setCwd(path);
            })}
        >
          <Icon name={onWorktree && !cleanCwd.startsWith(repo.path) ? 'worktree' : 'folder'} size={13} />
          <span class="ellipsis">{onWorktree && !cleanCwd.startsWith(repo.path) ? projectNameFromCwd(cleanCwd) : 'Current checkout'}</span>
        </button>
      {:else}
        <button
          type="button"
          class="nt-meta-btn"
          aria-haspopup="menu"
          title={cleanCwd || 'Choose a folder'}
          onclick={(e) => openProjectMenu(e.currentTarget)}
        >
          <Icon name="folder" size={13} />
          <span class="ellipsis">{scratchActive && !projectName ? 'Scratch' : 'Choose folder'}</span>
        </button>
      {/if}
      <span class="nt-spacer"></span>
      {#if repo && branch}
        <button
          type="button"
          class="nt-meta-btn mono"
          dir="ltr"
          aria-haspopup="menu"
          title="Git branch: {branch}"
          data-testid="new-thread-branch"
          disabled={onWorktree}
          onclick={(e) => void openBranchMenu(e.currentTarget, repo!)}
        >
          <Icon name="branch" size={13} />
          <span class="ellipsis">{branch}</span>
        </button>
      {/if}
      {#if supportsInteractionMode(provider)}
        <button
          bind:this={modeEl}
          type="button"
          class="nt-meta-btn"
          class:warn={interactionMode === 'plan'}
          aria-haspopup="menu"
          aria-label="Access mode: {formatInteractionModeLabel(interactionMode)}"
          title="Access mode (/mode · ⇧⇥ toggles Plan)"
          data-testid="picker-mode"
          onclick={openModeMenu}
        >
          <Icon name={interactionMode === 'plan' ? 'compass' : interactionMode === 'accept-edits' ? 'shield' : 'unlock'} size={13} />
          <span>{formatInteractionModeLabel(interactionMode)}</span>
        </button>
      {/if}
    </div>
  </div>
</div>

{#if picker !== 'none'}
  <FolderPicker
    title={picker === 'file' ? 'Attach a file' : 'Choose project folder'}
    files={picker === 'file'}
    start={cleanCwd || homePath || ''}
    onpick={(picked) => {
      if (picker === 'file') attachPath(picked);
      else setCwd(picked);
      picker = 'none';
    }}
    onclose={() => (picker = 'none')}
  />
{/if}

<style>
  .nt-stage {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
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
    width: 100%;
    max-width: 720px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .nt-title {
    margin: 0 0 6px;
    overflow-wrap: anywhere;
    font-size: var(--fs-hero);
    font-weight: 400;
    letter-spacing: -0.01em;
    line-height: 1.25;
    color: var(--text);
    text-align: center;
  }
  .nt-project {
    max-width: 100%;
    padding: 0 2px;
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
  .nt-composer {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 14px 10px;
    border-radius: var(--radius-l);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .nt-composer:focus-within {
    border-color: var(--border-strong);
  }
  .nt-composer.plan {
    border-color: color-mix(in srgb, var(--warning) 42%, var(--border));
  }
  .nt-composer textarea {
    width: 100%;
    min-block-size: 56px;
    max-block-size: 240px;
    resize: none;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.55;
    unicode-bidi: plaintext;
    text-align: start;
    outline: none;
  }
  .nt-error {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .nt-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .nt-spacer {
    flex: 1;
  }
  .nt-agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 260px;
    block-size: 30px;
    padding-inline: 8px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .nt-agent:hover:not(:disabled) {
    background: var(--hover);
  }
  .nt-send {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    inline-size: 32px;
    block-size: 32px;
    border: 0;
    border-radius: 50%;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    cursor: pointer;
  }
  .nt-send:disabled {
    background: var(--surface-3);
    color: var(--text-dim);
    cursor: default;
  }
  .nt-send .spinner {
    --spinner-size: 14px;
  }
  .nt-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    padding-inline: 6px;
    min-width: 0;
  }
  .nt-meta-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 240px;
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
  .nt-meta-btn:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .nt-meta-btn:disabled {
    cursor: default;
  }
  .nt-meta-btn.warn {
    color: var(--warning);
  }
  @media (max-width: 640px) {
    .nt-title {
      font-size: var(--fs-2xl);
    }
  }
</style>
