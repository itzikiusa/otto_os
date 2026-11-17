<script lang="ts">
  // THE breakpoint list (I2-11), shown in the project tree's Breakpoints tab:
  // grouped by file; a row is line · code · condition/hit/log chip; click opens
  // the location; Edit opens the same popover as the gutter; unbound ones are
  // muted with the adapter's reason. Mute / Exceptions and Remove all live in
  // the header (the debugger's ⋯ has Mute and Exceptions too).
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { idePreferences } from '../../../lib/idePreferences.svelte';
  import { plural } from '../../../lib/plural';
  import IdeBreakpointPopover from '../editor/IdeBreakpointPopover.svelte';
  import { ide, ideBasename, type IdeBreakpoint } from '../ideStore.svelte';
  import { mutatePathBreakpoints } from '../shell/ideProjectHelpers';
  import { ideKeymapPresetOf, ideShortcutLabel } from '../shell/ideShortcuts';
  import { groupBreakpoints } from './ideBreakpointList';

  const toggleKey = $derived(ideShortcutLabel('toggleBreakpoint', ideKeymapPresetOf(idePreferences.current.keymapPreset)));
  let editing = $state<{ path: string; line: number; x: number; y: number } | null>(null);

  const groups = $derived(
    groupBreakpoints(ide.allBreakpoints, (p) => ide.tabs.find((t) => t.path === p)?.content ?? null),
  );

  function edit(e: MouseEvent, bp: IdeBreakpoint): void {
    const row = (e.currentTarget as HTMLElement).closest('.bp-row') ?? (e.currentTarget as HTMLElement);
    const r = row.getBoundingClientRect();
    editing = { path: bp.path, line: bp.line, x: r.left + 8, y: r.bottom };
  }

  async function removeAll(): Promise<void> {
    const n = ide.allBreakpoints.length;
    if (n === 0) return;
    const ok = await confirmer.ask(`Remove all ${plural(n, 'breakpoint')} in this project?`, {
      title: 'Remove all breakpoints',
      confirmLabel: 'Remove all',
      danger: true,
    });
    if (!ok) return;
    for (const path of new Set(ide.allBreakpoints.map((b) => b.path))) mutatePathBreakpoints(ide, path, () => []);
  }
</script>

<div class="bp-bar">
  <div class="bp-switch">
    <Switch checked={ide.breakpointsMuted} label="Mute all breakpoints" onchange={() => ide.toggleMuteBreakpoints()} />
    <span>Mute all</span>
  </div>
  <div class="bp-switch">
    <Switch checked={ide.breakOnExceptions} label="Break on uncaught exceptions" onchange={(next) => { ide.breakOnExceptions = next; void ide.syncBreakpointsToSession(); }} />
    <span>Exceptions</span>
  </div>
  <span class="grow"></span>
  {#if ide.allBreakpoints.length > 0}
    <button type="button" class="btn small ghost bp-remove-all" onclick={() => void removeAll()} data-testid="ide-bp-remove-all">Remove all</button>
  {/if}
</div>
<div class="bp-scroll" data-testid="ide-breakpoints-list">
  {#if groups.length === 0}
    <EmptyState headingLevel={2} icon="bug" title="No breakpoints" body={`Click the editor line gutter${toggleKey ? ` or press ${toggleKey}` : ''} to toggle a breakpoint.`} />
  {:else}
    {#each groups as g (g.path)}
      <div class="bp-group" role="group" aria-label={`Breakpoints in ${ideBasename(g.path)}`}>
        <div class="bp-file" dir="ltr" title={ide.relativePath(g.path)}>
          <Icon name="file" size={12} /><span class="bp-file-name">{ideBasename(g.path)}</span><span class="bp-file-count dim">{g.rows.length}</span>
        </div>
        {#each g.rows as row (`${row.bp.path}:${row.bp.line}`)}
          {@const bp = row.bp}
          <div class="bp-row" class:is-unbound={row.unbound} class:is-off={!bp.enabled} data-testid="ide-bp-row">
            <Switch checked={bp.enabled} label="Enable breakpoint at {ideBasename(bp.path)} line {bp.line}" onchange={(next) => ide.updateBreakpoint(bp.path, bp.line, { enabled: next })} />
            <button
              type="button"
              class="bp-jump"
              dir="ltr"
              onclick={() => void ide.openFile(bp.path, { line: bp.line, preview: false })}
              title={row.unbound ? `Not bound: ${bp.message || 'the debugger could not set it'}` : `${ide.relativePath(bp.path)}:${bp.line}`}
            >
              <span class="bp-line mono">{bp.line}</span>
              {#if row.preview}<span class="bp-code mono">{row.preview}</span>{/if}
              {#if row.unbound}<span class="bp-msg">{bp.message || 'Not bound'}</span>{/if}
              {#if row.chip}<Badge variant="outline" tone={row.chip.kind === 'condition' ? 'warn' : 'info'} label={row.chip.label} />{/if}
            </button>
            <button type="button" class="bp-icon-btn" aria-label={`Edit breakpoint at line ${bp.line}`} title="Edit condition, hit count or log message…" onclick={(e) => edit(e, bp)} data-testid="ide-bp-edit">
              <Icon name="edit" size={12} />
            </button>
            <button type="button" class="bp-icon-btn" aria-label={`Remove breakpoint at line ${bp.line}`} title="Remove breakpoint" onclick={() => ide.removeBreakpoint(bp.path, bp.line)} data-testid="ide-bp-remove">
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </div>
    {/each}
  {/if}
</div>

{#if editing}
  <IdeBreakpointPopover path={editing.path} line={editing.line} anchorX={editing.x} anchorY={editing.y} onclose={() => (editing = null)} />
{/if}

<style>
  .bp-bar { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; padding-block: 6px; padding-inline: 10px; border-block-end: 1px solid var(--border); }
  .bp-switch { display: flex; align-items: center; gap: 6px; font-size: var(--fs-xs); color: var(--text-dim); }
  .bp-remove-all { color: var(--danger); }
  .bp-scroll { flex: 1; min-block-size: 0; overflow-y: auto; overflow-x: hidden; padding-block: 4px; }
  .bp-group + .bp-group { margin-block-start: 4px; }
  .bp-file { display: flex; align-items: center; gap: 6px; padding-block: 4px; padding-inline: 10px; font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .bp-file-name { min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
  .bp-file-count { font-weight: 400; }
  .bp-row { display: flex; align-items: center; gap: 6px; padding-block: 2px; padding-inline: 18px 8px; min-inline-size: 0; }
  .bp-row:hover { background: var(--surface-2); }
  .bp-row.is-unbound .bp-jump, .bp-row.is-off .bp-jump { color: var(--text-dim); }
  .bp-jump { flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 6px; background: transparent; border: none; color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer; overflow: hidden; }
  .bp-line { flex-shrink: 0; min-inline-size: 2.5ch; text-align: end; color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .bp-code { min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bp-msg { min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-style: italic; }
  .bp-icon-btn {
    display: inline-flex; align-items: center; justify-content: center;
    inline-size: 20px; block-size: 20px; border: 0; border-radius: var(--radius-s);
    background: transparent; color: var(--text-dim); cursor: pointer; flex-shrink: 0;
  }
  .bp-icon-btn:hover { background: var(--hover); color: var(--text); }
  @media (pointer: coarse) { .bp-icon-btn { inline-size: var(--hit-min); block-size: var(--hit-min); } .bp-row { min-block-size: var(--hit-min); } }
</style>
