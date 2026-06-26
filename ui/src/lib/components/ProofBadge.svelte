<script lang="ts">
  // A single proof badge chip: maps a snake_case badge string (see `ProofBadge`
  // in the API types) to a short label + a semantic color tone.
  interface Props {
    badge: string;
  }
  let { badge }: Props = $props();

  type Tone = 'ok' | 'bad' | 'accent' | 'warn' | 'dim' | 'neutral';

  const MAP: Record<string, { label: string; tone: Tone }> = {
    tests_passed: { label: 'Tests ✓', tone: 'ok' },
    tests_failed: { label: 'Tests ✗', tone: 'bad' },
    human_approved: { label: 'Approved', tone: 'ok' },
    db_api_verified: { label: 'DB/API ✓', tone: 'accent' },
    risky_change: { label: 'Risky', tone: 'warn' },
    ci_missing: { label: 'No CI', tone: 'warn' },
    ci_passed: { label: 'CI ✓', tone: 'ok' },
    ci_failed: { label: 'CI ✗', tone: 'bad' },
    ci_pending: { label: 'CI…', tone: 'warn' },
    ui_verified: { label: 'UI ✓', tone: 'accent' },
    pr_inconsistent: { label: 'PR ✗', tone: 'bad' },
    review_unresolved: { label: 'Review ✗', tone: 'bad' },
    no_proof: { label: 'No proof', tone: 'dim' },
    waived: { label: 'Waived', tone: 'neutral' },
  };

  const info = $derived(MAP[badge] ?? { label: badge, tone: 'dim' as Tone });
</script>

<span class="proof-badge {info.tone}" title={badge}>{info.label}</span>

<style>
  .proof-badge {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 7px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.01em;
    white-space: nowrap;
    border: 1px solid transparent;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .proof-badge.ok {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 14%, transparent);
    border-color: color-mix(in srgb, var(--status-working) 35%, transparent);
  }
  .proof-badge.bad {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
    border-color: color-mix(in srgb, var(--status-exited) 35%, transparent);
  }
  .proof-badge.accent {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .proof-badge.warn {
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 14%, transparent);
    border-color: color-mix(in srgb, var(--status-warn) 38%, transparent);
  }
  .proof-badge.dim {
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    border-color: color-mix(in srgb, var(--text-dim) 28%, transparent);
  }
  .proof-badge.neutral {
    color: var(--text-dim);
    background: var(--surface-2);
    border-color: var(--border);
  }
</style>
