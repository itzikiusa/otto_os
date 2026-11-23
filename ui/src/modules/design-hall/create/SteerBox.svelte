<script lang="ts" module>
  export interface SteerPin {
    n: number;
    /** What the pin sits on, in plain words ("the monster"), when known. */
    on: string | null;
    note: string;
  }
</script>

<script lang="ts">
  // The ONE way to change anything in Create — the plan, the scene, one
  // thing: say it in words, and (where there is a picture) tap it to pin a
  // numbered note. The same heading, box and button everywhere.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';

  interface Props {
    /** Tap-to-pin notes the picture added (bindable so the notes can be typed here). */
    pins?: SteerPin[];
    /** Whether the picture above takes taps (adds the hint). */
    canPin?: boolean;
    busy?: boolean;
    placeholder?: string;
    onsend: (comment: string, pins: SteerPin[]) => void;
    onremovepin?: (n: number) => void;
    /** Start as one line ("Want something different? Tell me.") that opens on tap. */
    collapsed?: boolean;
    /** True while something is typed (the caller can make "Change it" the one blue button). */
    hasText?: boolean;
    /** Words to start with (notes written while the rest was still being made). */
    initial?: string;
  }
  let { pins = $bindable([]), canPin = false, busy = false, placeholder = 'Make the monster bigger. Don’t make the key…', onsend, onremovepin, collapsed = false, hasText = $bindable(false), initial = '' }: Props = $props();

  let comment = $state(untrack(() => initial));
  // Opened once, it stays open; it starts closed only where the caller asks.
  let opened = $state(false);
  const open = $derived(!collapsed || opened || pins.length > 0);
  const can = $derived(!busy && (comment.trim().length > 0 || pins.some((p) => p.note.trim())));
  $effect(() => {
    hasText = can;
  });

  function send(): void {
    if (!can) return;
    onsend(comment.trim(), pins.filter((p) => p.note.trim()));
    comment = '';
  }
</script>

{#if !open}
  <button type="button" class="cr-secondary sb-open" onclick={() => ((opened = true), queueMicrotask(() => document.getElementById('create-steer')?.focus()))} data-testid="create-steer-open">
    <Icon name="edit" size={16} /> Want something different? Tell me.
  </button>
{:else}
<section class="cr-card sb" aria-labelledby="steer-h">
  <h2 class="sb-h" id="steer-h">Want something different? Tell me.</h2>
  {#if canPin}<p class="sb-hint">You can also tap the picture to put a note on one place.</p>{/if}
  <label class="sb-sr" for="create-steer">What should be different?</label>
  <textarea id="create-steer" class="cr-field sb-field" dir="auto" maxlength="2000" {placeholder} bind:value={comment} data-testid="create-steer"></textarea>
  {#each pins as p, k (p.n)}
    <div class="sb-pin">
      <span class="sb-n" aria-hidden="true">{p.n}</span>
      <input
        id={`sb-pin-${p.n}`}
        class="sb-input"
        dir="auto"
        maxlength="600"
        placeholder={p.on ? `What should change on ${p.on}?` : 'What should change here?'}
        aria-label={`Note ${p.n}${p.on ? ` on ${p.on}` : ''}`}
        bind:value={pins[k].note}
        data-testid="create-pin-note"
      />
      <button type="button" class="sb-x" aria-label={`Remove note ${p.n}`} title="Remove" onclick={() => onremovepin?.(p.n)}>
        <Icon name="x" size={16} />
      </button>
    </div>
  {/each}
  <button type="button" class={can ? 'cr-primary sb-go' : 'cr-secondary sb-go'} disabled={!can} onclick={send} data-testid="create-steer-go">
    {busy ? 'Thinking…' : 'Change it'}
  </button>
</section>
{/if}

<style>
  .sb {
    margin-block-start: var(--sp-4);
  }
  .sb-h {
    font-size: var(--fs-l);
    margin: 0 0 var(--sp-2);
  }
  .sb-hint {
    margin: 0 0 var(--sp-2);
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .sb-sr {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .sb-field {
    min-block-size: 64px;
  }
  .sb-open {
    inline-size: 100%;
    margin-block-start: var(--sp-4);
  }
  .sb-pin {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-block-start: var(--sp-2);
  }
  .sb-n {
    flex: none;
    display: grid;
    place-items: center;
    inline-size: 30px;
    block-size: 30px;
    border-radius: 50%;
    background: var(--danger-solid);
    color: var(--danger-contrast);
    font-weight: 600;
  }
  .sb-input {
    flex: 1;
    min-inline-size: 0;
    min-block-size: var(--cr-hit);
    padding: var(--sp-2);
    border: 1.5px solid var(--control-border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
  }
  .sb-x {
    flex: none;
    display: grid;
    place-items: center;
    inline-size: var(--cr-hit);
    block-size: var(--cr-hit);
    border: 0;
    border-radius: var(--radius-m);
    background: none;
    color: var(--text-dim);
    cursor: pointer;
  }
  .sb-go {
    inline-size: 100%;
    margin-block-start: var(--sp-3);
  }
</style>
