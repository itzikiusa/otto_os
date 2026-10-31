///<reference types="svelte" />
;
import { formatBytes } from '../../lib/metric-format';
import { pluralNoun } from '../../lib/plural';
import Badge from '../../lib/components/Badge.svelte';
// Object structure: a columns table (name/type/nullable/default/key), primary
// key, indexes, foreign keys, and a collapsible DDL block. For Redis keys /
// Mongo collections (no columns) it renders the `extra` JSON.
import { databaseAccessChild } from '../../lib/access-options';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import TableDesigner from './TableDesigner.svelte';
import JsonTree from '../../lib/components/JsonTree.svelte';
import { database } from '../../lib/stores/database.svelte';
import { toasts } from '../../lib/toast.svelte';
import type { DbForeignKey, DbIndexDef, SchemaNode } from '../../lib/api/types';
import { copyTextOrThrow } from '../../lib/clipboard';
import { quoteIdent, stringLiteral } from '../../lib/database/sql-dialect';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const scope = $derived(databaseAccessChild(database.selectedObjectPath ?? database.activeDb));
  const canSchema = $derived(!!database.selectedConnId && resourceAccess.can('connection',database.selectedConnId,'db_schema','database','edit',scope));
  $effect(()=>{if(database.selectedConnId)void resourceAccess.load('connection',database.selectedConnId,scope);});
  const detail = $derived(database.objectDetail);
  /** The object being opened, named from its path while its detail loads. */
  const loadingName = $derived.by(() => {
    const last = (database.selectedObjectPath ?? '').split('/').pop() ?? '';
    const sep = last.indexOf(':');
    return sep > 0 ? last.slice(sep + 1) : last;
  });

  /** Re-issue the failed openObject for the still-selected path. openObject only
   *  needs the node's id (path) for the fetch — label/kind are rebuilt from the
   *  path's last segment for display parity. */
  function retryOpen(): void {
    const path = database.selectedObjectPath;
    if (!path) return;
    const last = path.split('/').pop() ?? path;
    const sep = last.indexOf(':');
    const kind = (sep > 0 ? last.slice(0, sep) : 'table') as SchemaNode['kind'];
    const label = sep > 0 ? last.slice(sep + 1) : last;
    void database.openObject({ id: path, label, kind, has_children: false });
  }
  // A stored procedure / function — its "columns" are parameters and its main
  // content is the DDL (the SHOW CREATE body).
  const isRoutine = $derived(detail?.kind === 'procedure' || detail?.kind === 'function');
  const titleIcon = $derived(
    detail?.kind === 'procedure'
      ? 'procedure'
      : detail?.kind === 'function'
        ? 'function'
        : detail?.kind === 'view'
          ? 'eye'
          : 'grid',
  );
  let ddlOpen = $state(false);
  let designerOpen = $state(false);

  // Auto-expand the DDL for objects whose DDL is the primary content — stored
  // procedures/functions (and any object that has a DDL but no columns) — so the
  // routine body shows immediately instead of hidden behind the toggle. Re-runs
  // per object (tracks `detail`); the user can still collapse it afterwards.
  $effect(() => {
    const d = detail;
    ddlOpen = !!d?.ddl && (d.kind === 'procedure' || d.kind === 'function' || d.columns.length === 0);
    openIdxDef = null;
    // A half-filled index builder belongs to the object it was opened on.
    resetIdxBuilder();
  });

  function prettyExtra(extra: unknown): string {
    try {
      return JSON.stringify(extra, null, 2);
    } catch {
      return String(extra);
    }
  }

  async function copyDdl(): Promise<void> {
    if (!detail?.ddl) return;
    try {
      await copyTextOrThrow(detail.ddl);
      toasts.success('Copied DDL');
    } catch {
      toasts.error('Couldn’t copy', 'Select the text and copy it manually.');
    }
  }

  function explain(): void {
    if (!detail) return;
    const content = detail.ddl
      ? `DDL for ${detail.name}:\n\n${detail.ddl}`
      : `Object ${detail.name} (${detail.kind})\n\n${prettyExtra(detail.extra)}`;
    void database.explainWithAgent(
      content,
      `Explain the structure of ${detail.name} and how it is used.`,
      `Explain ${detail.name}`,
    );
  }

  // ── FK navigation ──────────────────────────────────────────────────────────
  // Clicking a foreign key row navigates to the referenced table. The current
  // object's node path is used to derive the schema prefix (db:foo/), then the
  // ref_table replaces the table segment. Falls back to a bare name when the
  // current path lacks a db segment (unusual but not fatal).
  function navigateToFkTable(fk: DbForeignKey): void {
    const currentPath = database.selectedObjectPath;
    if (!currentPath) return;

    // Reconstruct the target path from the FK's schema + table.
    // Path format: "db:{db}/table:{table}" for MySQL, or "schema:{s}/table:{t}".
    const segs = currentPath.split('/');
    const dbSeg = segs.find((s) => s.startsWith('db:') || s.startsWith('schema:'));
    const targetDb = fk.ref_schema ?? (dbSeg ? dbSeg.split(':')[1] : null);

    let targetPath: string;
    if (dbSeg && targetDb) {
      const prefix = dbSeg.startsWith('db:') ? 'db' : 'schema';
      targetPath = `${prefix}:${targetDb}/table:${fk.ref_table}`;
    } else {
      targetPath = `table:${fk.ref_table}`;
    }

    const node: SchemaNode = {
      id: targetPath,
      label: fk.ref_table,
      kind: 'table',
      has_children: false,
    };
    void database.openObject(node);
  }

  // ── Full index definition viewer ────────────────────────────────────────────
  // `definition` is the engine-native spec: Mongo = the raw listIndexes doc
  // (partialFilterExpression, collation, TTL…), Postgres = a pg_get_indexdef
  // DDL string. Clicking an index row expands it; reset on object switch above.
  let openIdxDef = $state<number | null>(null);
  function idxDefText(idx: DbIndexDef): string | null {
    const d = idx.definition;
    if (d == null) return null;
    return typeof d === 'string' ? d : JSON.stringify(d, null, 2);
  }
  // Rebuild the shell statement that recreates this index — Mongo only (the
  // definition is an object there); SQL engines already carry a DDL string.
  function idxCreateSnippet(idx: DbIndexDef): string | null {
    const d = idx.definition;
    if (!detail || d == null || typeof d !== 'object') return null;
    const rec = d as Record<string, unknown>;
    if (rec.key == null) return null;
    // Everything the server reports except the key itself and its internal
    // bookkeeping (v/ns) is a createIndex option.
    const opts: Record<string, unknown> = {};
    for (const [k, val] of Object.entries(rec)) {
      if (k !== 'key' && k !== 'v' && k !== 'ns') opts[k] = val;
    }
    const optsStr = Object.keys(opts).length > 0 ? `, ${JSON.stringify(opts, null, 2)}` : '';
    return `db.${detail.name}.createIndex(${JSON.stringify(rec.key, null, 2)}${optsStr})`;
  }
  async function copyNotify(text: string, label: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${label}`);
    } catch {
      toasts.error('Couldn’t copy', 'Select the text and copy it manually.');
    }
  }

  // ── Index builder (per-engine) ──────────────────────────────────────────────
  const isSql = $derived(database.capabilities?.sql === true);
  const engine = $derived(database.capabilities?.engine ?? null);
  const canIndex = $derived(
    !!detail && (isSql ? detail.kind === 'table' : detail.kind === 'collection'),
  );
  /** Engine-correct identifier quoting for the statements this panel prepares —
   *  Postgres uses double quotes, MySQL/ClickHouse backticks. */
  function q(name: string): string {
    return quoteIdent(engine, name);
  }
  // The prepared statements run in a query tab against the ACTIVE db, which
  // need not be this object's db — qualify from the selected object's path
  // (`db:{db}/table:{t}` / `schema:{s}/table:{t}`), like `tableRefFromNode`.
  const objSchema = $derived.by(() => {
    const seg = (database.selectedObjectPath ?? '')
      .split('/')
      .find((s) => s.startsWith('db:') || s.startsWith('schema:'));
    return seg ? seg.slice(seg.indexOf(':') + 1) : null;
  });
  /** `db`.`table` (schema omitted when the path lacks one) for SQL statements. */
  function qualifiedObj(): string {
    const obj = detail!.name;
    return objSchema ? `${q(objSchema)}.${q(obj)}` : q(obj);
  }
  const indexFields = $derived.by(() => {
    if (!detail) return [] as string[];
    if (isSql) return detail.columns.map((c) => c.name);
    const extra = detail.extra as Record<string, unknown> | null;
    // Prefer the nested dotted paths (so you can index `players.playerId` /
    // `templatesAwardedCollections.succeededAwarded.templateId`); fall back to the
    // top-level sampled fields when the server didn't return paths.
    const paths = extra?.sampled_paths;
    if (Array.isArray(paths) && paths.length > 0) {
      const list = (paths as unknown[]).filter((p): p is string => typeof p === 'string');
      return ['_id', ...list.filter((f) => f !== '_id')];
    }
    const sampled = extra?.sampled_fields as Record<string, unknown> | undefined;
    return ['_id', ...Object.keys(sampled ?? {}).filter((f) => f !== '_id')];
  });
  let idxOpen = $state(false);
  let idxCols = $state<string[]>([]);
  let idxUnique = $state(false);
  let idxName = $state('');
  // The index being edited, when the builder was opened from a row's Edit
  // button. No engine here can alter an index in place, so "edit" means
  // "prepare a drop + recreate" — this also carries the engine-native options
  // (Mongo sparse/TTL/partialFilter, the SQL access method) we must not lose.
  let idxEditing = $state<DbIndexDef | null>(null);

  // Every field the picker can offer: the object's indexable fields, plus any
  // already-selected field the sampler didn't surface (a Mongo index can name a
  // path absent from the sampled documents — without this the user would edit
  // it blind).
  const idxAllFields = $derived([
    ...indexFields,
    ...idxCols.filter((c) => !indexFields.includes(c)),
  ]);
  // The picker is a real list, not a tag cloud: a collection with 60+ sampled
  // paths turns chips into an unscannable wall. Selected fields pin to the top
  // in KEY ORDER (which is what actually makes an index useful), the rest are
  // filtered by the search box.
  let idxFieldQuery = $state('');
  const idxRestFields = $derived.by(() => {
    const qy = idxFieldQuery.trim().toLowerCase();
    return idxAllFields.filter(
      (f) => !idxCols.includes(f) && (qy === '' || f.toLowerCase().includes(qy)),
    );
  });
  const suggestedIdxName = $derived(
    detail && idxCols.length > 0
      ? `idx_${detail.name}_${idxCols.join('_')}`.replace(/[^A-Za-z0-9_]/g, '_')
      : '',
  );
  const effectiveIdxName = $derived(idxName.trim() || suggestedIdxName);

  function toggleIdxCol(c: string): void {
    idxCols = idxCols.includes(c) ? idxCols.filter((x) => x !== c) : [...idxCols, c];
  }

  // Per-key sort direction (Mongo). Only meaningful for a COMPOUND index whose
  // sort mixes directions — but the builder previously hard-coded `1`, so there
  // was no way to express `{ brand: 1, whenUpdated: -1 }` at all.
  let idxDirs = $state<Record<string, 1 | -1>>({});
  function idxDirOf(c: string): 1 | -1 {
    return idxDirs[c] ?? 1;
  }
  function flipIdxDir(c: string): void {
    idxDirs = { ...idxDirs, [c]: idxDirOf(c) === 1 ? -1 : 1 };
  }

  // A path the user TYPED that the picker doesn't list. Mongo indexes a path that
  // no sampled document happened to contain (rare/sparse fields, or a collection
  // whose sample missed it), so the builder must not be limited to what the
  // sampler saw — without this there is no way to name such a field at all.
  const customIdxPath = $derived.by<string | null>(() => {
    const raw = idxFieldQuery.trim();
    if (raw === '' || /\s/.test(raw)) return null;
    return idxAllFields.includes(raw) ? null : raw;
  });

  // ── Mongo fields table ──────────────────────────────────────────────────────
  // A collection has no `columns`, so the structure tab used to show NOTHING
  // field-shaped — just a raw `extra` JSON dump. Embedded paths were therefore
  // invisible unless you happened to open the index builder, which is exactly how
  // `lobbyMetaData.brand_id` became "impossible to find".
  /** Sampled BSON type per path (falls back to top-level types on older daemons). */
  const fieldTypes = $derived.by<Record<string, string>>(() => {
    const extra = detail?.extra as Record<string, unknown> | null | undefined;
    const src = extra?.sampled_path_types ?? extra?.sampled_fields;
    if (src && typeof src === 'object' && !Array.isArray(src)) {
      return Object.fromEntries(
        Object.entries(src as Record<string, unknown>).map(([k, v]) => [k, String(v)]),
      );
    }
    return {};
  });
  /** Paths already named by SOME index on this collection. */
  const indexedPaths = $derived.by<Set<string>>(() => {
    const s = new Set<string>();
    for (const idx of detail?.indexes ?? []) for (const c of idx.columns) s.add(c);
    return s;
  });
  const mongoFields = $derived(
    !isSql && detail?.kind === 'collection'
      ? indexFields.map((p) => ({
          path: p,
          type: fieldTypes[p] ?? '',
          indexed: indexedPaths.has(p),
          /** Depth 0 = top-level; deeper paths are embedded. */
          nested: p.includes('.'),
        }))
      : [],
  );
  let fieldQuery = $state('');
  const shownMongoFields = $derived.by(() => {
    const qy = fieldQuery.trim().toLowerCase();
    return qy === '' ? mongoFields : mongoFields.filter((f) => f.path.toLowerCase().includes(qy));
  });
  /** Open the index builder pre-seeded with one field (from the Fields table). */
  function indexField(path: string): void {
    resetIdxBuilder();
    idxCols = [path];
    idxOpen = true;
  }

  // ── Index conditions (partial indexes) ──────────────────────────────────────
  // A condition narrows WHICH rows/documents the index covers. Two operators —
  // `exists` (Mongo `$exists: true` / SQL `IS NOT NULL`) and `in` — ANDed
  // together. Engine reality differs sharply and the generator says so:
  //   • MongoDB  — native `partialFilterExpression`.
  //   • Postgres — native `CREATE INDEX … WHERE`.
  //   • MySQL    — has NO partial index; emulated with a functional key part
  //                over a CASE expression (see mysqlPartialNote).
  //   • ClickHouse — no equivalent at all, so the section is hidden.
  type IdxCondOp = 'exists' | 'in';
  interface IdxCond {
    field: string;
    op: IdxCondOp;
    /** Comma-separated literals, for `in` only. */
    values: string;
  }
  let idxConds = $state<IdxCond[]>([]);
  // Parts of an EXISTING partial filter this UI can't represent as rows, kept
  // verbatim so editing an index never silently widens what it covers.
  let idxCondExtraMongo = $state<Record<string, unknown> | null>(null);
  // Postgres stores its predicate as catalog-normalized SQL (`= ANY (ARRAY[…])`,
  // `::character varying` casts); round-tripping that through rows would mangle
  // it, so an existing predicate is preserved as text and only REPLACED when the
  // user adds rows of their own.
  let idxCondRawSql = $state<string | null>(null);

  const canCondition = $derived(!isSql || engine === 'mysql' || engine === 'postgres');
  const idxCondFields = $derived(idxAllFields);

  function addIdxCond(): void {
    idxConds = [...idxConds, { field: idxCondFields[0] ?? '', op: 'exists', values: '' }];
  }
  function removeIdxCond(i: number): void {
    idxConds = idxConds.filter((_, n) => n !== i);
  }
  /** Split the comma-separated `in` input into typed literals (numbers, bools,
   *  null and strings) so Mongo gets real JSON and SQL gets correct quoting. */
  function parseCondValues(raw: string): unknown[] {
    return raw
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
      .map((s) => {
        if (s === 'null') return null;
        if (s === 'true') return true;
        if (s === 'false') return false;
        if (/^-?\d+(\.\d+)?$/.test(s)) return Number(s);
        return s.replace(/^(['"])(.*)\1$/s, '$2');
      });
  }
  /** Rows that will actually contribute (a field, and values when `in`). */
  const usableIdxConds = $derived(
    idxConds.filter(
      (c) => c.field !== '' && (c.op === 'exists' || parseCondValues(c.values).length > 0),
    ),
  );
  /** Mongo `partialFilterExpression`, merged over anything we preserved. */
  function mongoPartialFilter(): Record<string, unknown> | null {
    const out: Record<string, unknown> = { ...(idxCondExtraMongo ?? {}) };
    for (const c of usableIdxConds) {
      out[c.field] =
        c.op === 'exists' ? { $exists: true } : { $in: parseCondValues(c.values) };
    }
    return Object.keys(out).length > 0 ? out : null;
  }
  function sqlLiteral(v: unknown): string {
    if (v === null) return 'NULL';
    if (typeof v === 'number' || typeof v === 'boolean') return String(v);
    return stringLiteral(engine, String(v));
  }
  /** The SQL predicate for the condition rows, or the preserved one untouched. */
  function sqlPredicate(): string | null {
    const parts = usableIdxConds.map((c) =>
      c.op === 'exists'
        ? `${q(c.field)} IS NOT NULL`
        : `${q(c.field)} IN (${parseCondValues(c.values).map(sqlLiteral).join(', ')})`,
    );
    return parts.length > 0 ? parts.join(' AND ') : idxCondRawSql;
  }
  const mysqlPartialNote =
    '-- MySQL has no partial indexes; this emulates one with a functional key\n' +
    '-- part. The optimizer uses it ONLY for queries written with the same CASE\n' +
    '-- expression.';

  function resetIdxBuilder(): void {
    idxOpen = false;
    idxEditing = null;
    idxCols = [];
    idxDirs = {};
    idxUnique = false;
    idxName = '';
    idxFieldQuery = '';
    idxConds = [];
    idxCondExtraMongo = null;
    idxCondRawSql = null;
  }

  // ── Index edit / drop ───────────────────────────────────────────────────────
  // Mongo refuses to drop `_id_` — offering the action would only produce a
  // statement the server rejects.
  function isProtectedIdx(idx: DbIndexDef): boolean {
    return !isSql && idx.name === '_id_';
  }
  /** True when this index is the one BACKING the table's primary key (its
   *  columns are exactly the PK). Postgres refuses `DROP INDEX` on those. */
  function backsPrimaryKey(idx: DbIndexDef): boolean {
    const pk = detail?.primary_key ?? [];
    if (!idx.unique || pk.length === 0 || pk.length !== idx.columns.length) return false;
    const cols = new Set(idx.columns);
    return pk.every((c) => cols.has(c));
  }
  function dropIndexStmt(idx: DbIndexDef): string {
    const obj = detail!.name;
    if (!isSql) return `db.${obj}.dropIndex(${JSON.stringify(idx.name)})`;
    const ref = qualifiedObj();
    if (engine === 'mysql') {
      // MySQL exposes the PK as an index literally named PRIMARY; it has its
      // own verb and can't be named in DROP INDEX.
      return idx.name === 'PRIMARY'
        ? `ALTER TABLE ${ref} DROP PRIMARY KEY;`
        : `ALTER TABLE ${ref} DROP INDEX ${q(idx.name)};`;
    }
    if (engine === 'postgres') {
      // A Postgres index is schema-scoped — DROP INDEX must name that schema
      // when the session's search_path points elsewhere.
      const idxRef = objSchema ? `${q(objSchema)}.${q(idx.name)}` : q(idx.name);
      return backsPrimaryKey(idx)
        ? `ALTER TABLE ${ref} DROP CONSTRAINT ${q(idx.name)};`
        : `DROP INDEX ${idxRef};`;
    }
    return `ALTER TABLE ${ref} DROP INDEX ${q(idx.name)};`;
  }
  /**
   * Rebuild a CREATE INDEX / createIndex for the given shape. `base` (set when
   * editing) supplies what the columns+unique summary can't carry: Mongo's key
   * DIRECTIONS (-1, "text", "2dsphere") and extra options (sparse, expireAfter‐
   * Seconds, partialFilterExpression, collation), and the SQL access method.
   */
  function createIndexStmt(
    name: string,
    cols: string[],
    unique: boolean,
    base: DbIndexDef | null,
  ): string {
    const obj = detail!.name;
    const def = base?.definition;
    const rec = def && typeof def === 'object' ? (def as Record<string, unknown>) : null;
    if (!isSql) {
      const baseKey = (rec?.key ?? null) as Record<string, unknown> | null;
      // Direction precedence: what the user picked → what the edited index already
      // used (so "edit" preserves -1/"text"/"2dsphere") → ascending.
      const key = cols
        .map(
          (c) =>
            `${JSON.stringify(c)}: ${JSON.stringify(idxDirs[c] ?? baseKey?.[c] ?? 1)}`,
        )
        .join(', ');
      const opts: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(rec ?? {})) {
        // `partialFilterExpression` is rebuilt from the condition rows below —
        // carrying the old one over too would merge two filters.
        if (!['key', 'v', 'ns', 'name', 'unique', 'partialFilterExpression'].includes(k)) {
          opts[k] = v;
        }
      }
      if (unique) opts.unique = true;
      const partial = mongoPartialFilter();
      if (partial) {
        opts.partialFilterExpression = partial;
        // MongoDB rejects an index that specifies both ("Cannot specify both
        // partialFilterExpression and sparse") — the filter supersedes sparse.
        delete opts.sparse;
      }
      opts.name = name;
      return `db.${obj}.createIndex({ ${key} }, ${JSON.stringify(opts, null, 2)})`;
    }
    if (engine === 'clickhouse') {
      // ClickHouse has no CREATE INDEX (and no UNIQUE index) — its secondary
      // indexes are DATA-SKIPPING indexes added via ALTER, and they need a
      // TYPE. minmax is the safe default; the user reviews before running.
      return (
        '-- ClickHouse data-skipping index — adjust TYPE to fit the data\n' +
        '-- (minmax / set(N) / bloom_filter / ngrambf_v1 / tokenbf_v1).\n' +
        `ALTER TABLE ${qualifiedObj()} ADD INDEX ${q(name)} (${cols.map(q).join(', ')}) TYPE minmax GRANULARITY 1;`
      );
    }
    const method = (base?.method ?? '').toUpperCase();
    // MySQL spells FULLTEXT / SPATIAL as the index KIND (and neither can be
    // unique); everywhere else UNIQUE is the only kind we emit.
    const kind =
      engine === 'mysql' && (method === 'FULLTEXT' || method === 'SPATIAL')
        ? `${method} `
        : unique
          ? 'UNIQUE '
          : '';
    // Postgres carries the access method in USING; btree is the default and
    // stays implicit.
    const using =
      engine === 'postgres' && method && method !== 'BTREE' ? ` USING ${method.toLowerCase()}` : '';
    const pred = sqlPredicate();
    // MySQL has no partial index. Its documented stand-in is a FUNCTIONAL key
    // part: wrap the leading column in a CASE that yields NULL for rows outside
    // the condition, so they never enter the b-tree (and, for a UNIQUE index,
    // don't collide — MySQL allows many NULLs). The extra parens are required
    // syntax for an expression key part.
    if (pred && engine === 'mysql') {
      const parts = cols.map(q);
      parts[0] = `(CASE WHEN ${pred} THEN ${q(cols[0])} END)`;
      return `${mysqlPartialNote}\nCREATE ${kind}INDEX ${q(name)} ON ${qualifiedObj()} (${parts.join(', ')});`;
    }
    const where = pred && engine === 'postgres' ? ` WHERE ${pred}` : '';
    return `CREATE ${kind}INDEX ${q(name)} ON ${qualifiedObj()}${using} (${cols.map(q).join(', ')})${where};`;
  }

  /** Split an existing Mongo `partialFilterExpression` into editable rows; keys
   *  whose shape we don't model are handed back to be preserved verbatim. */
  function parseMongoPartial(pf: unknown): {
    rows: IdxCond[];
    extra: Record<string, unknown> | null;
  } {
    const rows: IdxCond[] = [];
    const extra: Record<string, unknown> = {};
    if (pf && typeof pf === 'object') {
      for (const [field, v] of Object.entries(pf as Record<string, unknown>)) {
        const o = v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
        const keys = o ? Object.keys(o) : [];
        if (keys.length === 1 && o!.$exists === true) {
          rows.push({ field, op: 'exists', values: '' });
        } else if (keys.length === 1 && Array.isArray(o!.$in)) {
          rows.push({
            field,
            op: 'in',
            values: (o!.$in as unknown[])
              .map((x) => (typeof x === 'string' ? x : JSON.stringify(x)))
              .join(', '),
          });
        } else {
          extra[field] = v;
        }
      }
    }
    return { rows, extra: Object.keys(extra).length > 0 ? extra : null };
  }

  /** Toggle the builder open for a BRAND-NEW index (never inherits edit state). */
  function startNewIndex(): void {
    if (idxOpen && !idxEditing) {
      resetIdxBuilder();
      return;
    }
    resetIdxBuilder();
    idxOpen = true;
  }
  /** Open the builder pre-filled from an existing index (edit = drop + recreate). */
  function editIndex(idx: DbIndexDef): void {
    idxEditing = idx;
    // A MySQL functional key part reports an EMPTY column name (the expression
    // lives in SHOW INDEX's `Expression`, which the summary doesn't carry) —
    // keeping it would emit an empty identifier.
    idxCols = idx.columns.filter((c) => c !== '');
    idxUnique = idx.unique;
    idxName = idx.name;
    idxFieldQuery = '';
    idxConds = [];
    idxCondExtraMongo = null;
    idxCondRawSql = null;
    const def = idx.definition;
    if (!isSql && def && typeof def === 'object') {
      const parsed = parseMongoPartial((def as Record<string, unknown>).partialFilterExpression);
      idxConds = parsed.rows;
      idxCondExtraMongo = parsed.extra;
    } else if (engine === 'postgres' && typeof def === 'string') {
      // `pg_get_indexdef` ends with the predicate when the index is partial.
      const m = /\sWHERE\s+(.+?)\s*;?\s*$/is.exec(def);
      idxCondRawSql = m ? m[1] : null;
    }
    idxOpen = true;
    openIdxDef = null;
  }
  /** Prepare a DROP for review in a query tab — never auto-applied. */
  function dropIndex(idx: DbIndexDef): void {
    if (!detail) return;
    void database.openInNewTab(dropIndexStmt(idx), { name: `DROP ${idx.name}` });
    toasts.warn(
      'Review before running',
      `This will drop the index ${idx.name}. Press Run to apply.`,
    );
  }
  // Prepare a CREATE INDEX / createIndex statement (preceded by the DROP when
  // editing) and open it in a query tab for the user to review and run.
  function buildIndex(): void {
    if (!detail || idxCols.length === 0) return;
    const editing = idxEditing;
    const create = createIndexStmt(effectiveIdxName, idxCols, idxUnique, editing);
    const stmt = editing ? `${dropIndexStmt(editing)}\n${create}` : create;
    void database.openInNewTab(stmt, {
      name: editing ? `EDIT ${editing.name}` : effectiveIdxName,
    });
    if (editing) {
      toasts.warn(
        'Review before running',
        `No engine edits an index in place — this drops ${editing.name} and recreates it. The table is unindexed on that key in between.`,
      );
    }
    resetIdxBuilder();
  }

  // ── Stats (Mongo collStats) ─────────────────────────────────────────────────
  const SIZE_KEYS = new Set(['size', 'storageSize', 'avgObjSize', 'totalIndexSize', 'totalSize']);
  const mongoStats = $derived.by(() => {
    const s = (detail?.extra as Record<string, unknown> | null)?.stats as
      | Record<string, unknown>
      | undefined;
    if (!s) return null;
    return Object.entries(s).map(
      ([k, v]) =>
        [k, SIZE_KEYS.has(k) && typeof v === 'number' ? formatBytes(v) : String(v)] as [string, string],
    );
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`structure`,});
  if(database.objectLoading){
    
     { svelteHTML.createElement("div", {       "class":`st-head`,"role":`status`,"aria-live":`polite`,"aria-label":`Loading structure`,});
       { svelteHTML.createElement("div", { "class":`st-title`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":14,}});}
         { svelteHTML.createElement("h2", { "class":`mono`,});loadingName; }
         { svelteHTML.createElement("span", { "class":`kind-chip skel-chip`,});  }
       }
       { svelteHTML.createElement("div", { "class":`st-head-actions`,}); }
     }
     { svelteHTML.createElement("div", { "class":`block`,});
       { svelteHTML.createElement("div", { "class":`block-title`,});  }
       { svelteHTML.createElement("div", {   "class":`tbl-wrap skel-tbl`,"aria-hidden":`true`,});
            for(let _ of __sveltets_2_ensureArray({ length: 7 })){let i = 1;i;
           { svelteHTML.createElement("div", { "class":`skel-line`,}); { svelteHTML.createElement("span", { "style":`width:${30 + ((i * 23) % 45)}%`,}); } }
        }
       }
     }
  } else if (!detail && database.objectError){
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`the object`,"variant":`page`,"empty":true,"error":database.objectError,"onretry":retryOpen,}});}
  } else if (!detail){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`box`,"title":`No object selected`,"body":`Pick a table, view, collection or key from the schema tree to inspect its structure.`,}});}
  }else{
     { svelteHTML.createElement("div", { "class":`st-head`,});
       { svelteHTML.createElement("div", { "class":`st-title`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":titleIcon,"size":14,}});}
         { svelteHTML.createElement("h2", { "class":`mono`,});detail.name; }
         { svelteHTML.createElement("span", { "class":`kind-chip`,});detail.kind; }
        if(detail.row_count != null){
           { svelteHTML.createElement("span", { "class":`rowcount`,});detail.row_count.toLocaleString(); pluralNoun(detail.row_count, 'row'); }
        }
       }
       { svelteHTML.createElement("div", { "class":`st-head-actions`,});
        if(isSql && detail.kind === 'table'){
           { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":!canSchema,"onclick":() => (designerOpen = true),"title":`Edit columns → generates ALTER TABLE for review`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
           }
        }
         { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":explain,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}  }
       }
     }

    if(detail.columns.length > 0){
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`block-title`,});isRoutine ? 'Parameters' : 'Columns';  { svelteHTML.createElement("span", { "class":`count`,});detail.columns.length; } }
         { svelteHTML.createElement("div", { "class":`tbl-wrap`,});
           { svelteHTML.createElement("table", { "class":`tbl mono`,});
             { svelteHTML.createElement("thead", {});
               { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } }
             }
             { svelteHTML.createElement("tbody", {});
                  for(let c of __sveltets_2_ensureArray(detail.columns)){let i = 1;i;
                 { svelteHTML.createElement("tr", {});
                   { svelteHTML.createElement("td", { "class":`cn`,});
                    c.name;
                    if(detail.primary_key.includes(c.name)){ { svelteHTML.createElement("span", {   "class":`pk`,"title":`Primary key`,});  }}
                   }
                   { svelteHTML.createElement("td", { "class":`ty`,});c.data_type; }
                   { svelteHTML.createElement("td", { "class":`nullable`,});c.nullable ? 'YES' : 'NO'; }
                   { svelteHTML.createElement("td", {});c.key ?? ''; }
                   { svelteHTML.createElement("td", { "class":`dim`,});c.default ?? ''; }
                   { svelteHTML.createElement("td", { "class":`dim`,});c.extra ?? ''; }
                 }
                if(c.comment){
                   { svelteHTML.createElement("tr", { "class":`comment-row`,}); { svelteHTML.createElement("td", {}); } { svelteHTML.createElement("td", {   "colspan":5,"class":`comment`,});c.comment; } }
                }
              }
             }
           }
         }
       }
    }

    if(mongoFields.length > 0){
      
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`block-title`,});
            { svelteHTML.createElement("span", { "class":`count`,});mongoFields.length; }
           { svelteHTML.createElement("span", { "class":`hint dim`,});  }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("input", {              "dir":`auto`,"aria-label":`Filter fields`,"class":`ib-search`,"type":`search`,"bind:value":fieldQuery,"placeholder":`Filter fields…`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fieldQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", { "class":`tbl-wrap`,});
           { svelteHTML.createElement("table", { "class":`tbl mono`,});
             { svelteHTML.createElement("thead", {});
               { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {}); } }
             }
             { svelteHTML.createElement("tbody", {});
                 for(let f of __sveltets_2_ensureArray(shownMongoFields)){f.path;
                 { svelteHTML.createElement("tr", {});
                   { svelteHTML.createElement("td", { "class":`cn`,});
                    f.path;
                    if(f.nested){' '; { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {    "label":`Nested`,"title":`Embedded field path`,}});}}
                   }
                   { svelteHTML.createElement("td", { "class":`ty`,});f.type; }
                   { svelteHTML.createElement("td", {});
                    if(f.indexed){ { svelteHTML.createElement("span", {   "class":`pk`,"title":`Named by an existing index`,});  }}
                   }
                   { svelteHTML.createElement("td", { "class":`fld-act`,});
                    if(canIndex){
                       { svelteHTML.createElement("button", {     "class":`mini-btn`,"disabled":!canSchema,"onclick":() => indexField(f.path),});
                         { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}
                       }
                    }
                   }
                 }
              }
              if(shownMongoFields.length === 0){
                 { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", {   "colspan":4,"class":`dim`,});   fieldQuery;  } }
              }
             }
           }
         }
       }
    }

    if(detail.primary_key.length > 0){
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`block-title`,});  }
         { svelteHTML.createElement("div", { "class":`chips`,});
             for(let col of __sveltets_2_ensureArray(detail.primary_key)){col; { svelteHTML.createElement("span", { "class":`key-chip mono`,});col; }}
         }
       }
    }

    if(detail.indexes.length > 0 || canIndex){
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`block-title`,});
            { svelteHTML.createElement("span", { "class":`count`,});detail.indexes.length; }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
          if(canIndex){
             { svelteHTML.createElement("button", {     "class":`mini-btn`,"disabled":!canSchema,"onclick":startNewIndex,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} 
             }
          }
         }
        if(detail.indexes.length > 0){
           { svelteHTML.createElement("ul", { "class":`idx-list`,});
                for(let idx of __sveltets_2_ensureArray(detail.indexes)){let i = 1;i;
              const defText = idxDefText(idx);
               { svelteHTML.createElement("li", { "class":`idx-item`,});
                 { svelteHTML.createElement("div", { "class":`idx-row`,});
                   { svelteHTML.createElement("button", {            "class":`idx`,"disabled":defText == null,"title":defText != null ? 'View full definition' : undefined,"aria-expanded":defText != null ? openIdxDef === i : undefined,"onclick":() => (openIdxDef = openIdxDef === i ? null : i),});defText != null;
                     { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}
                     { svelteHTML.createElement("span", {   "class":`idx-name mono`,"title":idx.name,});idx.name; }
                    if(idx.unique){ { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Unique`,}});}}
                    if(idx.method){ { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "variant":`outline`,"label":idx.method,}});}}
                     { svelteHTML.createElement("span", { "class":`idx-cols mono`,}); idx.columns.join(', ');  }
                    if(defText != null){
                       { svelteHTML.createElement("span", { "class":`grow`,}); }
                       { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":openIdxDef === i ? 'chevronDown' : 'chevronRight',"size":12,}});}
                    }
                   }
                  if(canIndex){
                    const locked = isProtectedIdx(idx);
                     { svelteHTML.createElement("div", { "class":`idx-acts`,});
                       { svelteHTML.createElement("button", {           "class":`idx-act`,"aria-label":`Edit index ${idx.name}`,"disabled":locked || !canSchema,"title":locked
                          ? "MongoDB’s _id_ index can’t be changed"
                          : 'Edit — prepares a drop + recreate for you to review and run',"onclick":() => editIndex(idx),});
                         { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
                       }
                       { svelteHTML.createElement("button", {           "class":`idx-act danger`,"aria-label":`Drop index ${idx.name}`,"disabled":locked || !canSchema,"title":locked
                          ? "MongoDB’s _id_ index can’t be dropped"
                          : 'Drop — prepares the statement for you to review and run',"onclick":() => dropIndex(idx),});
                         { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
                       }
                     }
                  }
                 }
                if(openIdxDef === i && defText != null){
                  const snippet = idxCreateSnippet(idx);
                   { svelteHTML.createElement("div", { "class":`idx-def`,});
                     { svelteHTML.createElement("div", { "class":`idx-def-actions`,});
                      if(snippet){
                         { svelteHTML.createElement("button", {   "class":`copy-ddl`,"onclick":() => copyNotify(snippet, 'createIndex'),});
                           { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} 
                         }
                      }
                       { svelteHTML.createElement("button", {   "class":`copy-ddl`,"onclick":() => copyNotify(defText, 'definition'),});
                         { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}
                       }
                     }
                     { svelteHTML.createElement("pre", { "class":`ddl mono`,});defText; }
                   }
                }
               }
            }
           }
        }
        if(idxOpen){
           { svelteHTML.createElement("div", { "class":`idx-builder`,});
            if(idxEditing){
               { svelteHTML.createElement("div", { "class":`ib-editing`,});
                  { svelteHTML.createElement("span", { "class":`mono`,});idxEditing.name; }       
                      { svelteHTML.createElement("strong", {});   }
               }
            }
             { svelteHTML.createElement("div", { "class":`ib-section`,});
               { svelteHTML.createElement("div", { "class":`ib-label`,});
                
                 { svelteHTML.createElement("span", { "class":`ib-count`,});idxCols.length;  }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("input", {              "dir":`auto`,"aria-label":`Filter index fields`,"class":`ib-search`,"type":`search`,"bind:value":idxFieldQuery,"placeholder":`Filter fields…`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => idxFieldQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               }
               { svelteHTML.createElement("div", { "class":`ib-list`,});
                    for(let f of __sveltets_2_ensureArray(idxCols)){let i = 1;f;
                  
                   { svelteHTML.createElement("div", { "class":`ib-row on`,});
                     { svelteHTML.createElement("button", {   "class":`ib-pick`,"onclick":() => toggleIdxCol(f),});
                       { svelteHTML.createElement("span", { "class":`ib-ord`,});i + 1; }
                       { svelteHTML.createElement("span", { "class":`ib-fname mono`,});f; }
                     }
                     { svelteHTML.createElement("span", { "class":`grow`,}); }
                    if(!isSql){
                       { svelteHTML.createElement("button", {       "class":`ib-dir`,"title":`Key direction — ascending (1) or descending (-1)`,"onclick":() => flipIdxDir(f),});
                        idxDirOf(f) === 1 ? '↑ 1' : '↓ -1';
                       }
                    }
                     { svelteHTML.createElement("button", {       "class":`ib-x`,"aria-label":`Remove ${f}`,"title":`Remove ${f}`,"onclick":() => toggleIdxCol(f),});
                       { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
                     }
                   }
                }
                if(idxCols.length > 0 && (idxRestFields.length > 0 || customIdxPath)){
                   { svelteHTML.createElement("div", { "class":`ib-sep`,}); }
                }
                if(customIdxPath){
                  
                   { svelteHTML.createElement("button", {   "class":`ib-row custom`,"onclick":() => toggleIdxCol(customIdxPath),});
                     { svelteHTML.createElement("span", { "class":`ib-ord`,});  }
                     { svelteHTML.createElement("span", { "class":`ib-fname mono`,});customIdxPath; }
                     { svelteHTML.createElement("span", { "class":`grow`,}); }
                     { svelteHTML.createElement("span", { "class":`ib-custom-tag`,});   }
                   }
                }
                   for(let f of __sveltets_2_ensureArray(idxRestFields)){f;
                   { svelteHTML.createElement("button", {   "class":`ib-row`,"onclick":() => toggleIdxCol(f),});
                     { svelteHTML.createElement("span", { "class":`ib-ord`,}); }
                     { svelteHTML.createElement("span", { "class":`ib-fname mono`,});f; }
                   }
                }
                if(idxCols.length === 0 && idxRestFields.length === 0 && !customIdxPath){
                   { svelteHTML.createElement("div", { "class":`ib-empty dim`,});   idxFieldQuery;  }
                }
               }
             }

            if(canCondition){
               { svelteHTML.createElement("div", { "class":`ib-section`,});
                 { svelteHTML.createElement("div", { "class":`ib-label`,});
                  
                   { svelteHTML.createElement("span", { "class":`ib-count`,});
                    engine === 'mongodb' ? 'partial filter' : 'partial index';
                   }
                   { svelteHTML.createElement("span", { "class":`grow`,}); }
                   { svelteHTML.createElement("button", {   "class":`mini-btn`,"onclick":addIdxCond,});
                     { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} 
                   }
                 }
                    for(let cond of __sveltets_2_ensureArray(idxConds)){let ci = 1;ci;
                   { svelteHTML.createElement("div", { "class":`ib-cond`,});
                     { svelteHTML.createElement("select", {     "aria-label":`Condition field`,"class":`mono`,"bind:value":cond.field,});/*Ωignore_startΩ*/() => cond.field = __sveltets_2_any(null);/*Ωignore_endΩ*/
                         for(let f of __sveltets_2_ensureArray(idxCondFields)){f; { svelteHTML.createElement("option", { "value":f,});f; }}
                     }
                     { svelteHTML.createElement("select", {   "aria-label":`Condition operator`,"bind:value":cond.op,});/*Ωignore_startΩ*/() => cond.op = __sveltets_2_any(null);/*Ωignore_endΩ*/
                       { svelteHTML.createElement("option", { "value":`exists`,});  }
                       { svelteHTML.createElement("option", { "value":`in`,});  }
                     }
                    if(cond.op === 'in'){
                       { svelteHTML.createElement("input", {              "dir":`ltr`,"aria-label":`Condition values`,"class":`mono`,"type":`text`,"bind:value":cond.values,"placeholder":`a, b, 3, true`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => cond.values = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                    }else{
                       { svelteHTML.createElement("span", { "class":`ib-cond-hint dim`,});
                        engine === 'mongodb' ? '$exists: true' : 'IS NOT NULL';
                       }
                    }
                     { svelteHTML.createElement("button", {         "class":`idx-act danger`,"aria-label":`Remove condition ${ci + 1}`,"title":`Remove condition ${ci + 1}`,"onclick":() => removeIdxCond(ci),});
                       { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
                     }
                   }
                }
                if(engine === 'mysql' && usableIdxConds.length > 0){
                   { svelteHTML.createElement("div", { "class":`ib-warn`,});
                                { svelteHTML.createElement("strong", {}); 
                     }    { svelteHTML.createElement("code", {});  }       
                        
                   }
                }
                if(engine === 'mongodb' && usableIdxConds.length > 0 && idxEditing?.definition && typeof idxEditing.definition === 'object' && (idxEditing.definition as Record<string, unknown>).sparse === true){
                   { svelteHTML.createElement("div", { "class":`ib-warn`,});
                     { svelteHTML.createElement("code", {});  }           
                     { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }
                   }
                }
                if(idxCondExtraMongo){
                   { svelteHTML.createElement("div", { "class":`ib-warn`,});
                            
                     { svelteHTML.createElement("span", { "class":`mono`,});Object.keys(idxCondExtraMongo).join(', '); }  
                     
                   }
                }
                if(idxCondRawSql){
                   { svelteHTML.createElement("div", { "class":`ib-warn`,});
                       
                     { svelteHTML.createElement("span", { "class":`mono`,});idxCondRawSql; }usableIdxConds.length > 0
                      ? ' — replaced by the condition(s) above.'
                      : '';
                   }
                }
               }
            }

             { svelteHTML.createElement("label", { "class":`ib-name`,});
              
               { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`mono`,"type":`text`,"bind:value":idxName,"placeholder":suggestedIdxName || 'idx_…',"spellcheck":`false`,});/*Ωignore_startΩ*/() => idxName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
            if(engine !== 'clickhouse'){
               { svelteHTML.createElement("label", { "class":`ib-unique`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":idxUnique,});/*Ωignore_startΩ*/() => idxUnique = __sveltets_2_any(null);/*Ωignore_endΩ*/}  }
            }else{
               { svelteHTML.createElement("div", { "class":`ib-hint dim`,});
                    { svelteHTML.createElement("strong", {});  } 
                 { svelteHTML.createElement("code", {});        }     
                            
               }
            }
             { svelteHTML.createElement("div", { "class":`ib-actions`,});
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":resetIdxBuilder,});  }
               { svelteHTML.createElement("button", {     "class":`btn small primary`,"disabled":!canSchema || idxCols.length === 0,"onclick":buildIndex,});
                if(idxEditing){    }else{ isSql ? 'CREATE INDEX' : 'createIndex'; }
               }
             }
             { svelteHTML.createElement("div", { "class":`ib-hint dim`,});             }
           }
        }
       }
    }

    if(mongoStats){
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`block-title`,});  }
         { svelteHTML.createElement("div", { "class":`stats-grid`,});
             for(let [k, v] of __sveltets_2_ensureArray(mongoStats)){k;
             { svelteHTML.createElement("div", { "class":`stat`,}); { svelteHTML.createElement("span", { "class":`sk mono`,});k; } { svelteHTML.createElement("span", { "class":`sv mono`,});v; } }
          }
         }
       }
    }

    if(detail.foreign_keys.length > 0){
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`block-title`,});   { svelteHTML.createElement("span", { "class":`count`,});detail.foreign_keys.length; } }
         { svelteHTML.createElement("ul", { "class":`fk-list`,});
              for(let fk of __sveltets_2_ensureArray(detail.foreign_keys)){let i = 1;i;
             { svelteHTML.createElement("li", { "class":`fk`,});
               { svelteHTML.createElement("span", { "class":`fk-name mono`,});fk.name; }
               { svelteHTML.createElement("span", { "class":`fk-map mono`,});
                fk.columns.join(', ');
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}
                 { svelteHTML.createElement("button", {       "class":`fk-ref-btn mono`,"title":`Open ${fk.ref_schema ? `${fk.ref_schema}.` : ''}${fk.ref_table}`,"onclick":() => navigateToFkTable(fk),});fk.ref_schema ? `${fk.ref_schema}.` : '';fk.ref_table; } fk.ref_columns.join(', ');
               }
             }
          }
         }
       }
    }

    if(detail.extra != null && detail.columns.length === 0){
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`block-title`,});
          
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {   "class":`copy-ddl`,"onclick":() => copyNotify(prettyExtra(detail.extra), 'details'),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}
           }
         }
        
         { svelteHTML.createElement("div", { "class":`extra-tree mono`,}); { const $$_eerTnosJ3C = __sveltets_2_ensureComponent(JsonTree); new $$_eerTnosJ3C({ target: __sveltets_2_any(), props: {  "value":detail.extra,}});} }
       }
    }

    if(detail.ddl){
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`ddl-head`,});
           { svelteHTML.createElement("button", {     "class":`block-title toggle`,"aria-expanded":ddlOpen,"onclick":() => (ddlOpen = !ddlOpen),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":ddlOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
            
           }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
          if(ddlOpen){
             { svelteHTML.createElement("button", {   "class":`copy-ddl`,"onclick":copyDdl,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}
             }
          }
         }
        if(ddlOpen){
           { svelteHTML.createElement("pre", { "class":`ddl mono`,});detail.ddl; }
        }
       }
    } else if (isRoutine){
      
       { svelteHTML.createElement("div", { "class":`block`,});
         { svelteHTML.createElement("div", { "class":`block-title`,});  }
         { svelteHTML.createElement("div", { "class":`ddl-missing`,});
                    
                 { svelteHTML.createElement("code", {});  } 
           { svelteHTML.createElement("code", {});  }   
         }
       }
    }
  }
 }

if(designerOpen && detail && detail.kind === 'table'){
   { const $$_rengiseDelbaT0C = __sveltets_2_ensureComponent(TableDesigner); new $$_rengiseDelbaT0C({ target: __sveltets_2_any(), props: {       "table":detail.name,"columns":detail.columns,"onclose":() => (designerOpen = false),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StructureView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StructureView__SvelteComponent_ = ReturnType<typeof StructureView__SvelteComponent_>;
/*Ωignore_endΩ*/export default StructureView__SvelteComponent_;