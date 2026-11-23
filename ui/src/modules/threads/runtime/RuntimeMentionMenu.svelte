<script lang="ts">
  // Unified `@` mention popup shared across ALL Threads and IDE composers:
  // searches files & folders (`POST /ide/fs/complete` with `/ide/search` +
  // `/ide/fs/tree` fallback and 0ms open-tab matches) alongside `@agent`
  // provider/model mentions (`askAgents.ts`). Viewport-clamped with zero CLS.
  import { tick, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { formatModelLabel, modelsForProvider } from '../../../lib/providerModels';
  import { agentProviders, harnessLabel, providerReadiness } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { askableProviders, mentionOptions, type AskOption } from './askAgents';
  import { fetchMentionFileItems, instantOpenTabMentions } from './mentionClient';
  import {
    atMentionTokenAt,
    buildUnifiedMentionResults,
    replaceAtMentionToken,
    type MentionFilterMode,
    type RankedFileMention,
    type UnifiedMentionItem,
  } from './mentionMatcher';

  export interface PickedFileMention {
    path: string; display: string; insertToken: string; isDir: boolean; nextText: string; nextCaret: number;
  }
  export interface PickedAgentMention { option: AskOption; nextText: string; nextCaret: number }

  interface Props {
    text: string; cwd?: string | null; textareaEl?: HTMLTextAreaElement | null; anchorEl?: HTMLElement | null;
    allowAgents?: boolean; agentSectionLabel?: string;
    onpickfile: (picked: PickedFileMention) => void;
    onpickagent?: (picked: PickedAgentMention) => void;
    ondrilldir: (nextText: string, nextCaret: number) => void;
  }

  let {
    text, cwd = null, textareaEl = null, anchorEl = null, allowAgents = true,
    agentSectionLabel = 'Delegate to…', onpickfile, onpickagent, ondrilldir,
  }: Props = $props();

  let filter = $state<MentionFilterMode>('all');
  let dismissedKey = $state<string | null>(null);
  let activeIdx = $state(0);
  let loadingFiles = $state(false);
  let fileResults = $state<RankedFileMention[]>([]);
  let caretPos = $state<number | null>(null);
  let composerFocused = $state(false);
  let placeBelow = $state(false);
  let maxBlockPx = $state(300);
  let menuEl = $state<HTMLDivElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);

  const hasAtChar = $derived(text.includes('@'));

  function syncCaret(): void {
    if (!untrack(() => hasAtChar)) {
      if (caretPos !== null) caretPos = null;
      return;
    }
    if (textareaEl && typeof document !== 'undefined') {
      composerFocused = document.activeElement === textareaEl;
      if (composerFocused) {
        caretPos = textareaEl.selectionStart ?? text.length;
      }
    } else {
      composerFocused = true;
      caretPos = text.length;
    }
  }

  $effect(() => {
    void text;
    if (!hasAtChar) {
      if (untrack(() => caretPos) !== null) caretPos = null;
      return;
    }
    untrack(() => syncCaret());
  });

  $effect(() => {
    const el = textareaEl;
    if (!el) {
      composerFocused = true;
      return;
    }
    composerFocused = typeof document !== 'undefined' && document.activeElement === el;
    // A focused textarea removed from the page (its thread closing, a pane
    // swap) blurs DURING Svelte's teardown, when writing state is unsafe
    // (`state_unsafe_mutation`). Its listeners are gone by then in our
    // cleanup, but the removal can fire first: write only while this effect
    // is live and the field is still on the page, and after the update.
    let live = true;
    const onCaret = (): void => syncCaret();
    const onFocus = (): void => {
      composerFocused = true;
      syncCaret();
    };
    const onBlur = (): void => {
      if (!live || !el.isConnected) return;
      queueMicrotask(() => {
        if (live && document.activeElement !== el) composerFocused = false;
      });
    };
    el.addEventListener('keyup', onCaret);
    el.addEventListener('click', onCaret);
    el.addEventListener('focus', onFocus);
    el.addEventListener('blur', onBlur);
    return () => {
      live = false;
      el.removeEventListener('keyup', onCaret);
      el.removeEventListener('click', onCaret);
      el.removeEventListener('focus', onFocus);
      el.removeEventListener('blur', onBlur);
    };
  });

  const effectiveCaret = $derived(
    caretPos !== null && caretPos <= text.length ? caretPos : text.length,
  );
  const token = $derived(composerFocused ? atMentionTokenAt(text, effectiveCaret) : null);
  const tokenKey = $derived(token ? `${token.start}:${token.query}` : null);
  const isDismissed = $derived(tokenKey !== null && dismissedKey === tokenKey);

  // Reset filter to `all` when a brand-new `@` token opens.
  let lastStart = -1;
  $effect(() => {
    const st = token?.start ?? -1;
    if (st !== lastStart) {
      lastStart = st;
      if (st >= 0) filter = 'all';
    }
  });

  const agentOptions = $derived.by((): AskOption[] => {
    if (!token || isDismissed || !allowAgents || !onpickagent) return [];
    return mentionOptions(
      token.query,
      (p) => modelsForProvider(p, auth.meta),
      (p) => harnessLabel(p),
      8,
      askableProviders(agentProviders(), auth.meta).filter((p) => providerReadiness(p).available),
    ).map((o) => ({
      ...o,
      label: o.model ? `${harnessLabel(o.provider)} · ${formatModelLabel(o.provider, o.model, auth.meta)}` : o.label,
    }));
  });

  let fetchTimer: ReturnType<typeof setTimeout> | null = null;
  let abortCtl: AbortController | null = null;

  $effect(() => {
    const tok = token;
    const targetCwd = cwd;
    const curFilter = filter;
    const hidden = isDismissed;

    if (fetchTimer) {
      clearTimeout(fetchTimer);
      fetchTimer = null;
    }
    abortCtl?.abort();
    abortCtl = null;

    if (!tok || hidden || curFilter === 'agents') {
      loadingFiles = false;
      if (!tok || curFilter === 'agents') fileResults = [];
      return;
    }

    const q = tok.query;
    const instant = untrack(() => instantOpenTabMentions(q, targetCwd));
    if (instant.length > 0) fileResults = instant;

    const delayMs = q === '' || q.endsWith('/') ? 0 : 40;
    loadingFiles = true;
    const ctl = new AbortController();
    abortCtl = ctl;

    fetchTimer = setTimeout(() => {
      void fetchMentionFileItems({ query: q, cwd: targetCwd, signal: ctl.signal })
        .then((ranked) => {
          if (ctl.signal.aborted) return;
          fileResults = ranked;
          loadingFiles = false;
        })
        .catch(() => {
          if (!ctl.signal.aborted) loadingFiles = false;
        });
    }, delayMs);

    return () => {
      if (fetchTimer) clearTimeout(fetchTimer);
      ctl.abort();
    };
  });

  const unified = $derived.by(() => {
    if (!token || isDismissed) {
      return { files: [], agents: [], agentsFirst: false, flat: [] as UnifiedMentionItem[] };
    }
    return buildUnifiedMentionResults({
      query: token.query,
      filter,
      files: fileResults,
      agents: agentOptions,
      allowAgents: allowAgents && Boolean(onpickagent),
    });
  });

  const menuOpen = $derived(
    Boolean(token && !isDismissed && (unified.flat.length > 0 || loadingFiles || filter !== 'all')),
  );

  $effect(() => {
    void unified.flat.length;
    void tokenKey;
    void filter;
    activeIdx = 0;
  });

  // Strict viewport clamping: never negative top, never clipped at bottom, zero CLS.
  function recomputeClamp(): void {
    const ref = anchorEl ?? textareaEl?.parentElement ?? textareaEl;
    if (!ref || typeof window === 'undefined') return;
    const rect = ref.getBoundingClientRect();
    const vh = window.innerHeight || 720;
    const spaceAbove = Math.max(0, rect.top - 12);
    const spaceBelow = Math.max(0, vh - rect.bottom - 12);
    const below = spaceAbove < 180 && spaceBelow > spaceAbove;
    placeBelow = below;
    const avail = below ? spaceBelow : spaceAbove;
    maxBlockPx = Math.max(0, Math.min(320, Math.floor(vh * 0.5), Math.floor(avail)));
  }

  $effect(() => {
    if (!menuOpen || typeof window === 'undefined') return;
    recomputeClamp();
    const onPointerDown = (ev: PointerEvent): void => {
      const target = ev.target as Node | null;
      if (!target) return;
      if (menuEl?.contains(target) || textareaEl?.contains(target)) return;
      dismissedKey = tokenKey;
    };
    window.addEventListener('resize', recomputeClamp);
    document.addEventListener('pointerdown', onPointerDown, true);
    return () => {
      window.removeEventListener('resize', recomputeClamp);
      document.removeEventListener('pointerdown', onPointerDown, true);
    };
  });

  function scrollActiveIntoView(): void {
    void tick().then(() => {
      const activeEl = listEl?.querySelector<HTMLElement>(`[data-mention-idx="${activeIdx}"]`);
      activeEl?.scrollIntoView({ block: 'nearest' });
    });
  }

  function applyCaret(nextCaret: number): void {
    caretPos = nextCaret;
    queueMicrotask(() => {
      if (!textareaEl) return;
      textareaEl.focus({ preventScroll: true });
      try {
        textareaEl.setSelectionRange(nextCaret, nextCaret);
      } catch {
        // Ignore selection errors on detached inputs.
      }
    });
  }

  function selectFileItem(item: RankedFileMention, forceAttachFolder = false): void {
    if (!token) return;
    if (item.isDir && !forceAttachFolder) {
      const drilled = replaceAtMentionToken(text, token, item.insertToken, { keepOpen: true });
      ondrilldir(drilled.text, drilled.caret);
      applyCaret(drilled.caret);
      return;
    }
    const cleanTok = item.isDir
      ? item.insertToken.endsWith('/') ? item.insertToken : `${item.insertToken}/`
      : item.insertToken;
    const replaced = replaceAtMentionToken(text, token, cleanTok, { keepOpen: false });
    onpickfile({
      path: item.path,
      display: item.display,
      insertToken: cleanTok,
      isDir: item.isDir,
      nextText: replaced.text,
      nextCaret: replaced.caret,
    });
    applyCaret(replaced.caret);
  }

  function selectAgentItem(opt: AskOption): void {
    if (!token || !onpickagent) return;
    const replaced = replaceAtMentionToken(text, token, '', { keepOpen: false });
    onpickagent({
      option: opt,
      nextText: replaced.text,
      nextCaret: replaced.caret,
    });
    applyCaret(replaced.caret);
  }

  function activateEntry(entry: UnifiedMentionItem | undefined): void {
    if (!entry) return;
    if (entry.kind === 'file') selectFileItem(entry.file, false);
    else selectAgentItem(entry.agent);
  }

  export function onKey(e: KeyboardEvent): boolean {
    if (!menuOpen) return false;
    const total = unified.flat.length;
    if (e.key === 'Escape') {
      dismissedKey = tokenKey;
      e.preventDefault();
      return true;
    }
    if (total === 0) return false;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      activeIdx = (activeIdx + (e.key === 'ArrowDown' ? 1 : total - 1)) % total;
      scrollActiveIntoView();
      e.preventDefault();
      return true;
    }
    if (e.key === 'Tab' && !e.shiftKey) {
      activateEntry(unified.flat[activeIdx]);
      e.preventDefault();
      return true;
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.metaKey && !e.ctrlKey) {
      activateEntry(unified.flat[activeIdx]);
      e.preventDefault();
      return true;
    }
    return false;
  }

  const fileOffset = $derived(unified.agentsFirst ? unified.agents.length : 0);
  const agentOffset = $derived(unified.agentsFirst ? 0 : unified.files.length);
</script>

{#snippet filesSection()}
  {#if unified.files.length > 0 || (filter === 'files' && !loadingFiles)}
    <div class="rmm-sec" data-testid="at-mention-files">
      <div class="rmm-sec-head">
        <span>Files &amp; folders</span>
        {#if loadingFiles}<span class="rmm-status">Searching…</span>{/if}
      </div>
      {#each unified.files as f, idx (f.key)}
        {@const flatIndex = fileOffset + idx}
        <div
          class="rmm-item"
          role="presentation"
          class:active={flatIndex === activeIdx}
          data-mention-idx={flatIndex}
          onmouseenter={() => (activeIdx = flatIndex)}
        >
          <button
            type="button"
            class="rmm-row"
            role="option"
            aria-selected={flatIndex === activeIdx}
            onmousedown={(ev) => ev.preventDefault()}
            onclick={() => selectFileItem(f, false)}
          >
            <span class="rmm-ico" class:dir={f.isDir}><Icon name={f.isDir ? 'folder' : 'file'} size={13} /></span>
            <span class="rmm-name mono ellipsis" dir="ltr">
              {#each f.nameParts as part, pi (pi)}
                {#if part.hit}<mark class="rmm-hit">{part.text}</mark>{:else}{part.text}{/if}
              {/each}
            </span>
            {#if f.openTab}<span class="rmm-tag">Open</span>{/if}
            <span class="rmm-meta mono ellipsis" dir="ltr" title={f.path}>{f.dirHint || f.display}</span>
          </button>
          {#if f.isDir}
            <button
              type="button"
              class="rmm-attach-dir"
              aria-label="Attach folder {f.display}"
              title="Attach folder {f.display}"
              data-testid="at-mention-attach-folder"
              onmousedown={(ev) => ev.preventDefault()}
              onclick={() => selectFileItem(f, true)}
            >Attach</button>
          {/if}
        </div>
      {:else}
        <p class="rmm-empty">No matching files or folders</p>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet agentsSection()}
  {#if unified.agents.length > 0 || filter === 'agents'}
    <div class="rmm-sec" data-testid="at-mention-agents">
      <div class="rmm-sec-head"><span>{agentSectionLabel}</span></div>
      {#each unified.agents as a, idx (`${a.provider}:${a.model ?? ''}`)}
        {@const flatIndex = agentOffset + idx}
        <div
          class="rmm-item"
          role="presentation"
          class:active={flatIndex === activeIdx}
          data-mention-idx={flatIndex}
          onmouseenter={() => (activeIdx = flatIndex)}
        >
          <button
            type="button"
            class="rmm-row"
            role="option"
            aria-selected={flatIndex === activeIdx}
            onmousedown={(ev) => ev.preventDefault()}
            onclick={() => selectAgentItem(a)}
          >
            <span class="rmm-ico">
              {#if hasProviderIcon(a.provider)}<ProviderIcon provider={a.provider} size={13} />{:else}<Icon name="assistant" size={13} />{/if}
            </span>
            <span class="rmm-name ellipsis">{a.label}</span>
            <span class="rmm-meta ellipsis">{a.detail}</span>
          </button>
        </div>
      {:else}
        <p class="rmm-empty">No matching agents</p>
      {/each}
    </div>
  {/if}
{/snippet}

{#if menuOpen}
  <div bind:this={menuEl} class="rmm" class:below={placeBelow} style:--rmm-max="{maxBlockPx}px" data-testid="runtime-at-mention-menu">
    <div class="rmm-bar">
      <div class="rmm-filters" role="group" aria-label="Filter mentions">
        <button type="button" class="rmm-pill" class:on={filter === 'all'} aria-pressed={filter === 'all'} onmousedown={(e) => e.preventDefault()} onclick={() => (filter = 'all')}>All</button>
        <button type="button" class="rmm-pill" class:on={filter === 'files'} aria-pressed={filter === 'files'} onmousedown={(e) => e.preventDefault()} onclick={() => (filter = 'files')}>Files</button>
        {#if allowAgents && onpickagent}
          <button type="button" class="rmm-pill" class:on={filter === 'agents'} aria-pressed={filter === 'agents'} onmousedown={(e) => e.preventDefault()} onclick={() => (filter = 'agents')}>Agents</button>
        {/if}
      </div>
      <span class="rmm-hint">↑↓ navigate · ⇥ open folder · ↵ select</span>
    </div>

    <div bind:this={listEl} class="rmm-scroll scroll-thin" role="listbox" aria-label="Mentions" data-testid="runtime-ask-mentions">
      {#if unified.agentsFirst}
        {@render agentsSection()}
        {@render filesSection()}
      {:else}
        {@render filesSection()}
        {@render agentsSection()}
      {/if}
      {#if unified.flat.length === 0 && loadingFiles}
        <p class="rmm-empty">Searching workspace files…</p>
      {:else if unified.flat.length === 0 && filter === 'all'}
        <p class="rmm-empty">No matching files or agents</p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .rmm {
    position: absolute; inset-inline: 0; inset-block-end: calc(100% + 6px); z-index: var(--z-popover);
    display: flex; flex-direction: column; max-block-size: min(var(--rmm-max, 320px), 320px);
    border: 1px solid var(--border-strong); border-radius: var(--radius-m);
    background: var(--surface); box-shadow: var(--glass-shadow); overflow: hidden;
  }
  .rmm.below { inset-block-end: auto; inset-block-start: calc(100% + 6px); }
  .rmm-bar {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding: 4px 8px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0;
  }
  .rmm-filters { display: inline-flex; align-items: center; gap: 2px; }
  .rmm-pill {
    padding: 2px 8px; border: 0; border-radius: 999px; background: transparent;
    color: var(--text-dim); font: inherit; font-size: var(--fs-xs); font-weight: 500; cursor: pointer;
  }
  .rmm-pill:hover { color: var(--text); background: var(--hover); }
  .rmm-pill.on { background: var(--accent-soft); color: var(--accent-text); }
  .rmm-hint {
    font-size: var(--fs-xs); color: var(--text-dim);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .rmm-scroll {
    flex: 1; min-block-size: 0; overflow-y: auto; padding: 4px;
    display: flex; flex-direction: column; gap: 4px;
  }
  .rmm-sec { display: flex; flex-direction: column; gap: 2px; }
  .rmm-sec-head {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding: 2px 8px; font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim);
  }
  .rmm-status { font-weight: 400; }
  .rmm-item {
    display: flex; align-items: center; gap: 4px; min-inline-size: 0;
    padding-inline-end: 4px; border-radius: var(--radius-s);
  }
  .rmm-item.active { background: var(--accent-soft); }
  .rmm-row {
    flex: 1; display: flex; align-items: center; gap: 6px; min-inline-size: 0;
    padding: 4px 8px; border: 0; border-radius: var(--radius-s); background: transparent;
    font: inherit; font-size: var(--fs-s); color: var(--text); text-align: start; cursor: pointer;
  }
  .rmm-ico { display: inline-flex; align-items: center; color: var(--text-dim); flex-shrink: 0; }
  .rmm-ico.dir { color: var(--accent-text); }
  .rmm-name { min-inline-size: 0; flex-shrink: 1; }
  .rmm-hit { background: transparent; color: var(--accent-text); font-weight: 600; }
  .rmm-tag {
    padding-inline: 6px; border-radius: 999px; background: var(--surface-2);
    color: var(--text-dim); font-size: var(--fs-xs); flex-shrink: 0;
  }
  .rmm-meta {
    margin-inline-start: auto; max-inline-size: 48%;
    font-size: var(--fs-xs); color: var(--text-dim); text-align: end;
  }
  .rmm-attach-dir {
    padding: 2px 6px; border: 1px solid var(--border); border-radius: var(--radius-s);
    background: var(--surface-2); color: var(--text-dim); font: inherit; font-size: var(--fs-xs);
    cursor: pointer; flex-shrink: 0;
  }
  .rmm-attach-dir:hover {
    border-color: var(--accent-line); color: var(--accent-text); background: var(--accent-soft);
  }
  .rmm-empty { margin: 0; padding: 8px; font-size: var(--fs-xs); color: var(--text-dim); text-align: center; }
  @media (max-width: 640px) { .rmm-hint { display: none; } }
</style>
