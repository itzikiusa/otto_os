// The API client's bulk collection sync: import a file or a parsed document
// (collection tree or Postman environment), the one-shot Postman account sync,
// and git pull / push of collection files. Split out of apiClient.svelte.ts
// (arch-6-03); the store forwards every public name. Nothing here holds state:
// every write goes through the store's own save/load methods, so the tree and
// the publication-ownership rules stay in one place.

import { api } from '../../api/client';
import { toasts } from '../../toast.svelte';
import { loadErrorText } from '../../loadError';
import { plural } from '../../plural';
import { ws } from '../workspace.svelte';
import type { ApiCollection, ApiRequest, Id, UpsertApiRequestReq } from '../../api/types';
import {
  detectAndParse, collectionToPostman, isImportedEnvironment, environmentUpsertFor, type ImportedDoc, type ImportedEnvironment,
} from '../../api/importers';
import type { ApiClientStore } from '../apiClient.svelte';

/** The store members this part reads and calls (its whole contract with the façade). */
export type CollectionsSyncHost = Pick<
  ApiClientStore,
  | 'base'
  | 'bulkWrites'
  | 'collections'
  | 'loadCollections'
  | 'loadRequests'
  | 'saveCollection'
  | 'saveEnvironment'
  | 'workspaceOwner'
>;

export class ApiCollectionsSync {
  constructor(private readonly host: CollectionsSyncHost) {}

  /** File reads outlive the import sheet; keep their destination workspace. */
  async importFile(file: Pick<File, 'name' | 'text'>): Promise<void> {
    const current = this.host.workspaceOwner();
    try {
      const text = await file.text();
      if (current()) await this.importParsed(detectAndParse(text, file.name));
    } catch (error) {
      if (current()) toasts.error('Couldn’t import the file', loadErrorText(error));
    }
  }

  /** Create a collection (+ nested folders + requests) from an imported doc.
   *  A Postman ENVIRONMENT export routes to {@link importEnvironment} instead.
   *  `quiet` suppresses the per-item success toast (bulk account sync).
   *  `created`: the root collection (or environment) now exists — false when
   *  read-only or its save failed (already toasted). `failed`: how many of its
   *  folders/requests couldn't be created (summarised in one toast here). */
  async importParsed(parsed: ImportedDoc, quiet = false): Promise<{ created: boolean; failed: number }> {
    if (ws.myRole === 'viewer') {
      toasts.error('Read-only', 'You have viewer access to this workspace');
      return { created: false, failed: 0 };
    }
    if (isImportedEnvironment(parsed)) {
      return { created: await this.importEnvironment(parsed, quiet), failed: 0 };
    }
    const base = this.host.base();
    const root = await this.host.saveCollection({ name: parsed.name, parent_id: null });
    if (!root || !base) return { created: false, failed: 0 };
    // Items are created in order (the daemon assigns positions by count), but
    // in bulk: no per-item list reassignment / tree rebuild — one reload at
    // the end — and one summary toast instead of one per failure.
    const folderCache = new Map<string, Id>();
    let failed = 0;
    let firstError = '';
    const note = (e: unknown): void => { failed++; firstError ||= loadErrorText(e); };
    // Our own N creates echo back as N `api_client_changed` events: the one
    // reload below covers them (see noteClientChanged).
    this.host.bulkWrites++;
    try {
      for (const req of parsed.requests) {
        if (this.host.base() !== base) break; // workspace switched mid-import
        let parentId: Id = root.id;
        let pathKey = '';
        for (const folder of req.folderPath) {
          pathKey += '/' + folder;
          if (!folderCache.has(pathKey)) {
            try {
              folderCache.set(pathKey, (await api.post<ApiCollection>(`${base}/collections`, { name: folder, parent_id: parentId })).id);
            } catch (e) { note(e); }
          }
          parentId = folderCache.get(pathKey) ?? parentId;
        }
        try {
          await api.post<ApiRequest>(`${base}/requests`, {
            collection_id: parentId, name: req.name, method: req.method, url: req.url,
            headers: req.headers, query: req.query, body_mode: req.body_mode, body: req.body, auth: req.auth,
            extras: req.extras ?? null,
          } satisfies UpsertApiRequestReq);
        } catch (e) { note(e); }
      }
    } finally {
      await Promise.all([this.host.loadCollections(), this.host.loadRequests()]);
      this.host.bulkWrites--;
    }
    if (failed > 0) {
      toasts.error(`Couldn’t import ${plural(failed, 'item')} of “${parsed.name}”`, firstError);
    }
    if (!quiet) {
      toasts.success('Imported', `${parsed.name} · ${parsed.requests.length} request(s) (${parsed.format})`);
    }
    return { created: true, failed };
  }

  /** Import a Postman environment export as a new API environment. True when
   *  the environment was saved (a failure is already toasted). */
  async importEnvironment(env: ImportedEnvironment, quiet = false): Promise<boolean> {
    // Secret variables go to the Keychain (write-only `secret_values`), never
    // into the environment row.
    const saved = await this.host.saveEnvironment(environmentUpsertFor(env));
    if (saved && !quiet) {
      const secrets = env.secretKeys?.length ?? 0;
      toasts.success(
        'Environment imported',
        `${env.name} · ${Object.keys(env.variables).length} variable(s)${secrets ? `, ${secrets} kept in the Keychain` : ''}`,
      );
    }
    return saved != null;
  }

  /** One-shot Postman ACCOUNT sync: the daemon fetches every collection +
   *  environment via the Postman API (api.getpostman.com) and this imports
   *  them all through the normal import pipeline — no per-collection manual
   *  export. Returns true when the sync call itself succeeded. */
  async postmanSync(apiKey: string, remember: boolean): Promise<boolean> {
    const base = this.host.base();
    if (!base) return false;
    let res: {
      collections: unknown[];
      environments: { name?: string }[];
      failed: { name: string; error: string }[];
    };
    try {
      res = await api.post(`${base}/postman/sync`, {
        api_key: apiKey || null,
        remember,
      });
    } catch (e) {
      toasts.error('Couldn’t sync with Postman', loadErrorText(e));
      return false;
    }
    let cols = 0;
    let envs = 0;
    const failed = [...res.failed];
    for (const doc of res.collections) {
      const name =
        (doc as { info?: { name?: string } }).info?.name ?? 'collection';
      try {
        // Only a created root counts; a failed save already toasted its cause.
        if ((await this.importParsed(detectAndParse(JSON.stringify(doc), 'postman.json'), true)).created) cols++;
        else failed.push({ name, error: 'The collection couldn’t be saved.' });
      } catch (e) {
        failed.push({ name, error: loadErrorText(e) });
      }
    }
    for (const env of res.environments) {
      try {
        const { created } = await this.importParsed(
          detectAndParse(JSON.stringify(env), 'postman_environment.json'),
          true,
        );
        if (created) envs++;
        else failed.push({ name: env.name ?? 'environment', error: 'The environment couldn’t be saved.' });
      } catch (e) {
        failed.push({ name: env.name ?? 'environment', error: loadErrorText(e) });
      }
    }
    toasts.success(
      'Postman sync complete',
      `${cols} collection(s), ${envs} environment(s) imported`,
    );
    for (const f of failed.slice(0, 3)) {
      toasts.error(`Skipped: ${f.name}`, f.error);
    }
    if (failed.length > 3) {
      toasts.error('More items skipped', `${plural(failed.length - 3, 'further item')} failed`);
    }
    return true;
  }

  /** Pull Postman collection files from a connected git repo and import them. */
  async gitPullCollections(repoId: Id): Promise<void> {
    const res = await api.post<{ files: { name: string; content: string }[] }>(`/repos/${repoId}/api-collections/pull`, {});
    // Three different outcomes, reported separately: a file that isn't a
    // collection (parse error) is SKIPPED; one whose save failed (API error,
    // read-only — importParsed already toasted the cause) FAILED; only a
    // created root counts as imported.
    let imported = 0;
    let skipped = 0;
    let failed = 0;
    for (const f of res.files) {
      let doc: ImportedDoc;
      try {
        doc = detectAndParse(f.content, f.name);
      } catch {
        skipped++;
        continue;
      }
      try {
        if ((await this.importParsed(doc)).created) imported++;
        else failed++;
      } catch {
        failed++;
      }
    }
    const skippedNote = skipped > 0 ? `${plural(skipped, 'file')} skipped (not a collection)` : '';
    if (imported > 0) {
      toasts.success('Pulled from git', [plural(imported, 'collection file'), skippedNote].filter(Boolean).join(' · '));
    } else if (failed === 0) {
      toasts.info('Nothing imported from git', skippedNote || 'The repo has no collection files.');
    }
    if (failed > 0) {
      // Each file's cause was already toasted by importParsed / saveCollection.
      toasts.error(`Couldn’t import ${plural(failed, 'collection file')} from git`, imported === 0 ? skippedNote || undefined : undefined);
    }
  }

  /** Export all root collections to Postman files and push them to a git repo. */
  async gitPushCollections(repoId: Id, message: string, branch: string | null): Promise<boolean> {
    const roots = this.host.collections.filter((c) => (c.parent_id ?? null) === null);
    if (roots.length === 0) {
      toasts.error('Nothing to push', 'No collections to export.');
      return false;
    }
    try {
      // Export needs bodies/scripts: the one action that reads every full row.
      const full = await api.get<ApiRequest[]>(`${this.host.base()}/requests`);
      const files = roots.map((c) => ({
        name: `${c.name.replace(/[^\w.-]+/g, '_')}.postman_collection.json`,
        content: JSON.stringify(collectionToPostman(c.id, this.host.collections, full), null, 2),
      }));
      const res = await api.post<{ commit: string; files: number }>(`/repos/${repoId}/api-collections/push`, {
        files, message: message || 'Update API collections', branch: branch || null,
      });
      toasts.success('Pushed to git', `${res.files} file(s) · ${res.commit.slice(0, 8)}`);
      return true;
    } catch (e) {
      toasts.error('Couldn’t git push', loadErrorText(e));
      return false;
    }
  }
}
