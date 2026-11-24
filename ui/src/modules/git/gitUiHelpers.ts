// Pure UI helpers shared by the Git module views (PR header compaction,
// provider-aware close wording, review config normalization, composer
// shortcuts/outward notices, RTL sidebar drag, tab close focus, accounts fetch).
// Kept DOM- and Svelte-free so node --test exercises every branch directly.
import type { RepoReviewConfigResp, ReviewAgentCfg, ReviewConfig } from '../../lib/api/types';
import { advancedOf } from './reviewConfigMerge';

export type PrTabId = 'summary' | 'timeline' | 'code' | 'review';

/** Code and AI review tabs need vertical space for diffs/findings, so the PR
 *  header collapses to a compact single-row bar on those tabs. */
export function isCompactPrHeaderTab(tab: PrTabId): boolean {
  return tab === 'code' || tab === 'review';
}

export interface PrCloseLabels {
  /** Action button in Summary footer ("Close PR…" vs "Decline…"). */
  button: string;
  /** In-flight button text ("Closing…" vs "Declining…"). */
  busy: string;
  /** Confirm modal verb ("Close PR" vs "Decline PR"). */
  verb: string;
  /** Confirm modal title builder. */
  title: (number: number) => string;
  /** Overflow menu entry ("Close pull request…" vs "Decline pull request…"). */
  menu: string;
  /** Past-tense verb for success toast ("closed" vs "declined"). */
  toastDone: string;
  /** Failure toast headline. */
  toastFail: string;
}

/** Provider-aware wording for closing a PR without merging: Bitbucket calls
 *  it "Decline", while GitHub and GitLab call it "Close". */
export function prCloseLabels(provider: string | null | undefined): PrCloseLabels {
  if (provider === 'bitbucket') {
    return {
      button: 'Decline…',
      busy: 'Declining…',
      verb: 'Decline PR',
      title: (n) => `Decline PR #${n}?`,
      menu: 'Decline pull request…',
      toastDone: 'declined',
      toastFail: 'Couldn’t decline the PR',
    };
  }
  return {
    button: 'Close PR…',
    busy: 'Closing…',
    verb: 'Close PR',
    title: (n) => `Close pull request #${n}?`,
    menu: 'Close pull request…',
    toastDone: 'closed',
    toastFail: 'Couldn’t close the PR',
  };
}

/** Short badge text for an active per-repository review config override. */
export function repoReviewConfigBadge(
  rc: Pick<RepoReviewConfigResp, 'scope' | 'preset_name'> | null,
): string {
  if (rc === null || rc.scope === 'global') return '';
  return rc.scope === 'preset' ? (rc.preset_name ?? 'preset') : 'repo config';
}

/** Ensure a review agent carries a non-empty `providers` array (migrating old
 *  single-provider configs). */
export function normalizeReviewAgent(
  a: ReviewAgentCfg,
  fallbackProvider: string,
): ReviewAgentCfg {
  if (!a.providers || a.providers.length === 0) {
    return { ...a, providers: [a.provider || fallbackProvider] };
  }
  return a;
}

/** Toggle `target` in/out of an agent's providers list, never leaving it empty. */
export function toggleAgentProvider(
  providers: readonly string[] | undefined,
  target: string,
  fallbackProvider: string,
): string[] {
  const current = providers && providers.length > 0 ? providers : [fallbackProvider];
  if (current.includes(target)) {
    const next = current.filter((p) => p !== target);
    return next.length === 0 ? [fallbackProvider] : next;
  }
  return [...current, target];
}

/** Compare two review configs ignoring global per-agent presets (preset drift check). */
export function sameReviewConfig(
  a: ReviewConfig,
  b: ReviewConfig,
  fallbackProvider: string,
): boolean {
  const strip = (c: ReviewConfig): unknown => ({
    agents: c.agents.map((x) => ({
      ...normalizeReviewAgent(x, fallbackProvider),
      provider: undefined,
    })),
    summarizer: { ...c.summarizer },
    ...advancedOf(c),
  });
  return JSON.stringify(strip(a)) === JSON.stringify(strip(b));
}

/** On a completed review, keep the Jira/focus inputs collapsed behind the
 *  "Re-run review…" toggle until the user opens them. */
export function shouldCollapseReviewInputs(
  status: string | undefined,
  rerunOpen: boolean,
): boolean {
  return status === 'done' && !rerunOpen;
}

/** True on ⌘/Ctrl+Enter outside IME composition (standard composer submit). */
export function isComposerSubmitKey(e: {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  isComposing?: boolean;
}): boolean {
  return e.key === 'Enter' && (e.metaKey || e.ctrlKey) && !e.isComposing;
}

/** Compact outward destination/visibility notice for PR comment composers. */
export function prCommentPostHint(
  target: string | null | undefined,
  mode: 'post' | 'reply' | 'edit' = 'post',
): string {
  const dest = target && target.trim() !== '' ? target.trim() : 'this PR';
  if (mode === 'edit') {
    return `Updates your comment on ${dest} · visible to the author and reviewers`;
  }
  if (mode === 'reply') {
    return `Replies on ${dest} · visible to the author and reviewers`;
  }
  return `Posts to ${dest} · visible to the author and reviewers`;
}

/** Compute clamped sidebar width during a horizontal drag, flipping the delta
 *  when the layout direction is RTL (inline-start sits on the right). */
export function computeSidebarResizeWidth(
  startW: number,
  startX: number,
  clientX: number,
  rtl: boolean,
  min = 180,
  max = 520,
): number {
  const delta = rtl ? startX - clientX : clientX - startX;
  return Math.max(min, Math.min(max, Math.round(startW + delta)));
}

/** Pick the adjacent PR tab to receive focus after `closed` is removed, or
 *  `null` when no tabs remain. */
export function nextTabAfterClose(open: readonly number[], closed: number): number | null {
  const idx = open.indexOf(closed);
  if (idx === -1) return open[0] ?? null;
  const remaining = open.filter((n) => n !== closed);
  if (remaining.length === 0) return null;
  return remaining[Math.min(idx, remaining.length - 1)] ?? null;
}

/** Fetch git accounts once on mount and whenever the Add Repository modal opens
 *  — never on modal close. */
export function shouldFetchGitAccounts(addOpen: boolean, initialized: boolean): boolean {
  return !initialized || addOpen;
}

export interface PickRect {
  top: number;
  bottom: number;
  left: number;
}

/** Place the floating "Explain lines" button under the selection when it fits,
 *  else above it, clamping both axes inside the viewport with `gap` padding. */
export function clampLinePickPos(
  rect: PickRect,
  size: { width: number; height: number },
  viewport: { width: number; height: number },
  gap = 8,
): { top: number; left: number } {
  const w = Math.max(0, size.width);
  const h = Math.max(0, size.height);
  let top = rect.bottom + gap;
  if (top + h > viewport.height - gap) top = rect.top - gap - h;
  return {
    top: Math.max(gap, Math.min(viewport.height - h - gap, top)),
    left: Math.max(gap, Math.min(viewport.width - w - gap, rect.left)),
  };
}

/** Host of a remote URL for the unsupported-forge message — handles
 *  https://, ssh:// and scp-like (git@host:path) forms (mirrors the daemon's
 *  detect.rs split). */
export function remoteHost(url: string | null): string {
  if (!url) return 'this remote';
  const u = url.trim();
  for (const scheme of ['https://', 'http://', 'ssh://', 'git://']) {
    if (u.startsWith(scheme)) {
      const rest = u.slice(scheme.length).replace(/^[^@/]+@/, '');
      return rest.split('/')[0]?.split(':')[0] || 'this remote';
    }
  }
  const scp = u.match(/^(?:[^@/]+@)?([^:/]+):/);
  return scp?.[1] ?? 'this remote';
}
