// "Add row" for the results views: WHICH table a new row goes to (`target`)
// and the schema-derived form that builds its INSERT (`form`, rendered by
// InsertRowForm.svelte). One instance per EditFlow (`flow.insert`); the built
// statement goes through the flow's normal review modal — nothing runs here.
//
// The target is deliberately NOT the `editable` gate: inserting addresses no
// existing row, so it needs neither a primary key nor any result ROWS — an
// EMPTY table (no rows; for Mongo not even columns) is exactly where it
// matters. It does need write access and a single plain SELECT / find naming
// one table, and (SQL) a known database for the schema lookup. ClickHouse is
// included: INSERT is fine there, only row edits / deletes are not.
import { database } from '../../lib/stores/database.svelte';
import { parseSimpleSelect, tableRef } from './edit-sql';
import { mongoCollectionForEdit } from './edit-mongo';
import { buildFormInsert, fieldsFromResult, fieldsFromSchema, type InsertField } from './insert-form';
import type { EditFlow } from './EditFlow.svelte';

export interface InsertFormState {
  table: string;
  db: string | null;
  fields: InsertField[];
  loading: boolean;
  /** The schema couldn't be loaded and there is no fallback (Retry). */
  loadError: string | null;
  /** Fields came from the result's column names only — types/defaults unknown. */
  partial: boolean;
  err: string | null;
}

export class InsertFlow {
  private readonly flow: EditFlow;
  constructor(flow: EditFlow) {
    this.flow = flow;
  }

  readonly target: { db: string | null; table: string } | null = $derived.by(() => {
    const f = this.flow;
    const sql = f.statement;
    if (!sql || !f.connectionId || !f.result || !f.canModify || f.mini) return null;
    if (f.resultCount > 1 || /;\s*\S/.test(sql.trim().replace(/;\s*$/, ''))) return null;
    if (f.engine === 'redis') return null;
    if (f.engine === 'mongodb') {
      const coll = mongoCollectionForEdit(sql);
      return coll ? { db: f.ranDb(), table: coll } : null;
    }
    if (database.capabilities?.sql !== true) return null;
    const parsed = parseSimpleSelect(sql, f.engine);
    return parsed ? { db: parsed.db ?? f.defaultSqlDb(), table: parsed.table } : null;
  });

  form: InsertFormState | null = $state(null);
  private gen = 0;

  /** Open the insert UI for the current target: the JSON document editor on
   *  Mongo (schemaless), the schema form on SQL engines. */
  async open(): Promise<void> {
    const target = this.target;
    const result = this.flow.result;
    if (!target || !result) return;
    if (this.flow.engine === 'mongodb') {
      this.flow.openDocEditor(-1);
      return;
    }
    const gen = ++this.gen;
    this.form = { ...target, fields: [], loading: true, loadError: null, partial: false, err: null };
    const detail = target.db ? await database.fetchObject(`db:${target.db}/table:${target.table}`) : null;
    if (gen !== this.gen || !this.form) return;
    if (detail && detail.columns.length > 0) {
      this.form = { ...this.form, fields: fieldsFromSchema(detail.columns, detail.primary_key), loading: false };
    } else if (result.columns.length > 0) {
      this.form = { ...this.form, fields: fieldsFromResult(result.columns), loading: false, partial: true };
    } else {
      this.form = {
        ...this.form,
        loading: false,
        loadError: target.db
          ? `Couldn’t load the columns of “${target.table}”.`
          : `Select a database (or qualify the table as db.${target.table}) and re-run the query to add rows.`,
      };
    }
  }

  close(): void {
    this.gen++;
    this.form = null;
  }

  /** Validate the form, build the INSERT and hand it to the review modal. */
  submit(): void {
    const form = this.form;
    if (!form || form.loading || form.loadError) return;
    const ref = tableRef(this.flow.ctx({ db: form.db, table: form.table, pkCols: [] }));
    const built = buildFormInsert(this.flow.engine, ref, form.fields);
    if ('error' in built) {
      form.err = built.error;
      return;
    }
    this.form = null;
    this.flow.openReview('Review INSERT', built.sql, built.diff);
  }
}
