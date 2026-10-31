<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import { api } from '../../lib/api/client';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { checkFolder } from '../../lib/folderCheck';
  import { agentProviders, defaultAgentProvider, providerReadiness } from '../../lib/providers';
  import { radioKey } from '../../lib/radioKey';
  import { onTabKey } from '../../lib/tabKeys';
  import { git } from '../../lib/stores/git.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { compactPath, harnessLabel, projectNameFromCwd } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  const STARTERS = [
    {
      label: 'Review uncommitted changes',
      prompt: 'Inspect git status and uncommitted diffs in this repo, highlight any bugs or edge cases, and suggest concrete fixes.',
    },
    {
      label: 'Trace a bug across the repo',
      prompt: 'Help me trace a bug end-to-end across this codebase: locate the entry point, data flow, and root cause before proposing a minimal fix.',
    },
    {
      label: 'Write tests for recent edits',
      prompt: 'Check the most recently modified files, identify uncovered behavior and edge cases, and write clean unit or integration tests.',
    },
    {
      label: 'Coordinate 2 sessions in split view',
      prompt: 'Outline a clear plan splitting this task into two independent tracks so I can run a second worker thread beside you in Split view.',
    },
  ] as const;

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
  let autonomy = $state<'standard' | 'auto'>('standard');
  let cwd = $state('');
  let useScratch = $state(false);
  let prompt = $state('');
  let cwdError = $state('');
  let busy = $state(false);
  let browsingFolder = $state(false);
  let openInBeside = $state(false);
  let composerEl = $state<HTMLTextAreaElement | null>(null);

  const homePath = $derived(ws.scratch?.root_path ?? '~');
  const scratchActive = $derived(useScratch || ws.current === null);

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
    const items: MenuItem[] = [
      ...ws.workspaces.map(
        (w): MenuItem => ({
          label: w.name,
          hint: compactPath(w.root_path, homePath),
          checked: !scratchActive && ws.currentId === w.id,
          action: () => {
            useScratch = false;
            cwd = w.root_path;
            cwdError = '';
            void ws.select(w.id);
          },
        }),
      ),
      { separator: true },
      {
        label: 'No workspace (Scratch)',
        hint: compactPath(homePath, homePath),
        checked: scratchActive,
        action: () => {
          useScratch = true;
          cwd = homePath;
          cwdError = '';
        },
      },
    ];
    ctxMenu.show(e, items);
  }

  function openFolderMenu(e: MouseEvent): void {
    const candidates: { label: string; path: string }[] = [];
    const seen = new Set<string>();
    const add = (label: string, path: string | null | undefined) => {
      const p = (path ?? '').trim().replace(/\/+$/, '');
      if (!p || seen.has(p)) return;
      seen.add(p);
      candidates.push({ label, path: p });
    };
    if (!scratchActive && ws.current?.root_path) add(ws.current.name, ws.current.root_path);
    for (const r of git.repos) add(r.name, r.path);
    for (const s of ws.sessions) {
      if (s.cwd) add(projectNameFromCwd(s.cwd), s.cwd);
    }
    const items: MenuItem[] = [
      ...candidates.slice(0, 10).map(
        (c): MenuItem => ({
          label: c.label,
          hint: compactPath(c.path, homePath),
          checked: cwd.replace(/\/+$/, '') === c.path,
          action: () => {
            cwd = c.path;
            cwdError = '';
          },
        }),
      ),
      ...(candidates.length > 0 ? [{ separator: true }] : []),
      {
        label: 'Browse folder…',
        icon: 'folder',
        action: () => (browsingFolder = true),
      },
    ];
    ctxMenu.show(e, items);
  }

  async function launch(withPrompt: boolean): Promise<void> {
    if (busy) return;
    const readiness = providerReadiness(provider);
    if (!readiness.available) {
      toasts.error(`Couldn’t start ${harnessLabel(provider)}`, readiness.message);
      return;
    }
    const firstMsg = withPrompt ? prompt.trim() : '';
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
        autonomy_mode: autonomy,
        ...(autonomy === 'auto' ? { auto_approve: true, permission_mode: 'auto' } : {}),
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

  function onComposerKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
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
          onclick={() => (provider = h)}
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

    <div class="nt-composer-card">
      <textarea
        bind:this={composerEl}
        bind:value={prompt}
        dir="auto"
        rows={3}
        placeholder="What should {harnessLabel(provider)} build, debug, or review?"
        aria-label="Opening prompt for new thread"
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

          <div class="segmented" role="group" aria-label="Permission mode">
            <button
              type="button"
              class:active={autonomy === 'standard'}
              aria-pressed={autonomy === 'standard'}
              onkeydown={onTabKey}
              title="Ask before risky commands or external actions"
              onclick={() => (autonomy = 'standard')}
            >
              Standard
            </button>
            <button
              type="button"
              class:active={autonomy === 'auto'}
              aria-pressed={autonomy === 'auto'}
              onkeydown={onTabKey}
              title="Auto-approve routine tool calls inside the sandbox"
              onclick={() => (autonomy = 'auto')}
            >
              <Icon name="zap" size={12} /> Auto-approve
            </button>
          </div>
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
      {#each STARTERS as s (s.label)}
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
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    border-radius: var(--radius-l);
    border: 1px solid var(--control-border);
    background: var(--surface);
    box-shadow: var(--shadow-card);
  }
  .nt-composer-card:focus-within {
    border-color: var(--accent-line-strong);
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
