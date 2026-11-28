<script lang="ts">
  // The ONE phone Back (design.md §3 "Phone Back"): a chevron and the name of
  // the list the content was pushed from ("‹ Threads"), in PageHeader's
  // leading slot — PageHeader draws it from the innermost ListDetail
  // (pageNav.back), so a page never places it by hand. A page with no
  // PageHeader (the Agents session pane) uses this same component in its own
  // header row. A long label shortens before the page title does, but never
  // below its first word ("‹ Pull…", never "‹ Ses…"): then the title gives way.
  import Icon from '../Icon.svelte';

  interface Props {
    /** The list's name: "Threads", "Tables", "Files". */
    label: string;
    onclick: () => void;
    testid?: string;
  }
  let { label, onclick, testid = 'phone-back' }: Props = $props();
  const cut = $derived(label.indexOf(' '));
</script>

<button type="button" class="phone-back" aria-label="Back to {label}" title="Back to {label}" data-testid={testid} {onclick}>
  <Icon name="chevronLeft" size={20} />
  <span class="pb-label"><span class="pb-first">{cut < 0 ? label : label.slice(0, cut)}</span>{#if cut >= 0}<span class="pb-rest">{label.slice(cut)}</span>{/if}</span>
</button>

<style>
  .phone-back {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    /* auto: the chevron and the first word (pb-first) always show. */
    max-inline-size: 40vw;
    block-size: 44px;
    padding-block: 0;
    padding-inline: 2px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-m);
    font-weight: 500;
    cursor: pointer;
    /* The label gives way before the page title (design.md §7.11). */
    flex: 0 100 auto;
  }
  .phone-back :global(svg) {
    flex: none;
  }
  .pb-label {
    display: flex;
    min-inline-size: 0;
    white-space: pre;
  }
  .pb-first {
    flex: none;
  }
  .pb-rest {
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .phone-back:hover {
    background: var(--hover);
  }
</style>
