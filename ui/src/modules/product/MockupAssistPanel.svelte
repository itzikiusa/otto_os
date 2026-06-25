<script lang="ts">
  // The Mockups "Assistant": a specialized agent that builds a mockup IN PLACE —
  // its live shell (the same Terminal as Agents, reused) embedded right here on the
  // Product page (never in the Agents section), beside a LIVE preview of the mockup
  // as the agent writes it. You type what you want; the agent edits the mockup's
  // backing file and the preview + committed attachment update.
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import MockupLivePreview from './MockupLivePreview.svelte';
  import { mockupAssist, type MockupFormat } from '../../lib/stores/mockup-assist.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { ProductAttachment } from './types';

  interface Props {
    /** Called after a turn commits, with the committed mockup attachment. */
    oncommit?: (att: ProductAttachment) => void;
    onclose: () => void;
  }
  const { oncommit, onclose }: Props = $props();

  let draft = $state('');

  // The format can only be chosen before the mockup exists; once it does (or we're
  // refining), it's locked.
  const locked = $derived(mockupAssist.attachmentId !== null);

  const STARTERS = [
    'A dashboard with KPI cards and a recent-activity table',
    'A settings page with tabs for profile, security, and billing',
    'A sign-up form with validation states',
    'A sequence diagram of the checkout flow',
  ];

  function setFormat(f: MockupFormat): void {
    if (!locked) mockupAssist.format = f;
  }

  async function send(): Promise<void> {
    const p = draft.trim();
    if (!p || mockupAssist.busy) return;
    draft = '';
    try {
      const att = await mockupAssist.ask(p);
      oncommit?.(att);
    } catch (e) {
      toasts.error('Mockup agent failed', e instanceof Error ? e.message : String(e));
    }
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }
  function useStarter(s: string): void {
    draft = s;
  }
</script>

<section class="mockup-assist">
  <header class="ma-head">
    <span class="ma-title"><Icon name="zap" size={15} /> Mockup agent</span>
    <div class="ma-format" role="group" aria-label="Mockup format">
      <button class:on={mockupAssist.format === 'html'} disabled={locked} onclick={() => setFormat('html')}>
        HTML
      </button>
      <button
        class:on={mockupAssist.format === 'mermaid'}
        disabled={locked}
        onclick={() => setFormat('mermaid')}
      >
        Diagram
      </button>
    </div>
    {#if mockupAssist.busy}<span class="ma-working">working…</span>{/if}
    <button class="ma-close" onclick={onclose} aria-label="Close mockup agent">
      <Icon name="x" size={15} />
    </button>
  </header>

  <div class="ma-body">
    <!-- Live preview of the mockup as the agent writes it. -->
    <div class="ma-preview">
      <MockupLivePreview format={mockupAssist.format} content={mockupAssist.liveContent} />
    </div>

    <!-- The agent's live shell + the request composer. -->
    <aside class="ma-side">
      <div class="ma-shell">
        {#if mockupAssist.sessionId}
          {#key mockupAssist.sessionId}
            <Terminal sessionId={mockupAssist.sessionId} readOnly={false} forceDark={true} />
          {/key}
        {:else}
          <div class="ma-empty">
            <p class="lead">Describe the mockup and the agent builds it here.</p>
            <p class="hint">It writes a self-contained {mockupAssist.format === 'mermaid'
                ? 'Mermaid diagram'
                : 'HTML screen'} and the preview updates live. Keep chatting to refine it. The
              agent's shell appears here once it starts.</p>
            <div class="ma-starters">
              {#each STARTERS as s (s)}
                <button class="ma-starter" onclick={() => useStarter(s)}>{s}</button>
              {/each}
            </div>
          </div>
        {/if}
      </div>

      <div class="ma-composer">
        <textarea
          bind:value={draft}
          onkeydown={onKey}
          placeholder={locked ? 'Ask for a change…' : 'Describe the mockup to create…'}
          rows="2"
          disabled={mockupAssist.busy}
        ></textarea>
        <button
          class="ma-send"
          onclick={send}
          disabled={mockupAssist.busy || !draft.trim()}
          aria-label="Send"
        >
          <Icon name="arrowUp" size={16} />
        </button>
      </div>
    </aside>
  </div>
</section>

<style>
  .mockup-assist {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    background: var(--surface);
  }
  .ma-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .ma-title {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 13px;
    font-weight: 600;
  }
  .ma-format {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 999px;
    overflow: hidden;
  }
  .ma-format button {
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 600;
    padding: 3px 10px;
    cursor: pointer;
  }
  .ma-format button.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .ma-format button:disabled {
    cursor: default;
    opacity: 0.55;
  }
  .ma-working {
    font-size: 11px;
    color: var(--accent);
    font-weight: 600;
  }
  .ma-close {
    margin-inline-start: auto;
    display: inline-flex;
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 4px;
    border-radius: 6px;
  }
  .ma-close:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .ma-body {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 0;
  }
  .ma-preview {
    flex: 1.6;
    min-width: 0;
    min-height: 0;
    display: flex;
    padding: 12px;
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .ma-side {
    flex: 1;
    min-width: 300px;
    max-width: 460px;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-inline-start: 1px solid var(--border);
  }
  .ma-shell {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    position: relative;
    background: #1e1e1e;
  }
  .ma-shell > :global(*) {
    flex: 1 1 auto;
    min-height: 0;
  }
  .ma-empty {
    margin: auto;
    text-align: center;
    color: var(--text-dim, #aaa);
    padding: 20px;
  }
  .ma-empty .lead {
    margin: 0 0 6px;
    font-size: 14px;
    font-weight: 600;
    color: #eee;
  }
  .ma-empty .hint {
    margin: 0 0 14px;
    font-size: 12px;
    line-height: 1.5;
    max-width: 320px;
  }
  .ma-starters {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ma-starter {
    border: 1px solid color-mix(in srgb, #fff 18%, transparent);
    background: color-mix(in srgb, #fff 6%, transparent);
    color: #ddd;
    border-radius: 8px;
    font-size: 11.5px;
    padding: 6px 9px;
    cursor: pointer;
    text-align: start;
    line-height: 1.4;
  }
  .ma-starter:hover {
    border-color: var(--accent);
    color: #fff;
  }
  .ma-composer {
    flex: none;
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid var(--border);
  }
  .ma-composer textarea {
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
  .ma-composer textarea:focus {
    border-color: var(--accent);
  }
  .ma-send {
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
  .ma-send:disabled {
    opacity: 0.45;
    cursor: default;
  }

  @media (max-width: 760px) {
    .ma-body {
      flex-direction: column;
    }
    .ma-side {
      max-width: none;
      border-inline-start: none;
      border-top: 1px solid var(--border);
    }
  }
</style>
