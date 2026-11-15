// Pure Web Storage draft & undo-journal map helpers for IDE buffers (no CodeMirror imports).

export const IDE_UNDO_JOURNAL_KEY = 'otto.ide.undoJournal.v1';
export const IDE_UNDO_DRAFT_PREFIX = 'otto.ide.undoDraft:';
export const IDE_UNDO_DRAFT_INDEX_KEY = 'otto.ide.undoDraftIndex.v1';
export const IDE_CURSOR_POS_KEY = 'otto.ide.cursorPos.v1';

export const MAX_JOURNAL_FILES = 24;
export const MAX_DRAFT_FILES = 24;
export const MAX_JOURNAL_ENTRY_CHARS = 160_000;

const SECRET_EXACT_NAMES = new Set([
  'id_rsa',
  'id_dsa',
  'id_ecdsa',
  'id_ed25519',
  'credentials',
  'credentials.toml',
  'auth.json',
  'oauth_creds.json',
  '.env',
  '.netrc',
  '.pgpass',
  '.npmrc',
  '.pypirc',
  '.dockercfg',
  '.git-credentials',
  '.claude.json',
  '.bash_history',
  '.zsh_history',
  '.sh_history',
  '.python_history',
  '.node_repl_history',
  '.psql_history',
  '.mysql_history',
  '.rediscli_history',
  'fish_history',
]);

const SECRET_FILE_SUFFIXES = ['.pem', '.key', '.pfx', '.p12', '.keystore', '.jks', '.credentials'];

const ANYWHERE_CREDENTIAL_DIRS = new Set([
  '.ssh',
  '.aws',
  '.gnupg',
  '.kube',
  '.docker',
  '.password-store',
  '.terraform.d',
]);

const HOME_REL_DENY_PREFIXES = [
  '.ssh/',
  '.aws/',
  '.gnupg/',
  '.kube/',
  '.docker/',
  '.config/gcloud/',
  '.config/gh/',
  '.config/op/',
  '.config/git/credentials',
  '.config/',
  '.azure/',
  '.password-store/',
  '.terraform.d/',
  '.codex/',
  '.gemini/',
  '.claude/',
  '.cursor/',
  '.cargo/credentials',
  'library/keychains/',
  'library/cookies/',
  'library/logs/otto/',
  'library/application support/otto/',
];

const SYSTEM_DENY_PREFIXES = [
  '/etc/',
  '/private/etc/',
  '/root/',
  '/var/root/',
  '/private/var/root/',
  '/proc/',
  '/sys/',
];

function extractHomeRelativeSubpath(lowerClean: string): string | null {
  const folded = lowerClean.startsWith('/system/volumes/data/')
    ? lowerClean.slice('/system/volumes/data'.length)
    : lowerClean;
  if (folded.startsWith('~/')) return folded.slice(2);
  const m = /^(?:\/users\/[^/]+|\/home\/[^/]+|\/var\/root|\/root)\/(.+)$/.exec(folded);
  return m ? m[1] : null;
}

/** Returns true when `filePath` names a credential, private key, shell history, or protected directory file that must never be cached in Web Storage or routed to LSP. */
export function isSecretIdePath(filePath?: string | null): boolean {
  if (!filePath) return false;
  const clean = filePath.split('?')[0].replace(/\\/g, '/').replace(/\/+$/, '');
  if (!clean) return false;
  const lowerClean = clean.toLowerCase();
  const base = lowerClean.slice(lowerClean.lastIndexOf('/') + 1);
  if (!base) return false;

  if (SECRET_EXACT_NAMES.has(base)) return true;
  if (base.startsWith('.env.')) return true;
  if (
    base.startsWith('id_rsa.') ||
    base.startsWith('id_dsa.') ||
    base.startsWith('id_ed25519.') ||
    base.startsWith('id_ecdsa.') ||
    base.startsWith('credentials.')
  ) {
    return true;
  }
  for (let i = 0; i < SECRET_FILE_SUFFIXES.length; i++) {
    if (base.endsWith(SECRET_FILE_SUFFIXES[i])) return true;
  }

  const folded = lowerClean.startsWith('/system/volumes/data/')
    ? lowerClean.slice('/system/volumes/data'.length)
    : lowerClean;
  for (let i = 0; i < SYSTEM_DENY_PREFIXES.length; i++) {
    const sysPrefix = SYSTEM_DENY_PREFIXES[i];
    if (folded.startsWith(sysPrefix) || `${folded}/` === sysPrefix) return true;
  }

  const segments = folded.split('/');
  for (let i = 0; i < segments.length - 1; i++) {
    if (ANYWHERE_CREDENTIAL_DIRS.has(segments[i])) return true;
  }
  if (
    folded.includes('/.config/gcloud/') ||
    folded.includes('/.config/gh/') ||
    folded.includes('/.config/op/') ||
    folded.includes('/.cargo/credentials') ||
    folded.includes('/library/keychains/')
  ) {
    return true;
  }

  const homeRel = extractHomeRelativeSubpath(lowerClean);
  if (homeRel) {
    if (homeRel.startsWith('.otto/') && !homeRel.startsWith('.otto/vault/')) {
      return true;
    }
    for (let i = 0; i < HOME_REL_DENY_PREFIXES.length; i++) {
      const p = HOME_REL_DENY_PREFIXES[i];
      if (homeRel === p || homeRel.startsWith(p)) return true;
    }
  }
  return false;
}

const SENSITIVE_EXACT_KEYS = [
  IDE_UNDO_JOURNAL_KEY,
  IDE_UNDO_DRAFT_INDEX_KEY,
  'otto.ide.clipboardHistory.v1',
  'otto.ide.projects.v1',
  'otto.ide.namedProjects.v1',
  'otto.ide.lastRoot',
  'otto.ide.activeProjectId.v1',
  'otto.ide.openThreadHandoff',
  IDE_CURSOR_POS_KEY,
];

const SENSITIVE_KEY_PREFIXES = [
  IDE_UNDO_DRAFT_PREFIX,
  'otto.ide.roots:',
  'otto.ide.bps:',
  'otto.ide.projectTabs:',
  'otto.ide.openTabs.v1:',
  'otto.ide.buildToolPrefs:',
];

export function clearIdeSensitiveWebStorage(
  storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem' | 'key' | 'length'> | null,
  sessionStore?: Pick<Storage, 'removeItem'> | null,
): void {
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  if (store) {
    removeIdeUndoDraft('all', store);
    for (let i = 0; i < SENSITIVE_EXACT_KEYS.length; i++) {
      try {
        store.removeItem?.(SENSITIVE_EXACT_KEYS[i]);
      } catch { /* ignore */ }
    }
    try {
      if (typeof store.length === 'number' && typeof store.key === 'function') {
        const toRemove: string[] = [];
        for (let i = 0; i < store.length; i++) {
          const k = store.key(i);
          if (!k) continue;
          for (let j = 0; j < SENSITIVE_KEY_PREFIXES.length; j++) {
            if (k.startsWith(SENSITIVE_KEY_PREFIXES[j])) {
              toRemove.push(k);
              break;
            }
          }
        }
        for (let i = 0; i < toRemove.length; i++) {
          store.removeItem(toRemove[i]);
        }
      }
    } catch { /* ignore */ }
  }
  const ss = sessionStore ?? (typeof sessionStorage !== 'undefined' ? sessionStorage : null);
  if (ss) {
    try {
      ss.removeItem(IDE_CURSOR_POS_KEY);
    } catch { /* ignore */ }
  }
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
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? (parsed as UndoJournalMap) : {};
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
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? (parsed as UndoDraftIndexMap) : {};
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
      const parsed = JSON.parse(raw) as unknown;
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        const rec = parsed as Partial<IdeUndoDraftRecord>;
        if (typeof rec.savedContent === 'string' && typeof rec.draftContent === 'string') {
          return {
            savedContent: rec.savedContent,
            draftContent: rec.draftContent,
            updatedAt: Number(rec.updatedAt ?? 0),
          };
        }
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

const liveLspDiagKeys = new Set<string>();

function resolveDiagAbsPath(rootPath: string | null | undefined, filePath: string): string {
  const cleanFile = filePath.split('?')[0].replace(/\\/g, '/').replace(/\/+$/, '');
  const cleanRoot = (rootPath ?? '').replace(/\\/g, '/').replace(/\/+$/, '');
  if (!cleanRoot || cleanFile.startsWith('/') || cleanFile.includes('://')) return cleanFile;
  return `${cleanRoot}/${cleanFile.replace(/^\/+/, '')}`;
}

function normalizeDiagPathKey(rootPath: string | null | undefined, filePath: string): string {
  const cleanRoot = (rootPath ?? '').replace(/\\/g, '/').replace(/\/+$/, '');
  const absFile = resolveDiagAbsPath(rootPath, filePath);
  return cleanRoot ? `${cleanRoot}::${absFile}` : absFile;
}

/** Record that a live pooled/direct LSP session has published authoritative diagnostics for `(rootPath, filePath)`. */
export function markLiveLspDiagnostics(rootPath: string | null | undefined, filePath: string): void {
  if (!filePath) return;
  const absFile = resolveDiagAbsPath(rootPath, filePath);
  liveLspDiagKeys.add(normalizeDiagPathKey(rootPath, filePath));
  liveLspDiagKeys.add(absFile);
}

/** Clear live LSP diagnostic ownership for a specific file, a root, or all sessions. */
export function clearLiveLspDiagnostics(rootPath?: string | null, filePath?: string | null): void {
  if (!rootPath && !filePath) {
    liveLspDiagKeys.clear();
    return;
  }
  if (filePath) {
    const absFile = resolveDiagAbsPath(rootPath, filePath);
    liveLspDiagKeys.delete(normalizeDiagPathKey(rootPath, filePath));
    liveLspDiagKeys.delete(absFile);
    return;
  }
  const cleanRoot = (rootPath ?? '').replace(/\\/g, '/').replace(/\/+$/, '');
  const prefix = `${cleanRoot}::`;
  const pathPrefix = `${cleanRoot}/`;
  for (const k of [...liveLspDiagKeys]) {
    if (k.startsWith(prefix) || (cleanRoot && (k === cleanRoot || k.startsWith(pathPrefix)))) {
      liveLspDiagKeys.delete(k);
    }
  }
}

/** True when an active LSP client already owns diagnostics for `(rootPath, filePath)`. */
export function hasLiveLspDiagnosticsForFile(rootPath: string | null | undefined, filePath: string): boolean {
  if (!filePath) return false;
  const absFile = resolveDiagAbsPath(rootPath, filePath);
  return (
    liveLspDiagKeys.has(normalizeDiagPathKey(rootPath, filePath)) ||
    liveLspDiagKeys.has(absFile)
  );
}
