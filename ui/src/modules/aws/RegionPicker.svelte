<script lang="ts">
  // Shared region switcher for the regional AWS views (A-1). Seeds from the
  // last region picked for this account + service (localStorage, per viewer),
  // falling back to the account's default region. `allowAll` adds the
  // "All enabled regions" option (EC2 / EKS / RDS fan out server-side).
  import { untrack } from 'svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { ALL_REGIONS } from '../../lib/api/aws';
  import type { AwsAccount } from '../../lib/api/types';

  interface Props {
    account: AwsAccount;
    /** Storage key part — `sqs`, `athena`, `ec2`, … */
    service: string;
    region: string;
    allowAll?: boolean;
  }
  let { account, service, region = $bindable(), allowAll = false }: Props = $props();

  const key = (a: AwsAccount) => `otto.aws.region.${a.id}.${service}`;

  // Restore once per mount (AwsPage remounts the view per account + service).
  $effect(() => {
    untrack(() => {
      void aws.loadRegions();
      try {
        const saved = localStorage.getItem(key(account));
        if (saved && (saved !== ALL_REGIONS || allowAll)) region = saved;
      } catch {
        // storage blocked (private window / preview) — keep the default
      }
    });
  });

  function pick(v: string): void {
    region = v;
    try {
      if (v === account.region) localStorage.removeItem(key(account));
      else localStorage.setItem(key(account), v);
    } catch {
      // ignore — the pick still applies for this view
    }
  }
</script>

<label class="sel">
  <span class="lbl">Region</span>
  {#if aws.regions.length}
    <select value={region} onchange={(e) => pick((e.currentTarget as HTMLSelectElement).value)} aria-label="Region" title="Region">
      {#if allowAll}<option value={ALL_REGIONS}>All enabled regions</option>{/if}
      {#each aws.regions as r (r.code)}
        <option value={r.code}>{r.code}{r.code === account.region ? ' (default)' : ''}</option>
      {/each}
    </select>
  {:else}
    <input class="mono" value={region} onchange={(e) => pick((e.currentTarget as HTMLInputElement).value.trim() || account.region)} aria-label="Region" size={12} />
  {/if}
</label>

<style>
  .sel {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
  }
  .lbl {
    color: var(--text-dim);
  }
  .sel select,
  .sel input {
    height: 26px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    padding: 0 4px;
  }
</style>
