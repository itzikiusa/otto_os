<script lang="ts">
  // The viewport's STATES bar (Spline-style): Idle · Hover · Flipped… Picking
  // one tweens the scene to it; while a non-default state shows, gizmo and
  // inspector transform edits are recorded as that state's overrides. ⋯ holds
  // rename / timing / start state / duplicate / delete, all through `ops.ts`.
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import {
    EASINGS,
    addState,
    clearStateOverride,
    initialState,
    removeState,
    renameState,
    setDefaultState,
    setStateTiming,
    type Scene3dDoc,
  } from '../../product/design/scene3d';

  interface Props {
    doc: Scene3dDoc;
    stateId: string | null;
    selectedId?: string | null;
    readonly?: boolean;
    onpick: (id: string | null) => void;
    onchange: (doc: Scene3dDoc) => void;
  }
  let { doc, stateId, selectedId = null, readonly = false, onpick, onchange }: Props = $props();

  const states = $derived(doc.states ?? []);
  const start = $derived(initialState(doc));
  const current = $derived(states.find((s) => s.id === stateId) ?? states.find((s) => s.id === start) ?? null);

  async function add(): Promise<void> {
    const name = await confirmer.promptText('Name the new state. It starts as a copy of the one showing; move things to record how it differs.', {
      title: 'New state',
      confirmLabel: 'Add state',
      placeholder: states.length ? 'Flipped' : 'Idle',
    });
    if (!name) return;
    const r = addState(doc, name, current?.id ?? null);
    onchange(r.doc);
    onpick(r.id);
  }

  function menu(e: MouseEvent): void {
    const st = current;
    if (!st) return;
    const selOverride = selectedId ? st.overrides?.[selectedId] : undefined;
    const items: MenuItem[] = [
      {
        label: 'Rename…',
        icon: 'edit',
        disabled: readonly,
        action: async () => {
          const n = await confirmer.promptText('State name', { title: 'Rename state', confirmLabel: 'Rename', initial: st.name ?? st.id });
          if (n) onchange(renameState(doc, st.id, n));
        },
      },
      {
        label: `Transition: ${st.duration_ms ?? 400} ms…`,
        icon: 'clock',
        disabled: readonly,
        action: async () => {
          const v = await confirmer.promptText('How long the tween INTO this state takes, in milliseconds (0–10 000).', {
            title: 'Transition duration',
            confirmLabel: 'Set',
            initial: String(st.duration_ms ?? 400),
          });
          const n = v === null ? NaN : Number(v);
          if (Number.isFinite(n)) onchange(setStateTiming(doc, st.id, { duration_ms: n }));
        },
      },
      { separator: true },
      ...EASINGS.map<MenuItem>((ez) => ({
        label: `${(st.easing ?? 'ease-in-out') === ez ? '✓ ' : ''}Easing: ${ez}`,
        disabled: readonly,
        action: () => onchange(setStateTiming(doc, st.id, { easing: ez })),
      })),
      { separator: true },
      { label: 'Start in this state', icon: 'check', disabled: readonly || st.id === start, action: () => onchange(setDefaultState(doc, st.id)) },
      {
        label: 'Duplicate',
        icon: 'copy',
        disabled: readonly,
        action: () => {
          const r = addState(doc, `${st.name ?? st.id} copy`, st.id);
          onchange(r.doc);
          onpick(r.id);
        },
      },
    ];
    if (selectedId && selOverride) {
      items.push({ label: 'Reset selection in this state', icon: 'refresh', disabled: readonly, action: () => onchange(clearStateOverride(doc, st.id, selectedId!)) });
    }
    items.push(
      { separator: true },
      {
        label: 'Delete state',
        icon: 'trash',
        danger: true,
        disabled: readonly,
        action: () => {
          onchange(removeState(doc, st.id));
          onpick(null);
        },
      },
    );
    ctxMenu.show(e, items);
  }
</script>

<div class="states" role="group" aria-label="States" data-testid="s3d-states">
  <span class="k">States</span>
  {#if states.length}
    <div class="seg" role="radiogroup" aria-label="Scene state">
      {#each states as s (s.id)}
        <button
          role="radio"
          aria-checked={current?.id === s.id}
          class:on={current?.id === s.id}
          title={s.id === start ? `${s.name ?? s.id} (start state)` : `${s.name ?? s.id} · ${s.duration_ms ?? 400} ms`}
          onclick={() => onpick(s.id)}
          data-testid="s3d-state-{s.id}"
        >
          {s.name ?? s.id}
        </button>
      {/each}
    </div>
  {:else}
    <span class="none">None yet</span>
  {/if}
  {#if !readonly}
    <button class="icon-btn" onclick={() => void add()} aria-label="Add state" title="Add state">
      <Icon name="plus" size={12} />
    </button>
    {#if current}
      <button class="icon-btn" onclick={menu} aria-label="State options" title="State options" aria-haspopup="menu">
        <Icon name="more" size={12} />
      </button>
    {/if}
  {/if}
</div>

<style>
  .states {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 5px 6px 5px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: color-mix(in srgb, var(--surface) 92%, transparent);
    backdrop-filter: blur(8px);
    box-shadow: var(--shadow);
  }
  .k {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .seg {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .seg button {
    appearance: none;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    font: var(--fs-s) / 1 var(--font-ui);
    padding: 6px 11px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .seg button:hover:not(.on) {
    color: var(--text);
  }
  .seg button.on {
    background: var(--bg);
    color: var(--text);
    box-shadow: 0 1px 2px color-mix(in srgb, var(--text) 18%, transparent);
  }
  .seg button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .none {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
