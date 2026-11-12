<script lang="ts">
  // Muted badges on one drafted review comment, from GET /reviews/{id}/triage:
  // already on the PR (another review posted it — "Post all" skips it), its
  // finding dismissed (also skipped), or an unverified reviewer claim (skipped
  // unless the person includes it in the Post-all confirm).
  import type { CommentTriage, ReviewComment } from '../../lib/api/types';
  import { isUnverified } from './reviewTriage';

  let { comment, triage }: { comment: ReviewComment; triage: CommentTriage | undefined } = $props();
  const DISMISSED: Record<string, string> = { false_positive: 'Not a problem', waived: 'Won’t fix' };
  const dismissed = $derived(triage?.finding_status ? DISMISSED[triage.finding_status] : undefined);
</script>

{#if triage?.on_pr && !comment.posted}
  <span class="chip rct dim" data-testid="rp-already-on-pr"
    title="An earlier review of this PR already posted this comment — Post all skips it">Already on the PR</span>
{/if}
{#if dismissed && comment.state === 'draft'}
  <span class="chip rct dim" title="Its finding was dismissed on the Findings board — Post all skips it">{dismissed}</span>
{/if}
{#if isUnverified(comment, triage) && comment.state === 'draft'}
  <span class="chip rct rct-warn" title="The final verifier did not check this claim — Post all leaves it out unless you include it">Unverified</span>
{/if}

<style>
  .rct { font-size: var(--fs-xs); padding: 1px 6px; }
  .rct-warn { color: var(--warning); background: var(--warning-soft); }
  .dim { color: var(--text-dim); }
</style>
