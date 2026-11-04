<script lang="ts">
  // The composer's access chip: how freely the agent may act from the next
  // turn on. Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/runtimeModeConfig.ts (labels, icons,
  // descriptions). Full access asks once (Otto's confirmer) and stays in the
  // warning colour while picked.
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import type { RuntimeAccessMode } from '../../../lib/api/types';

  interface Props {
    mode: RuntimeAccessMode;
    /** Who acts and where, for the full-access confirm. */
    agent: string;
    cwd: string;
    disabled?: boolean;
    onpick: (mode: RuntimeAccessMode) => void;
  }
  let { mode, agent, cwd, disabled = false, onpick }: Props = $props();

  const ACCESS: Record<RuntimeAccessMode, { label: string; description: string; icon: IconName }> = {
    'approval-required': { label: 'Supervised', description: 'Ask before commands and file changes.', icon: 'lock' },
    'auto-accept-edits': { label: 'Auto-accept edits', description: 'Auto-approve edits, ask before other actions.', icon: 'edit' },
    'full-access': { label: 'Full access', description: 'Allow commands and edits without prompts.', icon: 'shieldOff' },
  };
  const FULL_OK_KEY = 'otto_runtime_full_access_ok';
  const cur = $derived(ACCESS[mode]);

  async function pick(m: RuntimeAccessMode): Promise<void> {
    let asked = false;
    try {
      asked = localStorage.getItem(FULL_OK_KEY) === '1';
    } catch {
      /* no storage: ask */
    }
    if (m === 'full-access' && !asked) {
      const ok = await confirmer.ask(
        `${agent} will run commands and change files in ${cwd} without asking first, from the next message on. The macOS sandbox still applies.`,
        { title: 'Allow full access?', confirmLabel: 'Allow full access', danger: true },
      );
      if (!ok) return;
      try {
        localStorage.setItem(FULL_OK_KEY, '1');
      } catch {
        /* asks again next time */
      }
    }
    onpick(m);
  }

  function openMenu(e: MouseEvent): void {
    ctxMenu.showAt(
      e.currentTarget as HTMLElement,
      (Object.keys(ACCESS) as RuntimeAccessMode[]).map((m) => ({
        label: ACCESS[m].label,
        icon: ACCESS[m].icon,
        checked: m === mode,
        title: ACCESS[m].description,
        action: () => void pick(m),
      })),
    );
  }
</script>

<button
  type="button"
  class="ac"
  class:full={mode === 'full-access'}
  aria-haspopup="menu"
  aria-label="Access for the next turn: {cur.label}"
  title={cur.description}
  data-testid="runtime-access"
  {disabled}
  onclick={openMenu}
>
  <Icon name={cur.icon} size={13} />
  <span class="ellipsis">{cur.label}</span>
</button>

<style>
  .ac {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    block-size: 28px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .ac:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .ac.full,
  .ac.full:hover:not(:disabled) {
    background: var(--warning-soft);
    color: var(--warning);
  }
</style>
