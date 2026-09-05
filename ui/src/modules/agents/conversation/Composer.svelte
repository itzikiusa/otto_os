<script lang="ts">
  // Chat composer (sessionId mode, editor role, session not exited): ⏎ submits
  // exactly the typed text as ONE prompt into the agent's PTY, ⇧⏎ inserts a
  // newline, `/` passes through untouched (slash commands are the CLI's).
  // Pasted / dropped images upload to the session inbox and land as an
  // explicit `[Image: <path>]` line. The status line shows the session status
  // and any board tasks still waiting to be nudged in.
  import { toasts } from '../../../lib/toast.svelte';
  import { activity } from '../../../lib/stores/activity.svelte';
  import type { SessionStatus } from '../../../lib/api/types';
  import { submitPrompt, uploadInboxImage } from './api';

  interface Props {
    sessionId: string;
    status: SessionStatus;
    /** Exited / reconnectable → Resume instead of a textarea. */
    onresume: () => void;
  }
  let { sessionId, status, onresume }: Props = $props();

  let text = $state('');
  let sending = $state(false);
  let uploading = $state(0);
  let ta = $state<HTMLTextAreaElement | null>(null);

  const exited = $derived(status === 'exited' || status === 'reconnectable');
  const pendingNudges = $derived(activity.tasks(sessionId).filter((t) => t.nudge_pending).length);
  const statusLabel = $derived(
    status === 'working' ? 'Working…' : status === 'idle' ? 'Idle' : status === 'running' ? 'Running' : status,
  );

  function autosize(): void {
    if (!ta) return;
    ta.style.height = 'auto';
    ta.style.height = `${Math.min(220, ta.scrollHeight)}px`;
  }

  async function send(): Promise<void> {
    const body = text.replace(/\s+$/, '');
    if (!body || sending) return;
    sending = true;
    try {
      await submitPrompt(sessionId, body);
      text = '';
      queueMicrotask(autosize);
    } catch (e) {
      toasts.error('Send failed', e instanceof Error ? e.message : String(e));
    } finally {
      sending = false;
      ta?.focus();
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    }
  }

  function insertAtCursor(snippet: string): void {
    const el = ta;
    const start = el?.selectionStart ?? text.length;
    const end = el?.selectionEnd ?? text.length;
    const before = text.slice(0, start);
    const after = text.slice(end);
    const sep = before && !before.endsWith('\n') ? '\n' : '';
    text = `${before}${sep}${snippet}\n${after}`;
    queueMicrotask(() => {
      autosize();
      if (el) {
        const pos = before.length + sep.length + snippet.length + 1;
        el.setSelectionRange(pos, pos);
        el.focus();
      }
    });
  }

  async function addImages(files: File[]): Promise<void> {
    const imgs = files.filter((f) => f.type.startsWith('image/'));
    if (!imgs.length) return;
    uploading += imgs.length;
    for (const f of imgs) {
      try {
        const name = f.name && f.name !== 'image.png' ? f.name : `paste-${Date.now()}.${(f.type.split('/')[1] ?? 'png').replace('jpeg', 'jpg')}`;
        const path = await uploadInboxImage(sessionId, f, name);
        insertAtCursor(`[Image: ${path}]`);
      } catch (e) {
        toasts.error('Image upload failed', e instanceof Error ? e.message : String(e));
      } finally {
        uploading -= 1;
      }
    }
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
</script>

<div class="composer" data-status={status} ondragover={(e) => e.preventDefault()} ondrop={onDrop} role="group" aria-label="Message composer">
  {#if exited}
    <div class="exited">
      <span class="dim">This session has ended.</span>
      <button class="btn small primary" onclick={onresume}>Resume</button>
    </div>
  {:else}
    <div class="box">
      <textarea
        bind:this={ta}
        bind:value={text}
        rows="1"
        placeholder="Message the agent — ⏎ to send, ⇧⏎ for a newline, paste an image to attach"
        spellcheck="true"
        dir="auto"
        disabled={sending}
        oninput={autosize}
        onkeydown={onKeydown}
        onpaste={onPaste}
      ></textarea>
      <button class="send icon-btn" onclick={() => void send()} disabled={sending || !text.trim()} title="Send (⏎)" aria-label="Send">➤</button>
    </div>
    <div class="status">
      <span class="sdot {status}"></span>
      <span>{statusLabel}</span>
      {#if uploading}<span class="dim">· uploading {uploading} image{uploading > 1 ? 's' : ''}…</span>{/if}
      {#if pendingNudges}
        <span class="dim" title="Board tasks waiting for the agent to go idle">· {pendingNudges} board task{pendingNudges > 1 ? 's' : ''} pending</span>
      {/if}
      <span class="grow"></span>
      <span class="dim hint">Slash commands pass straight to the CLI</span>
    </div>
  {/if}
</div>

<style>
  .composer {
    border-top: 1px solid var(--border);
    background: var(--surface);
    padding: 8px 12px 6px;
    flex-shrink: 0;
  }
  .box {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    padding: 6px 6px 6px 10px;
  }
  .box:focus-within {
    border-color: color-mix(in srgb, var(--accent) 60%, var(--border));
  }
  textarea {
    flex: 1;
    min-width: 0;
    resize: none;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    line-height: 1.45;
    max-height: 220px;
    padding: 2px 0;
  }
  textarea::placeholder {
    color: var(--text-dim);
  }
  .send {
    flex-shrink: 0;
    color: var(--accent);
    font-size: 14px;
  }
  .send:disabled {
    opacity: 0.4;
    cursor: default;
  }
  :global([dir='rtl']) .send {
    transform: scaleX(-1);
  }
  .status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 10.5px;
    color: var(--text-dim);
    padding: 5px 2px 0;
    min-width: 0;
  }
  .sdot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--status-idle, var(--text-dim));
  }
  .sdot.working,
  .sdot.running {
    background: var(--status-working, #3fb950);
  }
  .hint {
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
    font-size: 12px;
  }
  @media (max-width: 640px) {
    .hint {
      display: none;
    }
  }
</style>
