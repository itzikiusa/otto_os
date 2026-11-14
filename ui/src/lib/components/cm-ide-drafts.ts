// Pure Web Storage draft & undo-journal map helpers for IDE buffers (no CodeMirror imports).

export const IDE_UNDO_JOURNAL_KEY = 'otto.ide.undoJournal.v1';
export const IDE_UNDO_DRAFT_PREFIX = 'otto.ide.undoDraft:';
export const IDE_UNDO_DRAFT_INDEX_KEY = 'otto.ide.undoDraftIndex.v1';

export const MAX_JOURNAL_FILES = 24;
export const MAX_DRAFT_FILES = 24;
export const MAX_JOURNAL_ENTRY_CHARS = 160_000;

/** Returns true when `filePath` names a credential, private key, or dotenv file that must never be written to Web Storage. */
export function isSecretIdePath(filePath?: string | null): boolean {
  if (!filePath) return false;
  const clean = filePath.split('?')[0].replace(/\\/g, '/').replace(/\/+$/, '');
  const base = (clean.slice(clean.lastIndexOf('/') + 1) || '').toLowerCase();
  if (!base) return false;
  if (base === '.env' || base.startsWith('.env.')) return true;
  if (base.endsWith('.pem') || base.endsWith('.key') || base.endsWith('.p12') || base.endsWith('.pfx')) return true;
  if (
    base === 'id_rsa' ||
    base.startsWith('id_rsa.') ||
    base === 'id_ed25519' ||
    base.startsWith('id_ed25519.') ||
    base === 'id_ecdsa' ||
    base.startsWith('id_ecdsa.')
  ) {
    return true;
  }
  if (base === 'credentials' || base.startsWith('credentials.') || base.endsWith('.credentials')) return true;
  return false;
}

export interface IdeUndoJournalEntry {
  stateJson: { doc: string; [k: string]: unknown };
  baseSavedContent: string | null;
  savedContent?: string;
  draftContent?: string;
  unsaved: boolean;
  scrollTop: number;
  updatedAt: number;
}

export interface IdeUndoDraftRecord {
  savedContent: string;
  draftContent: string;
  updatedAt: number;
}

export type UndoJournalMap = Record<string, IdeUndoJournalEntry>;
type UndoDraftIndexMap = Record<string, number>;

export function readJournalMap(storage?: Pick<Storage, 'getItem'> | null): UndoJournalMap {
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  if (!store) return {};
  try {
    const raw = store.getItem(IDE_UNDO_JOURNAL_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === 'object' ? (parsed as UndoJournalMap) : {};
  } catch {
    return {};
  }
}

export function writeJournalMap(map: UndoJournalMap, storage?: Pick<Storage, 'setItem'> | null): void {
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  if (!store) return;
  const entries = Object.entries(map).sort((a, b) => (b[1]?.updatedAt ?? 0) - (a[1]?.updatedAt ?? 0)).slice(0, MAX_JOURNAL_FILES);
  const trimmed: UndoJournalMap = {};
  for (const [k, v] of entries) trimmed[k] = v;
  try {
    store.setItem(IDE_UNDO_JOURNAL_KEY, JSON.stringify(trimmed));
  } catch {
    const compact: UndoJournalMap = {};
    for (const [k, v] of entries.slice(0, 8)) compact[k] = v;
    try { store.setItem(IDE_UNDO_JOURNAL_KEY, JSON.stringify(compact)); } catch { /* ignore */ }
  }
}

function readDraftIndex(storage?: Pick<Storage, 'getItem'> | null): UndoDraftIndexMap {
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  if (!store) return {};
  try {
    const raw = store.getItem(IDE_UNDO_DRAFT_INDEX_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : null;
    return parsed && typeof parsed === 'object' ? (parsed as UndoDraftIndexMap) : {};
  } catch {
    return {};
  }
}

export function writeIdeUndoDraftImmediate(
  filePath: string,
  savedContent: string,
  draftContent: string,
  storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null,
): void {
  if (!filePath || isSecretIdePath(filePath)) return;
  if (savedContent.length > MAX_JOURNAL_ENTRY_CHARS || draftContent.length > MAX_JOURNAL_ENTRY_CHARS) return;
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  if (!store) return;
  if (savedContent === draftContent) {
    removeIdeUndoDraft(filePath, store);
    return;
  }
  try {
    const now = Date.now();
    const rec: IdeUndoDraftRecord = { savedContent, draftContent, updatedAt: now };
    store.setItem(`${IDE_UNDO_DRAFT_PREFIX}${filePath}`, JSON.stringify(rec));
    const idx = readDraftIndex(store);
    idx[filePath] = now;
    const sorted = Object.entries(idx).sort((a, b) => b[1] - a[1]);
    const nextIdx: UndoDraftIndexMap = {};
    for (let i = 0; i < sorted.length; i++) {
      const [p, ts] = sorted[i];
      if (i < MAX_DRAFT_FILES) {
        nextIdx[p] = ts;
      } else if (typeof store.removeItem === 'function') {
        store.removeItem(`${IDE_UNDO_DRAFT_PREFIX}${p}`);
      }
    }
    store.setItem(IDE_UNDO_DRAFT_INDEX_KEY, JSON.stringify(nextIdx));
  } catch { /* ignore quota */ }
}

export function readIdeUndoDraft(
  filePath: string,
  storage?: Pick<Storage, 'getItem'> | null,
): IdeUndoDraftRecord | null {
  if (!filePath || isSecretIdePath(filePath)) return null;
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  if (!store) return null;
  try {
    const raw = store.getItem(`${IDE_UNDO_DRAFT_PREFIX}${filePath}`);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<IdeUndoDraftRecord>;
      if (typeof parsed?.savedContent === 'string' && typeof parsed?.draftContent === 'string') {
        return {
          savedContent: parsed.savedContent,
          draftContent: parsed.draftContent,
          updatedAt: Number(parsed.updatedAt ?? 0),
        };
      }
    }
  } catch { /* fall back to journal entry */ }
  const j = readJournalMap(store)[filePath];
  if (j && j.unsaved && typeof j.draftContent === 'string') {
    const saved = j.savedContent ?? j.baseSavedContent;
    if (typeof saved === 'string') {
      return { savedContent: saved, draftContent: j.draftContent, updatedAt: j.updatedAt };
    }
  }
  return null;
}

export function removeIdeUndoDraft(
  filePath: string | 'all',
  storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null,
): void {
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  if (!store) return;
  try {
    const idx = readDraftIndex(store);
    if (filePath === 'all') {
      for (const p of Object.keys(idx)) {
        store.removeItem?.(`${IDE_UNDO_DRAFT_PREFIX}${p}`);
      }
      store.removeItem?.(IDE_UNDO_DRAFT_INDEX_KEY);
      return;
    }
    store.removeItem?.(`${IDE_UNDO_DRAFT_PREFIX}${filePath}`);
    if (filePath in idx) {
      delete idx[filePath];
      store.setItem(IDE_UNDO_DRAFT_INDEX_KEY, JSON.stringify(idx));
    }
  } catch { /* ignore */ }
}
