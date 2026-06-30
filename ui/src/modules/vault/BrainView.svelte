<script lang="ts">
  // Brain tab — assemble the "Repo Brain" for a focus string: POST /vault/brain
  // returns ranked context (markdown + the reasons it used + a token estimate).
  import { vault } from './vault.svelte';
  import { renderMarkdown } from '../../lib/md';
  import Icon from '../../lib/components/Icon.svelte';
  import type { ContextReason } from '../../lib/api/types';

  function run() {
    void vault.runBrain();
  }

  function reasonColor(kind: string): string {
    switch (kind) {
      case 'vector': return '#6ea8fe';
      case 'keyword': return '#7ee787';
      case 'symbol': return '#da77f2';
      case 'graph': return '#63e6be';
      case 'recent': return '#adb5bd';
      case 'test': return '#ffa94d';
      case 'doc': return '#ff9ff3';
      case 'scope': return '#4dd2e6';
      default: return '#74c0fc';
    }
  }
  function reasonValue(r: ContextReason): string {
    const d = r.detail?.trim();
    return d ? d : r.score.toFixed(2);
  }
</script>

<div class="brain">
  <section class="brain-bar">
    <h2><Icon name="radar" size={16} /> Repo Brain</h2>
    <p class="hint">Assemble focused context across knowledge and code for a task or question.</p>
    <div class="brain-form">
      <input
        type="text"
        placeholder="Focus — e.g. “how are rate limits enforced on the upload endpoint?”"
        bind:value={vault.brainFocus}
        onkeydown={(e) => e.key === 'Enter' && vault.brainFocus.trim() && run()}
      />
      <button class="run-btn" disabled={vault.brainBusy || !vault.brainFocus.trim()} onclick={run}>
        {#if vault.brainBusy}Thinking…{:else}<Icon name="zap" size={13} /> Assemble{/if}
      </button>
    </div>
  </section>

  {#if vault.brain}
    {@const b = vault.brain}
    <section class="brain-out">
      <div class="brain-meta">
        <span class="focus">“{b.focus}”</span>
        <span class="tokens" title="Estimated tokens">~{b.token_estimate.toLocaleString()} tokens</span>
      </div>

      {#if b.reasons.length}
        <div class="reasons">
          <span class="why-label">context selected by:</span>
          {#each b.reasons as r (r.kind + r.detail)}
            <span
              class="reason"
              style:--c={reasonColor(r.kind)}
              title={`${r.kind}: ${r.detail || 'matched'} (score ${r.score.toFixed(2)})`}
            >
              <b>{r.kind}</b> {reasonValue(r)}
            </span>
          {/each}
        </div>
      {/if}

      {#if b.markdown.trim()}
        <article class="brain-md">{@html renderMarkdown(b.markdown)}</article>
      {:else if b.sections.length}
        {#each b.sections as s, i (i)}
          <section class="brain-section">
            <h3>{s.heading}</h3>
            <div class="brain-md">{@html renderMarkdown(s.body_md)}</div>
          </section>
        {/each}
      {:else}
        <p class="empty">No context assembled for this focus.</p>
      {/if}
    </section>
  {:else}
    <div class="placeholder">
      <Icon name="radar" size={22} />
      <p>Enter a focus above to assemble the Repo Brain.</p>
    </div>
  {/if}
</div>

<style>
  .brain {
    height: 100%;
    overflow-y: auto;
    padding: 18px 22px 32px;
  }
  .brain-bar { max-width: 860px; margin-bottom: 18px; }
  h2 { display: flex; align-items: center; gap: 7px; font-size: 16px; margin: 0 0 4px; }
  .hint { font-size: 12.5px; color: var(--text-dim); margin: 0 0 12px; }
  .brain-form { display: flex; gap: 8px; flex-wrap: wrap; }
  .brain-form input {
    flex: 1;
    min-width: 260px;
    font-size: 13px;
    padding: 8px 11px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  .run-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 13px;
    font-weight: 600;
    padding: 8px 16px;
    border-radius: 8px;
    border: 1px solid #7ee787;
    background: #7ee787;
    color: #0b0b0b;
    cursor: pointer;
  }
  .run-btn:disabled { opacity: 0.45; cursor: default; }

  .brain-out { max-width: 860px; }
  .brain-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
    margin-bottom: 10px;
  }
  .focus { font-size: 13px; font-style: italic; color: var(--text); }
  .tokens {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 5px;
    background: var(--surface-2);
    color: var(--text-dim);
    border: 1px solid var(--border);
  }
  .reasons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
    margin-bottom: 14px;
  }
  .why-label { font-size: 11.5px; color: var(--text-dim); margin-inline-end: 2px; }
  .reason {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--c) 55%, transparent);
    background: color-mix(in srgb, var(--c) 16%, transparent);
    color: color-mix(in srgb, var(--c) 82%, var(--text));
    white-space: nowrap;
  }
  .reason b { color: var(--c); font-weight: 700; }
  .brain-md {
    line-height: 1.6;
    font-size: 13.5px;
  }
  .brain-section { margin-bottom: 18px; }
  .brain-section h3 { font-size: 14px; margin: 0 0 6px; }
  .empty { font-size: 13px; color: var(--text-dim); }
  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 60%;
    color: var(--text-dim);
    text-align: center;
  }
  .placeholder p { margin: 0; font-size: 13px; }
</style>
