// Workbench "Send to…" — hand a scratch file to another Otto surface.
//
// Every target only PREPARES: nothing is run, sent or executed on the user's
// behalf. The DB Explorer opens its Run on… sheet on the setup stage; the API
// client opens an unsaved request; a session gets the text PASTED (bracketed
// paste, no Enter) after a confirm naming the session; a vault note is written
// only after a confirm (and a second one before overwriting).

import { router } from '../../../lib/router.svelte';
import { whenMounted } from '../../../lib/uiCommands';
import { database } from '../../../lib/stores/database.svelte';
import { apiClient } from '../../../lib/stores/apiClient.svelte';
import { ws as wsStore } from '../../../lib/stores/workspace.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { copyText } from '../../../lib/clipboard';
import { ApiError } from '../../../lib/api/client';
import { listVaults, vaultNote, writeVaultNote } from '../../../lib/api/vault';
import type { Session, WorkbenchDocFull } from '../../../lib/api/types';
import { dbHandoff } from '../../database/handoff.svelte';
import { fillPlaceholders } from '../lib/placeholders';

export interface SendCtx {
  ws: string;
  doc: WorkbenchDocFull;
  content: string;
  lang: string;
  values: Record<string, string>;
}

/** Content with every placeholder that HAS a value filled in (others kept). */
export function filled(ctx: SendCtx): string {
  return fillPlaceholders(ctx.content, ctx.values, ctx.lang);
}

/** Only the non-empty values (an empty input is "not set"). */
export function setValues(values: Record<string, string>): Record<string, string> {
  return Object.fromEntries(Object.entries(values).filter(([, v]) => v.trim() !== ''));
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

// ── (a) Database — Run on… ─────────────────────────────────────────────────

/** Most connections offered in the picker before falling back to "open one on
 *  the Explorer" (the sheet itself adds further targets). */
const MAX_DB_CHOICES = 6;

export async function sendToDatabase(ctx: SendCtx): Promise<void> {
  if (!ctx.content.trim()) {
    toasts.error('Nothing to run', 'This file is empty.');
    return;
  }
  try {
    if (!database.connections.length && !database.connectionsLoading) await database.loadConnections();
  } catch (e) {
    toasts.error('Couldn’t load connections', errText(e));
    return;
  }
  const conns = database.connections;
  if (conns.length === 0) {
    toasts.push('error', 'No database connections', 'Add one in Connections, then send the script again.', 6000, {
      action: { label: 'Open Connections', run: () => router.go('connections') },
    });
    return;
  }
  let connId: string | null = database.selectedConnId && conns.some((c) => c.id === database.selectedConnId)
    ? database.selectedConnId
    : null;
  if (!connId && conns.length === 1) connId = conns[0]!.id;
  if (!connId && conns.length <= MAX_DB_CHOICES) {
    const picked = await confirmer.choose(
      'Run on… starts from one connection; add more targets (and databases) in the sheet. Nothing runs until you preview and confirm.',
      {
        title: 'Open the script on which connection?',
        options: conns.map((c) => ({ label: c.name, value: c.id, kind: 'normal' as const })),
      },
    );
    if (!picked.value) return;
    connId = picked.value;
  }
  // Statement as-is: the sheet substitutes the placeholders itself (one run per
  // value), so the values travel as parameters, not baked into the text.
  dbHandoff.runOn({ statement: ctx.content, vars: setValues(ctx.values) });
  router.go('database');
  if (!connId) {
    toasts.info('Open a database connection', 'The Run on… sheet opens with your script as soon as one is connected.');
    return;
  }
  try {
    // Same order as the agent `db_*` commands: let the Explorer mount (and
    // restore its open set) first, then focus our connection on the Query view.
    await whenMounted('.db-root', undefined, 15_000).catch(() => null);
    await database.openConnection(connId);
    database.setMainTab('query');
  } catch (e) {
    toasts.error('Couldn’t open the connection', errText(e));
  }
}

// ── (b) API client — new request ───────────────────────────────────────────

const CURL_RE = /^\s*curl\s/;

export function looksLikeCurl(text: string): boolean {
  return CURL_RE.test(text);
}

function looksLikeJson(text: string): boolean {
  const t = text.trim();
  if (!/^[[{]/.test(t)) return false;
  try {
    JSON.parse(t);
    return true;
  } catch {
    return false;
  }
}

export async function sendToApiClient(ctx: SendCtx): Promise<void> {
  // Only set values are filled: unset `{{name}}` stays for the API client's
  // own environment variables to resolve.
  const text = filled(ctx);
  try {
    await apiClient.ensureLoaded();
    if (looksLikeCurl(text)) {
      if (!(await apiClient.importCurl(text))) {
        toasts.error('Couldn’t import the curl command', 'Check its syntax, or send it as a request body instead.');
        return;
      }
    } else {
      apiClient.newDraft();
      const json = ctx.lang === 'json' || looksLikeJson(text);
      apiClient.draft = {
        ...apiClient.draft,
        name: ctx.doc.name,
        method: 'POST',
        body_mode: json ? 'json' : 'raw',
        body: text,
      };
    }
    router.go('api');
    apiClient.showRequestView();
    toasts.success('Opened in the API client', 'An unsaved request — set the URL and press Send when ready.');
  } catch (e) {
    toasts.error('Couldn’t open the API client', errText(e));
  }
}

// ── (c) Terminal / agent session — paste, never run ────────────────────────

/** Sessions the text can be pasted into: this workspace's, not archived, still
 *  alive. */
export function pasteTargets(sessions: Session[], ws: string): Session[] {
  return sessions.filter((s) => s.workspace_id === ws && !s.archived && s.status !== 'exited');
}

/** Bracketed paste (`ESC[200~ … ESC[201~`) without a trailing newline: shells
 *  and the agent TUIs insert it as typed text and do NOT execute it. */
export function bracketedPaste(text: string): string {
  return `\u001b[200~${text.replace(/\r?\n$/, '')}\u001b[201~`;
}

export async function pasteIntoSession(ctx: SendCtx, session: Session): Promise<boolean> {
  const text = filled(ctx);
  const lines = text.split('\n').length;
  const ok = await confirmer.ask(
    `Paste ${lines} line${lines === 1 ? '' : 's'} from “${ctx.doc.name}” into “${session.title || session.id}”? ` +
      'It is pasted only — nothing runs until you press Enter in that session.',
    { title: 'Paste into session', confirmLabel: 'Paste' },
  );
  if (!ok) return false;
  try {
    await wsStore.sendInput(session.id, bracketedPaste(text), false);
    toasts.push('success', 'Pasted into the session', 'Review it there and press Enter to run.', 6000, {
      action: { label: 'Open session', run: () => wsStore.navigateToSession(session.id) },
    });
    return true;
  } catch (e) {
    toasts.error('Couldn’t paste into the session', errText(e));
    return false;
  }
}

// ── (d) Vault — save as note ───────────────────────────────────────────────

/** `workbench/<slug>.md` from the file name. */
export function defaultNotePath(name: string): string {
  const base = name.replace(/\.[A-Za-z0-9]+$/, '').trim();
  const slug = base
    .toLowerCase()
    .replace(/[^a-z0-9\-_ ]+/g, '')
    .trim()
    .replace(/\s+/g, '-');
  return `workbench/${slug || 'untitled'}.md`;
}

/** Markdown stays as-is; anything else becomes a fenced code block. */
export function noteBody(name: string, content: string, lang: string): string {
  if (lang === 'md' || lang === 'markdown') return content;
  const fence = content.includes('```') ? '````' : '```';
  const tag = lang && lang !== 'txt' && lang !== 'auto' ? lang : '';
  return `# ${name}\n\n${fence}${tag}\n${content.replace(/\n$/, '')}\n${fence}\n`;
}

export async function sendToVault(ctx: SendCtx): Promise<void> {
  let vaults;
  try {
    vaults = await listVaults(ctx.ws);
  } catch (e) {
    toasts.error('Couldn’t load vaults', errText(e));
    return;
  }
  if (vaults.length === 0) {
    toasts.push('error', 'No vault yet', 'Create a vault first, then save the file as a note.', 6000, {
      action: { label: 'Open Vault', run: () => router.go('vault') },
    });
    return;
  }
  let vaultId = vaults[0]!.id;
  if (vaults.length > 1) {
    const picked = await confirmer.choose('Which vault should the note go to?', {
      title: 'Save as note',
      options: vaults.slice(0, 8).map((v) => ({ label: v.name, value: String(v.id), kind: 'normal' as const })),
    });
    if (!picked.value) return;
    vaultId = Number(picked.value);
  }
  const vaultName = vaults.find((v) => v.id === vaultId)?.name ?? 'the vault';
  const raw = await confirmer.promptText(`Note path inside “${vaultName}”.`, {
    title: 'Save as note',
    confirmLabel: 'Save note',
    initial: defaultNotePath(ctx.doc.name),
    placeholder: 'folder/note.md',
  });
  if (!raw) return;
  const path = /\.md$/i.test(raw) ? raw : `${raw}.md`;
  let ifHash: string | undefined;
  try {
    const existing = await vaultNote(ctx.ws, vaultId, path);
    const ok = await confirmer.ask(`“${path}” already exists in ${vaultName}. Overwrite it? (The vault keeps its previous revision.)`, {
      title: 'Overwrite note?',
      confirmLabel: 'Overwrite',
    });
    if (!ok) return;
    ifHash = existing.meta.hash;
  } catch (e) {
    if (!(e instanceof ApiError && e.status === 404)) {
      toasts.error('Couldn’t check the note path', errText(e));
      return;
    }
  }
  try {
    await writeVaultNote(ctx.ws, vaultId, {
      path,
      content: noteBody(ctx.doc.name, filled(ctx), ctx.lang),
      ...(ifHash ? { if_hash: ifHash } : {}),
    });
    toasts.push('success', 'Saved to the vault', `${vaultName} · ${path}`, 6000, {
      action: {
        label: 'Open note',
        run: async () => {
          router.go('vault');
          const { vault } = await import('../../vault/vault.svelte');
          if (vault.vaults.length === 0 && !vault.loading) await vault.load();
          if (vault.current?.id !== vaultId) await vault.select(vaultId);
          await vault.open(path);
        },
      },
    });
  } catch (e) {
    toasts.error('Couldn’t save the note', errText(e));
  }
}

// ── (e) Copy with placeholders filled ──────────────────────────────────────

export async function copyFilled(ctx: SendCtx): Promise<void> {
  if (await copyText(filled(ctx))) toasts.success('Copied', 'Placeholders with a value were filled in.');
  else toasts.error('Couldn’t copy to the clipboard', 'The clipboard is not available here.');
}
