// Pure pieces of the debugger panel: the failure card's next step (I2-10), the
// re-attach rule (I2-14), the thread picker's text (I2-05) and the toolbar
// tooltips built from the shared shortcut table so they follow the keymap
// preset (I2-06).
//
// Import-free (type-only imports), so Node runs it directly in unit tests.

import type { IdeChord } from '../shell/ideShortcuts';

export type DebugErrorKind = 'adapter_missing' | 'toolchain_missing' | 'build_failed';

export interface DebugErrorSource {
  message: string;
  kind: DebugErrorKind | string | null | undefined;
  ecosystem: string | null | undefined;
}

export type DebugErrorActionId = 'install' | 'sdks' | 'build-output' | 'retry';

export interface DebugErrorCard {
  title: string;
  message: string;
  /** The one primary next step. */
  primary: { id: DebugErrorActionId; label: string } | null;
  /** Retry the launch, when a retry can work without changing anything first. */
  canRetry: boolean;
  /** The toolchain ecosystem the install action targets. */
  ecosystem: string | null;
}

const ADAPTERS: Record<string, string> = {
  go: 'Delve',
  python: 'debugpy',
  rust: 'CodeLLDB',
  java: 'Java debugger',
  node: 'JS debugger',
  shell: 'Bash debugger',
};

/** "Delve" for go, … — the name a person knows the debugger by. */
export function adapterName(ecosystem: string | null | undefined): string {
  return (ecosystem && ADAPTERS[ecosystem]) || 'the debugger';
}

const PREFIX = /^(not found|invalid|upstream|conflict|internal|forbidden):\s*/i;

/** The failure card for a refused start (`startError`) or a session that
 *  ended in error (`error` + `error_kind`). Null when there is nothing to say. */
export function debugErrorCard(
  startError: DebugErrorSource | null | undefined,
  session: { state: string; language?: string; error?: string | null; error_kind?: string | null; error_ecosystem?: string | null } | null | undefined,
): DebugErrorCard | null {
  const src: DebugErrorSource | null = startError
    ? startError
    : session?.error
      ? { message: session.error, kind: session.error_kind ?? null, ecosystem: session.error_ecosystem ?? null }
      : null;
  if (!src) return null;
  const message = (src.message || 'The debugger couldn’t start.').replace(PREFIX, '');
  const inferred = src.kind ? null : inferErrorKind(src.message || '', session?.language);
  const kind = src.kind ?? inferred?.kind ?? null;
  const ecosystem = src.ecosystem ?? inferred?.ecosystem ?? null;
  switch (kind) {
    case 'adapter_missing':
      return {
        title: `${adapterName(ecosystem)} isn’t installed`,
        message,
        primary: ecosystem && ADAPTERS[ecosystem] ? { id: 'install', label: `Install ${adapterName(ecosystem)}` } : { id: 'sdks', label: 'Open SDKs…' },
        canRetry: false,
        ecosystem,
      };
    case 'toolchain_missing':
      return { title: 'No SDK for this project', message, primary: { id: 'sdks', label: 'Open SDKs…' }, canRetry: false, ecosystem };
    case 'build_failed':
      return { title: 'The build failed', message, primary: { id: 'build-output', label: 'Show build output' }, canRetry: true, ecosystem };
    default:
      return { title: 'The debugger couldn’t start', message, primary: null, canRetry: true, ecosystem };
  }
}

const LANG_ECOSYSTEM: Record<string, string> = {
  go: 'go', python: 'python', rust: 'rust', java: 'java', kotlin: 'java',
  javascript: 'node', typescript: 'node', node: 'node', shell: 'shell', bash: 'shell',
};

/** A daemon that predates `error_kind` only sends text ("not found: Go debugger
 *  'dlv' (Delve) not found; install it …"): recognise the two common cases so
 *  the card still offers the next step. */
export function inferErrorKind(
  message: string,
  language: string | null | undefined,
): { kind: DebugErrorKind; ecosystem: string | null } | null {
  const eco = language ? (LANG_ECOSYSTEM[language.toLowerCase()] ?? null) : null;
  if (/(debugger|adapter|delve|debugpy|lldb)[^.;]*\b(not found|not installed|missing)\b/i.test(message)) {
    return { kind: 'adapter_missing', ecosystem: eco };
  }
  if (/\b(build|compil\w*)\b[^.;]*\bfailed\b|\bfailed to (build|compile)\b/i.test(message)) {
    return { kind: 'build_failed', ecosystem: eco };
  }
  return null;
}

/** Re-attach on mount only to a live session launched from THIS project's
 *  root. A daemon that does not send `target_root` yet: any live session (the
 *  old behaviour), since there is nothing to compare. */
export function pickReattachSession<S extends { state: string; target_root?: string | null }>(
  sessions: readonly S[],
  root: string,
): S | null {
  const norm = (p: string): string => p.replace(/\/+$/, '');
  for (const s of sessions) {
    if (s.state === 'terminated' || s.state === 'error') continue;
    if (typeof s.target_root === 'string' && s.target_root && norm(s.target_root) !== norm(root)) continue;
    return s;
  }
  return null;
}

/** Thread picker text: "#1 MainThread · paused: breakpoint" — the stop reason
 *  is known for the active thread only (the snapshot carries one reason). */
export function threadOptionLabel(
  th: { id: number; name: string; stopped_reason?: string | null },
  active: { id: number | null | undefined; reason: string | null | undefined },
): string {
  const reason = th.stopped_reason ?? (th.id === active.id ? active.reason : null);
  return reason ? `#${th.id} ${th.name} · ${reason}` : `#${th.id} ${th.name}`;
}

const ARIA_KEY: Record<string, string> = { ArrowUp: 'ArrowUp', ArrowDown: 'ArrowDown', ArrowLeft: 'ArrowLeft', ArrowRight: 'ArrowRight' };

/** `aria-keyshortcuts` value for a chord (`Meta+Alt+R`). */
export function chordAria(c: IdeChord): string {
  const mods = [c.ctrl ? 'Control' : '', c.alt ? 'Alt' : '', c.shift ? 'Shift' : '', c.mod || c.meta ? 'Meta' : ''].filter(Boolean);
  const key = ARIA_KEY[c.key] ?? (c.key.length === 1 ? c.key.toUpperCase() : c.key);
  return [...mods, key].join('+');
}

/** A toolbar tooltip: "Step over (F8)", or the bare title when the preset has no key. */
export function withKeyHint(title: string, labels: readonly string[]): string {
  return labels.length > 0 ? `${title} (${labels.join(' / ')})` : title;
}
