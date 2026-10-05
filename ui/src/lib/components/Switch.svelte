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
    background: color-mix(in srgb, var(--text-dim) 30%, transparent);
    cursor: pointer;
    transition: background var(--dur-fast) ease;
  }
  .sw::after {
    content: '';
    position: absolute;
    inset-block-start: 2px;
    inset-inline-start: 2px;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: var(--accent-contrast);
    transition: inset-inline-start var(--dur-fast) ease;
  }
  .sw.on {
    background: var(--accent-solid);
  }
  .sw.on::after {
    inset-inline-start: 15px;
  }
  .sw.on.success {
    background: var(--success);
  }
  .sw:disabled {
    opacity: 0.5;
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
