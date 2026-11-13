<script lang="ts">
  // Viewport-clamped GoLand-style gutter breakpoint editor popover:
  // Enabled switch, Condition, Hit Count, Log Message (logpoint), and Remove.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { ide, ideBasename, type IdeBreakpoint } from '../ideStore.svelte';
  import { areIdeBreakpointsMuted } from './ideEditorStoreBridge.svelte';

  interface Props {
    path: string;
    line: number;
    anchorX: number;
    anchorY: number;
    onclose: () => void;
  }

  let { path, line, anchorX, anchorY, onclose }: Props = $props();

  let popEl = $state<HTMLDivElement | undefined>(undefined);
  const existing = $derived(
    (ide.breakpointsByPath[path] ?? []).find((b) => b.line === line) as
      | (IdeBreakpoint & { verified?: boolean; message?: string | null })
      | undefined,
  );

  let enabled = $state(true);
  let condition = $state('');
  let hitCondition = $state('');
  let logMessage = $state('');

  let lastSeedKey = '';
  $effect(() => {
    const k = `${path}:${line}`;
    if (k === lastSeedKey) return;
    lastSeedKey = k;
    const cur = (ide.breakpointsByPath[path] ?? []).find((b) => b.line === line);
    enabled = cur?.enabled ?? true;
    condition = cur?.condition ?? '';
    hitCondition = cur?.hit_condition ?? '';
    logMessage = cur?.log_message ?? '';
  });

  // Strictly clamp popover coordinates inside the viewport on every screen size.
  const clampedPos = $derived.by(() => {
    const vw = typeof window !== 'undefined' ? window.innerWidth : 1280;
    const vh = typeof window !== 'undefined' ? window.innerHeight : 800;
    const width = Math.min(360, vw - 24);
    const estHeight = 290;
    const left = Math.max(12, Math.min(anchorX + 8, vw - width - 12));
    const top = Math.max(12, Math.min(anchorY - 12, vh - estHeight - 12));
    return { left, top, width };
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onclose();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function saveAndClose(e?: SubmitEvent): void {
    e?.preventDefault();
    ide.updateBreakpoint(path, line, {
      enabled,
      condition: condition.trim() || null,
      hit_condition: hitCondition.trim() || null,
      log_message: logMessage.trim() || null,
    });
    onclose();
  }

  function removeAndClose(): void {
    ide.removeBreakpoint(path, line);
    onclose();
  }
</script>

<div
  class="bp-pop-backdrop"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) saveAndClose();
  }}
>
  <div
    class="bp-popover"
    bind:this={popEl}
    role="dialog"
    aria-modal="true"
    aria-label="Edit breakpoint at {ideBasename(path)} line {line}"
    style:inset-inline-start={`${clampedPos.left}px`}
    style:inset-block-start={`${clampedPos.top}px`}
    style:inline-size={`${clampedPos.width}px`}
    data-testid="ide-breakpoint-popover"
  >
    <header class="bp-pop-head">
      <div class="bp-pop-title" dir="ltr">
        <Switch
          checked={enabled}
          onchange={(next) => (enabled = next)}
          label="Enable breakpoint"
        />
        <span class="mono bp-pop-loc">{ideBasename(path)}:{line}</span>
      </div>
      <div class="bp-pop-badges">
        {#if areIdeBreakpointsMuted()}
          <Badge tone="warn" variant="outline" label="Muted" />
        {:else if logMessage.trim()}
          <Badge tone="warn" variant="outline" label="Logpoint" />
        {:else if existing?.verified}
          <Badge tone="ok" variant="outline" label="Verified" />
        {/if}
        <button
          type="button"
          class="icon-btn"
          aria-label="Close breakpoint editor"
          title="Close"
          onclick={() => saveAndClose()}
        >
          <Icon name="x" size={12} />
        </button>
      </div>
    </header>

    <form class="bp-pop-form" onsubmit={saveAndClose}>
      <label class="bp-field">
        <span>Condition (pause when true)</span>
        <input
          class="input bp-input mono"
          type="text"
          dir="ltr"
          aria-label="Breakpoint condition expression"
          placeholder="err != nil && count > 10"
          bind:value={condition}
          data-testid="ide-bp-condition-input"
        />
      </label>

      <div class="bp-row-split">
        <label class="bp-field">
          <span>Hit count</span>
          <input
            class="input bp-input mono"
            type="text"
            dir="ltr"
            aria-label="Breakpoint hit count condition"
            placeholder=">= 5"
            bind:value={hitCondition}
            data-testid="ide-bp-hit-input"
          />
        </label>
        <label class="bp-field bp-field-grow">
          <span>Log message (evaluates {'{'}expr{'}'} without pausing)</span>
          <input
            class="input bp-input mono"
            type="text"
            dir="ltr"
            aria-label="Breakpoint logpoint message"
            placeholder="req={'{'}req.ID{'}'} status={'{'}code{'}'}"
            bind:value={logMessage}
            data-testid="ide-bp-log-input"
          />
        </label>
      </div>

      <footer class="bp-pop-foot">
        {#if existing}
          <button
            type="button"
            class="btn small ghost bp-del-btn"
            onclick={removeAndClose}
            data-testid="ide-bp-remove-btn"
          >
            <Icon name="trash" size={12} />
            <span>Remove</span>
          </button>
        {/if}
        <span class="grow"></span>
        <button type="submit" class="btn small primary" data-testid="ide-bp-done-btn">Done</button>
      </footer>
    </form>
  </div>
</div>

<style>
  .bp-pop-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-popover-backdrop);
  }
  .bp-popover {
    position: fixed;
    z-index: var(--z-popover);
    max-block-size: calc(100vh - 24px);
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow-card);
    padding: var(--sp-4);
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .bp-pop-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding-block-end: var(--sp-2);
    border-block-end: 1px solid var(--separator);
  }
  .bp-pop-title,
  .bp-pop-badges {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    min-inline-size: 0;
  }
  .bp-pop-loc {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bp-pop-form {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .bp-row-split {
    display: flex;
    gap: var(--sp-3);
    flex-wrap: wrap;
  }
  .bp-field {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-inline-size: 96px;
  }
  .bp-field-grow {
    flex: 1;
    min-inline-size: 160px;
  }
  .bp-input {
    block-size: 28px;
    font-size: var(--fs-xs);
  }
  .bp-pop-foot {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding-block-start: var(--sp-2);
  }
  .bp-del-btn {
    color: var(--danger);
  }
</style>
