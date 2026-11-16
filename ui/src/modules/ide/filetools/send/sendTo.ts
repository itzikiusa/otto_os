// IDE "Send to…" — hand the open file to another Otto surface.
//
// Every target only PREPARES: nothing is run, sent or executed on the user's
// behalf. The DB Explorer opens its Run on… sheet on the setup stage; the API
// client opens an unsaved request; a session gets the text PASTED (cleaned of
// control bytes, bracketed paste, no Enter — one line only for a connection
// session) after a confirm naming the session; a vault note is written
// only after a confirm (and a second one before overwriting).

import { router } from '../../../../lib/router.svelte';
import { whenMounted } from '../../../../lib/uiCommands';
import { ws as wsStore } from '../../../../lib/stores/workspace.svelte';
import { confirmer } from '../../../../lib/confirm.svelte';
import { toasts } from '../../../../lib/toast.svelte';
import { copyText } from '../../../../lib/clipboard';
import { ApiError } from '../../../../lib/api/client';
import { listVaults, vaultNote, writeVaultNote } from '../../../../lib/api/vault';
import type { Session } from '../../../../lib/api/types';
import { dbHandoff } from '../../../database/handoff.svelte';
import { loadErrorText } from '../../../../lib/loadError';
import { multiLineRefusal, pasteConfirmText, planPaste } from './pasteSafety';

export interface SendCtx {
  /** File name, e.g. `daily.sql`. */
  name: string;
  content: string;
  /** Language id or file extension (`sql`, `json`, `md`, …). */
  lang: string;
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
  // The stores load on demand: they are ~50 KB gzip the Workbench page would
  // otherwise ship to every visit (bundle budget).
  const { database } = await import('../../../../lib/stores/database.svelte');
  // An in-flight load is joined (loadConnections shares it), so a send made
  // while the list is still loading never reads as "no connections".
  if (!database.connections.length) await database.loadConnections();
  const conns = database.connections;
  // `loadConnections` records a failure in `connectionsError` rather than
  // throwing: a failed load must not read as "you have no connections".
  if (conns.length === 0 && database.connectionsError) {
    toasts.push('error', 'Couldn’t load your database connections', database.connectionsError, 8000, {
      action: { label: 'Try again', run: () => void sendToDatabase(ctx) },
    });
    return;
  }
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
  // Statement as-is: the sheet finds and fills its placeholders itself.
  dbHandoff.runOn({ statement: ctx.content, vars: {} });
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
    toasts.error('Couldn’t open the connection', loadErrorText(e));
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
  // `{{name}}` placeholders stay for the API client's environment variables.
  const text = ctx.content;
  try {
    const { apiClient } = await import('../../../../lib/stores/apiClient.svelte');
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
        name: ctx.name,
        method: 'POST',
        body_mode: json ? 'json' : 'raw',
        body: text,
      };
    }
    router.go('api');
    apiClient.showRequestView();
    toasts.success('Opened in the API client', 'An unsaved request — set the URL and press Send when ready.');
  } catch (e) {
    toasts.error('Couldn’t open the API client', loadErrorText(e));
  }
}

// ── (c) Terminal / agent session — paste, never run ────────────────────────

// The rules (control-byte cleaning, one line for connection sessions) live in
// pasteSafety.ts, which the picker and the unit tests load directly.

export async function pasteIntoSession(ctx: SendCtx, session: Session): Promise<boolean> {
  const plan = planPaste(ctx.content, session);
  if (!plan.ok) {
    if (plan.reason === 'empty') {
      toasts.error('Nothing to paste', 'After removing control characters the file has no text left.');
    } else {
      toasts.push('error', 'Not pasted — connection sessions take one line', multiLineRefusal(plan.lines), 12000, {
        action: { label: 'Copy', run: () => void copyText(ctx.content) },
      });
    }
    return false;
  }
  const ok = await confirmer.ask(pasteConfirmText(plan, ctx.name, session), {
    title: plan.multiLine ? 'Paste into session' : 'Type into session',
    confirmLabel: plan.multiLine ? 'Paste' : 'Type it',
  });
  if (!ok) return false;
  try {
    await wsStore.sendInput(session.id, plan.payload, false);
    toasts.push('success', plan.multiLine ? 'Pasted into the session' : 'Typed into the session', 'Review it there and press Enter to run.', 6000, {
      action: { label: 'Open session', run: () => wsStore.navigateToSession(session.id) },
    });
    return true;
  } catch (e) {
    toasts.error('Couldn’t paste into the session', loadErrorText(e));
    return false;
  }
}

// ── (d) Vault — save as note ───────────────────────────────────────────────

/** `scratches/<slug>.md` from the file name. */
export function defaultNotePath(name: string): string {
  const base = name.replace(/\.[A-Za-z0-9]+$/, '').trim();
  const slug = base
    .toLowerCase()
    .replace(/[^a-z0-9\-_ ]+/g, '')
    .trim()
    .replace(/\s+/g, '-');
  return `scratches/${slug || 'untitled'}.md`;
}

/** Markdown stays as-is; anything else becomes a fenced code block. */
export function noteBody(name: string, content: string, lang: string): string {
  if (lang === 'md' || lang === 'markdown') return content;
  const fence = content.includes('```') ? '````' : '```';
  const tag = lang && lang !== 'txt' && lang !== 'auto' ? lang : '';
  return `# ${name}\n\n${fence}${tag}\n${content.replace(/\n$/, '')}\n${fence}\n`;
}

export async function sendToVault(ctx: SendCtx): Promise<void> {
  const wsId = wsStore.currentId;
  if (!wsId) {
    toasts.error('No workspace open', 'Vault notes belong to a workspace — open one first.');
    return;
  }
  let vaults;
  try {
    vaults = await listVaults(wsId);
  } catch (e) {
    toasts.error('Couldn’t load vaults', loadErrorText(e));
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
    initial: defaultNotePath(ctx.name),
    placeholder: 'folder/note.md',
  });
  if (!raw) return;
  const path = /\.md$/i.test(raw) ? raw : `${raw}.md`;
  let ifHash: string | undefined;
  try {
    const existing = await vaultNote(wsId, vaultId, path);
    const ok = await confirmer.ask(`“${path}” already exists in ${vaultName}. Overwrite it? (The vault keeps its previous revision.)`, {
      title: 'Overwrite note?',
      confirmLabel: 'Overwrite',
    });
    if (!ok) return;
    ifHash = existing.meta.hash;
  } catch (e) {
    if (!(e instanceof ApiError && e.status === 404)) {
      toasts.error('Couldn’t check the note path', loadErrorText(e));
      return;
    }
  }
  try {
    await writeVaultNote(wsId, vaultId, {
      path,
      content: noteBody(ctx.name, ctx.content, ctx.lang),
      ...(ifHash ? { if_hash: ifHash } : {}),
    });
    toasts.push('success', 'Saved to the vault', `${vaultName} · ${path}`, 6000, {
      action: {
        label: 'Open note',
        run: async () => {
          router.go('vault');
          const { vault } = await import('../../../vault/vault.svelte');
          if (vault.vaults.length === 0 && !vault.loading) await vault.load();
          if (vault.current?.id !== vaultId) await vault.select(vaultId);
          await vault.open(path);
        },
      },
    });
  } catch (e) {
    toasts.error('Couldn’t save the note', loadErrorText(e));
  }
}
