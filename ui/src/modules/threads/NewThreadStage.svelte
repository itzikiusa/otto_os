<script lang="ts">
  // The calm start state (T3 Code's draft hero): one heading that names the
  // project, ONE composer — attach on the start, a single agent chip (harness ·
  // model · effort) and a round send on the end — unified `@` file/folder &
  // agent completion (`RuntimeMentionMenu`), `/` slash popup, Council picker,
  // and `NewThreadMetaRow` beneath it.
  import { untrack } from 'svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { api } from '../../lib/api/client';
  import type { Repo } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { enterSends } from '../../lib/enterSends';
  import { checkFolder } from '../../lib/folderCheck';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import {
    formatEffortShort,
    formatModelLabel,
    nextInteractionModeToggle,
    supportsEffortSelection,
    supportsInteractionMode,
    type EffortLevel,
    type InteractionMode,
  } from '../../lib/providerModels';
  import { agentProviders, defaultAgentProvider, orderThreadHarnesses, providerReadiness } from '../../lib/providers';
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
  import { buildFolderMenuItems, buildWorkspaceMenuItems, STAGE_SLASH_COMMANDS } from './newThreadMenus';
  import NewThreadMetaRow from './NewThreadMetaRow.svelte';
  import { addComposerContext, assembleComposerMessage, fileMentionContext, type ComposerContext } from './runtime/composerContext';
  import CouncilStagePicker from './runtime/CouncilStagePicker.svelte';
  import { council, councilAgentLabel } from './runtime/councilStore.svelte';
  import RuntimeContextChips from './runtime/RuntimeContextChips.svelte';
  import type { PickedAgentMention, PickedFileMention } from './runtime/RuntimeMentionMenu.svelte';
  import { isRuntimeProvider, runtimeThreads } from './runtime/runtimeThreads.svelte';
  import StageAttach from './runtime/StageAttach.svelte';
  import { StageAttachments } from './runtime/stageAttachments.svelte';
  import { FolderPickerModal, MentionMenuView, ProjectCardView, StageSlashView } from './threadLazy.svelte';
  import { buildModeMenu } from './threadPickerMenus';
  import { threadRepos } from './threadRepos.svelte';
  import { harnessLabel, projectNameFromCwd, relToRoot } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  const ModelPicker = lazyComponent(() => import('../../lib/components/ModelPickerPopover.svelte'));

  interface Props {
    showCard?: boolean;
  }
  let { showCard = false }: Props = $props();

  const harnesses = $derived(orderThreadHarnesses(agentProviders()));

  let provider = $state('');
  let selectedModel = $state<string | null>(null);
  let selectedEffort = $state<EffortLevel | null>(null);
  let interactionMode = $state<InteractionMode>('full');
  let cwd = $state('');
  let useScratch = $state(false);
  let prompt = $state('');
  let stageContext = $state<ComposerContext[]>([]);
  let cwdError = $state('');
  let busy = $state(false);
  let picker = $state<'none' | 'folder' | 'file'>('none');
  let openInBeside = $state(false);
  let worktree = $state<{ path: string; branch: string | null; repo: Repo } | null>(null);
  let composerBoxEl = $state<HTMLDivElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let agentEl = $state<HTMLButtonElement | null>(null);
  let projectEl = $state<HTMLButtonElement | null>(null);
  let mentionEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);

  const homePath = $derived(ws.scratch?.root_path ?? '~');
  const scratchActive = $derived(useScratch || ws.current === null);
  const targetWs = $derived(scratchActive ? SCRATCH_WORKSPACE_ID : (ws.currentId ?? SCRATCH_WORKSPACE_ID));
  const runtime = $derived(isRuntimeProvider(provider));
  const files = new StageAttachments(() => provider, () => runtime && !council.on);
  const modeSupported = $derived(supportsInteractionMode(provider) && !runtime);
  const cleanCwd = $derived(cwd.trim().replace(/\/+$/, ''));
  const onWorktree = $derived(worktree !== null && worktree.path.replace(/\/+$/, '') === cleanCwd);
  const repo = $derived(onWorktree ? worktree!.repo : threadRepos.repoFor(cleanCwd, scratchActive ? null : ws.currentId));
  const branch = $derived(onWorktree ? worktree!.branch : repo ? (git.statusById[repo.id]?.branch ?? null) : null);
  const projectName = $derived(
    !cleanCwd || cleanCwd === homePath || cleanCwd === '~' ? null : projectNameFromCwd(cleanCwd),
  );
  const agentLabel = $derived.by(() => {
    const meta = auth.meta;
    if (council.on) return councilAgentLabel(meta);
    const model = selectedModel ? formatModelLabel(provider, selectedModel, meta) : harnessLabel(provider);
    return supportsEffortSelection(provider) && selectedEffort ? `${model} · ${formatEffortShort(selectedEffort)}` : model;
  });

  $effect(() => {
    if (!provider || (harnesses.length > 0 && !harnesses.includes(provider))) {
      const def = defaultAgentProvider();
      const ready = harnesses.find((h) => providerReadiness(h).available);
      provider = harnesses.includes(def) && providerReadiness(def).available ? def : (ready ?? harnesses[0] ?? def);
    }
  });

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

  let pickerOpen = $state(false);
  function openModelMenu(): void {
    if (agentEl) pickerOpen = true;
  }
  const openEffortMenu = openModelMenu;
  $effect(() => (pickerOpen ? untrack(() => council.watchModifiers({ provider, model: selectedModel, effort: selectedEffort })) : undefined));

  function openModeMenu(anchor?: HTMLElement): void {
    const target = anchor ?? agentEl;
    if (!target || !modeSupported) return;
    ctxMenu.showAt(target, buildModeMenu(interactionMode, (m) => (interactionMode = m)), { align: 'end' });
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
            queueMicrotask(() => openModeMenu());
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
    const rawTrimmed = rawPrompt.trim();
    if (withPrompt && !rawTrimmed && files.list.length === 0 && stageContext.length === 0) {
      composerEl?.focus();
      return;
    }
    if (withPrompt && council.on && !council.ready) {
      cwdError = 'Pick at least two agents to ask, or turn Ask several off.';
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
    const firstMsg = withPrompt ? assembleComposerMessage(rawTrimmed, stageContext) : '';
    const titleSeed = rawTrimmed || stageContext[0]?.label || '';
    busy = true;
    try {
      const meta: Record<string, unknown> = {
        surface: 'threads',
        interaction_mode: interactionMode,
        ...(selectedModel ? { model: selectedModel } : {}),
        ...(selectedEffort ? { effort: selectedEffort } : {}),
      };
      const title = titleSeed ? titleSeed.replace(/\s+/g, ' ').slice(0, 56) : `${harnessLabel(provider)} thread`;
      const opts = { scratch: scratchActive && ws.currentId !== SCRATCH_WORKSPACE_ID };
      const asCouncil = withPrompt && council.ready;
      const created = asCouncil
        ? (await council.start(targetWs, { prompt: firstMsg, title, cwd: dir || null }))[0]
        : runtime && isRuntimeProvider(provider)
          ? await runtimeThreads.start(targetWs, {
              provider,
              title,
              cwd: dir || null,
              model: selectedModel,
              effort: selectedEffort ?? undefined,
              prompt: firstMsg,
              attachments: withPrompt ? files.list : [],
            })
          : firstMsg
            ? await ws.openSessionWithPrompt({ provider, title, cwd: dir || null, prompt: firstMsg, meta }, opts)
            : await ws.createSessionQuiet({ kind: 'agent', provider, title, cwd: dir || null, meta }, opts);
      if (firstMsg && !runtime && !asCouncil) transcript.recordOptimisticPrompt(created.id, firstMsg);
      prompt = '';
      stageContext = [];
      files.picked = [];
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

  function handleMentionFile(picked: PickedFileMention): void {
    prompt = picked.nextText;
    stageContext = addComposerContext(
      stageContext,
      fileMentionContext({
        path: picked.path,
        label: picked.display,
        isDir: picked.isDir,
      }),
    );
    queueMicrotask(() => composerEl && autoGrow(composerEl));
  }

  function handleMentionAgent(picked: PickedAgentMention): void {
    prompt = picked.nextText;
    const o = picked.option;
    if (council.on) {
      council.pick({ provider: o.provider, model: o.model ?? null, effort: null });
    } else {
      provider = o.provider;
      selectedModel = o.model || null;
      if (!supportsEffortSelection(o.provider)) selectedEffort = null;
    }
  }

  function onComposerKeydown(e: KeyboardEvent): void {
    if (e.isComposing || mentionEl?.onKey(e)) return;
    if (e.key === 'Tab' && e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && !cmdOpen) {
      if (modeSupported) {
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
    if (enterSends(e)) {
      e.preventDefault();
      void launch(!((e.metaKey || e.ctrlKey) && !prompt.trim() && stageContext.length === 0));
    }
  }

  function attachPath(path: string): void {
    const rel = relToRoot(path, cleanCwd);
    const token = `@${/\s/.test(rel) ? `"${rel}"` : rel} `;
    prompt = prompt && !/\s$/.test(prompt) ? `${prompt} ${token}` : `${prompt}${token}`;
    stageContext = addComposerContext(stageContext, fileMentionContext({ path, label: rel, isDir: false }));
    focusComposer(true);
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

    <div class="nt-composer" class:plan={interactionMode === 'plan'} bind:this={composerBoxEl}>
      {#if cmdOpen && StageSlashView.component}
        <StageSlashView.component
          id={slashPopId}
          items={slashSuggestions}
          active={cmdIdx}
          onaccept={acceptStageCmd}
          onhover={(i) => (cmdIdx = i)}
        />
      {/if}
      {#if prompt.includes('@') && MentionMenuView.component}
        <MentionMenuView.component
          bind:this={mentionEl}
          text={prompt}
          cwd={cleanCwd || null}
          textareaEl={composerEl}
          anchorEl={composerBoxEl}
          allowAgents={true}
          agentSectionLabel={council.on ? 'Add council agent' : 'Switch agent'}
          onpickfile={handleMentionFile}
          onpickagent={handleMentionAgent}
          ondrilldir={(nextText) => { prompt = nextText; }}
        />
      {/if}
      <RuntimeContextChips
        context={stageContext}
        onremove={(id) => (stageContext = stageContext.filter((c) => c.id !== id))}
        onfocus={() => focusComposer()}
      />
      <textarea
        bind:this={composerEl}
        bind:value={prompt}
        dir="auto"
        rows={2}
        placeholder={interactionMode === 'plan'
          ? 'Describe what to explore and plan. @ files, / commands, ⇧⇥ leaves Plan'
          : 'Ask anything. @ files or agents, / commands'}
        aria-label="Opening prompt for new thread"
        aria-autocomplete="list"
        aria-controls={cmdOpen ? `${slashPopId}-list` : undefined}
        aria-activedescendant={cmdOpen ? `${slashPopId}-opt-${cmdIdx}` : undefined}
        data-testid="new-thread-input"
        disabled={busy}
        onfocus={() => { MentionMenuView.prefetch(); StageSlashView.prefetch(); }}
        oninput={(e) => {
          if (e.currentTarget.value.includes('@')) MentionMenuView.prefetch();
          if (e.currentTarget.value.startsWith('/')) StageSlashView.prefetch();
          autoGrow(e.currentTarget);
        }}
        onkeydown={onComposerKeydown}
        onpaste={(e) => files.paste(e)}
      ></textarea>
      <StageAttach part="list" {files} />

      {#if cwdError}
        <p class="nt-error" use:announceShown={{ text: cwdError, assertive: true }}>{cwdError}</p>
      {/if}

      <CouncilStagePicker part="seats" {provider} model={selectedModel} effort={selectedEffort} />
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
        <StageAttach part="button" {files} disabled={busy} />
        <span class="nt-spacer"></span>
        <CouncilStagePicker part="toggle" {provider} model={selectedModel} effort={selectedEffort} disabled={busy} />
        <button
          bind:this={agentEl}
          type="button"
          class="nt-agent"
          aria-haspopup="menu"
          aria-label="Agent: {harnessLabel(provider)}, {agentLabel}"
          title="Harness, model and effort (/model · /effort)"
          data-testid="picker-agent"
          onpointerenter={() => ModelPicker.prefetch()}
          disabled={busy}
          aria-expanded={pickerOpen}
          onclick={openModelMenu}
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
          class:ask={council.ready}
          aria-label={busy ? 'Starting thread' : council.ready ? `Ask ${council.seats.length} agents` : 'Start thread'}
          title="Start thread (↵) — ⌘↵ with no text opens an empty one"
          data-testid="new-thread-send-btn"
          disabled={busy || (!prompt.trim() && stageContext.length === 0)}
          onclick={() => void launch(true)}
        >
          {#if busy}
            <span class="spinner" aria-hidden="true"></span>
          {:else if council.ready}
            Ask {council.seats.length}
          {:else}
            <Icon name="arrowUp" size={16} />
          {/if}
        </button>
      </div>
    </div>

    <NewThreadMetaRow
      {repo}
      {cleanCwd}
      {onWorktree}
      {scratchActive}
      {projectName}
      {openInBeside}
      {branch}
      {modeSupported}
      {interactionMode}
      onPickWorktree={(path, br, r) => {
        worktree = { path, branch: br, repo: r };
        setCwd(path);
      }}
      onOpenProjectMenu={openProjectMenu}
      onToggleBeside={() => (openInBeside = !openInBeside)}
      onOpenModeMenu={(anchor) => openModeMenu(anchor)}
    />
  </div>
</div>

{#if pickerOpen && agentEl && ModelPicker.component}
  <ModelPicker.component
    anchor={agentEl}
    providers={harnesses}
    {provider}
    model={selectedModel}
    effort={selectedEffort}
    mode={interactionMode}
    onprovider={selectProvider}
    onmodel={(m) => { council.pick({ provider, model: m, effort: selectedEffort }); selectedModel = m; }}
    oneffort={(e) => (selectedEffort = e)}
    onmode={modeSupported ? (m) => (interactionMode = m) : undefined}
    onclose={() => (pickerOpen = false)}
  />
{/if}

{#if picker !== 'none' && FolderPickerModal.component}
  <FolderPickerModal.component
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
    inline-size: 100%;
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
    min-inline-size: 0;
  }
  .nt-spacer {
    flex: 1;
  }
  .nt-agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    max-inline-size: 260px;
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
  .nt-send.ask {
    inline-size: auto;
    padding-inline: 12px;
    border-radius: 999px;
    font-weight: 600;
  }
  .nt-send .spinner {
    --spinner-size: 14px;
  }
  @media (max-width: 640px) {
    .nt-title {
      font-size: var(--fs-2xl);
    }
  }
</style>
