<script lang="ts" module>
  // Providers with a real brand mark below. Everything else (shell, custom
  // slugs…) keeps the plain text label — callers gate on this.
  const KNOWN = new Set(['claude', 'codex', 'agy']);
  export function hasProviderIcon(provider: string | null | undefined): boolean {
    return !!provider && KNOWN.has(provider.toLowerCase());
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
{/if}
