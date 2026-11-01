<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import { api } from '../../lib/api/client';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import ThreadModelBar from '../../lib/components/ThreadModelBar.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { checkFolder } from '../../lib/folderCheck';
  import {
    nextInteractionModeToggle,
    supportsInteractionMode,
    type EffortLevel,
    type InteractionMode,
  } from '../../lib/providerModels';
  import { agentProviders, defaultAgentProvider, providerReadiness } from '../../lib/providers';
  import { radioKey } from '../../lib/radioKey';
  import {
    isImmediateSlashCommand,
    parseInteractiveSlashInput,
    type ComposerSlashCommand,
  } from '../../lib/slashActions';
  import { git } from '../../lib/stores/git.svelte';
  import { transcript } from '../../lib/stores/transcript.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import {
    buildFolderMenuItems,
    buildWorkspaceMenuItems,
    NEW_THREAD_STARTERS,
    STAGE_SLASH_COMMANDS,
  } from './newThreadMenus';
  import { compactPath, harnessLabel } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  const harnesses = $derived.by(() => {
    const registered = agentProviders();
    const core = ['claude', 'codex', 'agy'];
    const merged = [...core];
    for (const p of registered) {
      if (!merged.includes(p)) merged.push(p);
    }
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
  let browsingFolder = $state(false);
  let openInBeside = $state(false);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let modelBar = $state<ReturnType<typeof ThreadModelBar> | null>(null);

  const homePath = $derived(ws.scratch?.root_path ?? '~');
  const scratchActive = $derived(useScratch || ws.current === null);
  const currentBranch = $derived.by(() => {
    const dir = cwd.trim().replace(/\/+$/, '');
    if (!dir) return null;
    const repo = git.repos
      .filter((r) => dir === r.path || dir.startsWith(`${r.path.replace(/\/+$/, '')}/`))
      .sort((a, b) => b.path.length - a.path.length)[0];
    if (!repo) return null;
    return git.statusById[repo.id]?.branch ?? null;
  });

  function selectProvider(next: string): void {
    if (provider === next) return;
    provider = next;
    selectedModel = null;
    selectedEffort = null;
  }

  // Sync initial provider & cwd when NewThreadStage mounts or draft target changes.
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
      cwd = scratchActive ? homePath : (ws.current?.root_path ?? homePath);
    }
  });

  function autoGrow(el: HTMLTextAreaElement): void {
    el.style.height = 'auto';
    el.style.height = `${Math.min(240, Math.max(88, el.scrollHeight))}px`;
  }

  function pickStarter(text: string): void {
    prompt = text;
    queueMicrotask(() => {
      if (composerEl) {
        autoGrow(composerEl);
        composerEl.focus();
      }
    });
  }

  function openWorkspaceMenu(e: MouseEvent): void {
    ctxMenu.show(
      e,
      buildWorkspaceMenuItems({
        workspaces: ws.workspaces,
        currentId: ws.currentId,
        scratchActive,
        homePath,
        onSelectWorkspace: (w) => {
          useScratch = false;
          cwd = w.root_path;
          cwdError = '';
          void ws.select(w.id);
        },
        onSelectScratch: () => {
          useScratch = true;
          cwd = homePath;
          cwdError = '';
        },
      }),
    );
  }

  function openFolderMenu(e: MouseEvent): void {
    ctxMenu.show(
      e,
      buildFolderMenuItems({
        scratchActive,
        currentWorkspace: ws.current,
        repos: git.repos,
        sessions: ws.sessions,
        cwd,
        homePath,
        onPickPath: (path) => {
          cwd = path;
          cwdError = '';
        },
        onBrowse: () => (browsingFolder = true),
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
            queueMicrotask(() => modelBar?.openModelPicker());
            return;
          case 'set-model':
            selectedModel = slash.model;
            return;
          case 'open-effort-picker':
            queueMicrotask(() => modelBar?.openEffortPicker());
            return;
          case 'set-effort':
            selectedEffort = slash.effort;
            return;
          case 'open-mode-picker':
            queueMicrotask(() => modelBar?.openModePicker());
            return;
          case 'set-mode':
            interactionMode = slash.mode;
            if (slash.prompt) {
              await launch(true, slash.prompt);
            }
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
    if (homePath !== '~' && (dir === '~' || dir.startsWith('~/'))) {
      dir = homePath + dir.slice(1);
    }
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
      const derivedTitle = firstMsg
        ? firstMsg.replace(/\s+/g, ' ').slice(0, 56)
        : `${harnessLabel(provider)} thread`;
      const opts = { scratch: scratchActive && ws.currentId !== SCRATCH_WORKSPACE_ID };
      const created = firstMsg
        ? await ws.openSessionWithPrompt(
            {
              provider,
              title: derivedTitle,
              cwd: dir || null,
              prompt: firstMsg,
              meta,
            },
            opts,
          )
        : await ws.createSessionQuiet(
            {
              kind: 'agent',
              provider,
              title: derivedTitle,
              cwd: dir || null,
              meta,
            },
            opts,
          );
      if (firstMsg) {
        transcript.recordOptimisticPrompt(created.id, firstMsg);
      }
      prompt = '';
      threads.draftCwd = null;
      if (openInBeside && threads.paneIds.length > 0) {
        threads.openBeside(created.id);
      } else {
        threads.selectThread(created.id);
      }
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
    const next = `/${c.name} `;
    prompt = next;
    cmdDismissed = true;
    queueMicrotask(() => {
      if (composerEl) {
        autoGrow(composerEl);
        composerEl.focus();
        composerEl.setSelectionRange(next.length, next.length);
      }
    });
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
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        cmdIdx = (cmdIdx + 1) % slashSuggestions.length;
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        cmdIdx = (cmdIdx - 1 + slashSuggestions.length) % slashSuggestions.length;
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
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void launch(true);
    }
  }
</script>

<div class="nt-stage" data-testid="new-thread-stage">
  <div class="nt-inner">
    <div class="nt-hero">
      <div class="nt-mark" aria-hidden="true">
        {#if hasProviderIcon(provider)}
          <ProviderIcon {provider} size={24} />
        {:else}
          <Icon name="sparkle" size={24} />
        {/if}
      </div>
      <h2 class="nt-title">Start a new thread</h2>
      <p class="nt-subtitle">
        Pick a coding harness, choose your project folder, and write an opening prompt — or launch side-by-side in Split.
      </p>
    </div>

    <div class="nt-harnesses" role="radiogroup" aria-label="Agent harness">
      {#each harnesses as h (h)}
        {@const ready = providerReadiness(h)}
        {@const active = provider === h}
        <button
          type="button"
          role="radio"
          aria-checked={active}
          tabindex={active ? 0 : -1}
          onkeydown={radioKey}
          class="nt-harness-card"
          class:active
          class:unavailable={!ready.available}
          title={ready.available ? `${harnessLabel(h)} — Ready` : ready.message}
          data-testid="harness-{h}"
          onclick={() => selectProvider(h)}
        >
          {#if hasProviderIcon(h)}
            <ProviderIcon provider={h} size={16} />
          {:else}
            <Icon name="terminal" size={16} />
          {/if}
          <span class="nt-harness-name">{harnessLabel(h)}</span>
          {#if !ready.available}
            <span class="nt-unavail">Not installed</span>
          {/if}
        </button>
      {/each}
    </div>

    <div class="nt-composer-card" class:nt-plan={interactionMode === 'plan'}>
      {#if cmdOpen}
        <div
          class="nt-slash-pop"
          id="{slashPopId}-list"
          role="listbox"
          aria-label="Slash commands"
          data-slash-pop
        >
          {#each slashSuggestions as c, i (c.name)}
            <div
              id="{slashPopId}-opt-{i}"
              class="nt-slash-row"
              class:active={i === cmdIdx}
              role="option"
              tabindex="-1"
              aria-selected={i === cmdIdx}
              onmousedown={(ev) => {
                ev.preventDefault();
                acceptStageCmd(c);
              }}
              onmouseenter={() => (cmdIdx = i)}
            >
              <span class="nt-slash-name mono" dir="ltr">/{c.name}</span>
              <span class="nt-slash-desc">{c.description}</span>
            </div>
          {/each}
        </div>
      {/if}
      <textarea
        bind:this={composerEl}
        bind:value={prompt}
        dir="auto"
        rows={3}
        placeholder={interactionMode === 'plan'
          ? `What should ${harnessLabel(provider)} explore and plan? (/model · /effort · ⇧⇥ exit Plan)`
          : `What should ${harnessLabel(provider)} build, debug, or review? (/model · /effort · ⇧⇥ Plan)`}
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

      <div class="nt-composer-bar">
        <div class="nt-pickers">
          <ThreadModelBar
            bind:this={modelBar}
            {provider}
            pinnedModel={selectedModel}
            effort={selectedEffort}
            {interactionMode}
            disabled={busy}
            onmodelchange={(m) => (selectedModel = m)}
            oneffortchange={(eff) => (selectedEffort = eff)}
            onmodechange={(mode) => (interactionMode = mode)}
          />

          <button
            type="button"
            class="btn small ghost"
            aria-haspopup="menu"
            title="Select workspace"
            onclick={openWorkspaceMenu}
          >
            <Icon name="layers" size={12} />
            <span class="ellipsis">{scratchActive ? 'Scratch' : (ws.current?.name ?? 'Workspace')}</span>
            <Icon name="chevronDown" size={12} />
          </button>

          <button
            type="button"
            class="btn small ghost mono"
            dir="ltr"
            aria-haspopup="menu"
            title={cwd || 'Choose working directory'}
            onclick={openFolderMenu}
          >
            <Icon name="folder" size={12} />
            <span class="ellipsis">{compactPath(cwd, homePath)}</span>
            <Icon name="chevronDown" size={12} />
          </button>

          {#if currentBranch}
            <span class="nt-branch mono ellipsis" dir="ltr" title="Git branch: {currentBranch}">
              <Icon name="branch" size={12} />
              {currentBranch}
            </span>
          {/if}
        </div>

        <div class="nt-submit-row">
          {#if threads.paneIds.length > 0}
            <button
              type="button"
              class="pill-toggle"
              class:active={openInBeside}
              aria-pressed={openInBeside}
              title="Open this new thread beside your current thread in Split view"
              onclick={() => (openInBeside = !openInBeside)}
            >
              <Icon name="split" size={12} />
              Beside
            </button>
            <button
              type="button"
              class="btn small ghost"
              disabled={busy}
              onclick={() => (threads.newDraftMode = false)}
            >
              Cancel
            </button>
          {/if}
          <button
            type="button"
            class="btn small"
            disabled={busy}
            title="Open interactive session without sending a prompt yet"
            data-testid="new-thread-empty-btn"
            onclick={() => void launch(false)}
          >
            Empty thread
          </button>
          <button
            type="button"
            class="btn small primary"
            disabled={busy || !prompt.trim()}
            data-testid="new-thread-send-btn"
            onclick={() => void launch(true)}
          >
            <Icon name="send" size={12} />
            {busy ? 'Starting…' : 'Start thread'}
          </button>
        </div>
      </div>
    </div>

    <div class="nt-starters" aria-label="Starter prompts">
      {#each NEW_THREAD_STARTERS as s (s.label)}
        <button
          type="button"
          class="chip nt-starter-chip"
          onclick={() => pickStarter(s.prompt)}
        >
          <Icon name="sparkle" size={12} />
          <span>{s.label}</span>
        </button>
      {/each}
    </div>
  </div>
</div>

{#if browsingFolder}
  <FolderPicker
    start={cwd || homePath || ''}
    onpick={(picked) => {
      cwd = picked;
      cwdError = '';
      browsingFolder = false;
    }}
    onclose={() => (browsingFolder = false)}
  />
{/if}

<style>
  .nt-stage {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px 20px;
    background: var(--bg);
  }
  .nt-inner {
    width: 100%;
    max-width: 680px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .nt-hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 8px;
  }
  .nt-mark {
    inline-size: 48px;
    block-size: 48px;
    border-radius: var(--radius-l);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-card);
    display: grid;
    place-items: center;
    color: var(--accent-text);
  }
  .nt-title {
    margin: 0;
    font-size: var(--fs-2xl);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text);
  }
  .nt-subtitle {
    margin: 0;
    max-width: 500px;
    font-size: var(--fs-m);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .nt-harnesses {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
  .nt-harness-card {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 500;
    cursor: pointer;
    transition: background var(--dur-fast) var(--ease-out), border-color var(--dur-fast) var(--ease-out);
  }
  .nt-harness-card:hover {
    background: var(--surface-2);
    border-color: var(--border-strong);
  }
  .nt-harness-card.active {
    background: var(--accent-soft);
    border-color: var(--accent-line-strong);
    color: var(--text);
  }
  .nt-harness-card.unavailable {
    opacity: var(--disabled-opacity);
  }
  .nt-harness-name {
    white-space: nowrap;
  }
  .nt-unavail {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .nt-composer-card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    border-radius: var(--radius-l);
    border: 1px solid var(--control-border);
    background: var(--surface);
    box-shadow: var(--shadow-card);
  }
  .nt-slash-pop {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: min(220px, 35vh);
    overflow-y: auto;
    padding: 4px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface-2);
  }
  .nt-slash-row {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .nt-slash-row.active {
    background: var(--accent-soft);
  }
  .nt-slash-name {
    color: var(--accent-text);
    white-space: nowrap;
  }
  .nt-slash-desc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .nt-composer-card.nt-plan {
    border-color: color-mix(in srgb, var(--warning) 42%, var(--border));
  }
  .nt-composer-card:focus-within {
    border-color: var(--accent-line-strong);
  }
  .nt-branch {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    max-width: 150px;
  }
  .nt-composer-card textarea {
    width: 100%;
    min-block-size: 88px;
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
  .nt-composer-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
    padding-block-start: 8px;
    border-block-start: 1px solid var(--border);
  }
  .nt-pickers {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
  }
  .nt-pickers :global(.btn) {
    max-width: 180px;
  }
  .nt-submit-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-inline-start: auto;
  }
  .nt-starters {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
  .nt-starter-chip {
    cursor: pointer;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    padding: 6px 12px;
    font-size: var(--fs-xs);
    transition: background var(--dur-fast) var(--ease-out), color var(--dur-fast) var(--ease-out);
  }
  .nt-starter-chip:hover {
    background: var(--surface-2);
    color: var(--text);
    border-color: var(--border-strong);
  }
</style>
