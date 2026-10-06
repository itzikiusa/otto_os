<script lang="ts">
  // The one on/off switch (macOS-style): a real <button role="switch"> so it is
  // keyboard- and screen-reader-operable, with logical (RTL-safe) knob travel.
  interface Props {
    checked: boolean;
    onchange: (next: boolean) => void;
    /** Accessible name — a noun phrase naming WHAT is toggled ("Enable server filesystem"). */
    label: string;
    disabled?: boolean;
    title?: string;
    /** `success` reads as "armed/allowed" (tool enablement); default is the accent. */
    tone?: 'accent' | 'success';
  }
  let { checked, onchange, label, disabled = false, title, tone = 'accent' }: Props = $props();
</script>

<button
  type="button"
  class="sw"
  class:on={checked}
  class:success={tone === 'success'}
  role="switch"
  aria-checked={checked}
  aria-label={label}
  title={title ?? label}
  {disabled}
  onclick={() => onchange(!checked)}
></button>

<style>
  .sw {
    position: relative;
    flex: none;
    width: 30px;
    height: 17px;
    padding: 0;
    border-radius: 999px;
    border: none;
    /* OFF is an outlined track: a --text-dim ring and knob on --surface-2.
       Both are text tokens (>= 4.5:1 on every ground), so the control and its
       state stay visible on white cards (WCAG 1.4.11; the old 30% grey track
       with a white knob was 1.6:1). unit/tokenContrast.test.ts pins it. */
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--text-dim);
    cursor: pointer;
    transition: background var(--dur-fast) ease, box-shadow var(--dur-fast) ease;
  }
  .sw::after {
    content: '';
    position: absolute;
    inset-block-start: 2px;
    inset-inline-start: 2px;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: var(--text-dim);
    box-shadow: 0 1px 2px var(--scrim-soft);
    transition: inset-inline-start var(--dur-fast) ease;
  }
  .sw.on {
    background: var(--accent-solid);
    box-shadow: none;
  }
  .sw.on::after {
    inset-inline-start: 15px;
    background: var(--accent-contrast);
  }
  .sw.on.success {
    background: var(--success);
  }
  /* The tone is text-safe on --bg, so --bg is a >= 4.5:1 knob on it in every
     scheme (a white knob on the dark-scheme green was 1.9:1). */
  .sw.on.success::after {
    background: var(--bg);
  }
  .sw:disabled {
    opacity: var(--disabled-opacity);
    cursor: not-allowed;
  }
  .sw:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  @media (prefers-reduced-motion: reduce) {
    .sw, .sw::after { transition: none; }
  }
</style>
