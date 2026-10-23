// The vault store lives in the Vault module (modules/vault/vault.svelte.ts),
// and lib/ never imports modules/. So the Vault page binds the store to this
// port while it is mounted (`$effect(() => registerVaultPort(vault))` in
// VaultPage.svelte), and the `otto.ui_vault_*` handlers (./vault.ts) navigate
// to the vault and then await the port — which also covers the Vault chunk
// still loading after the navigation. Structural types only: the store is the
// one implementation.

import type { Vault, VaultNote, VaultSearchHit } from '../api/types';
import { createPagePort } from './pagePort';

/** What the handlers drive — the slice of the vault store's public surface
 *  they read and call. */
export interface VaultPort {
  readonly wsId: string;
  readonly vaults: Vault[];
  readonly loading: boolean;
  readonly loadError: string | null;
  readonly current: Vault | null;
  readonly tabs: readonly { path: string }[];
  readonly notePath: string | null;
  readonly note: VaultNote | null;
  readonly editing: boolean;
  readonly dirty: boolean;
  readonly draft: string;
  readonly centerMode: string;
  leftMode: 'files' | 'search' | 'tags';
  searchQuery: string;
  readonly searchedQuery: string;
  readonly searchError: string | null;
  readonly searchHits: readonly VaultSearchHit[];
  /** True while an agent edit is staged in the editor and awaiting confirm. */
  holdAutosave: boolean;
  load(): Promise<void>;
  select(id: number): Promise<void>;
  open(path: string, opts?: { edit?: boolean }): Promise<void>;
  runSearch(): Promise<void>;
  createNote(path: string, content: string): Promise<boolean>;
  onDraftChange(text: string): void;
  saveNow(): Promise<boolean>;
}

export const vaultPort = createPagePort<VaultPort>('the Vault page');

/** Called by the Vault page while it is mounted; returns the unbind. */
export function registerVaultPort(store: VaultPort): () => void {
  return vaultPort.bind(store);
}
