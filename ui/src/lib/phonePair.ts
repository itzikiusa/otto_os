// Phone pairing — the pure parts (plain-Node importable for unit tests).
//
// A "Pair phone" QR encodes `<served url>/#/pair/<code>[/<next…>]`. The router
// captures the one-time code from the fragment, strips it from the visible URL
// and history (like a share token), and keeps it — plus the validated `next`
// route the phone lands on after approval — in tab-scoped sessionStorage so a
// reload of the pairing page still works. The code is single-use and expires
// in two minutes server-side; nothing here is a long-lived secret.

/** Session/thread ids the `next` allow-list accepts (ULIDs, uuids, slugs). */
const ID = /^[A-Za-z0-9_-]{1,64}$/;

/**
 * The in-app routes a pairing may land on — mirrors the daemon's allow-list
 * exactly: `''`, `home`, `agents`, `threads`, `agents/<id>`, `threads/<id>`.
 * Anything else (another module, a share route, an external URL, `..`) is
 * dropped to `null` so a crafted QR can't steer the new device anywhere else.
 */
export function validNext(raw: string | null | undefined): string | null {
  if (raw == null) return null;
  const r = raw.replace(/^#?\/?/, '');
  if (r === '' || r === 'home' || r === 'agents' || r === 'threads') return r;
  const m = /^(agents|threads)\/(.+)$/.exec(r);
  if (m && ID.test(m[2])) return r;
  return null;
}

/** A one-time pairing code: URL-safe, long (≥32 random bytes encoded). */
export function validPairCode(raw: string | null | undefined): boolean {
  return !!raw && /^[A-Za-z0-9_-]{32,128}$/.test(raw);
}

/** `#/pair/<code>[/<next…>]` route segments → the code and validated next. */
export function parsePairRoute(parts: readonly string[]): { code: string; next: string | null } | null {
  if (parts[0] !== 'pair' || parts.length < 2) return null;
  const code = parts[1];
  if (!validPairCode(code)) return null;
  const rest = parts.slice(2).join('/');
  return { code, next: rest ? validNext(rest) : null };
}

/** The pairing URL a QR encodes (what the daemon returns, rebuilt for links). */
export function pairUrl(servedUrl: string, code: string, next?: string | null): string {
  const base = servedUrl.replace(/\/+$/, '');
  const n = validNext(next ?? null);
  return `${base}/#/pair/${code}${n ? `/${n}` : ''}`;
}

/** The deep link to one session / thread on the served URL. */
export function continueUrl(servedUrl: string, route: string): string | null {
  const n = validNext(route);
  if (n == null) return null;
  return `${servedUrl.replace(/\/+$/, '')}/#/${n}`;
}

// ---- tab-scoped pairing state -------------------------------------------

export type PairStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

const KEY = 'otto_pair';

/** What the phone remembers while it pairs (tab-scoped). */
export interface PairState {
  code: string | null;
  next: string | null;
  /** Set once claimed: the handle the phone polls with. */
  pairingId?: string;
  pollSecret?: string;
  confirmCode?: string;
  expiresAt?: string;
}

function store(s?: PairStorage | null): PairStorage | null {
  if (s !== undefined) return s;
  try {
    return typeof sessionStorage === 'undefined' ? null : sessionStorage;
  } catch {
    return null;
  }
}

let memory: PairState | null = null;

export function savePairState(state: PairState | null, s?: PairStorage | null): void {
  memory = state;
  try {
    const st = store(s);
    if (!st) return;
    if (state) st.setItem(KEY, JSON.stringify(state));
    else st.removeItem(KEY);
  } catch {
    /* storage disabled: memory still carries this load */
  }
}

export function loadPairState(s?: PairStorage | null): PairState | null {
  if (memory) return memory;
  try {
    const raw = store(s)?.getItem(KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as Partial<PairState>;
    if (!v || typeof v !== 'object') return null;
    return {
      code: typeof v.code === 'string' && validPairCode(v.code) ? v.code : null,
      next: validNext(typeof v.next === 'string' ? v.next : null),
      pairingId: typeof v.pairingId === 'string' ? v.pairingId : undefined,
      pollSecret: typeof v.pollSecret === 'string' ? v.pollSecret : undefined,
      confirmCode: typeof v.confirmCode === 'string' ? v.confirmCode : undefined,
      expiresAt: typeof v.expiresAt === 'string' ? v.expiresAt : undefined,
    };
  } catch {
    return null;
  }
}

/** Test seam: forget the in-memory copy. */
export function resetPairMemory(): void {
  memory = null;
}

/** Seconds left until `expiresAt` (0 when past / unparsable). */
export function secondsLeft(expiresAt: string | null | undefined, nowMs: number): number {
  const t = expiresAt ? Date.parse(expiresAt) : NaN;
  if (!Number.isFinite(t)) return 0;
  return Math.max(0, Math.ceil((t - nowMs) / 1000));
}
