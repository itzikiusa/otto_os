// Lazy `+adds −dels` for PR list rows. The list API carries no line counts,
// so each VISIBLE row asks the PR diff summary (the same memoized fetch the
// Code tab uses — Bitbucket answers it from the cheap diffstat). Results are
// keyed by the PR's head commit and kept in localStorage, so a PR is counted
// once per push, never again on the next visit. Two requests at a time, and
// rows scrolled past before their turn are skipped.
import { fileStat } from './diff-model';
import { SvelteMap } from 'svelte/reactivity';
import type { PrSummary } from '../../lib/api/types';
import { prDiffSummary } from './diff-load';

export interface PrStat {
  add: number;
  del: number;
  files: number;
}

const STORE_KEY = 'otto_pr_stats_v1';
const MAX_STORED = 600;
const CONCURRENCY = 2;

function readStore(): Record<string, PrStat> {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    const v = raw ? (JSON.parse(raw) as unknown) : null;
    return v && typeof v === 'object' ? (v as Record<string, PrStat>) : {};
  } catch {
    return {};
  }
}

function writeStore(k: string, s: PrStat): void {
  try {
    const all = readStore();
    delete all[k];
    all[k] = s;
    const keys = Object.keys(all);
    for (const old of keys.slice(0, Math.max(0, keys.length - MAX_STORED))) delete all[old];
    localStorage.setItem(STORE_KEY, JSON.stringify(all));
  } catch {
    /* storage blocked / full: counts just aren't remembered */
  }
}

export const statKey = (repoId: string, pr: Pick<PrSummary, 'number' | 'head_sha'>): string | null =>
  pr.head_sha ? `${repoId}#${pr.number}@${pr.head_sha}` : null;

export class PrStatsLoader {
  readonly stats = new SvelteMap<string, PrStat>();
  #repoId: string;
  #queue: PrSummary[] = [];
  #queued = new Set<string>();
  #visible = new Set<string>();
  /** Failed once this session: not retried on every scroll past. */
  #failed = new Set<string>();
  #running = 0;
  #ctl = new AbortController();
  #io: IntersectionObserver | null = null;
  #els = new WeakMap<Element, PrSummary>();

  constructor(repoId: string) {
    this.#repoId = repoId;
    const stored = readStore();
    const prefix = `${repoId}#`;
    for (const [k, v] of Object.entries(stored)) if (k.startsWith(prefix)) this.stats.set(k, v);
    if (typeof IntersectionObserver !== 'undefined') {
      this.#io = new IntersectionObserver((entries) => {
        for (const e of entries) {
          const pr = this.#els.get(e.target);
          const k = pr ? statKey(this.#repoId, pr) : null;
          if (!pr || !k) continue;
          if (e.isIntersecting) {
            this.#visible.add(k);
            this.want(pr);
          } else {
            this.#visible.delete(k);
          }
        }
      }, { rootMargin: '120px 0px' });
    }
  }

  get(pr: Pick<PrSummary, 'number' | 'head_sha'>): PrStat | null {
    const k = statKey(this.#repoId, pr);
    return k ? (this.stats.get(k) ?? null) : null;
  }

  /** Attachment for a row element: count it once it scrolls into view. */
  observe = (pr: PrSummary) => (el: Element) => {
    if (!this.#io) {
      this.want(pr);
      return;
    }
    this.#els.set(el, pr);
    this.#io.observe(el);
    return () => {
      this.#io?.unobserve(el);
      const k = statKey(this.#repoId, pr);
      if (k) this.#visible.delete(k);
    };
  };

  want(pr: PrSummary): void {
    const k = statKey(this.#repoId, pr);
    if (!k || this.stats.has(k) || this.#queued.has(k) || this.#failed.has(k) || this.#ctl.signal.aborted) return;
    this.#queued.add(k);
    this.#queue.push(pr);
    this.#pump();
  }

  #pump(): void {
    while (this.#running < CONCURRENCY && this.#queue.length > 0) {
      const pr = this.#queue.shift() as PrSummary;
      const k = statKey(this.#repoId, pr) as string;
      // Scrolled away before its turn: drop it (it re-queues when seen again).
      if (this.#io && !this.#visible.has(k)) {
        this.#queued.delete(k);
        continue;
      }
      this.#running++;
      void prDiffSummary(this.#repoId, pr.number, this.#ctl.signal, pr.head_sha)
        .then((d) => {
          let add = d.total_added ?? 0;
          let del = d.total_deleted ?? 0;
          if (d.total_added == null || d.total_deleted == null) {
            add = 0;
            del = 0;
            for (const f of d.files) {
              const st = fileStat(f);
              add += st.add;
              del += st.del;
            }
          }
          const s: PrStat = { add, del, files: d.files.length };
          this.stats.set(k, s);
          writeStore(k, s);
        })
        .catch(() => {
          // Best-effort: the row simply shows no counts (an unmount abort isn't a failure).
          if (!this.#ctl.signal.aborted) this.#failed.add(k);
        })
        .finally(() => {
          this.#running--;
          this.#queued.delete(k);
          if (!this.#ctl.signal.aborted) this.#pump();
        });
    }
  }

  dispose(): void {
    this.#ctl.abort();
    this.#io?.disconnect();
    this.#queue = [];
  }
}
