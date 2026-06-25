<script lang="ts">
  // The canvas "Assistant": the agent's live SHELL (the same Terminal as Agents,
  // reused) embedded right here in Canvas — no navigating away — plus an input to
  // send a request. You type what you want; the agent edits the scene's file and
  // the board re-renders. The Terminal attaches to /ws/term/{id} directly, so it
  // shows live output regardless of whether the session is in the workspace store.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';

  interface Props {
    editor: { generate: (p: string) => Promise<void>; isGenerating: () => boolean } | undefined;
    onclose: () => void;
  }
  let { editor, onclose }: Props = $props();

  let draft = $state('');
  let busy = $state(false);

  // Which agent drives this canvas's Ask-AI (single choice; 'shell' isn't an
  // agent so it's excluded). Changing it persists on the scene.
  const providers = $derived(
    (auth.meta?.providers ?? ['claude', 'codex']).filter((p) => p !== 'shell'),
  );
  async function setProvider(p: string): Promise<void> {
    if (!canvas.currentId || p === canvas.provider) return;
    try {
      await canvas.updateMeta(canvas.currentId, { provider: p });
    } catch {
      /* surfaced elsewhere */
    }
  }

  // Make sure the workspace store knows about the canvas session (best-effort —
  // the Terminal works without it, this just enriches the header/status).
  onMount(() => {
    if (canvas.sessionId && !ws.sessions.find((s) => s.id === canvas.sessionId)) {
      void ws.refreshSessions().catch(() => {});
    }
  });

  async function send(): Promise<void> {
    const p = draft.trim();
    if (!p || busy || !editor) return;
    busy = true;
    draft = '';
    try {
      await editor.generate(p);
    } finally {
      busy = false;
    }
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  const working = $derived(busy || editor?.isGenerating());
</script>

<aside class="assistant">
  <header class="head">
    <span class="title"><Icon name="terminal" size={15} /> Assistant</span>
    {#if providers.length > 1}
      <select
        class="provider"
        value={canvas.provider}
        onchange={(e) => setProvider(e.currentTarget.value)}
        title="Which agent draws this canvas"
      >
        {#each providers as p (p)}
          <option value={p}>{p}</option>
        {/each}
      </select>
    {/if}
    {#if working}<span class="working">working…</span>{/if}
    <button class="close" onclick={onclose} aria-label="Close assistant">
      <Icon name="x" size={15} />
    </button>
  </header>

  <div class="shell">
    {#if canvas.sessionId}
      {#key canvas.sessionId}
        <Terminal sessionId={canvas.sessionId} readOnly={false} forceDark={true} />
      {/key}
    {:else}
      <div class="empty">
        <p class="lead">Describe a diagram and the agent draws it here.</p>
        <p class="hint">It edits this canvas's file and the board updates — keep chatting to
          refine it. The agent's live shell appears here once it starts.</p>
      </div>
    {/if}
  </div>

  <div class="composer">
    <textarea
      bind:value={draft}
      onkeydown={onKey}
      placeholder="Ask for a diagram or a change…"
      rows="2"
      disabled={busy}
    ></textarea>
    <button class="send" onclick={send} disabled={busy || !draft.trim()} aria-label="Send">
      <Icon name="arrowUp" size={16} />
    </button>
  </div>
</aside>

<style>
  .assistant {
    width: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface);
    color: var(--text);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .title {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 13px;
    font-weight: 600;
  }
  .provider {
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    border-radius: 6px;
    font-size: 11px;
    padding: 2px 5px;
    cursor: pointer;
    text-transform: capitalize;
  }
  .working {
    font-size: 11px;
    color: var(--accent);
    font-weight: 600;
  }
  .close {
    margin-inline-start: auto;
    display: inline-flex;
    border: none;
    background: none;
    color: var(--text-dim, #888);
    cursor: pointer;
    padding: 4px;
    border-radius: 6px;
  }
  .close:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .shell {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    position: relative;
    background: #1e1e1e;
  }
  .shell > :global(*) {
    flex: 1 1 auto;
    min-height: 0;
  }
  .empty {
    margin: auto;
    text-align: center;
    color: var(--text-dim, #aaa);
    padding: 20px;
  }
  .empty .lead {
    margin: 0 0 6px;
    font-size: 14px;
    font-weight: 600;
    color: #eee;
  }
  .empty .hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    max-width: 320px;
  }
  .composer {
    flex: none;
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid var(--border);
  }
  .composer textarea {
    flex: 1;
    min-width: 0;
    resize: none;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    padding: 8px 10px;
    outline: none;
  }
  .composer textarea:focus {
    border-color: var(--accent);
  }
  .send {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    flex: none;
    border: none;
    border-radius: 50%;
    background: var(--accent);
    color: #fff;
    cursor: pointer;
  }
  .send:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
