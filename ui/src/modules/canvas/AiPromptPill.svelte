<script lang="ts">
  // Floating "describe it and I'll draw it" prompt, anchored top-center over the
  // canvas. One agent turn → blocks inserted near the existing content. Stays
  // open after a run so you can refine/regenerate; Esc or the ✕ closes it.
  import Icon from '../../lib/components/Icon.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { AssistMode } from './types';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  let prompt = $state('');
  let mode = $state<AssistMode>('auto');
  let busy = $state(false);

  const MODES: { id: AssistMode; label: string }[] = [
    { id: 'auto', label: 'Auto' },
    { id: 'sequence', label: 'Sequence' },
    { id: 'flow', label: 'Flow' },
    { id: 'uml', label: 'UML' },
  ];

  // Place new content just to the right of the current scene's bounding box.
  function insertOrigin(): { x: number; y: number } {
    const ns = canvas.scene?.nodes ?? [];
    if (!ns.length) return { x: 80, y: 80 };
    const right = Math.max(...ns.map((n) => n.x + n.w));
    const top = Math.min(...ns.map((n) => n.y));
    return { x: right + 60, y: top };
  }

  async function run(): Promise<void> {
    const p = prompt.trim();
    if (!p || busy) return;
    busy = true;
    try {
      const res = await canvas.assist(p, mode);
      const { x, y } = insertOrigin();
      const n = canvas.insertAssist(res, x, y);
      if (n > 0) {
        toasts.success('Added to canvas', `${n} block${n === 1 ? '' : 's'} inserted.`);
        prompt = '';
      } else {
        toasts.info('Nothing to add', res.note || 'The agent did not return a diagram.');
      }
    } catch (e) {
      toasts.error('Ask AI failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') onclose();
    else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void run();
  }
</script>

<div class="pill-wrap">
  <!-- keydown on the wrapper so Esc closes even when a mode/Draw button is focused -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="pill" class:busy onkeydown={onkeydown}>
    <Icon name="zap" />
    <!-- svelte-ignore a11y_autofocus -->
    <input
      bind:value={prompt}
      autofocus
      disabled={busy}
      placeholder="Describe a diagram or blocks… e.g. 'service A calls B; B does 10 things'"
      {onkeydown}
    />
    <div class="modes">
      {#each MODES as m (m.id)}
        <button class:active={mode === m.id} onclick={() => (mode = m.id)} disabled={busy}>{m.label}</button>
      {/each}
    </div>
    <button class="run" onclick={run} disabled={busy || !prompt.trim()} title="Generate (⌘↵)">
      {busy ? 'Drawing…' : 'Draw'}
    </button>
    <button class="close" onclick={onclose} aria-label="Close"><Icon name="x" /></button>
  </div>
</div>

<style>
  .pill-wrap {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 20;
    width: min(720px, 92%);
  }
  .pill {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    background: var(--surface);
    border: 1px solid var(--accent);
    border-radius: 999px;
    box-shadow: var(--shadow);
  }
  .pill.busy {
    opacity: 0.9;
  }
  .pill input {
    flex: 1 1 auto;
    border: none;
    background: none;
    color: var(--text);
    font-size: 13px;
    outline: none;
    min-width: 0;
  }
  .modes {
    display: flex;
    gap: 2px;
  }
  .modes button {
    font-size: 11px;
    padding: 3px 7px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text-dim, #888);
    border-radius: 999px;
    cursor: pointer;
  }
  .modes button.active {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .run {
    padding: 5px 12px;
    border: none;
    background: var(--accent);
    color: #fff;
    border-radius: 999px;
    font-size: 13px;
    cursor: pointer;
    white-space: nowrap;
  }
  .run:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .close {
    display: inline-flex;
    border: none;
    background: none;
    color: var(--text);
    cursor: pointer;
    padding: 4px;
  }
</style>
