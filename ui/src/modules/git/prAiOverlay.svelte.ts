// Reactive controller for E-08 (Inline AI Review Overlay in PrDetail's Code tab).
// Loads active review draft comments + persisted findings for one PR, refreshes
// on WS reviewBus ticks, and wires Edit / Post to PR… / Decline-Dismiss / Ask
// Agent to Fix without inflating PrDetail.svelte.
import { untrack } from 'svelte';
import {
  api,
  ApiError,
  falsePositiveFinding,
  fixFinding,
  listFindings,
  waiveFinding,
} from '../../lib/api/client';
import type { Finding, Review, ReviewComment } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { confirmOutward } from '../../lib/confirmOutward';
import { reviewBus } from '../../lib/events.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { prAgentBridge } from './pr-agent-bridge.svelte';
import { askDismissal } from './reviewTriageApi';
import type { AiOverlayItem, DiffAiOverlayData } from './diffAiOverlay';

export interface PrAiOverlayHost {
  repoId: () => string;
  prNumber: () => number;
  where: () => string;
  onPosted?: () => void;
}

const PR_WHO = 'The PR author and reviewers see it, posted under your git account.';

export function createPrAiOverlay(host: PrAiOverlayHost): { readonly data: DiffAiOverlayData; refresh: () => void } {
  let reviewId = $state<string | null>(null);
  let comments = $state.raw<ReviewComment[]>([]);
  let findings = $state.raw<Finding[]>([]);
  let seq = 0;

  async function load(rid: string, num: number): Promise<void> {
    const mySeq = ++seq;
    try {
      const r = await api.get<Review>(`/repos/${rid}/prs/${num}/review`);
      if (mySeq !== seq) return;
      reviewId = r.id;
      comments = r.comments ?? [];
      const fs = await listFindings(r.id).catch(() => [] as Finding[]);
      if (mySeq !== seq) return;
      findings = fs;
    } catch (e) {
      if (mySeq !== seq) return;
      if (e instanceof ApiError && e.status === 404) {
        reviewId = null;
        comments = [];
        findings = [];
      }
    }
  }

  $effect(() => {
    const rid = host.repoId();
    const num = host.prNumber();
    untrack(() => {
      reviewId = null;
      comments = [];
      findings = [];
      void load(rid, num);
    });
  });

  $effect(() => {
    const _tick = reviewBus.tick;
    const evId = reviewBus.reviewId;
    if (!evId) return;
    const cur = untrack(() => reviewId);
    if (cur && cur !== evId) return;
    void load(untrack(host.repoId), untrack(host.prNumber));
  });

  async function onEditComment(c: ReviewComment, body: string): Promise<boolean> {
    try {
      const updated = await api.patch<ReviewComment>(`/pr-review-comments/${c.id}`, { body });
      comments = comments.map((x) => (x.id === updated.id ? updated : x));
      toasts.success('Draft comment updated');
      return true;
    } catch (e) {
      toastError('Couldn’t save the edit', e);
      return false;
    }
  }

  async function onPostComment(c: ReviewComment): Promise<boolean> {
    const loc = c.path !== null ? `${c.path}${c.line !== null ? `:${c.line}` : ''} — ` : '';
    const where = host.where();
    const ok = await confirmOutward({
      verb: 'Post to PR',
      title: `Post comment to PR #${host.prNumber()}?`,
      where,
      what: `${loc}${c.body}`,
      who: PR_WHO,
    });
    if (!ok) return false;
    try {
      const updated = await api.post<ReviewComment>(`/pr-review-comments/${c.id}/approve`);
      comments = comments.map((x) => (x.id === updated.id ? updated : x));
      if (!updated.posted) {
        toasts.error(
          'Couldn’t post the comment',
          `${where} refused it — it’s approved in Otto but not on the PR. Check the repository’s git account.`,
        );
        return false;
      }
      toasts.success('Comment posted', where);
      host.onPosted?.();
      return true;
    } catch (e) {
      toastError('Couldn’t post the comment', e);
      return false;
    }
  }

  async function onDismissItem(item: AiOverlayItem): Promise<boolean> {
    if (item.kind === 'comment') {
      const req = await askDismissal(item.comment);
      if (!req) return false;
      try {
        const updated = await api.post<ReviewComment>(`/pr-review-comments/${item.comment.id}/decline`, req);
        comments = comments.map((x) => (x.id === updated.id ? updated : x));
        if (reviewId) {
          listFindings(reviewId)
            .then((fs) => { findings = fs; })
            .catch(() => {});
        }
        toasts.info(req.dismiss === 'waived' ? 'Declined — won’t fix' : 'Declined — not a problem');
        return true;
      } catch (e) {
        toastError('Couldn’t decline comment', e);
        return false;
      }
    }

    const picked = await confirmer.choose(
      `Dismiss “${item.title || item.body.split('\n')[0]}”? It stops counting toward merge readiness.`,
      {
        title: 'Dismiss finding',
        options: [
          { label: 'Not a problem', value: 'false_positive', kind: 'primary' },
          { label: 'Won’t fix', value: 'waived' },
        ],
      },
    );
    if (picked.value !== 'false_positive' && picked.value !== 'waived') return false;
    try {
      const updated = picked.value === 'waived'
        ? await waiveFinding(item.finding.id)
        : await falsePositiveFinding(item.finding.id);
      findings = findings.map((f) => (f.id === updated.id ? updated : f));
      toasts.info(picked.value === 'waived' ? 'Dismissed — won’t fix' : 'Dismissed — not a problem');
      return true;
    } catch (e) {
      toastError('Couldn’t dismiss finding', e);
      return false;
    }
  }

  async function onFixItem(item: AiOverlayItem): Promise<boolean> {
    if (item.kind === 'finding' && (item.finding.status === 'open' || item.finding.status === 'accepted')) {
      try {
        const resp = await fixFinding(item.finding.id);
        findings = findings.map((f) => (f.id === resp.finding.id ? resp.finding : f));
        toasts.success(
          'Fix started',
          resp.session_id ? 'Agent session started — watch it in Agents.' : undefined,
        );
        return true;
      } catch (e) {
        toastError('Couldn’t start fix', e);
        return false;
      }
    }
    prAgentBridge.ask(host.repoId(), host.prNumber(), { kind: 'fix' });
    return true;
  }

  const data: DiffAiOverlayData = $derived({
    comments,
    findings,
    onEditComment,
    onPostComment,
    onDismissItem,
    onFixItem,
  });

  return {
    get data() {
      return data;
    },
    refresh() {
      void load(host.repoId(), host.prNumber());
    },
  };
}
