<script lang="ts">
  // Object structure: a columns table (name/type/nullable/default/key), primary
  // key, indexes, foreign keys, and a collapsible DDL block. For Redis keys /
  // Mongo collections (no columns) it renders the `extra` JSON.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import TableDesigner from './TableDesigner.svelte';
  import JsonTree from './JsonTree.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { DbForeignKey, DbIndexDef, SchemaNode } from '../../lib/api/types';

  const detail = $derived(database.objectDetail);
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
      await navigator.clipboard.writeText(detail.ddl);
      toasts.success('Copied DDL');
    } catch {
      toasts.error('Copy failed');
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
  async function copyText(text: string, label: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      toasts.success(`Copied ${label}`);
    } catch {
      toasts.error('Copy failed');
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
    return engine === 'postgres'
      ? `"${name.replace(/"/g, '""')}"`
      : '`' + name.replace(/`/g, '``') + '`';
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
    return `'${String(v).replace(/'/g, "''")}'`;
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
    if (engine === 'mysql') {
      // MySQL exposes the PK as an index literally named PRIMARY; it has its
      // own verb and can't be named in DROP INDEX.
      return idx.name === 'PRIMARY'
        ? `ALTER TABLE ${q(obj)} DROP PRIMARY KEY;`
        : `ALTER TABLE ${q(obj)} DROP INDEX ${q(idx.name)};`;
    }
    if (engine === 'postgres') {
      return backsPrimaryKey(idx)
        ? `ALTER TABLE ${q(obj)} DROP CONSTRAINT ${q(idx.name)};`
        : `DROP INDEX ${q(idx.name)};`;
    }
    return `ALTER TABLE ${q(obj)} DROP INDEX ${q(idx.name)};`;
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
      return `${mysqlPartialNote}\nCREATE ${kind}INDEX ${q(name)} ON ${q(obj)} (${parts.join(', ')});`;
    }
    const where = pred && engine === 'postgres' ? ` WHERE ${pred}` : '';
    return `CREATE ${kind}INDEX ${q(name)} ON ${q(obj)}${using} (${cols.map(q).join(', ')})${where};`;
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
  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    const u = ['KB', 'MB', 'GB', 'TB'];
    let v = n / 1024;
    let i = 0;
    while (v >= 1024 && i < u.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v.toFixed(1)} ${u[i]}`;
  }
  const mongoStats = $derived.by(() => {
    const s = (detail?.extra as Record<string, unknown> | null)?.stats as
      | Record<string, unknown>
      | undefined;
    if (!s) return null;
    return Object.entries(s).map(
      ([k, v]) =>
        [k, SIZE_KEYS.has(k) && typeof v === 'number' ? fmtBytes(v) : String(v)] as [string, string],
    );
  });
</script>

<div class="structure">
  {#if database.objectLoading}
    <div class="loading"><Icon name="refresh" size={16} /><span>Loading structure…</span></div>
  {:else if !detail}
    <EmptyState icon="box" title="No object selected" body="Pick a table, view, collection or key from the schema tree to inspect its structure." />
  {:else}
    <div class="st-head">
      <div class="st-title">
        <Icon name={titleIcon} size={15} />
        <h2 class="mono">{detail.name}</h2>
        <span class="kind-chip">{detail.kind}</span>
        {#if detail.row_count != null}
          <span class="rowcount">{detail.row_count.toLocaleString()} rows</span>
        {/if}
      </div>
      <div class="st-head-actions">
        {#if isSql && detail.kind === 'table'}
          <button class="btn small ghost" onclick={() => (designerOpen = true)} title="Edit columns → generates ALTER TABLE for review">
            <Icon name="edit" size={11} />Design
          </button>
        {/if}
        <button class="btn small ghost" onclick={explain}><Icon name="zap" size={11} />Explain</button>
      </div>
    </div>

    {#if detail.columns.length > 0}
      <div class="block">
        <div class="block-title">{isRoutine ? 'Parameters' : 'Columns'} <span class="count">{detail.columns.length}</span></div>
        <div class="tbl-wrap">
          <table class="tbl mono">
            <thead>
              <tr><th>Name</th><th>Type</th><th>Null</th><th>Key</th><th>Default</th><th>Extra</th></tr>
            </thead>
            <tbody>
              {#each detail.columns as c, i (i)}
                <tr>
                  <td class="cn">
                    {c.name}
                    {#if detail.primary_key.includes(c.name)}<span class="pk" title="Primary key">PK</span>{/if}
                  </td>
                  <td class="ty">{c.data_type}</td>
                  <td class="nullable">{c.nullable ? 'YES' : 'NO'}</td>
                  <td>{c.key ?? ''}</td>
                  <td class="dim">{c.default ?? ''}</td>
                  <td class="dim">{c.extra ?? ''}</td>
                </tr>
                {#if c.comment}
                  <tr class="comment-row"><td></td><td colspan="5" class="comment">{c.comment}</td></tr>
                {/if}
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    {/if}

    {#if mongoFields.length > 0}
      <!-- Mongo's stand-in for a Columns table: the sampled field paths, EMBEDDED
           ones included, each indexable in one click. Without this the only way to
           discover `a.b.c` was to open the index builder and guess. -->
      <div class="block">
        <div class="block-title">
          Fields <span class="count">{mongoFields.length}</span>
          <span class="hint dim">sampled</span>
          <span class="grow"></span>
          <input
            class="ib-search"
            type="search"
            bind:value={fieldQuery}
            placeholder="Filter fields…"
            spellcheck="false"
          />
        </div>
        <div class="tbl-wrap">
          <table class="tbl mono">
            <thead>
              <tr><th>Path</th><th>Type</th><th>Indexed</th><th></th></tr>
            </thead>
            <tbody>
              {#each shownMongoFields as f (f.path)}
                <tr>
                  <td class="cn">
                    {f.path}
                    {#if f.nested}<span class="nested-tag" title="Embedded field path">nested</span>{/if}
                  </td>
                  <td class="ty">{f.type}</td>
                  <td>
                    {#if f.indexed}<span class="pk" title="Named by an existing index">yes</span>{/if}
                  </td>
                  <td class="fld-act">
                    {#if canIndex}
                      <button class="mini-btn" onclick={() => indexField(f.path)}>
                        <Icon name="plus" size={10} />Index
                      </button>
                    {/if}
                  </td>
                </tr>
              {/each}
              {#if shownMongoFields.length === 0}
                <tr><td colspan="4" class="dim">No field matches “{fieldQuery}”.</td></tr>
              {/if}
            </tbody>
          </table>
        </div>
      </div>
    {/if}

    {#if detail.primary_key.length > 0}
      <div class="block">
        <div class="block-title">Primary key</div>
        <div class="chips">
          {#each detail.primary_key as col (col)}<span class="key-chip mono">{col}</span>{/each}
        </div>
      </div>
    {/if}

    {#if detail.indexes.length > 0 || canIndex}
      <div class="block">
        <div class="block-title">
          Indexes <span class="count">{detail.indexes.length}</span>
          <span class="grow"></span>
          {#if canIndex}
            <button class="mini-btn" onclick={startNewIndex}>
              <Icon name="plus" size={11} />New index
            </button>
          {/if}
        </div>
        {#if detail.indexes.length > 0}
          <ul class="idx-list">
            {#each detail.indexes as idx, i (i)}
              {@const defText = idxDefText(idx)}
              <li class="idx-item">
                <div class="idx-row">
                  <button
                    class="idx"
                    class:expandable={defText != null}
                    disabled={defText == null}
                    title={defText != null ? 'View full definition' : undefined}
                    onclick={() => (openIdxDef = openIdxDef === i ? null : i)}
                  >
                    <Icon name="key" size={11} />
                    <span class="idx-name mono" title={idx.name}>{idx.name}</span>
                    {#if idx.unique}<span class="tag unique">unique</span>{/if}
                    {#if idx.method}<span class="tag">{idx.method}</span>{/if}
                    <span class="idx-cols mono">({idx.columns.join(', ')})</span>
                    {#if defText != null}
                      <span class="grow"></span>
                      <Icon name={openIdxDef === i ? 'chevronDown' : 'chevronRight'} size={10} />
                    {/if}
                  </button>
                  {#if canIndex}
                    {@const locked = isProtectedIdx(idx)}
                    <div class="idx-acts">
                      <button
                        class="idx-act"
                        aria-label="Edit index {idx.name}"
                        disabled={locked}
                        title={locked
                          ? "MongoDB's _id_ index can't be changed"
                          : 'Edit — prepares a drop + recreate for you to review and run'}
                        onclick={() => editIndex(idx)}
                      >
                        <Icon name="edit" size={11} />
                      </button>
                      <button
                        class="idx-act danger"
                        aria-label="Drop index {idx.name}"
                        disabled={locked}
                        title={locked
                          ? "MongoDB's _id_ index can't be dropped"
                          : 'Drop — prepares the statement for you to review and run'}
                        onclick={() => dropIndex(idx)}
                      >
                        <Icon name="trash" size={11} />
                      </button>
                    </div>
                  {/if}
                </div>
                {#if openIdxDef === i && defText != null}
                  {@const snippet = idxCreateSnippet(idx)}
                  <div class="idx-def">
                    <div class="idx-def-actions">
                      {#if snippet}
                        <button class="copy-ddl" onclick={() => copyText(snippet, 'createIndex')}>
                          <Icon name="file" size={11} />Copy createIndex
                        </button>
                      {/if}
                      <button class="copy-ddl" onclick={() => copyText(defText, 'definition')}>
                        <Icon name="file" size={11} />Copy
                      </button>
                    </div>
                    <pre class="ddl mono">{defText}</pre>
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        {#if idxOpen}
          <div class="idx-builder">
            {#if idxEditing}
              <div class="ib-editing">
                Editing <span class="mono">{idxEditing.name}</span> — no engine alters an index in
                place, so this prepares a <strong>drop + recreate</strong>.
              </div>
            {/if}
            <div class="ib-section">
              <div class="ib-label">
                Fields
                <span class="ib-count">{idxCols.length} selected</span>
                <span class="grow"></span>
                <input
                  class="ib-search"
                  type="search"
                  bind:value={idxFieldQuery}
                  placeholder="Filter fields…"
                  spellcheck="false"
                />
              </div>
              <div class="ib-list">
                {#each idxCols as f, i (f)}
                  <!-- Row, not a single button: the direction toggle is its own
                       control and a <button> can't nest inside a <button>. -->
                  <div class="ib-row on">
                    <button class="ib-pick" onclick={() => toggleIdxCol(f)}>
                      <span class="ib-ord">{i + 1}</span>
                      <span class="ib-fname mono">{f}</span>
                    </button>
                    <span class="grow"></span>
                    {#if !isSql}
                      <button
                        class="ib-dir"
                        title="Key direction — ascending (1) or descending (-1)"
                        onclick={() => flipIdxDir(f)}
                      >
                        {idxDirOf(f) === 1 ? '↑ 1' : '↓ -1'}
                      </button>
                    {/if}
                    <button class="ib-x" aria-label="Remove {f}" onclick={() => toggleIdxCol(f)}>
                      <Icon name="x" size={10} />
                    </button>
                  </div>
                {/each}
                {#if idxCols.length > 0 && (idxRestFields.length > 0 || customIdxPath)}
                  <div class="ib-sep"></div>
                {/if}
                {#if customIdxPath}
                  <!-- Escape hatch: index a path the sampler never saw. -->
                  <button class="ib-row custom" onclick={() => toggleIdxCol(customIdxPath)}>
                    <span class="ib-ord">+</span>
                    <span class="ib-fname mono">{customIdxPath}</span>
                    <span class="grow"></span>
                    <span class="ib-custom-tag">use this path</span>
                  </button>
                {/if}
                {#each idxRestFields as f (f)}
                  <button class="ib-row" onclick={() => toggleIdxCol(f)}>
                    <span class="ib-ord"></span>
                    <span class="ib-fname mono">{f}</span>
                  </button>
                {/each}
                {#if idxCols.length === 0 && idxRestFields.length === 0 && !customIdxPath}
                  <div class="ib-empty dim">No field matches “{idxFieldQuery}”.</div>
                {/if}
              </div>
            </div>

            {#if canCondition}
              <div class="ib-section">
                <div class="ib-label">
                  Condition
                  <span class="ib-count">
                    {engine === 'mongodb' ? 'partial filter' : 'partial index'}
                  </span>
                  <span class="grow"></span>
                  <button class="mini-btn" onclick={addIdxCond}>
                    <Icon name="plus" size={10} />Add condition
                  </button>
                </div>
                {#each idxConds as cond, ci (ci)}
                  <div class="ib-cond">
                    <select class="mono" bind:value={cond.field}>
                      {#each idxCondFields as f (f)}<option value={f}>{f}</option>{/each}
                    </select>
                    <select bind:value={cond.op}>
                      <option value="exists">exists</option>
                      <option value="in">in</option>
                    </select>
                    {#if cond.op === 'in'}
                      <input
                        class="mono"
                        type="text"
                        bind:value={cond.values}
                        placeholder="a, b, 3, true"
                        spellcheck="false"
                      />
                    {:else}
                      <span class="ib-cond-hint dim">
                        {engine === 'mongodb' ? '$exists: true' : 'IS NOT NULL'}
                      </span>
                    {/if}
                    <button
                      class="idx-act danger"
                      aria-label="Remove condition {ci + 1}"
                      onclick={() => removeIdxCond(ci)}
                    >
                      <Icon name="trash" size={11} />
                    </button>
                  </div>
                {/each}
                {#if engine === 'mysql' && usableIdxConds.length > 0}
                  <div class="ib-warn">
                    MySQL has no partial index — this is emulated with a <strong>functional key
                    part</strong> over a <code>CASE</code>. The optimizer uses it only for queries
                    written with the same expression.
                  </div>
                {/if}
                {#if engine === 'mongodb' && usableIdxConds.length > 0 && idxEditing?.definition && typeof idxEditing.definition === 'object' && (idxEditing.definition as Record<string, unknown>).sparse === true}
                  <div class="ib-warn">
                    <code>sparse</code> will be dropped — MongoDB rejects an index that sets both
                    <code>sparse</code> and <code>partialFilterExpression</code>.
                  </div>
                {/if}
                {#if idxCondExtraMongo}
                  <div class="ib-warn">
                    This index has filter terms this builder can't edit
                    (<span class="mono">{Object.keys(idxCondExtraMongo).join(', ')}</span>) — they're
                    preserved as-is.
                  </div>
                {/if}
                {#if idxCondRawSql}
                  <div class="ib-warn">
                    Existing predicate kept as-is:
                    <span class="mono">{idxCondRawSql}</span>{usableIdxConds.length > 0
                      ? ' — replaced by the condition(s) above.'
                      : ''}
                  </div>
                {/if}
              </div>
            {/if}

            <label class="ib-name">
              Name
              <input
                class="mono"
                type="text"
                bind:value={idxName}
                placeholder={suggestedIdxName || 'idx_…'}
                spellcheck="false"
              />
            </label>
            <label class="ib-unique"><input type="checkbox" bind:checked={idxUnique} /> Unique</label>
            <div class="ib-actions">
              <button class="btn small" onclick={resetIdxBuilder}>Cancel</button>
              <button class="btn small primary" disabled={idxCols.length === 0} onclick={buildIndex}>
                {#if idxEditing}
                  Prepare drop + recreate →
                {:else}
                  Prepare {isSql ? 'CREATE INDEX' : 'createIndex'} →
                {/if}
              </button>
            </div>
            <div class="ib-hint dim">Opens the statement in a query tab for you to review and run.</div>
          </div>
        {/if}
      </div>
    {/if}

    {#if mongoStats}
      <div class="block">
        <div class="block-title">Stats</div>
        <div class="stats-grid">
          {#each mongoStats as [k, v] (k)}
            <div class="stat"><span class="sk mono">{k}</span><span class="sv mono">{v}</span></div>
          {/each}
        </div>
      </div>
    {/if}

    {#if detail.foreign_keys.length > 0}
      <div class="block">
        <div class="block-title">Foreign keys <span class="count">{detail.foreign_keys.length}</span></div>
        <ul class="fk-list">
          {#each detail.foreign_keys as fk, i (i)}
            <li class="fk">
              <span class="fk-name mono">{fk.name}</span>
              <span class="fk-map mono">
                ({fk.columns.join(', ')})
                <Icon name="arrowDown" size={10} />
                <button
                  class="fk-ref-btn mono"
                  title="Open {fk.ref_schema ? `${fk.ref_schema}.` : ''}{fk.ref_table}"
                  onclick={() => navigateToFkTable(fk)}
                >{fk.ref_schema ? `${fk.ref_schema}.` : ''}{fk.ref_table}</button>({fk.ref_columns.join(', ')})
              </span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if detail.extra != null && detail.columns.length === 0}
      <div class="block">
        <div class="block-title">
          Details
          <span class="grow"></span>
          <button class="copy-ddl" onclick={() => copyText(prettyExtra(detail.extra), 'details')}>
            <Icon name="file" size={11} />Copy
          </button>
        </div>
        <!-- Collapsible: `extra.sample` carries a WHOLE document, which for a
             fat collection is ~88KB — stringifying it into a <pre> made this
             panel a wall of text and buried the sampled paths. -->
        <div class="extra-tree mono"><JsonTree value={detail.extra} /></div>
      </div>
    {/if}

    {#if detail.ddl}
      <div class="block">
        <div class="ddl-head">
          <button class="block-title toggle" onclick={() => (ddlOpen = !ddlOpen)}>
            <Icon name={ddlOpen ? 'chevronDown' : 'chevronRight'} size={11} />
            DDL
          </button>
          <span class="grow"></span>
          {#if ddlOpen}
            <button class="copy-ddl" onclick={copyDdl}>
              <Icon name="file" size={11} />Copy
            </button>
          {/if}
        </div>
        {#if ddlOpen}
          <pre class="ddl mono">{detail.ddl}</pre>
        {/if}
      </div>
    {:else if isRoutine}
      <!-- MySQL blanks the "Create …" column when the account can't view the
           routine body — surface that as a privilege hint, not a blank panel. -->
      <div class="block">
        <div class="block-title">Definition</div>
        <div class="ddl-missing">
          The routine body isn't available — the connected account likely lacks
          privilege to view routine definitions (needs <code>SHOW_ROUTINE</code>, or
          <code>SELECT</code> on the routine).
        </div>
      </div>
    {/if}
  {/if}
</div>

{#if designerOpen && detail && detail.kind === 'table'}
  <TableDesigner
    table={detail.name}
    columns={detail.columns}
    onclose={() => (designerOpen = false)}
  />
{/if}

<style>
  .structure {
    height: 100%;
    overflow-y: auto;
    padding: 4px 2px 24px;
  }
  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 24px;
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .st-head-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
  .st-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 14px;
  }
  .st-title {
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--accent);
  }
  .st-title h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
  }
  .kind-chip {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    background: var(--surface-2);
    padding: 1px 7px;
    border-radius: 999px;
  }
  .rowcount {
    font-size: 11px;
    color: var(--text-dim);
  }
  .block {
    margin-bottom: 18px;
  }
  .block-title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    margin-bottom: 8px;
  }
  .grow {
    flex: 1;
  }
  .mini-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11px;
    cursor: pointer;
    text-transform: none;
    letter-spacing: 0;
  }
  .mini-btn:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
  }
  .idx-builder {
    margin-top: 8px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .ib-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ib-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .ib-count {
    font-weight: 500;
    text-transform: none;
    letter-spacing: 0;
  }
  .ib-search {
    width: 180px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 11.5px;
    text-transform: none;
    letter-spacing: 0;
  }
  .ib-search:focus {
    outline: none;
    border-color: var(--accent);
  }
  /* A real list, not a tag cloud: a Mongo collection routinely samples 60+ dotted
     paths, and wrapped chips make those unscannable. Selected fields pin to the
     top IN KEY ORDER, which is the part of an index that actually matters. */
  .ib-list {
    display: flex;
    flex-direction: column;
    max-height: 220px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
  }
  .ib-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 11.5px;
    text-align: start;
    cursor: pointer;
  }
  .ib-row:hover {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .ib-row.on {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .ib-fname {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Fixed-width gutter so the selected rows' key positions line up with the
     unselected names below them. */
  .ib-ord {
    flex-shrink: 0;
    width: 16px;
    text-align: end;
    font-size: 10px;
    font-weight: 700;
    color: var(--accent);
  }
  /* The selected row is a flex CONTAINER now (name + direction + remove), so the
     name itself is the clickable part rather than the whole row. */
  .ib-pick {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .ib-dir {
    flex-shrink: 0;
    padding: 1px 6px;
    font-size: 10px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .ib-dir:hover {
    background: color-mix(in srgb, var(--accent) 26%, transparent);
  }
  .ib-x {
    display: inline-flex;
    flex-shrink: 0;
    padding: 2px;
    color: inherit;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .ib-x:hover {
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
  }
  .ib-row.custom .ib-ord {
    color: var(--ok, #3fb950);
  }
  .ib-custom-tag {
    flex-shrink: 0;
    font-size: 10px;
    color: var(--ok, #3fb950);
  }
  .ib-sep {
    height: 1px;
    margin: 3px 0;
    background: var(--border);
  }
  /* Mongo fields table */
  .nested-tag {
    margin-left: 6px;
    padding: 0 4px;
    font-size: 9.5px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    border-radius: var(--radius-s);
  }
  .fld-act {
    text-align: end;
    white-space: nowrap;
  }
  .hint {
    font-size: 10px;
    font-weight: 400;
  }
  .ib-empty {
    padding: 8px;
    font-size: 11.5px;
  }
  .ib-cond {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .ib-cond select,
  .ib-cond input {
    height: 24px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 11.5px;
  }
  .ib-cond select:first-child {
    max-width: 260px;
  }
  .ib-cond input {
    flex: 1;
    min-width: 120px;
    max-width: 320px;
  }
  .ib-cond select:focus,
  .ib-cond input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .ib-cond-hint {
    font-size: 11px;
    font-family: var(--font-mono, monospace);
  }
  .ib-warn {
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-dim);
    border-inline-start: 2px solid var(--danger, #e5534b);
    padding-inline-start: 8px;
  }
  .ib-warn code {
    font-size: 10.5px;
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    padding: 0 3px;
    border-radius: 3px;
  }
  .ib-unique,
  .ib-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text);
  }
  .ib-name input {
    flex: 1;
    min-width: 0;
    max-width: 320px;
    height: 24px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 11.5px;
  }
  .ib-name input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .ib-editing {
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-dim);
    border-inline-start: 2px solid var(--accent);
    padding-inline-start: 8px;
  }
  .ib-actions {
    display: flex;
    gap: 6px;
  }
  .ib-hint {
    font-size: 11px;
  }
  .stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 6px;
  }
  .stat {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: 11.5px;
  }
  .stat .sk {
    color: var(--text-dim);
  }
  .ddl-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .block-title.toggle {
    border: none;
    background: transparent;
    cursor: pointer;
    padding: 4px 0;
    text-align: start;
    color: var(--text-dim);
  }
  .copy-ddl {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    color: var(--text-dim);
    cursor: pointer;
    border: none;
    background: transparent;
    font-weight: 500;
  }
  .copy-ddl:hover {
    color: var(--accent);
  }
  .count {
    color: var(--text-dim);
    font-weight: 500;
  }
  .tbl-wrap {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: auto;
  }
  .tbl {
    width: 100%;
    border-collapse: collapse;
    user-select: text;
  }
  .tbl th {
    text-align: start;
    padding: 6px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .tbl td {
    padding: 5px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    font-size: 11.5px;
    vertical-align: top;
  }
  .tbl tbody tr:hover td {
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }
  .cn {
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
  }
  .ty {
    color: #0e8a8a;
  }
  :global(html[data-scheme='dark']) .ty {
    color: #56c8d8;
  }
  .nullable {
    color: var(--text-dim);
    font-size: 10.5px;
  }
  .pk {
    margin-inline-start: 6px;
    font-size: 9px;
    font-weight: 700;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    padding: 0 4px;
    border-radius: 3px;
    vertical-align: middle;
  }
  .comment-row td {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .comment {
    color: var(--text-dim);
    font-style: italic;
    font-size: 11px;
  }
  .chips,
  .idx-list,
  .fk-list {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .chips {
    flex-direction: row;
    flex-wrap: wrap;
  }
  .key-chip {
    font-size: 11.5px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 2px 9px;
    border-radius: 999px;
    color: var(--text);
  }
  /* The row carries the chrome; the expand button and the Edit/Drop actions are
     siblings inside it (a button can't nest buttons). */
  .idx-row,
  .fk {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: 11.5px;
    min-width: 0;
  }
  .idx-row:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .idx {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    background: transparent;
    border: none;
    padding: 0;
  }
  .idx-acts {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    /* Revealed on row hover/focus so the list stays scannable; always visible on
       touch, where there is no hover. */
    opacity: 0;
  }
  .idx-row:hover .idx-acts,
  .idx-acts:focus-within {
    opacity: 1;
  }
  @media (hover: none) {
    .idx-acts {
      opacity: 1;
    }
  }
  .idx-act {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 20px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .idx-act:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .idx-act.danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger, #e5534b) 18%, transparent);
    color: var(--danger, #e5534b);
  }
  .idx-act:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
  .idx-item {
    display: flex;
    flex-direction: column;
    /* Flex items refuse to shrink below content by default; without this a
       long auto-generated index name blows the whole section (and page) wide. */
    min-width: 0;
  }
  /* The index summary is a button so the full definition can expand under it.
     Indexes without a definition render identically but aren't clickable. */
  button.idx {
    min-width: 0;
    font: inherit;
    font-size: 11.5px;
    color: var(--text-dim);
    text-align: start;
    cursor: default;
  }
  button.idx.expandable {
    cursor: pointer;
  }
  .idx-def {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 4px;
    padding-inline-start: 10px;
  }
  .idx-def-actions {
    display: flex;
    justify-content: flex-end;
    gap: 14px;
  }
  .idx-name,
  .fk-name {
    font-weight: 600;
    color: var(--text);
    /* A Mongo compound-index name is one huge unbreakable token — ellipsize it
       instead of letting it push the row (and page) sideways. The expandable
       definition below always shows the full name. */
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .idx-cols,
  .fk-map {
    color: var(--text-dim);
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow: hidden;
  }
  .tag {
    font-size: 9.5px;
    text-transform: uppercase;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    padding: 0 5px;
    border-radius: 999px;
  }
  .tag.unique {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .ddl {
    margin: 0;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px;
    font-size: 11.5px;
    line-height: 1.5;
    overflow: auto;
    max-height: 360px;
    user-select: text;
    white-space: pre;
  }
  /* Same chrome as .ddl, but the tree wraps and owns its own indentation. */
  .extra-tree {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 12px;
    overflow: auto;
    max-height: 420px;
    user-select: text;
  }
  .ddl-missing {
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px 12px;
  }
  .ddl-missing code {
    font-size: 11px;
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    padding: 0 4px;
    border-radius: 3px;
  }
  .dim {
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  /* Clickable FK reference table — looks like a link but respects the mono row. */
  .fk-ref-btn {
    border: none;
    background: transparent;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    font-size: inherit;
    text-decoration: underline;
    text-underline-offset: 2px;
    text-decoration-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .fk-ref-btn:hover {
    text-decoration-color: var(--accent);
  }
</style>
