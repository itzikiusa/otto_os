<script lang="ts">
  import { announce } from '../../lib/announce.svelte';
  import { plural } from '../../lib/plural';
  // Inline note for an "All enabled regions" list: the regions that failed
  // (denied, opt-in disabled, throttled…) while the rest still rendered.
  import Icon from '../../lib/components/Icon.svelte';
  import type { AwsRegionError } from '../../lib/api/types';

  interface Props {
    errors: AwsRegionError[];
  }
  let { errors }: Props = $props();
  let open = $state(false);
  // A `truncated` row WAS listed, only partly (the page cap): a different
  // message from a region that failed outright.
  const failed = $derived(errors.filter((e) => !e.truncated));
  const partial = $derived(errors.filter((e) => e.truncated));
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  const count = $derived(failed.length);
  $effect(() => { if (count) announce(`${plural(count, 'region')} couldn’t be listed — the rows below come from the others.`); });
  $effect(() => { if (partial.length) announce(`The list is incomplete in ${plural(partial.length, 'region')}.`); });
</script>

{#each partial as p (p.region)}
  <div class="rerr" data-testid="aws-list-truncated">
    <Icon name="warning" size={13} />
    <span>List incomplete in <span class="mono">{p.region}</span> — {p.message}.</span>
  </div>
{/each}
{#if failed.length}
  <div class="rerr">
    <Icon name="warning" size={13} />
    <span>{plural(failed.length, 'region')} couldn’t be listed — the rows below come from the others.</span>
    <button class="link" onclick={() => (open = !open)} aria-expanded={open}>{open ? 'Hide' : 'Details'}</button>
    {#if open}
      <ul>
        {#each failed as e (e.region)}
          <li><span class="mono">{e.region}</span> — {e.message}</li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .rerr {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 8px 12px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .rerr :global(svg) {
    color: var(--warning);
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--accent-text);
    cursor: pointer;
    font: inherit;
  }
  ul {
    flex-basis: 100%;
    margin: 4px 0 0;
    padding-inline-start: 18px;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
