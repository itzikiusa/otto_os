// Agent UI control — Vault (docs home) handlers (`otto.ui_vault_*`). They
// drive the vault store the page renders (tabs, the open note, the search
// panel), so the note the agent reads is the one open in the user's pane.
//
// Writing is `local_write` and stays the user's call: the new text lands in
// the note's EDITOR first (edit mode, highlighted) and is only saved after the
// attributed confirm — a decline puts the original text back, so an agent
// edit is never persisted behind the user's back (guidelines: agent output is
// a draft until a person applies it).

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { vault } from '../../modules/vault/vault.svelte';
import { vaultNote } from '../api/vault';
import { ApiError } from '../api/client';
import { toasts } from '../toast.svelte';
import type { Vault } from '../api/types';
import { agentLabel, asUiError, highlightWhenReady, resolveByIdOrName, tailText, waitFor } from './pagePort';

async function showVault(key: string | number | undefined, ctx: UiCommandCtx): Promise<Vault> {
  if (router.parts[0] !== 'vault') router.go('vault');
  if (vault.vaults.length === 0 && !vault.loading) await vault.load();
  await waitFor(() => !vault.loading, ctx.signal, 15_000, 'the vault list');
  if (vault.loadError) throw new UiCommandError('failed', vault.loadError);
  let v: Vault | null = vault.current;
  if (key !== undefined && key !== null && key !== '') {
    v = resolveByIdOrName(vault.vaults, String(key), (x) => String(x.id), (x) => x.name, 'vault');
  }
  if (!v) throw new UiCommandError('not_found', 'There is no vault yet — the user can add one on the Vault page.');
  if (vault.current?.id !== v.id) await vault.select(v.id);
  if (vault.current?.id !== v.id) throw new UiCommandError('failed', 'The vault did not switch (an unsaved note may be blocking it).');
  return v;
}

function normPath(p: string): string {
  const s = String(p ?? '').trim().replace(/^\/+/, '');
  if (!s || s.split('/').some((seg) => seg === '..')) throw new UiCommandError('invalid_args', `Bad note path “${p}”.`);
  return s;
}

registerUiCommands('vault', {
  async vault_list(_args, ctx) {
    const v = await showVault(undefined, ctx).catch((e) => {
      if (e instanceof UiCommandError && e.code === 'not_found') return null;
      throw e;
    });
    return {
      vaults: vault.vaults.map((x) => ({ id: x.id, name: x.name, root_path: x.root_path, okf: x.okf, notes: x.notes })),
      current: v ? { id: v.id, name: v.name } : null,
      open_tabs: vault.tabs.map((t) => t.path),
      open_note: vault.notePath,
    };
  },

  async vault_open_note(args: { vault?: string; path: string; edit?: boolean }, ctx) {
    const v = await showVault(args.vault, ctx);
    const path = normPath(args.path);
    await vault.open(path, { edit: args.edit });
    if (vault.notePath !== path || !vault.note) {
      throw new UiCommandError('not_found', `Couldn't open “${path}” in ${v.name}.`);
    }
    void highlightWhenReady(ctx, '.note-view');
    const t = tailText(vault.draft, 60_000);
    return {
      vault: v.name,
      path,
      title: vault.note.meta.title ?? null,
      editing: vault.editing,
      unsaved_changes: vault.dirty,
      content: t.text,
      truncated: t.truncated,
      outgoing_links: vault.note.outgoing.slice(0, 100),
    };
  },

  async vault_search(args: { vault?: string; query: string }, ctx) {
    const v = await showVault(args.vault, ctx);
    vault.leftMode = 'search';
    vault.searchQuery = args.query;
    await vault.runSearch();
    if (vault.searchError) throw new UiCommandError('failed', vault.searchError);
    void highlightWhenReady(ctx, '.search');
    return {
      vault: v.name,
      query: args.query,
      hits: vault.searchHits.map((h) => ({ path: h.path, title: h.title, snippet: h.snippet })),
    };
  },

  async vault_write_note(args: { vault?: string; path: string; content: string }, ctx) {
    const v = await showVault(args.vault, ctx);
    const path = normPath(args.path);
    if (!/\.(md|markdown)$/i.test(path)) throw new UiCommandError('invalid_args', 'Only markdown notes (`.md`) can be written.');
    let exists = true;
    try {
      await vaultNote(vault.wsId, v.id, path);
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) exists = false;
      else throw asUiError(e);
    }
    const who = agentLabel(ctx.agent);
    const where = `${path} in vault “${v.name}”`;

    if (!exists) {
      const ok = await ctx.confirmWrite({ what: `Create note (${args.content.length} chars)`, where, connId: `vault:${v.id}`, verb: 'Create' });
      if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined creating the note.');
      if (!(await vault.createNote(path, args.content))) throw new UiCommandError('failed', `Couldn't create ${path}.`);
      toasts.success(`Created ${path}`, who);
      void highlightWhenReady(ctx, '.note-view');
      return { vault: v.name, path, created: true };
    }

    // Existing note: stage the new text in the editor so the user reviews it
    // in place, then save on confirm (or restore on decline).
    await vault.open(path, { edit: true });
    if (vault.notePath !== path || !vault.note) throw new UiCommandError('failed', `Couldn't open ${path}.`);
    if (vault.dirty) {
      throw new UiCommandError('failed', `${path} has unsaved edits by the user — not overwriting them.`);
    }
    const original = vault.note.raw;
    if (original === args.content) return { vault: v.name, path, changed: false };
    // Staged with autosave held, so nothing reaches disk before the confirm.
    vault.holdAutosave = true;
    let ok = false;
    try {
      vault.onDraftChange(args.content);
      void highlightWhenReady(ctx, '.note-view');
      ok = await ctx.confirmWrite({
        what: `Replace the note's text (${original.length} → ${args.content.length} chars; shown in the editor)`,
        where,
        connId: `vault:${v.id}`,
        verb: 'Save',
      });
      if (vault.notePath !== path) throw new UiCommandError('failed', 'The user moved to another note before confirming.');
      if (!ok) {
        vault.onDraftChange(original);
        throw new UiCommandError('cancelled_by_user', 'The user declined the edit.');
      }
    } finally {
      vault.holdAutosave = false;
    }
    if (!(await vault.saveNow())) throw new UiCommandError('failed', `Couldn't save ${path} (see the page).`);
    toasts.success(`Saved ${path}`, who);
    return { vault: v.name, path, changed: true };
  },
});

// `otto.ui_state` view: the vault, the open note and whether it's edited.
registerUiState('vault', () => ({
  vault: vault.current ? { id: vault.current.id, name: vault.current.name } : null,
  view: vault.centerMode,
  open_note: vault.notePath,
  editing: vault.editing,
  unsaved_changes: vault.dirty,
  tabs: vault.tabs.map((t) => t.path),
  left_panel: vault.leftMode,
  search: vault.leftMode === 'search' ? vault.searchedQuery || null : null,
}));
