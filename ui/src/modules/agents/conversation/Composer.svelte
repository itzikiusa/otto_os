<script module lang="ts">
  import { SvelteMap } from 'svelte/reactivity';

  /** Image uploads still in flight, per session. Module-level on purpose: the
   *  composer is keyed by session and remounts when you switch tabs, but an
   *  upload started in A keeps running and lands in A's draft — so A's Send
   *  must stay blocked after a remount until it finishes. */
  const pendingUploads = new SvelteMap<string, number>();
  function addPending(owner: string, n: number): void {
    const next = (pendingUploads.get(owner) ?? 0) + n;
    if (next > 0) pendingUploads.set(owner, next);
    else pendingUploads.delete(owner);
  }
</script>

<script lang="ts">
  // Chat composer (sessionId mode, editor role, session not exited). One box
  // that grows with the text (2 lines at rest, up to ~40% of the window) with
  // its tools on a row inside it: attach image · the key hints · Stop (while
  // the agent works: one Esc into its terminal, what "esc to interrupt" does)
  // · Send. While the agent is busy a send is still accepted — Claude Code /
  // Codex queue typed input and deliver it when the turn ends (the chat shows
  // it as a "Queued" chip until then), so the box says so instead of blocking.
  // ⏎ submits
  // exactly the typed text as ONE prompt into the agent's PTY, ⇧⏎ inserts a
  // newline, `/` passes through untouched (slash commands are the CLI's) — with
  // a completion popup listing the provider's built-ins plus the user's own
  // commands/skills (`GET …/slash-commands`). Pasted / dropped images upload
  // to the session inbox and show as thumbnails under the box; on send each
  // becomes an explicit `[Image: <path>]` line after the text. The status line
  // shows the session status and any board tasks still waiting to be nudged in.
  // OS text prediction / autocorrect is off here: the bubble it pops over the
  // box is noise when you are typing paths, flags and slash commands.
  import { untrack } from 'svelte';
  import { toastError } from '../../../lib/toastError';
  import { toasts } from '../../../lib/toast.svelte';
  import { activity } from '../../../lib/stores/activity.svelte';
  import type { SessionStatus, SlashCommand } from '../../../lib/api/types';
  import { fetchSlashCommands, interruptAgent, submitPrompt, uploadInboxImage } from './api';
  import { tick } from 'svelte';

  import { transcript } from '../../../lib/stores/transcript.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { events } from '../../../lib/events.svelte';
  import { sessionState } from '../../../lib/status';
  import StatusDot from '../../../lib/components/StatusDot.svelte';
  import Icon from '../../../lib/components/Icon.svelte';

  interface Props {
    sessionId: string;
    status: SessionStatus;
    /** Exited / reconnectable → Resume instead of a textarea. */
    onresume: () => void;
    /** Status row: where the agent runs, on which branch, which model, and
     *  whatever its own status line shows (context %, limits, mode …). */
    cwd?: string;
    branch?: string | null;
    model?: string | null;
    termStatus?: string;
    /** Unsent text sitting in the terminal's input box — a chat send is
     *  appended to it by the CLI, so it is shown as the message's prefix. */
    termInput?: string;
    /** "Claude" / "Codex" — the placeholder and hints name the agent. */
    agentName?: string;
  }
  let { sessionId, status, onresume, cwd = '', branch = null, model = null, termStatus = '', termInput = '', agentName = 'the agent' }: Props = $props();

  // The keyed parent creates one instance per session. Capture ownership once
  // so an upload or submit that finishes after navigation still updates A.
  const ownerId = untrack(() => sessionId);
  // Read the shared draft directly: an older pending send can finish after
  // this same session is reopened, so component-local copies would go stale.
  const text = $derived(transcript.draft(ownerId));
  const shortCwd = $derived(cwd.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~'));
  const sending = $derived(transcript.sending(ownerId));
  const uploading = $derived(pendingUploads.get(ownerId) ?? 0);
  let ta = $state<HTMLTextAreaElement | null>(null);

  interface Attachment {
    path: string;
    name: string;
    /** Object URL of the local file — preview only, never sent. */
    url: string;
  }
  const attachments = $derived(transcript.attachments(ownerId));

  // The shared session state (lib/status.ts): the same words the sidebar, tab
  // and pane header use — a resumable session is "Suspended", not "ended".
  const st = $derived(
    sessionState(
      ws.getSession(sessionId),
      status,
      ws.needsYou[sessionId] === true,
      { stale: events.state !== 'connected' },
    ),
  );
  const exited = $derived(st.inactive);
  const pendingNudges = $derived(activity.tasks(sessionId).filter((t) => t.nudge_pending).length);
  const statusLabel = $derived(st.key === 'working' ? 'Working…' : st.label);

  // Two lines at rest (CSS min-height; one in a narrow tile), growing with the
  // text up to ~40% of the window; the textarea itself never scrolls sideways (wrap +
  // overflow-x hidden), so nothing overlays the placeholder.
  // The cap is the smaller of 40% of the window and half of the PANE the composer
  // sits in (a short tile or split pane), so the box never crowds out the chat;
  // past it the textarea scrolls inside.
  // Half the pane is not enough on its own: in a short tile the composer's own
  // rows (attachments, toolbar) and the pane's fixed rows (header, status) eat
  // the rest, so the cap also reserves MIN_CHAT px for the conversation itself.
  let paneH = $state(0);
  /** Conversation viewport kept visible however tall the draft gets (~5 lines). */
  const MIN_CHAT = 96;
  function paneBudget(): number {
    const pane = composerEl?.parentElement;
    if (!pane || !composerEl || !ta) return Infinity;
    let fixed = 0;
    for (const el of Array.from(pane.children) as HTMLElement[]) {
      if (el === composerEl || parseFloat(getComputedStyle(el).flexGrow) > 0) continue;
      fixed += el.offsetHeight;
    }
    const chrome = composerEl.offsetHeight - ta.offsetHeight;
    return pane.clientHeight - fixed - chrome - MIN_CHAT;
  }
  function autosize(): void {
    if (!ta) return;
    const budget = paneBudget();
    ta.style.height = 'auto';
    const byWindow = Math.floor(window.innerHeight * 0.4);
    const byPane = paneH > 0 ? Math.floor(paneH * 0.5) : byWindow;
    // One line is the floor when the pane is too short for the budget.
    const cap = Math.max(24, Math.min(byWindow, byPane, Math.floor(budget)));
    ta.style.height = `${Math.min(cap, Math.max(ta.scrollHeight, 0))}px`;
  }
  let composerEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const pane = composerEl?.parentElement;
    if (!pane || typeof ResizeObserver === 'undefined') return;
    const measure = (): void => {
      const h = pane.clientHeight;
      if (Math.abs(h - paneH) > 1) {
        paneH = h;
        autosize();
      }
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(pane);
    return () => ro.disconnect();
  });
  // A draft restored on mount / reopened session sizes the box too.
  $effect(() => {
    void text;
    void tick().then(autosize);
  });

  // ---- busy: queue + interrupt -------------------------------------------------
  const busy = $derived(status === 'working');
  let stopping = $state(false);
  async function interrupt(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await interruptAgent(ownerId);
      toasts.info('Interrupt sent', `${agentName} stops after its current step`);
    } catch (e) {
      toastError('Couldn’t interrupt the agent', e);
    } finally {
      stopping = false;
      ta?.focus();
    }
  }
  let fileEl = $state<HTMLInputElement | null>(null);
  function pickImages(e: Event): void {
    const input = e.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    void addImages(files);
  }

  // ---- slash-command completion ------------------------------------------------
  let cmds = $state<SlashCommand[] | null>(null);
  let cmdsFor = '';
  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
  let listEl = $state<HTMLDivElement | null>(null);
  /** `/pre` on a single line → the prefix being completed, else null. */
  const cmdPrefix = $derived.by(() => {
    const m = /^\/([\w:-]*)$/.exec(text);
    return m ? m[1].toLowerCase() : null;
  });
  const suggestions = $derived.by(() => {
    if (cmdPrefix == null || cmdDismissed || !cmds) return [] as SlashCommand[];
    const starts = cmds.filter((c) => c.name.toLowerCase().startsWith(cmdPrefix));
    const within = cmds.filter((c) => !c.name.toLowerCase().startsWith(cmdPrefix) && c.name.toLowerCase().includes(cmdPrefix));
    return [...starts, ...within].slice(0, 12);
  });
  const cmdOpen = $derived(suggestions.length > 0);
  // The slash popup is a listbox driven from the textarea (aria-activedescendant):
  // focus never leaves the composer, ↑/↓ move the active option.
  const popId = $props.id();
  $effect(() => {
    // Load once per session, the first time a `/` is typed at the start.
    if (cmdPrefix == null || cmdsFor === sessionId) return;
    cmdsFor = sessionId;
    void fetchSlashCommands(sessionId)
      .then((list) => (cmds = list))
      .catch(() => (cmds = []));
  });
  $effect(() => {
    void suggestions.length;
    cmdIdx = 0;
  });
  $effect(() => {
    // Typing again after Esc re-opens the list.
    void text;
    cmdDismissed = false;
  });
  $effect(() => {
    const el = listEl?.children[cmdIdx] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  });
  // The popup opens ABOVE the box: cap its height to the room between the box and
  // the top of the pane (or the window), so it never runs off the top edge.
  let wrapEl = $state<HTMLDivElement | null>(null);
  let popMax = $state(320);
  $effect(() => {
    void suggestions.length;
    void paneH;
    if (!cmdOpen || !wrapEl) return;
    const paneTop = Math.max(0, composerEl?.parentElement?.getBoundingClientRect().top ?? 0);
    const room = wrapEl.getBoundingClientRect().top - paneTop - 12;
    // Honour the real room even when it is tiny (a short split + attachments):
    // a taller floor would push the list past the pane's clip. It scrolls.
    popMax = Math.floor(Math.max(0, Math.min(320, window.innerHeight * 0.5, room)));
  });
  function acceptCmd(c: SlashCommand): void {
    transcript.setDraft(ownerId, `/${c.name} `);
    cmdDismissed = true;
    queueMicrotask(() => {
      autosize();
      ta?.focus();
      ta?.setSelectionRange(text.length, text.length);
    });
  }

  async function send(): Promise<void> {
    const typed = text.replace(/\s+$/, '');
    const imgs = attachments.map((a) => `[Image: ${a.path}]`);
    const body = [typed, ...imgs].filter(Boolean).join('\n');
    // An upload still in flight would land in the NEXT draft: wait for it, send by hand.
    if (!body || uploading > 0 || !transcript.tryBeginSend(ownerId)) return;
    try {
      const submittedImages = [...attachments];
      await submitPrompt(ownerId, body);
      // Do not erase new text or files added while this send was in flight.
      if (transcript.draft(ownerId).replace(/\s+$/, '') === typed) {
        transcript.setDraft(ownerId, '');
      }
      for (const a of submittedImages) URL.revokeObjectURL(a.url);
      transcript.setAttachments(ownerId, transcript.attachments(ownerId).filter((a) => !submittedImages.includes(a)));
      queueMicrotask(autosize);
    } catch (e) {
      toastError('Couldn’t send the message', e);
    } finally {
      transcript.finishSend(ownerId);
      ta?.focus();
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.isComposing) return;
    if (cmdOpen) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        cmdIdx = (cmdIdx + 1) % suggestions.length;
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        cmdIdx = (cmdIdx - 1 + suggestions.length) % suggestions.length;
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        cmdDismissed = true;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        const pick = suggestions[cmdIdx];
        // ⏎ on an exact, fully-typed name sends it; anything else completes.
        if (e.key === 'Enter' && pick && `/${pick.name}`.toLowerCase() === text.trim().toLowerCase()) {
          e.preventDefault();
          void send();
          return;
        }
        e.preventDefault();
        if (pick) acceptCmd(pick);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }
  const sendTitle = $derived(
    uploading > 0
      ? 'Uploading images… send is available when they finish'
      : busy
        ? `Send (⏎) — ${agentName} is busy, so it is queued and delivered when the current turn ends`
        : 'Send (⏎)',
  );

  async function addImages(files: File[]): Promise<void> {
    const imgs = files.filter((f) => f.type.startsWith('image/'));
    if (!imgs.length) return;
    addPending(ownerId, imgs.length);
    for (const f of imgs) {
      try {
        const name = f.name && f.name !== 'image.png' ? f.name : `paste-${Date.now()}.${(f.type.split('/')[1] ?? 'png').replace('jpeg', 'jpg')}`;
        const path = await uploadInboxImage(ownerId, f, name);
        transcript.setAttachments(ownerId, [...transcript.attachments(ownerId), { path, name, url: URL.createObjectURL(f) }]);
      } catch (e) {
        toastError('Couldn’t upload the image', e);
      } finally {
        addPending(ownerId, -1);
      }
    }
    ta?.focus();
  }

  function removeAttachment(a: Attachment): void {
    URL.revokeObjectURL(a.url);
    transcript.setAttachments(ownerId, attachments.filter((x) => x !== a));
  }

  function onPaste(e: ClipboardEvent): void {
    const files = Array.from(e.clipboardData?.files ?? []).filter((f) => f.type.startsWith('image/'));
    if (!files.length) return;
    e.preventDefault();
    void addImages(files);
  }

  function onDrop(e: DragEvent): void {
    const files = Array.from(e.dataTransfer?.files ?? []);
    if (!files.some((f) => f.type.startsWith('image/'))) return;
    e.preventDefault();
    void addImages(files);
  }

  const canSend = $derived(!sending && uploading === 0 && (text.trim().length > 0 || attachments.length > 0));
</script>

<div class="composer" bind:this={composerEl} data-status={status} ondragover={(e) => e.preventDefault()} ondrop={onDrop} role="group" aria-label="Message composer">
  {#if exited}
    <div class="exited">
      <span class="dim">{st.key === 'suspended' ? `${st.hint}.` : 'This session has ended.'}</span>
      <button class="btn small primary" onclick={onresume}>Resume</button>
    </div>
  {:else}
    <div class="box-wrap" bind:this={wrapEl}>
      {#if cmdOpen}
        <div class="cmd-pop" id="{popId}-list" bind:this={listEl} style="--pop-max:{popMax}px" role="listbox" aria-label="Slash commands" data-slash-pop>
          {#each suggestions as c, i (c.name)}
            <!-- Picked on press: mousedown keeps focus (and the caret) in the
                 textarea; the keyboard path is ↑/↓ + Tab/⏎ there. -->
            <div
              id="{popId}-opt-{i}"
              class="cmd-row"
              class:active={i === cmdIdx}
              role="option"
              tabindex="-1"
              aria-selected={i === cmdIdx}
              onmousedown={(e) => { e.preventDefault(); acceptCmd(c); }}
              onmouseenter={() => (cmdIdx = i)}
            >
              <span class="cmd-name mono">/{c.name}</span>
              <span class="cmd-desc dim">{c.description}</span>
              <span class="cmd-src dim">{c.source === 'builtin' ? '' : c.source}</span>
            </div>
          {/each}
          <div class="cmd-hint dim">↑↓ choose · Tab/⏎ complete · Esc dismiss</div>
        </div>
      {/if}
      <div class="box" class:busy>
        <textarea
          bind:this={ta}
          bind:value={() => text, (value) => transcript.setDraft(ownerId, value)}
          rows="1"
          placeholder={busy ? `Queue a message for ${agentName}…` : `Message ${agentName} — / for commands`}
          aria-label="Message {agentName}"
          spellcheck="false"
          {...{ autocorrect: 'off' }}
          autocapitalize="off"
          autocomplete="off"
          dir="auto"
          oninput={autosize}
          onkeydown={onKeydown}
          onpaste={onPaste}
          aria-autocomplete="list"
          aria-controls={cmdOpen ? `${popId}-list` : undefined}
          aria-activedescendant={cmdOpen ? `${popId}-opt-${cmdIdx}` : undefined}
        ></textarea>
        <div class="tools">
          <input bind:this={fileEl} type="file" accept="image/*" multiple hidden onchange={pickImages} />
          <button class="icon-btn tool" onclick={() => fileEl?.click()} aria-label="Attach images" title="Attach images (or paste / drop them)"><Icon name="image" size={14} /></button>
          <span class="keys" aria-hidden="true">{busy ? 'Busy — ⏎ queues' : '⏎ send'} · ⇧⏎ new line</span>
          <span class="grow"></span>
          {#if busy}
            <button class="btn small stop" onclick={() => void interrupt()} disabled={stopping} title="Interrupt {agentName} — sends Esc to its terminal"><Icon name="stop" size={11} /> Stop</button>
          {/if}
          {#if uploading > 0}<span class="uploading" role="status">Uploading…</span>{/if}
          <button class="send" onclick={() => void send()} disabled={!canSend} title={sendTitle} aria-label={uploading > 0 ? 'Send (uploading images)' : 'Send'}><Icon name="send" size={14} /></button>
        </div>
      </div>
      {#if attachments.length}
        <div class="thumbs" data-attachments={attachments.length}>
          {#each attachments as a (a.path)}
            <div class="thumb" title={a.path}>
              <img src={a.url} alt={a.name} />
              <button class="thumb-x" onclick={() => removeAttachment(a)} title="Remove" aria-label="Remove {a.name}"><Icon name="x" size={12} /></button>
            </div>
          {/each}
        </div>
      {/if}
    </div>
    {#if termInput}
      <div class="term-input" data-term-input title="Typed in the terminal but not sent. Sending from here appends your text after it — the CLI submits both as one message.">
        <span class="dim">In terminal:</span> <span class="mono">{termInput}</span>
      </div>
    {/if}
    <div class="status" data-status-line>
      <StatusDot state={st} />
      <span>{statusLabel}</span>
      {#if uploading}<span class="dim">· uploading {uploading} image{uploading > 1 ? 's' : ''}…</span>{/if}
      {#if pendingNudges}
        <span class="dim" title="Board tasks waiting for the agent to go idle">· {pendingNudges} board task{pendingNudges > 1 ? 's' : ''} pending</span>
      {/if}
      {#if shortCwd}<span class="sep sep-cwd">·</span><span class="mono cwd" title={cwd}>{shortCwd}</span>{/if}
      {#if branch}<span class="sep sep-branch">·</span><span class="mono branch" title="Git branch">⎇ {branch}</span>{/if}
      {#if model}<span class="sep sep-model">·</span><span class="mono model" title="Model">{model}</span>{/if}
      {#if termStatus}<span class="sep">·</span><span class="term-status" title="The agent’s own status line">{termStatus}</span>{/if}
    </div>
  {/if}
</div>

<style>
  .composer {
    border-top: 1px solid var(--border);
    background: var(--bg);
    /* The chat column's gutters, so the box lines up with the messages. */
    padding-block: 8px 6px;
    padding-inline: clamp(12px, 3.2cqi, 40px);
    /* May shrink with a short pane (the box scrolls inside) instead of pushing
       the chat out; the textarea is capped by the pane height in autosize(). */
    flex-shrink: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    /* Shed the tool row's hints and the status line's secondary spans by the
       COMPOSER's width — it sits in a narrow tiled pane as readily as a
       full-window chat. Same numbers as the `@container` blocks below. */
    container-type: inline-size;
  }
  .box-wrap {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: var(--chat-measure, none);
    margin-inline: auto;
    width: 100%;
    min-height: 0;
  }
  .box {
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    padding-block: 8px 4px; padding-inline: 12px 6px;
    overflow: hidden;
    min-width: 0;
    min-height: 0;
    box-shadow: var(--shadow-card);
  }
  .box:focus-within {
    border-color: color-mix(in srgb, var(--accent) 60%, var(--border));
    box-shadow: 0 0 0 3px var(--accent-soft), var(--shadow-card);
  }
  textarea {
    min-width: 0;
    resize: none;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.5;
    min-height: 40px;
    max-height: 40vh;
    flex: 0 1 auto;
    padding-block: 2px; padding-inline: 0 6px;
    overflow-x: hidden;
    overflow-y: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    scrollbar-gutter: stable;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 28px;
    min-width: 0;
  }
  .tool {
    color: var(--text-dim);
  }
  .keys {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .stop {
    flex-shrink: 0;
  }
  .uploading {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* Send: the composer's one filled control. */
  .send {
    flex-shrink: 0;
    display: inline-grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 0;
    padding: 0;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    cursor: pointer;
  }
  .send:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .send:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .send:disabled {
    background: var(--surface-3);
    color: var(--text-dim);
    cursor: default;
  }
  textarea::placeholder {
    color: var(--text-dim);
  }
  /* Completion popup: anchored above the box, clamped to the pane, scrolls. */
  .cmd-pop {
    position: absolute;
    bottom: calc(100% + 6px);
    inset-inline-start: 0;
    width: min(100%, 560px);
    max-width: 100%;
    max-height: var(--pop-max, min(320px, 50vh));
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-m);
    box-shadow: var(--glass-shadow);
    padding: 4px;
    z-index: var(--z-popover);
  }
  .cmd-row {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: baseline;
    gap: 10px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .cmd-row.active {
    background: var(--accent-soft);
  }
  .cmd-name {
    color: var(--accent-text);
    white-space: nowrap;
  }
  .cmd-desc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .cmd-src {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
  }
  .cmd-hint {
    font-size: var(--fs-xs);
    padding: 4px 8px 2px;
    border-top: 1px solid var(--border);
    margin-top: 2px;
  }
  .thumbs {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .thumb {
    position: relative;
    width: 84px;
    height: 84px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface-2);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .thumb-x {
    position: absolute;
    top: 3px;
    inset-inline-end: 3px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 0;
    background: var(--scrim-media);
    color: var(--on-scrim);
    display: grid;
    place-items: center;
    padding: 0;
    cursor: pointer;
  }
  .status,
  .term-input {
    width: 100%;
    box-sizing: border-box;
    max-width: var(--chat-measure, none);
    margin-inline: auto;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 4px 6px 0;
    min-width: 0;
    flex-wrap: wrap;
    row-gap: 2px;
  }
  .sep {
    opacity: 0.6;
  }
  .cwd,
  .branch {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
  }
  .branch {
    color: var(--accent-text);
  }
  .term-status {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 0 1 auto;
    max-width: 48%;
  }
  .term-input {
    margin-block-start: 6px;
    font-size: var(--fs-xs);
    padding: 4px 10px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    border: 1px dashed color-mix(in srgb, var(--warning) 50%, var(--border));
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .exited {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 6px 0;
    font-size: var(--fs-s);
  }
  /* ≤480px (a tile in a 2×2 grid): the box rests at one line and the status
     row keeps only the state — cwd / branch / model are in the pane header. */
  @container (max-width: 480px) {
    .composer {
      padding-block: 6px 4px;
      padding-inline: 8px;
    }
    textarea {
      min-height: 22px;
    }
    .cwd,
    .branch,
    .model,
    .sep-cwd,
    .sep-branch,
    .sep-model {
      display: none;
    }
  }
  /* ≤420px: the key hints go too (a tip, not state). */
  @container (max-width: 420px) {
    .keys {
      display: none;
    }
  }
</style>
