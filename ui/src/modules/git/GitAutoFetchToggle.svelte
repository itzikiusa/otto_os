<script lang="ts">
  // The auto-fetch toggle (split out of GitTabs.svelte): the repo tab strip
  // carries it on desktop, the phone's one header row next to Fetch.
  import { git } from '../../lib/stores/git.svelte';
  import Icon from '../../lib/components/Icon.svelte';
</script>

<button
  class="git-autofetch"
  class:on={git.autoFetchEnabled}
  title={git.autoFetchEnabled
    ? `Auto-fetch on — selected repo every 60s, other recently used repos every ${Math.max(60, git.autoFetchIntervalSec)}s while Git is open and this window is focused. Click to pause.`
    : 'Auto-fetch paused — a repo is still fetched once when you open it. Click to fetch open repos automatically.'}
  aria-label="Auto-fetch open repositories"
  aria-pressed={git.autoFetchEnabled}
  onclick={() => git.setAutoFetch(!git.autoFetchEnabled)}
>
  <Icon name="fetch" size={14} />
  <!-- Paused is a lasting, per-device choice: say so in words, not only in
       the icon's tint (it was paused for weeks without anyone noticing).
       One short word next to the fetch icon — the full sentence is the
       tooltip — so it doesn't eat the room the repo tabs' branches need. -->
  {#if !git.autoFetchEnabled}<span class="git-autofetch-off">Paused</span>{/if}
</button>

<style>
  /* Auto-fetch toggle: dim text when paused, accent when on. No opacity on the
     paused state: it pushed the icon below 3:1 in the light theme, and
     aria-pressed plus the accent already tell on from off. */
  .git-autofetch {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0 8px;
    border-radius: var(--radius-s);
    flex-shrink: 0;
  }
  .git-autofetch:hover {
    background: var(--hover);
    color: var(--text);
  }
  .git-autofetch:not(.on) {
    gap: 4px;
    color: var(--warning);
  }
  .git-autofetch-off {
    font-size: var(--fs-s);
    white-space: nowrap;
  }
  @media (max-width: 1024px) {
    .git-autofetch {
      min-width: 44px;
      min-height: 44px;
    }
  }
</style>
