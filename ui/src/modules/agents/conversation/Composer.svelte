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
  // its tools on a row inside it: attach image · key hints · Stop / Steer /
  // Queue (while busy) or Send, plus an inline Pending Approval dock on the top
  // edge when `status === 'approval'` / `waiting`, and `@file` workspace mention
  // typeahead alongside `/` slash-command completion.
  import { tick, untrack } from 'svelte';
  import { toastError } from '../../../lib/toastError';
  import { announce } from '../../../lib/announce.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { detectBlockDirection } from '../../../lib/textDirection';
  import type { SessionStatus, SlashCommand } from '../../../lib/api/types';
  import { fetchSlashCommands, interruptAgent, steerAgent, submitPrompt, uploadInboxImage } from './api';
  import type { ToolCallBlock } from './format';
  import {
    detectMentionAtCaret,
    extractMentionedFiles,
    removeMentionFromDraft,
    searchWorkspaceFiles,
    type MentionEntry,
  } from './fileMentions';
  import ComposerApprovalDock from './ComposerApprovalDock.svelte';
  import ComposerAttachments, { type ComposerAttachment } from './ComposerAttachments.svelte';
  import ComposerFooter from './ComposerFooter.svelte';
  import ComposerPopups from './ComposerPopups.svelte';

  import { transcript } from '../../../lib/stores/transcript.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { events } from '../../../lib/events.svelte';
  import { sessionState } from '../../../lib/status';
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
    /** "Claude" / "Codex" / "Antigravity" — placeholder and hints name the agent. */
    agentName?: string;
    /** Provider slug (`claude` / `codex` / `agy` / …) for the footer badge. */
    provider?: string;
    /** Open tool call when the session is paused on an approval / question. */
    pendingCall?: ToolCallBlock | null;
    /** True when the session needs user confirmation right now. */
    waiting?: boolean;
  }
  let {
    sessionId,
    status,
    onresume,
    cwd = '',
    branch = null,
    model = null,
    termStatus = '',
    termInput = '',
    agentName = 'the agent',
    provider = '',
    pendingCall = null,
    waiting = false,
  }: Props = $props();

  const ownerId = untrack(() => sessionId);
  const text = $derived(transcript.draft(ownerId));
  const draftDir = $derived(detectBlockDirection(text));
  const workspaceRoot = $derived(cwd || ws.current?.root_path || '');
  const sending = $derived(transcript.sending(ownerId));
  const uploading = $derived(pendingUploads.get(ownerId) ?? 0);
  const isUploading = $derived(uploading > 0);
  $effect(() => { if (isUploading) announce('Uploading images…'); });
  let ta = $state<HTMLTextAreaElement | null>(null);
  let customReplyHint = $state(false);

  function restoreComposerFocus(): void {
    const focused = document.activeElement;
    if (ta?.isConnected && (focused === document.body || composerEl?.contains(focused))) {
      ta.focus();
    }
  }

  const attachments = $derived(transcript.attachments(ownerId));
  const mentionedFiles = $derived(extractMentionedFiles(text));

  const st = $derived(
    sessionState(
      ws.getSession(sessionId),
      status,
      ws.needsYou[sessionId] === true,
      { stale: events.state !== 'connected' },
    ),
  );
  const exited = $derived(st.inactive);
  const busy = $derived(status === 'working');
  const needsApproval = $derived(!exited && !busy && ((status as string) === 'approval' || waiting));

  let paneH = $state(0);
  const MIN_CHAT = 96;
  let budget = Infinity;
  function measureBudget(): void {
    budget = paneBudget();
  }
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
    ta.style.height = 'auto';
    const byWindow = Math.floor(window.innerHeight * 0.4);
    const byPane = paneH > 0 ? Math.floor(paneH * 0.5) : byWindow;
    const cap = Math.max(24, Math.min(byWindow, byPane, Math.floor(budget)));
    ta.style.height = `${Math.min(cap, Math.max(ta.scrollHeight, 0))}px`;
  }
  let composerEl = $state<HTMLDivElement | null>(null);
  let wrapEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const pane = composerEl?.parentElement;
    if (!pane || typeof ResizeObserver === 'undefined') return;
    const measure = (): void => {
      const h = pane.clientHeight;
      if (Math.abs(h - paneH) > 1) {
        paneH = h;
        measureBudget();
        autosize();
      }
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(pane);
    return () => ro.disconnect();
  });
  $effect(() => {
    void attachments.length;
    void mentionedFiles.length;
    void needsApproval;
    void !!termInput;
    void tick().then(() => {
      measureBudget();
      autosize();
    });
  });
  $effect(() => {
    void text;
    if (!text) customReplyHint = false;
    void tick().then(autosize);
  });

  // ---- busy: queue, steer + interrupt ------------------------------------------
  let stopping = $state(false);
  let steering = $state(false);
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
      restoreComposerFocus();
    }
  }
  let fileEl = $state<HTMLInputElement | null>(null);
  function pickImages(e: Event): void {
    const input = e.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    void addImages(files);
  }

  // ---- slash-command & @file mention completion --------------------------------
  let cmds = $state<SlashCommand[] | null>(null);
  let cmdsFor = '';
  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
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
  const popId = $props.id();

  let caret = $state(0);
  let mentionDismissed = $state(false);
  let fileHits = $state<MentionEntry[]>([]);
  let fileIdx = $state(0);

  function syncCaret(): void {
    if (ta) caret = ta.selectionStart ?? text.length;
  }

  const activeMention = $derived(!cmdOpen && !mentionDismissed ? detectMentionAtCaret(text, caret) : null);
  const mentionOpen = $derived(activeMention !== null && fileHits.length > 0);

  $effect(() => {
    const m = activeMention;
    const root = workspaceRoot;
    if (!m || !root) {
      fileHits = [];
      return;
    }
    const ctrl = new AbortController();
    void searchWorkspaceFiles(root, m.query, ctrl.signal)
      .then((hits) => {
        if (!ctrl.signal.aborted) {
          fileHits = hits;
          fileIdx = 0;
        }
      })
      .catch(() => {
        if (!ctrl.signal.aborted) fileHits = [];
      });
    return () => ctrl.abort();
  });

  $effect(() => {
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
    void text;
    cmdDismissed = false;
    mentionDismissed = false;
  });

  function acceptCmd(c: SlashCommand): void {
    const next = `/${c.name} `;
    transcript.setDraft(ownerId, next);
    cmdDismissed = true;
    queueMicrotask(() => {
      autosize();
      ta?.focus();
      ta?.setSelectionRange(next.length, next.length);
      caret = next.length;
    });
  }

  function acceptMention(entry: MentionEntry): void {
    const m = activeMention;
    if (!m) return;
    const insert = entry.isDir ? `@${entry.relPath}/` : `@${entry.relPath} `;
    const next = `${text.slice(0, m.start)}${insert}${text.slice(m.end)}`;
    const nextCaret = m.start + insert.length;
    transcript.setDraft(ownerId, next);
    if (!entry.isDir) mentionDismissed = true;
    caret = nextCaret;
    queueMicrotask(() => {
      autosize();
      ta?.focus();
      ta?.setSelectionRange(nextCaret, nextCaret);
    });
  }

  function removeFileMention(rel: string): void {
    const next = removeMentionFromDraft(text, rel);
    transcript.setDraft(ownerId, next);
    queueMicrotask(() => {
      autosize();
      ta?.focus();
      syncCaret();
    });
  }

  function beginCustomReply(): void {
    customReplyHint = true;
    ta?.focus();
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
      restoreComposerFocus();
    }
  }

  async function steer(): Promise<void> {
    const typed = text.replace(/\s+$/, '');
    const imgs = attachments.map((a) => `[Image: ${a.path}]`);
    const body = [typed, ...imgs].filter(Boolean).join('\n');
    if (!body || uploading > 0 || steering || !transcript.tryBeginSend(ownerId)) return;
    steering = true;
    try {
      const submittedImages = [...attachments];
      await steerAgent(ownerId, body);
      if (transcript.draft(ownerId).replace(/\s+$/, '') === typed) {
        transcript.setDraft(ownerId, '');
      }
      for (const a of submittedImages) URL.revokeObjectURL(a.url);
      transcript.setAttachments(ownerId, transcript.attachments(ownerId).filter((a) => !submittedImages.includes(a)));
      queueMicrotask(autosize);
      toasts.info('Steered', `Interrupted and redirected ${agentName}`);
    } catch (e) {
      toastError('Couldn’t steer the agent', e);
    } finally {
      steering = false;
      transcript.finishSend(ownerId);
      restoreComposerFocus();
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.isComposing) return;
    if (busy && e.key === 'Enter' && e.shiftKey && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void steer();
      return;
    }
    if (mentionOpen) {
      if (e.key === 'ArrowDown') { e.preventDefault(); fileIdx = (fileIdx + 1) % fileHits.length; return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); fileIdx = (fileIdx - 1 + fileHits.length) % fileHits.length; return; }
      if (e.key === 'Escape') { e.preventDefault(); mentionDismissed = true; return; }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        const pick = fileHits[fileIdx];
        if (pick) { e.preventDefault(); acceptMention(pick); return; }
      }
    }
    if (cmdOpen) {
      if (e.key === 'ArrowDown') { e.preventDefault(); cmdIdx = (cmdIdx + 1) % suggestions.length; return; }
      if (e.key === 'ArrowUp') { e.preventDefault(); cmdIdx = (cmdIdx - 1 + suggestions.length) % suggestions.length; return; }
      if (e.key === 'Escape') { e.preventDefault(); cmdDismissed = true; return; }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        const pick = suggestions[cmdIdx];
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
        ? `Queue (⏎) — ${agentName} is busy, so this waits until the current turn finishes`
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
    restoreComposerFocus();
  }

  function removeAttachment(a: ComposerAttachment): void {
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

  const canSend = $derived(!sending && !steering && uploading === 0 && (text.trim().length > 0 || attachments.length > 0));
  const placeholderText = $derived(
    customReplyHint && needsApproval
      ? `Type a custom reply for ${agentName} and press ⏎…`
      : busy
        ? `Queue a follow-up (⏎) or steer ${agentName} mid-turn (⇧⌘⏎)…`
        : `Message ${agentName} — @ files · / commands`,
  );
</script>

<div class="composer" bind:this={composerEl} data-status={status} ondragover={(e) => e.preventDefault()} ondrop={onDrop} role="group" aria-label="Message composer">
  {#if exited}
    <div class="exited">
      <span class="dim">{st.key === 'suspended' ? `${st.hint}.` : 'This session has ended.'}</span>
      <button class="btn small primary" onclick={onresume}>Resume</button>
    </div>
  {:else}
    <div class="box-wrap" bind:this={wrapEl}>
      <ComposerPopups
        {popId}
        anchorEl={wrapEl}
        paneEl={composerEl?.parentElement ?? null}
        {paneH}
        attachmentCount={attachments.length}
        {mentionOpen}
        {fileHits}
        {fileIdx}
        onfilehover={(i) => (fileIdx = i)}
        onacceptmention={acceptMention}
        {cmdOpen}
        {suggestions}
        {cmdIdx}
        oncmdhover={(i) => (cmdIdx = i)}
        onacceptcmd={acceptCmd}
      />
      <div class="box" class:busy class:needs-approval={needsApproval}>
        {#if needsApproval}
          <ComposerApprovalDock sessionId={ownerId} {agentName} {pendingCall} oncustomreply={beginCustomReply} />
        {/if}
        <ComposerAttachments {mentionedFiles} onremovemention={removeFileMention} />
        <textarea
          bind:this={ta}
          bind:value={() => text, (value) => transcript.setDraft(ownerId, value)}
          rows="1"
          placeholder={placeholderText}
          aria-label="Message {agentName}"
          spellcheck="false"
          {...{ autocorrect: 'off' }}
          autocapitalize="off"
          autocomplete="off"
          dir={draftDir}
          oninput={syncCaret}
          onclick={syncCaret}
          onkeyup={syncCaret}
          onselect={syncCaret}
          onkeydown={onKeydown}
          onpaste={onPaste}
          aria-autocomplete="list"
          aria-controls={mentionOpen ? `${popId}-files` : cmdOpen ? `${popId}-list` : undefined}
          aria-activedescendant={mentionOpen ? `${popId}-file-${fileIdx}` : cmdOpen ? `${popId}-opt-${cmdIdx}` : undefined}
        ></textarea>
        <div class="tools">
          <input bind:this={fileEl} type="file" accept="image/*" multiple hidden onchange={pickImages} />
          <button class="icon-btn tool" onclick={() => fileEl?.click()} aria-label="Attach images" title="Attach images (or paste / drop them)"><Icon name="image" size={14} /></button>
          <span class="keys" aria-hidden="true">
            {busy ? '⏎ queue · ⇧⌘⏎ steer · ⇧⏎ new line' : '⏎ send · ⇧⏎ new line · @ file'}
          </span>
          <span class="grow"></span>
          {#if uploading > 0}<span class="uploading">Uploading…</span>{/if}
          {#if busy}
            <button class="btn small stop" onclick={() => void interrupt()} disabled={stopping} title="Interrupt {agentName} — sends Esc to its terminal"><Icon name="stop" size={12} /> Stop</button>
            <button
              class="btn small steer-btn"
              onclick={() => void steer()}
              disabled={!canSend}
              title="Steer {agentName} (⇧⌘⏎) — interrupt the current step and send this prompt immediately"
            >
              <Icon name="zap" size={12} /> Steer
            </button>
            <button class="btn small primary queue-btn" onclick={() => void send()} disabled={!canSend} title={sendTitle}>
              <Icon name="send" size={12} /> Queue
            </button>
          {:else}
            <button class="send" onclick={() => void send()} disabled={!canSend} title={sendTitle} aria-label={uploading > 0 ? 'Send (uploading images)' : 'Send'}><Icon name="send" size={14} /></button>
          {/if}
        </div>
      </div>
      <ComposerAttachments {attachments} onremoveattachment={removeAttachment} />
    </div>
    <ComposerFooter
      {sessionId}
      {agentName}
      {provider}
      {st}
      {cwd}
      {branch}
      {model}
      {termStatus}
      {termInput}
      {uploading}
    />
  {/if}
</div>

<style>
  .composer {
    border-top: 1px solid var(--border);
    background: var(--bg);
    padding-block: 8px 6px;
    padding-inline: clamp(12px, 3.2cqi, 40px);
    flex-shrink: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
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
    padding-block: 8px 4px;
    padding-inline: 12px 6px;
    overflow: hidden;
    min-width: 0;
    min-height: 0;
    box-shadow: var(--shadow-card);
  }
  .box.needs-approval {
    border-color: color-mix(in srgb, var(--warning) 48%, var(--border));
  }
  .box:focus-within {
    border-color: var(--accent-line-strong);
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
    padding-block: 2px;
    padding-inline: 0 6px;
    overflow-x: hidden;
    overflow-y: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    scrollbar-gutter: stable;
    unicode-bidi: plaintext;
    text-align: start;
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
  .stop,
  .steer-btn,
  .queue-btn {
    flex-shrink: 0;
  }
  .steer-btn {
    border-color: var(--accent-line);
    color: var(--accent-text);
    background: var(--accent-faint);
  }
  .steer-btn:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .uploading {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
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
  @media (max-width: 640px), (pointer: coarse) {
    .send {
      width: var(--hit-min);
      height: var(--hit-min);
    }
  }
  .exited {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 6px 0;
    font-size: var(--fs-s);
  }
  @container (max-width: 480px) {
    .composer {
      padding-block: 6px 4px;
      padding-inline: 8px;
    }
    textarea {
      min-height: 22px;
    }
  }
  @container (max-width: 420px) {
    .keys {
      display: none;
    }
  }
</style>
