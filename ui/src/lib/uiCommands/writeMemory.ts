// "Allow writes on ‹where› for this session" — the write-confirm memory behind
// an agent's UI-control writes (lib/uiCommands.ts `confirmWrite`, kept by
// stores/uiControl.svelte.ts). Pure and storage-injected so the rules are
// unit-tested (ui/unit/uiCommandsWriteMemory.test.ts):
//
//   • only an UNGUARDED, non-outward write with a scope (a connection id) is
//     rememberable — a guarded (production) connection or anything that leaves
//     the machine asks every time, and never offers the checkbox;
//   • memory is keyed (session, scope): a different connection still asks;
//   • it is dropped whenever the grant goes away — Stop / toggle off
//     (`setGrant(false)`), a revoke seen as `session_meta_updated`, and
//     `session_removed` — so a later re-grant asks again.

/** localStorage key (survives reloads; per device). */
export const WRITES_KEY = 'otto_ui_write_allow';

export interface WriteMemoryStorage {
  get(key: string): string | null;
  set(key: string, value: string): void;
}

/** The confirm's options that decide whether it may be remembered. */
export interface WriteScopeOpts {
  connId?: string;
  guarded?: boolean;
  outward?: boolean;
}

/** The scope a confirm may be remembered under, or null when it must always ask. */
export function writeScope(opts: WriteScopeOpts): string | null {
  const scope = opts.connId?.trim() || '';
  return scope && !opts.guarded && !opts.outward ? scope : null;
}

export class WriteMemory {
  private storage: WriteMemoryStorage;

  constructor(storage: WriteMemoryStorage) {
    this.storage = storage;
  }

  private all(): Record<string, string[]> {
    let v: unknown = null;
    try {
      v = JSON.parse(this.storage.get(WRITES_KEY) ?? 'null');
    } catch {
      v = null;
    }
    const out: Record<string, string[]> = {};
    if (!v || typeof v !== 'object' || Array.isArray(v)) return out;
    for (const [k, list] of Object.entries(v as Record<string, unknown>)) {
      if (Array.isArray(list)) out[k] = list.filter((x): x is string => typeof x === 'string');
    }
    return out;
  }

  /** The user ticked "Allow writes on ‹scope› for this session" before. */
  allowed(sessionId: string, scope: string): boolean {
    return (this.all()[sessionId] ?? []).includes(scope);
  }

  remember(sessionId: string, scope: string): void {
    const all = this.all();
    const list = all[sessionId] ?? [];
    if (list.includes(scope)) return;
    all[sessionId] = [...list, scope].slice(-100);
    this.storage.set(WRITES_KEY, JSON.stringify(all));
  }

  forget(sessionId: string): void {
    const all = this.all();
    if (!(sessionId in all)) return;
    delete all[sessionId];
    this.storage.set(WRITES_KEY, JSON.stringify(all));
  }
}
