<script lang="ts">
  // Namespace combobox: type to filter, "All namespaces" pinned first, arrow
  // keys + Enter/Esc. The popup is `position: fixed`, CLAMPED into the viewport
  // and height-capped (scrolls) so a cluster with hundreds of namespaces stays
  // reachable (AGENTS.md "Floating UI must survive long content"). Phones get
  // a native <select> instead.
  import { tick } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import type { K8sNamespace } from '../../lib/api/types';

  interface Props {
    value: string;
    namespaces: K8sNamespace[];
    /** Non-empty when the namespace list failed to load — typing still works. */
    error?: string;
    disabled?: boolean;
    onchange: (ns: string) => void;
  }
  let { value, namespaces, error = '', disabled = false, onchange }: Props = $props();

  const ALL = '';
  let open = $state(false);
  let query = $state('');
  let active = $state(0);
  let inputEl = $state<HTMLInputElement | null>(null);
  let popEl = $state<HTMLDivElement | null>(null);
  let pos = $state({ top: 0, left: 0, width: 240, maxH: 320 });

  const options = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const names = namespaces.map((n) => n.name).filter((n) => !q || n.toLowerCase().includes(q));
    const list: { value: string; label: string }[] = [];
    if (!q || 'all namespaces'.includes(q)) list.push({ value: ALL, label: 'All namespaces' });
    for (const n of names) list.push({ value: n, label: n });
    // Let the user type a namespace the list doesn't know (RBAC-limited
    // `get namespaces`, or the list failed) and still select it.
    if (q && !names.some((n) => n.toLowerCase() === q)) list.push({ value: query.trim(), label: `Use “${query.trim()}”` });
    return list;
  });

  const shown = $derived(value === ALL ? 'All namespaces' : value);

  function place(): void {
    if (!inputEl) return;
    const r = inputEl.getBoundingClientRect();
    const pad = 8;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.max(220, Math.min(360, r.width));
    const left = Math.max(pad, Math.min(r.left, vw - width - pad));
    const below = vh - r.bottom - pad;
    // Prefer below; if there's more room above, open upward but keep it capped
    // and INSIDE the viewport (never a negative top).
    const above = r.top - pad;
    const wantH = 320;
    let top: number;
    let maxH: number;
    if (below >= Math.min(wantH, 160) || below >= above) {
      top = r.bottom + 4;
      maxH = Math.max(120, Math.min(wantH, below - 4));
    } else {
      maxH = Math.max(120, Math.min(wantH, above - 4));
      top = Math.max(pad, r.top - 4 - maxH);
    }
    pos = { top, left, width, maxH };
  }

  async function show(): Promise<void> {
    if (disabled) return;
    query = '';
    active = 0;
    open = true;
    await tick();
    place();
  }

  function choose(v: string): void {
    open = false;
    query = '';
    if (v !== value) onchange(v);
    inputEl?.blur();
  }

  function onKey(e: KeyboardEvent): void {
    if (!open && (e.key === 'ArrowDown' || e.key === 'Enter')) {
      e.preventDefault();
      void show();
      return;
    }
    if (!open) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      active = Math.min(options.length - 1, active + 1);
      scrollActive();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      active = Math.max(0, active - 1);
      scrollActive();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const o = options[active];
      if (o) choose(o.value);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      open = false;
      query = '';
      inputEl?.blur();
    }
  }

  function scrollActive(): void {
    popEl?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }

  function onInput(): void {
    active = 0;
    if (!open) void show();
  }

  // Close on outside pointerdown + keep placed on resize/scroll.
  $effect(() => {
    if (!open) return;
    const down = (e: PointerEvent): void => {
      const t = e.target as Node;
      if (popEl?.contains(t) || inputEl?.contains(t)) return;
      open = false;
      query = '';
    };
    const re = (): void => place();
    window.addEventListener('pointerdown', down, true);
    window.addEventListener('resize', re);
    window.addEventListener('scroll', re, true);
    return () => {
      window.removeEventListener('pointerdown', down, true);
      window.removeEventListener('resize', re);
      window.removeEventListener('scroll', re, true);
    };
  });

  /** Let the workspace focus the field (`n` shortcut). */
  export function focus(): void {
    inputEl?.focus();
    void show();
  }
</script>

{#if viewport.isPhone}
  <select
    class="input ns-select"
    aria-label="Namespace"
    {disabled}
    value={value}
    onchange={(e) => onchange((e.currentTarget as HTMLSelectElement).value)}
    data-testid="k8s-ns-picker"
  >
    <option value="">All namespaces</option>
    {#each namespaces as n (n.name)}<option value={n.name}>{n.name}</option>{/each}
    {#if value && !namespaces.some((n) => n.name === value)}<option value={value}>{value}</option>{/if}
  </select>
{:else}
  <div class="ns" class:has-error={!!error} data-testid="k8s-ns-picker">
    <Icon name="layers" size={13} />
    <input
      bind:this={inputEl}
      class="ns-input"
      role="combobox"
      aria-label="Namespace"
      aria-expanded={open}
      aria-controls="k8s-ns-listbox"
      aria-autocomplete="list"
      aria-activedescendant={open ? `k8s-ns-opt-${active}` : undefined}
      placeholder={shown}
      title={error ? `Namespaces couldn't be listed: ${error}` : shown}
      value={open ? query : shown}
      {disabled}
      onfocus={() => void show()}
      onclick={() => void show()}
      oninput={(e) => { query = (e.currentTarget as HTMLInputElement).value; onInput(); }}
      onkeydown={onKey}
    />
    <Icon name="chevronDown" size={12} />
  </div>
  {#if open}
    <div
      bind:this={popEl}
      id="k8s-ns-listbox"
      class="ns-pop"
      role="listbox"
      aria-label="Namespaces"
      style="top:{pos.top}px;left:{pos.left}px;width:{pos.width}px;max-height:{pos.maxH}px"
    >
      {#if error}<div class="ns-err">Couldn't list namespaces — type one to use it.</div>{/if}
      {#each options as o, i (o.value + ':' + o.label)}
        <div
          id="k8s-ns-opt-{i}"
          data-i={i}
          class="ns-opt"
          class:active={i === active}
          class:current={o.value === value}
          role="option"
          aria-selected={o.value === value}
          tabindex="-1"
          onpointerenter={() => (active = i)}
          onpointerdown={(e) => { e.preventDefault(); choose(o.value); }}
          onkeydown={(e) => { if (e.key === 'Enter') choose(o.value); }}
        >
          <span class="ns-opt-label" class:mono={o.value !== ''}>{o.label}</span>
          {#if o.value === value}<Icon name="check" size={12} />{/if}
        </div>
      {/each}
      {#if !options.length}<div class="ns-err">No match</div>{/if}
    </div>
  {/if}
{/if}

<style>
  .ns {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    min-width: 0;
    width: 220px;
  }
  .ns:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .ns.has-error {
    border-color: color-mix(in srgb, var(--status-exited) 50%, var(--border));
  }
  .ns-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    font-family: var(--font-mono);
    outline: none;
  }
  .ns-input::placeholder {
    color: var(--text);
    opacity: 0.9;
  }
  .ns-select {
    max-width: 100%;
  }
  .ns-pop {
    position: fixed;
    z-index: 60;
    overflow-y: auto;
    padding: 4px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.25);
  }
  .ns-opt {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    cursor: pointer;
  }
  .ns-opt.active {
    background: var(--surface-2);
  }
  .ns-opt.current {
    color: var(--accent);
  }
  .ns-opt-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ns-err {
    padding: 6px 8px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
