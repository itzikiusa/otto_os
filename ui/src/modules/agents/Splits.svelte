<script lang="ts">
  // Split layout host: renders the nested split TREE (`layout.tree`) through the
  // recursive SplitNode, and owns the layout-wide keyboard chords + ⌘K commands.
  // Pane membership, fractions and focus live in the layout store — this file
  // only mounts the root and keeps the broadcast bar.
  import SplitNode from './SplitNode.svelte';
  import { ws, DB_PANE_ID, SCRATCH_WORKSPACE_ID } from '../../lib/stores/workspace.svelte';
  import { layout, type Preset, type Rect, type Side } from '../../lib/stores/splitLayout.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
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
      { id: 'layout.move-left', title: 'Move Pane Left', group: 'Layout', shortcut: '⌘⌥←', keywords: 'split pane arrange tile', run: () => move('left') },
      { id: 'layout.move-right', title: 'Move Pane Right', group: 'Layout', shortcut: '⌘⌥→', keywords: 'split pane arrange tile', run: () => move('right') },
      { id: 'layout.move-up', title: 'Move Pane Up', group: 'Layout', shortcut: '⌘⌥↑', keywords: 'split pane arrange tile', run: () => move('up') },
      { id: 'layout.move-down', title: 'Move Pane Down', group: 'Layout', shortcut: '⌘⌥↓', keywords: 'split pane arrange tile', run: () => move('down') },
      {
        id: 'layout.swap-next',
        title: 'Swap Pane With Next',
        group: 'Layout',
        shortcut: '⌘⌥S',
        keywords: 'split pane arrange tile',
        run: () => {
          layout.swapWithNext();
          ws.focusPane(layout.focusedIndex);
        },
      },
      { id: 'layout.preset-cols', title: 'Layout: Equal Columns', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('cols') },
      { id: 'layout.preset-rows', title: 'Layout: Equal Rows', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('rows') },
      { id: 'layout.preset-one-two-below', title: 'Layout: One Above Two', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('one-two-below') },
      { id: 'layout.preset-one-two-beside', title: 'Layout: One Beside Two', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('one-two-beside') },
      { id: 'layout.preset-grid', title: 'Layout: Grid', group: 'Layout', keywords: 'split pane arrange tile', run: () => preset('grid') },
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

  // `POST /workspaces/{id}/broadcast` is a WORKSPACE route: it needs a current
  // workspace, and the daemon only relays to sessions that live in it. A
  // workspace-less (scratch) pane can therefore never be a target, and with no
  // workspace selected there is no id to post to at all — in both cases the bar
  // would offer a compose box whose Enter silently does nothing, so hide it.
  const broadcastable = $derived(
    ws.currentId !== null &&
      !broadcastTargets.some(
        (id) => ws.sessions.find((s) => s.id === id)?.workspace_id === SCRATCH_WORKSPACE_ID,
      ),
  );

  // Auto-disable broadcast mode when panes collapse to 1 or 0, or when the
  // targets stop being broadcastable (a scratch session dropped into a pane).
  $effect(() => {
    if (ws.panes.length < 2 || !broadcastable) broadcastMode = false;
  });

  async function sendBroadcast(): Promise<void> {
    const text = broadcastText.trim();
    if (!text || broadcastBusy || !ws.currentId) return;
    broadcastBusy = true;
    try {
      const resp = await api.post<BroadcastResp>(
        `/workspaces/${ws.currentId}/broadcast`,
        { text, session_ids: broadcastTargets },
      );
      broadcastText = '';
      toasts.info('Broadcast sent', `Delivered to ${resp.session_ids.length} session(s).`);
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
  {#if ws.panes.length >= 2 && broadcastTargets.length >= 2 && broadcastable}
    <div class="broadcast-bar-wrap">
      <button
        class="broadcast-toggle"
        class:active={broadcastMode}
        onclick={() => { broadcastMode = !broadcastMode; }}
        title={broadcastMode ? 'Exit broadcast mode' : 'Broadcast input to all visible sessions'}
        aria-pressed={broadcastMode}
      >{broadcastMode ? '↗ exit broadcast' : '↗ broadcast'}</button>
      {#if broadcastMode}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="broadcast-input"
          bind:value={broadcastText}
          placeholder="Send to all visible sessions — Enter to send"
          disabled={broadcastBusy}
          autofocus
          onkeydown={onBroadcastKeydown}
        />
        <button
          class="broadcast-send"
          disabled={broadcastBusy || broadcastText.trim() === ''}
          onclick={() => void sendBroadcast()}
        >{broadcastBusy ? '…' : 'Send'}</button>
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
    flex-shrink: 0;
    font-size: 11px;
    padding: 2px 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    cursor: pointer;
    transition: background 120ms, color 120ms, border-color 120ms;
  }
  .broadcast-toggle.active {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, var(--surface-2));
  }
  .broadcast-toggle:hover {
    color: var(--text);
    border-color: var(--accent);
  }
  .broadcast-input {
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 8px;
    font-size: 12px;
    background: var(--surface);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    color: var(--text);
    outline: none;
  }
  .broadcast-send {
    flex-shrink: 0;
    height: 26px;
    padding: 0 12px;
    font-size: 12px;
    border-radius: var(--radius-s);
    border: 1px solid var(--accent);
    background: var(--accent);
    color: #fff;
    cursor: pointer;
    transition: opacity 120ms;
  }
  .broadcast-send:disabled {
    opacity: 0.5;
    cursor: not-allowed;
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
