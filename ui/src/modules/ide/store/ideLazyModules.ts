// The IDE store's on-demand controllers: each module loads with its first use
// (one promise each), so the IDE page does not pay for them up front. Split out
// of ideStore.svelte.ts (file-size limit); no behaviour change.
import type * as RepoActionsMod from './ideRepoActions';
import type * as ProjMod from '../shell/ideProjectsManager';
import type * as LspSearchMod from '../shell/ideLspAndSearchHelpers';
import type * as CollabMod from './ideCollabController';
import type * as DebugEnvMod from './ideDebugAndEnvController';

let projModP: Promise<typeof ProjMod> | null = null;
let lspModP: Promise<typeof LspSearchMod> | null = null;
let collabModP: Promise<typeof CollabMod> | null = null;
let debugEnvModP: Promise<typeof DebugEnvMod> | null = null;
let repoActionsModP: Promise<typeof RepoActionsMod> | null = null;
let debugEnvSyncMod: typeof DebugEnvMod | null = null;
export const loadProjMod = () => (projModP ??= import('../shell/ideProjectsManager'));
export const loadLspMod = () => (lspModP ??= import('../shell/ideLspAndSearchHelpers'));
export const loadCollabMod = () => (collabModP ??= import('./ideCollabController'));
export const loadDebugEnvMod = () => (debugEnvModP ??= import('./ideDebugAndEnvController').then((m) => (debugEnvSyncMod = m)));
export const loadRepoActionsMod = () => (repoActionsModP ??= import('./ideRepoActions'));
/** The LSP / search module if something already asked for it (else nothing to cancel). */
export const lspRequested = (): Promise<typeof LspSearchMod> | null => lspModP;
/** The debug / environment controller if it has loaded (paths that must not wait for it). */
export const debugEnvLoaded = (): typeof DebugEnvMod | null => debugEnvSyncMod;
