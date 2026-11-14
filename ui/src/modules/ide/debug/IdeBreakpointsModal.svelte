<script lang="ts">
  // Modal for viewing & editing breakpoints, hit counts, conditions, and logpoints.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { ide, ideBasename, type IdeBreakpoint } from '../ideStore.svelte';

  interface Props {
    initialPath?: string;
    initialLine?: number;
    onclose: () => void;
  }

  let { initialPath = '', initialLine = 1, onclose }: Props = $props();

  let newBpPath = $state('');
  let newBpLine = $state('1');
  let initialized = false;
  $effect(() => {
    if (initialized) return;
    initialized = true;
    newBpPath = initialPath;
    newBpLine = String(initialLine || 1);
  });

  function addManualBreakpoint(e: SubmitEvent): void {
    e.preventDefault();
    const path = (newBpPath.trim() || ide.activePath || '').trim();
    const line = Math.max(1, Math.floor(Number(newBpLine) || 1));
    if (!path) return;
    ide.updateBreakpoint(path, line, { enabled: true });
    newBpPath = '';
    newBpLine = '1';
  }

  function updateBpField(bp: IdeBreakpoint, patch: Partial<IdeBreakpoint>): void {
    ide.updateBreakpoint(bp.path, bp.line, patch);
  }
</script>

<Modal title="Breakpoints and conditions" width={600} {onclose}>
  <form class="dbg-bp-add" onsubmit={addManualBreakpoint}>
    <input
      class="input dbg-inline-input mono"
      type="text"
      dir="ltr"
      aria-label="Breakpoint file path"
      placeholder={ide.activePath || 'File path…'}
      bind:value={newBpPath}
    />
    <input
      class="input dbg-bp-line mono"
      type="number"
      min="1"
      dir="ltr"
      aria-label="Breakpoint line number"
      bind:value={newBpLine}
    />
    <button type="submit" class="btn small" disabled={!(newBpPath.trim() || ide.activePath)}>
      <Icon name="plus" size={12} />
      <span>Add</span>
    </button>
  </form>

  {#if ide.allBreakpoints.length === 0}
    <p class="dbg-muted">
      No breakpoints configured yet. Click any line gutter in the editor or add one above.
    </p>
  {:else}
    <div class="dbg-bp-list">
      {#each ide.allBreakpoints as bp (`${bp.path}:${bp.line}`)}
        {@const bpExt = bp as IdeBreakpoint & { verified?: boolean }}
        <div class="dbg-bp-card">
          <div class="dbg-bp-top">
            <Switch
              checked={bp.enabled}
              onchange={(enabled) => updateBpField(bp, { enabled })}
              label={`Enable breakpoint at ${ideBasename(bp.path)} line ${bp.line}`}
            />
            <button
              type="button"
              class="btn small ghost mono"
              dir="ltr"
              onclick={() => {
                onclose();
                void ide.openFile(bp.path, { line: bp.line, preview: false });
              }}
            >
              {ide.relativePath(bp.path)}:{bp.line}
            </button>
            {#if bpExt.verified}
              <Badge tone="ok" variant="outline" label="Verified" />
            {/if}
            <span class="grow"></span>
            <button
              type="button"
              class="icon-btn"
              aria-label={`Remove breakpoint at ${ideBasename(bp.path)}:${bp.line}`}
              title={`Remove breakpoint at ${ideBasename(bp.path)}:${bp.line}`}
              onclick={() => ide.removeBreakpoint(bp.path, bp.line)}
            >
              <Icon name="trash" size={13} />
            </button>
          </div>
          <div class="dbg-bp-fields">
            <label class="dbg-bp-field">
              <span>Condition</span>
              <input
                class="input mono"
                type="text"
                dir="ltr"
                placeholder="e.g. x == 20 && ready"
                value={bp.condition ?? ''}
                onchange={(e) =>
                  updateBpField(bp, {
                    condition: (e.currentTarget as HTMLInputElement).value.trim() || null,
                  })}
              />
            </label>
            <label class="dbg-bp-field">
              <span>Hit condition</span>
              <input
                class="input mono"
                type="text"
                dir="ltr"
                placeholder="e.g. >= 5"
                value={bp.hit_condition ?? ''}
                onchange={(e) =>
                  updateBpField(bp, {
                    hit_condition: (e.currentTarget as HTMLInputElement).value.trim() || null,
                  })}
              />
            </label>
            <label class="dbg-bp-field dbg-bp-wide">
              <span>Logpoint message (logs without pausing)</span>
              <input
                class="input mono"
                type="text"
                dir="ltr"
                placeholder="e.g. user={'{'}id{'}'} total={'{'}sum{'}'}"
                value={bp.log_message ?? ''}
                onchange={(e) =>
                  updateBpField(bp, {
                    log_message: (e.currentTarget as HTMLInputElement).value.trim() || null,
                  })}
              />
            </label>
          </div>
        </div>
      {/each}
    </div>
  {/if}

  {#snippet footer()}
    <button type="button" class="btn primary" onclick={onclose}>Done</button>
  {/snippet}
</Modal>

<style>
  .dbg-bp-add {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-block-end: 12px;
  }
  .dbg-inline-input {
    flex: 1;
    min-inline-size: 0;
    block-size: 26px;
    font-size: var(--fs-xs);
  }
  .dbg-bp-line {
    inline-size: 76px;
    block-size: 26px;
    font-size: var(--fs-xs);
  }
  .dbg-muted {
    margin: 0;
    padding: 12px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .dbg-bp-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .dbg-bp-card {
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .dbg-bp-top {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-block-end: 8px;
  }
  .dbg-bp-fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .dbg-bp-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .dbg-bp-wide {
    grid-column: 1 / -1;
  }
  @media (max-width: 640px) {
    .dbg-bp-fields {
      grid-template-columns: 1fr;
    }
  }
</style>
