<script lang="ts" module>
  // Providers with a real brand mark below (claude/codex/agy). ANY other
  // provider — shell or a custom slug like `grok` — still gets an icon: a
  // generic monogram tile, so custom providers look first-class everywhere
  // (never a bare text label as if they weren't there). So this is always true
  // for a non-empty provider; kept as a function so existing callers compile.
  const BRANDED = new Set(['claude', 'codex', 'agy']);
  export function hasProviderIcon(provider: string | null | undefined): boolean {
    return !!provider && provider.trim() !== '';
  }
  export function isBrandedProvider(provider: string | null | undefined): boolean {
    return !!provider && BRANDED.has(provider.toLowerCase());
  }
  /** Deterministic hue (0–360) from a provider slug, for the monogram tile. */
  function hueFor(slug: string): number {
    let h = 0;
    for (let i = 0; i < slug.length; i++) h = (h * 31 + slug.charCodeAt(i)) % 360;
    return h;
  }
</script>

<script lang="ts">
  // Per-provider brand marks as inline SVG (crisp at 12–16px, no assets):
  //  claude — terracotta tile + cream starburst
  //  codex  — light tile + blue-gradient cloud with a `>_` prompt
  //  agy    — Antigravity rainbow arch "A"
  interface Props {
    provider: string;
    size?: number;
  }
  let { provider, size = 14 }: Props = $props();
  const p = $derived((provider ?? '').toLowerCase());
  // Monogram: up to 2 leading alphanumerics of a custom provider slug.
  const mono = $derived((p.match(/[a-z0-9]/g) ?? []).slice(0, 2).join('').toUpperCase() || '?');
  const hue = $derived(hueFor(p));
</script>

{#if p === 'claude'}
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
    <rect x="1" y="1" width="22" height="22" rx="5.5" fill="#c96442" />
    <g stroke="#f5f0e8" stroke-width="2" stroke-linecap="round">
      <line x1="14.57" y1="12.36" x2="20.22" y2="13.16" />
      <line x1="14.05" y1="13.60" x2="17.67" y2="16.43" />
      <line x1="12.97" y1="14.41" x2="15.18" y2="19.88" />
      <line x1="11.64" y1="14.57" x2="11.03" y2="18.93" />
      <line x1="10.40" y1="14.05" x2="6.95" y2="18.46" />
      <line x1="9.59" y1="12.97" x2="5.14" y2="14.77" />
      <line x1="9.43" y1="11.64" x2="3.68" y2="10.83" />
      <line x1="9.95" y1="10.40" x2="6.41" y2="7.63" />
      <line x1="11.03" y1="9.59" x2="8.89" y2="4.30" />
      <line x1="12.36" y1="9.43" x2="13.02" y2="4.77" />
      <line x1="13.60" y1="9.95" x2="16.99" y2="5.62" />
      <line x1="14.41" y1="11.03" x2="18.95" y2="9.19" />
    </g>
  </svg>
{:else if p === 'codex'}
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
    <defs>
      <linearGradient id="otto-pi-codex" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stop-color="#8a7bf4" />
        <stop offset="1" stop-color="#2f3fe8" />
      </linearGradient>
    </defs>
    <rect x="1" y="1" width="22" height="22" rx="5.5" fill="#eef0f5" />
    <!-- fluffy cloud = center blob + 6 lobes -->
    <g fill="url(#otto-pi-codex)">
      <circle cx="12" cy="12" r="5.6" />
      <circle cx="12" cy="7.2" r="3.1" />
      <circle cx="16.2" cy="9.6" r="3.1" />
      <circle cx="16.2" cy="14.4" r="3.1" />
      <circle cx="12" cy="16.8" r="3.1" />
      <circle cx="7.8" cy="14.4" r="3.1" />
      <circle cx="7.8" cy="9.6" r="3.1" />
    </g>
    <g stroke="#eef0f5" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" fill="none">
      <polyline points="9.1,9.6 11.4,12 9.1,14.4" />
      <line x1="13.2" y1="14.6" x2="16" y2="14.6" />
    </g>
  </svg>
{:else if p === 'agy'}
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
    <defs>
      <linearGradient id="otto-pi-agy" x1="0" y1="0.8" x2="1" y2="0.4">
        <stop offset="0" stop-color="#34a853" />
        <stop offset="0.32" stop-color="#fbbc05" />
        <stop offset="0.5" stop-color="#ea4335" />
        <stop offset="0.78" stop-color="#4285f4" />
      </linearGradient>
    </defs>
    <!-- Antigravity arch "A" -->
    <path
      d="M3 18.5 C6 18.5 7.6 15.8 9.2 12.6 C10.4 10.2 11 7.5 12 7.5 C13 7.5 13.6 10.2 14.8 12.6 C16.4 15.8 18 18.5 21 18.5"
      fill="none"
      stroke="url(#otto-pi-agy)"
      stroke-width="3.4"
      stroke-linecap="round"
    />
  </svg>
{:else}
  <!-- Generic monogram tile for shell + custom providers (grok, …): a
       deterministic per-slug hue so each custom provider is visually distinct
       and first-class, not a bare label. -->
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" role="img">
    <rect x="1" y="1" width="22" height="22" rx="5.5" fill="hsl({hue} 55% 42%)" />
    <text
      x="12"
      y="12"
      text-anchor="middle"
      dominant-baseline="central"
      font-size={mono.length > 1 ? 9 : 12}
      font-weight="700"
      fill="#fff"
      font-family="ui-sans-serif, system-ui, sans-serif"
    >{mono}</text>
  </svg>
{/if}
