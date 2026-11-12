<script lang="ts">
  // "Explain these lines": one small action beside a text selection in a
  // PR's Code tab (mouse or touch — select the rows, then tap it). The pick
  // is read off the diff's DOM (pr-agent-diff-pick.ts), so DiffViewer needs
  // no hook; the PR's agent thread gets the path, the line range and the
  // rows themselves.
  import Icon from '../../lib/components/Icon.svelte';
  import { lineLabel, type LinePick } from './pr-agent-context';
  import { pickFromSelection } from './pr-agent-diff-pick';

  interface Props {
    onexplain: (pr: number, pick: LinePick) => void;
  }
  let { onexplain }: Props = $props();

  let shown = $state.raw<{ pick: LinePick; pr: number; top: number; left: number } | null>(null);
  let btn = $state<HTMLButtonElement | null>(null);
  const GAP = 8;

  /** Under the selection's end when it fits, else above it — clamped into
   *  the viewport either way (never flipped off screen). */
  function place(rect: DOMRect): { top: number; left: number } {
    const w = btn?.offsetWidth || 200;
    const h = btn?.offsetHeight || 34;
    let top = rect.bottom + GAP;
    if (top + h > window.innerHeight - GAP) top = rect.top - GAP - h;
    return {
      top: Math.max(GAP, Math.min(window.innerHeight - h - GAP, top)),
      left: Math.max(GAP, Math.min(window.innerWidth - w - GAP, rect.left)),
    };
  }

  $effect(() => {
    let raf = 0;
    let hide: ReturnType<typeof setTimeout> | null = null;
    const update = (): void => {
      raf = 0;
      const hit = pickFromSelection(document.getSelection());
      if (hit) {
        if (hide) clearTimeout(hide);
        hide = null;
        shown = { pick: hit.pick, pr: hit.pr, ...place(hit.rect) };
      } else if (shown && !hide) {
        // A tap on the action can collapse the selection before its click
        // lands (touch): keep the action a beat longer.
        hide = setTimeout(() => { hide = null; shown = null; }, 300);
      }
    };
    const schedule = (): void => { if (!raf) raf = requestAnimationFrame(update); };
    document.addEventListener('selectionchange', schedule);
    window.addEventListener('scroll', schedule, true);
    return () => {
      document.removeEventListener('selectionchange', schedule);
      window.removeEventListener('scroll', schedule, true);
      if (raf) cancelAnimationFrame(raf);
      if (hide) clearTimeout(hide);
    };
  });

  function explain(): void {
    const s = shown;
    if (!s) return;
    shown = null;
    document.getSelection()?.removeAllRanges();
    onexplain(s.pr, s.pick);
  }
</script>

{#if shown}
  {@const label = `Explain ${lineLabel(shown.pick)}`}
  <!-- pointerdown is cancelled so a mouse press keeps the selection it acts on. -->
  <button
    bind:this={btn}
    type="button"
    class="btn small pra-pick"
    data-testid="pr-agent-line-pick"
    style="left:{shown.left}px;top:{shown.top}px"
    title="{label} of {shown.pick.path} with the PR’s agent"
    onpointerdown={(e) => { if (e.pointerType === 'mouse') e.preventDefault(); }}
    onclick={explain}
  >
    <Icon name="sparkle" size={12} /> {label}
  </button>
{/if}

<style>
  .pra-pick {
    position: fixed;
    z-index: var(--z-popover);
    gap: 6px;
    max-inline-size: calc(100vw - 16px);
    white-space: nowrap;
    box-shadow: var(--shadow-card);
  }
  @media (max-width: 1024px) {
    .pra-pick { min-block-size: var(--hit-min); }
  }
</style>
