<script lang="ts">
  import { broadcastScope } from './viewFilters';
  // Split layout host: renders the nested split TREE (`layout.tree`) through the
  // recursive SplitNode, and owns the layout-wide keyboard chords + ⌘K commands.
  // Pane membership, fractions and focus live in the layout store — this file
  // only mounts the root and keeps the broadcast bar.
  import SplitNode from './SplitNode.svelte';
  import { ws, DB_PANE_ID } from '../../lib/stores/workspace.svelte';
  import { layout, type Preset, type Rect, type Side } from '../../lib/stores/splitLayout.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { BroadcastResp } from '../../lib/api/types';

  /** Live pane geometry, read at call time — the keyboard move picks the
   *  geometric neighbour on the requested side, exactly like a tiling WM. */
  function paneRects(): Map<string, Rect> {
    return new Map(
      [...document.querySelectorAll<HTMLElement>('[data-pane-key]')].map((el) => [
        el.dataset.paneKey ?? '',
        el.getBoundingClientRect() as Rect,
      ]),
    );
  }

  /** After any layout move the caller re-focuses the pane so `activeSessionId`,
   *  the route and the navigator highlight follow (only focusPane routes). */
  function move(side: Side): void {
    layout.moveFocused(side, paneRects());
    ws.focusPane(layout.focusedIndex);
  }

  function preset(p: Preset): void {
    layout.applyPreset(p);
    ws.focusPane(layout.focusedIndex);
  }

  // ── ⌘⌥ chords + ⌘K commands ──────────────────────────────────────────────
  // Capture phase: xterm turns modified arrows into CSI sequences and cancels
  // the DOM event in its own textarea handler, and QueryEditor binds ⌘⌥←/→ in
  // capture too — a bubble listener would never see the chord over a terminal.
  $effect(() => {
    if (layout.panes.length < 2) return;
    const SIDES: Record<string, Side> = {
      ArrowLeft: 'left',
      ArrowRight: 'right',
      ArrowUp: 'up',
      ArrowDown: 'down',
    };
    const h = (e: KeyboardEvent): void => {
      if (!(e.metaKey || e.ctrlKey) || !e.altKey || e.shiftKey) return;
      // Inside a Database pane ⌘⌥←/→ stay with the query editor (tab switch).
      if (document.activeElement?.closest('.db-pane')) return;
      const side = SIDES[e.key];
      // `e.code`, not `e.key`: with ⌥ held macOS delivers the ALTERED character
      // (⌘⌥S → `e.key === 'ß'`), so a key comparison is dead on the only shipped
      // platform. QueryEditor's ⌥⌘T/W use `e.code` for the same reason.
      if (!side && e.code !== 'KeyS') return;
      e.preventDefault();
      e.stopPropagation();
      if (side) move(side);
      else {
        layout.swapWithNext();
        ws.focusPane(layout.focusedIndex);
      }
    };
    window.addEventListener('keydown', h, { capture: true });
    const unregister = registry.register('pane-layout', [
      { id: 'layout.move-left', title: 'Move pane left', group: 'Layout', shortcut: '⌘⌥←', keywords: 'split pane arrange tile', run: () => move('left') },
      { id: 'layout.move-right', title: 'Move pane right', group: 'Layout', shortcut: '⌘⌥→', keywords: 'split pane arrange tile', run: () => move('right') },
      { id: 'layout.move-up', title: 'Move pane up', group: 'Layout', shortcut: '⌘⌥↑', keywords: 'split pane arrange tile', run: () => move('up') },
      { id: 'layout.move-down', title: 'Move pane down', group: 'Layout', shortcut: '⌘⌥↓', keywords: 'split pane arrange tile', run: () => move('down') },
      {
        id: 'layout.swap-next',
        title: 'Swap pane with next',
        group: 'Layout',
        shortcut: '⌘⌥S',
        keywords: 'split pane arrange tile',
        run: () => {
          layout.swapWithNext();
          ws.focusPane(layout.focusedIndex);
        },
      },
      { id: 'layout.preset-cols', title: 'Layout: equal columns', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('cols') },
      { id: 'layout.preset-rows', title: 'Layout: equal rows', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('rows') },
      { id: 'layout.preset-one-two-below', title: 'Layout: one above two', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('one-two-below') },
      { id: 'layout.preset-one-two-beside', title: 'Layout: one beside two', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('one-two-beside') },
      { id: 'layout.preset-grid', title: 'Layout: grid', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('grid') },
    ]);
    return () => {
      window.removeEventListener('keydown', h, { capture: true });
      unregister();
    };
  });

  // ── Broadcast-input mode ────────────────────────────────────────────────
  // When on (only available with ≥2 panes), a compose bar appears above the
  // tree; pressing Enter relays the text to all visible session panes via
  // the existing `POST /workspaces/{id}/broadcast` endpoint (targets only
  // the pane session ids, not every live session in the workspace).
  let broadcastMode = $state(false);
  let broadcastText = $state('');
  let broadcastBusy = $state(false);

  // Deduped: split() clones the focused id into the new pane, so a raw pane
  // list can carry the same session twice — the broadcast would hit it twice.
  const broadcastTargets = $derived(
    [...new Set(ws.panes.filter((id) => id !== DB_PANE_ID))],
  );

  // The target sessions determine the workspace, including scratch. Never
  // silently skip a pane belonging to another workspace.
  const targetWorkspace = $derived(broadcastScope(broadcastTargets, ws.sessions));
  const broadcastable = $derived(targetWorkspace !== null);

  // Auto-disable broadcast mode when panes collapse to 1 or 0, or when the
  // targets stop being broadcastable (panes from different workspaces).
  $effect(() => {
    if (ws.panes.length < 2 || !broadcastable) broadcastMode = false;
  });

  async function sendBroadcast(): Promise<void> {
    const text = broadcastText.trim();
    if (!text || broadcastBusy || !targetWorkspace) return;
    const scope = targetWorkspace;
    broadcastBusy = true;
    try {
      const resp = await api.post<BroadcastResp>(
        `/workspaces/${scope}/broadcast`,
        { text, session_ids: broadcastTargets },
      );
      if (targetWorkspace === scope && broadcastText.trim() === text) broadcastText = '';
      const n = resp.session_ids.length;
      toasts.info('Broadcast sent', `Delivered to ${n} session${n === 1 ? '' : 's'}.`);
    } catch (e) {
      toasts.error('Broadcast failed', e instanceof Error ? e.message : String(e));
    } finally {
      broadcastBusy = false;
    }
  }

  function onBroadcastKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void sendBroadcast();
    }
    if (e.key === 'Escape') {
      broadcastMode = false;
      broadcastText = '';
    }
  }
</script>

<div class="splits" class:has-broadcast={broadcastMode}>
  {#if broadcastTargets.length >= 2 && !broadcastable}
    <p class="hint">Broadcast requires every selected session to belong to the same workspace.</p>
  {/if}
  {#if ws.panes.length >= 2 && broadcastTargets.length >= 2 && broadcastable}
    <div class="broadcast-bar-wrap">
      <button
        class="btn small broadcast-toggle"
        class:active={broadcastMode}
        onclick={() => { broadcastMode = !broadcastMode; }}
        title={broadcastMode ? 'Exit broadcast mode (Esc)' : 'Type once, send to every visible session'}
        aria-pressed={broadcastMode}
      ><Icon name="send" size={12} />{broadcastMode ? 'Exit broadcast' : 'Broadcast'}</button>
      {#if broadcastMode}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="broadcast-input"
          aria-label="Broadcast message"
          bind:value={broadcastText}
          placeholder="Send to all visible sessions — Enter to send"
          disabled={broadcastBusy}
          autofocus
          onkeydown={onBroadcastKeydown}
        />
        <button
          class="btn small primary"
          disabled={broadcastBusy || broadcastText.trim() === ''}
          onclick={() => void sendBroadcast()}
        >{broadcastBusy ? 'Sending…' : `Send to ${broadcastTargets.length}`}</button>
      {/if}
    </div>
  {/if}
  <div class="tree">
    {#if layout.tree}
      <SplitNode node={layout.tree} />
    {/if}
  </div>
</div>

<style>
  .splits {
    position: relative;
    height: 100%;
    padding: 8px;
    display: flex;
    flex-direction: column;
  }
  .splits.has-broadcast {
    padding-bottom: 0;
  }
  .broadcast-bar-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 0 4px 0;
    flex-shrink: 0;
  }
  .broadcast-toggle {
    gap: 5px;
  }
  .broadcast-toggle.active {
    border-color: var(--accent);
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .broadcast-input {
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 8px;
    font-size: var(--fs-s);
    background: var(--surface);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    color: var(--text);
  }
  /* The root fills the host; every nested track is sized by SplitNode. */
  .tree {
    flex: 1;
    min-height: 0;
    display: grid;
  }
  .tree > :global(*) {
    min-width: 0;
    min-height: 0;
  }
</style>
