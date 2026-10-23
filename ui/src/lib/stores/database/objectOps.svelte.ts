// Database Explorer store — object operations from the schema tree: open an
// object in the Structure view, copy its CREATE statement, the table / Redis /
// collection actions of the tree's context menu (select, find, send to the
// editor, import into…), and the guarded truncate / drop. The DatabaseStore
// façade owns one instance as `objects` and delegates its public methods here.

import { api } from '../../api/client';
import type { ObjectDetail, SchemaNode } from '../../api/types';
import { toasts } from '../../toast.svelte';
import { tableRefFromNodeId } from '../../database/sql-dialect';
import { copyTextOrThrow } from '../../clipboard';
import { loadErrorText } from '../../loadError';
import { toastError } from '../../toastError';
import type { DatabaseStore } from '../database.svelte';

/** The DatabaseStore members this part reads and calls (its whole contract with the façade). */
export type ObjectOpsHost = Pick<
  DatabaseStore,
  | 'connBase'
  | 'epochs'
  | 'importDialogOpen'
  | 'importTable'
  | 'newTab'
  | 'objectDetail'
  | 'objectError'
  | 'objectLoading'
  | 'runs'
  | 'selectedConn'
  | 'selectedConnId'
  | 'selectedObjectPath'
  | 'setActiveDb'
  | 'setMainTab'
  | 'tab'
>;

export class DbObjectOpsStore {
  private readonly db: ObjectOpsHost;
  constructor(db: ObjectOpsHost) {
    this.db = db;
  }

  /** Open an object (table/view/collection/key) → detail + Structure tab. */
  async openObject(node: SchemaNode): Promise<void> {
    const id = this.db.selectedConnId;
    if (!id) return;
    const epoch = this.db.epochs.of(id);
    const seq = ++this.objectOpenSeq;
    this.db.selectedObjectPath = node.id;
    this.db.objectLoading = true;
    this.db.objectDetail = null;
    this.db.objectError = null;
    this.db.setMainTab('structure');
    // Stale when the connection was closed/switched, or another object was
    // opened while this fetch was in flight — the singletons moved on.
    const stale = (): boolean =>
      !this.db.epochs.live(id, epoch) ||
      this.db.selectedConnId !== id ||
      this.db.selectedObjectPath !== node.id;
    try {
      const detail = await api.post<ObjectDetail>(`${this.db.connBase(id)}/object`, {
        path: node.id,
      });
      if (stale()) return;
      this.db.objectDetail = detail;
    } catch (e) {
      if (stale()) return;
      // Keep the failure visible in the Structure view (the toast fades and the
      // neutral "no object" empty state would misreport a load error).
      this.db.objectError = loadErrorText(e);
      toastError('Couldn’t load object', e);
    } finally {
      // A NEWER openObject owns the loading flag; anything else (including a
      // stale completion) must release it so no spinner runs forever.
      if (seq === this.objectOpenSeq) this.db.objectLoading = false;
    }
  }

  /** Monotonic openObject sequence — the latest call owns `objectLoading`. */
  private objectOpenSeq = 0;

  /** Fetch object detail for an arbitrary table path (used by the builder). */
  async fetchObject(path: string): Promise<ObjectDetail | null> {
    const id = this.db.selectedConnId;
    if (!id) return null;
    try {
      return await api.post<ObjectDetail>(`${this.db.connBase(id)}/object`, { path });
    } catch (e) {
      toastError('Couldn’t load object', e);
      return null;
    }
  }

  /** Copy a table/view's CREATE statement (the DDL the server already derives via
   *  `SHOW CREATE` for MySQL + ClickHouse) to the clipboard. */
  async copyCreateStatement(node: SchemaNode): Promise<void> {
    const detail = await this.fetchObject(node.id);
    const ddl = detail?.ddl?.trim();
    if (!ddl) {
      // For a routine we DID fetch (detail present) but got no body, MySQL blanked
      // the "Create …" column — that's a privilege limitation, not a bug. Say so.
      const isRoutine = detail?.kind === 'procedure' || detail?.kind === 'function';
      if (isRoutine) {
        toasts.error(
          'Definition not available',
          `MySQL returned no body for ${node.label} — the connected account likely lacks privilege to view routine definitions (needs SHOW_ROUTINE, or SELECT on the routine).`,
        );
      } else {
        toasts.error('Couldn’t build the create statement', `Otto couldn’t derive the DDL for ${node.label}.`);
      }
      return;
    }
    try {
      await copyTextOrThrow(ddl);
      toasts.success('Create statement copied', node.label);
    } catch {
      toasts.error('Couldn’t copy the create statement', 'Select the text and copy it manually.');
    }
  }

  /** Re-fetch the currently-open object's detail (e.g. after an import changes
   *  its row count). No-op when no object is open. */
  async refreshObject(): Promise<void> {
    const path = this.db.selectedObjectPath;
    if (!path) return;
    const detail = await this.fetchObject(path);
    if (detail) this.db.objectDetail = detail;
  }

  // ── Table actions (schema-tree context menu) ──────────────────────────────

  /**
   * Build a qualified SQL table reference from a tree node id like
   * `db:configserver/table:props`, quoted for the selected connection's engine
   * (`"public"."orders"` on Postgres, backticks on MySQL / ClickHouse). Returns
   * the ref plus the raw parts, or null when the node isn't a SQL table/view.
   */
  tableRefFromNode(node: SchemaNode): { ref: string; db: string | null; table: string } | null {
    return tableRefFromNodeId(this.db.selectedConn?.kind, node.id);
  }

  /** Open a statement in a new query tab; optionally run it immediately. `node`
   *  scopes execution (e.g. a Redis keyspace `kdb:N` so the right DB is SELECTed). */
  async openInNewTab(
    sql: string,
    opts?: { run?: boolean; name?: string; node?: string },
  ): Promise<void> {
    this.db.newTab(sql);
    if (opts?.name) this.db.tab.name = opts.name;
    if (opts?.run) await this.db.runs.runQuery(undefined, opts.node);
  }

  // ── Redis key actions ─────────────────────────────────────────────────────

  /** Split a Redis key node id `kdb:<n>/key:<fullkey>` into its keyspace + key.
   *  The key may itself contain ':' / '/', so we slice at the first `/key:`. */
  redisKeyParts(node: SchemaNode): { key: string; keyspace: string } | null {
    const i = node.id.indexOf('/key:');
    if (i < 0) return null;
    return { key: node.label, keyspace: node.id.slice(0, i) };
  }

  /** The correct read command for a Redis key, based on its value TYPE (carried
   *  in the node's `detail`). GET only works on strings — hashes need HGETALL,
   *  lists LRANGE, etc. (using GET on a hash is what returned `(nil)`). */
  redisReadCommand(type: string | undefined, key: string): string {
    const k = /\s|"/.test(key) ? `"${key.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"` : key;
    switch ((type ?? '').toLowerCase()) {
      case 'hash':
        return `HGETALL ${k}`;
      case 'list':
        return `LRANGE ${k} 0 -1`;
      case 'set':
        return `SMEMBERS ${k}`;
      case 'zset':
        return `ZRANGE ${k} 0 -1 WITHSCORES`;
      case 'stream':
        return `XRANGE ${k} - +`;
      default:
        return `GET ${k}`;
    }
  }

  /** New tab with the type-correct read command for a Redis key, scoped to its
   *  keyspace; runs immediately unless `opts.run === false`. */
  async getRedisValue(node: SchemaNode, opts?: { run?: boolean }): Promise<void> {
    const r = this.redisKeyParts(node);
    if (!r) return;
    const cmd = this.redisReadCommand(node.detail, r.key);
    await this.openInNewTab(cmd, { run: opts?.run ?? true, name: r.key, node: r.keyspace });
  }

  /** New tab: `SELECT * FROM <table>` and run it (server applies the row cap). */
  async selectRows(node: SchemaNode): Promise<void> {
    const r = this.tableRefFromNode(node);
    if (!r) return;
    await this.openInNewTab(`SELECT * FROM ${r.ref}`, { run: true, name: r.table });
  }

  /** New tab: `SELECT * FROM <table>` without running (Send to SQL Editor). */
  async sendSelectToEditor(node: SchemaNode): Promise<void> {
    const r = this.tableRefFromNode(node);
    if (!r) return;
    await this.openInNewTab(`SELECT * FROM ${r.ref}`, { name: r.table });
  }

  /** Open the file→table Import dialog, prefilling the target table from a
   *  schema-tree node (the raw, unquoted table name the INSERT needs). */
  openImportDialog(node?: SchemaNode): void {
    // Mongo tree nodes are collections (`coll:`), never `table:`/`view:` — the
    // SQL resolver can't match them, so the prefill was always empty there.
    this.db.importTable = node
      ? (this.tableRefFromNode(node)?.table ?? this.collectionRefFromNode(node)?.coll) ?? ''
      : '';
    this.db.importDialogOpen = true;
  }

  /** Resolve a Mongo collection node to its `{ db, coll }`. */
  collectionRefFromNode(node: SchemaNode): { db: string | null; coll: string } | null {
    const segs = node.id.split('/').map((s) => {
      const i = s.indexOf(':');
      return i < 0 ? ([s, ''] as const) : ([s.slice(0, i), s.slice(i + 1)] as const);
    });
    const find = (k: string) => segs.find(([kk]) => kk === k)?.[1];
    const coll = find('coll') ?? find('collection');
    if (!coll) return null;
    return { db: find('db') ?? null, coll };
  }

  /** New tab: `db.<coll>.find({})` scoped to the collection's database, then run. */
  async findRows(node: SchemaNode): Promise<void> {
    const r = this.collectionRefFromNode(node);
    if (!r) return;
    if (r.db) this.db.setActiveDb(r.db);
    await this.openInNewTab(`db.${r.coll}.find({})`, { run: true, name: r.coll });
  }

  /** New tab: `db.<coll>.find({})` without running (Send to editor). */
  async sendFindToEditor(node: SchemaNode): Promise<void> {
    const r = this.collectionRefFromNode(node);
    if (!r) return;
    if (r.db) this.db.setActiveDb(r.db);
    await this.openInNewTab(`db.${r.coll}.find({})`, { name: r.coll });
  }

  /** New tab pre-filled with a TRUNCATE — NOT run; the user reviews + runs it. */
  async truncateTable(node: SchemaNode): Promise<void> {
    const r = this.tableRefFromNode(node);
    if (!r) return;
    await this.openInNewTab(`TRUNCATE TABLE ${r.ref};`, { name: `TRUNCATE ${r.table}` });
    toasts.warn('Review before running', 'This will delete all rows. Press Run to apply.');
  }

  /** New tab pre-filled with a DROP — NOT run; the user reviews + runs it. */
  async dropObject(node: SchemaNode): Promise<void> {
    const r = this.tableRefFromNode(node);
    if (!r) return;
    const verb = node.kind === 'view' ? 'DROP VIEW' : 'DROP TABLE';
    await this.openInNewTab(`${verb} ${r.ref};`, { name: `DROP ${r.table}` });
    toasts.warn('Review before running', 'This will drop the object. Press Run to apply.');
  }
}
