<script lang="ts">
  // Numeric field with studio-style scrubbing: drag horizontally on the label to
  // change the value by `step` per pixel (⇧ ×10, ⌥ ×0.1), or type into the input.
  // Emits `onchange(value)` live during the drag (the caller is undebounced by
  // contract; the arena debounces the save).
  interface Props {
    label: string;
    value: number;
    step?: number;
    min?: number;
    max?: number;
    /** Decimal places shown while not editing. */
    digits?: number;
    unit?: string;
    disabled?: boolean;
    onchange: (value: number) => void;
  }
  let { label, value, step = 0.01, min = -Infinity, max = Infinity, digits = 2, unit = '', disabled = false, onchange }: Props = $props();

  let editing = $state(false);
  let text = $state('');
  let dragging = $state(false);
  let startX = 0;
  let startVal = 0;
  let moved = false;

  const shown = $derived(editing ? text : fmt(value));

  function fmt(v: number): string {
    if (!Number.isFinite(v)) return '0';
    const s = v.toFixed(digits);
    return s.replace(/\.?0+$/, '') || '0';
  }
  function clamp(v: number): number {
    return Math.min(max, Math.max(min, v));
  }
  function commitText(): void {
    editing = false;
    // Accept simple arithmetic ("+0.5", "*2") the way studio tools do.
    const t = text.trim();
    let v: number;
    const m = /^([+\-*/])\s*(-?\d+(?:\.\d+)?)$/.exec(t);
    if (m) {
      const n = Number(m[2]);
      v = m[1] === '+' ? value + n : m[1] === '-' ? value - n : m[1] === '*' ? value * n : n === 0 ? value : value / n;
    } else {
      v = Number(t);
    }
    if (!Number.isFinite(v)) return;
    v = clamp(v);
    if (v !== value) onchange(v);
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      (e.currentTarget as HTMLInputElement).blur();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      text = fmt(value);
      (e.currentTarget as HTMLInputElement).blur();
    } else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault();
      const mult = e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
      const dir = e.key === 'ArrowUp' ? 1 : -1;
      const v = clamp(value + dir * step * 10 * mult);
      text = fmt(v);
      onchange(v);
    }
  }

  function onPointerDown(e: PointerEvent): void {
    if (disabled || e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragging = true;
    moved = false;
    startX = e.clientX;
    startVal = value;
  }
  function onPointerMove(e: PointerEvent): void {
    if (!dragging) return;
    const dx = e.clientX - startX;
    if (Math.abs(dx) < 2 && !moved) return;
    moved = true;
    const mult = e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
    const v = clamp(startVal + dx * step * mult);
    if (v !== value) onchange(Number(v.toFixed(6)));
  }
  function onPointerUp(e: PointerEvent): void {
    if (!dragging) return;
    dragging = false;
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
  }
</script>

<label class="nd" class:dragging class:disabled>
  <span
    class="nd-label"
    title="Drag to scrub (⇧ ×10, ⌥ ×0.1)"
    role="slider"
    aria-label={label}
    aria-valuenow={value}
    tabindex="-1"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}>{label}</span
  >
  <input
    class="nd-input"
    type="text"
    inputmode="decimal"
    value={shown}
    {disabled}
    onfocus={() => {
      editing = true;
      text = fmt(value);
    }}
    oninput={(e) => (text = (e.currentTarget as HTMLInputElement).value)}
    onblur={commitText}
    onkeydown={onKey}
    aria-label={label}
  />
  {#if unit}<span class="nd-unit">{unit}</span>{/if}
</label>

<style>
  .nd {
    display: inline-flex;
    align-items: center;
    min-width: 0;
    flex: 1 1 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 5px);
    background: var(--bg);
    height: 24px;
    overflow: hidden;
  }
  .nd:focus-within {
    border-color: var(--accent);
  }
  .nd.disabled {
    opacity: 0.5;
  }
  .nd-label {
    flex: 0 0 auto;
    padding: 0 5px;
    font-size: 10px;
    font-weight: 600;
    color: var(--text-dim);
    cursor: ew-resize;
    user-select: none;
    touch-action: none;
    border-right: 1px solid var(--border);
    height: 100%;
    display: inline-flex;
    align-items: center;
    background: var(--surface-2);
  }
  .nd.dragging .nd-label {
    color: var(--accent);
  }
  .nd-input {
    flex: 1 1 auto;
    min-width: 0;
    width: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: 11.5px;
    padding: 0 5px;
    font-variant-numeric: tabular-nums;
    outline: none;
  }
  .nd-unit {
    padding: 0 5px 0 0;
    font-size: 10px;
    color: var(--text-dim);
  }
</style>
