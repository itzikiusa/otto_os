///<reference types="svelte" />
;// Workbench-style column designer for a SQL table. Edit column name / type /
// NOT NULL / default, add or drop columns, then "Prepare ALTER" — which opens
// the generated `ALTER TABLE …` in a query tab for the user to review and run
// (never auto-applied). Generation is engine-aware: MySQL uses CHANGE COLUMN
// clauses in one ALTER, Postgres a sequence of standard ALTER statements
// (RENAME / ALTER COLUMN TYPE / SET-DROP NOT NULL / SET DEFAULT), ClickHouse
// RENAME/MODIFY COLUMN (no FKs; nullability lives in the type).

import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import { database } from '../../lib/stores/database.svelte';
import type { DbColumnDef } from '../../lib/api/types';
import { qualifiedName } from '../../lib/database/sql-dialect';
import {
    designerSql,
    newRow,
    rowsFromColumns,
    type DesignerFk,
    type DesignerIndex,
    type DesignerRow,
  } from './table-designer-sql';
;

  interface Props {
    table: string;
    columns: DbColumnDef[];
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { table, columns, onclose }: Props = $props();

  // Engine of the active connection — decides quoting + ALTER dialect. The
  // designer is only reachable for SQL engines (StructureView gates the button).
  const engine = $derived(database.capabilities?.engine ?? 'mysql');
  // ClickHouse has no foreign keys and no plain ADD INDEX-with-columns (its
  // secondary indexes are data-skipping and need a TYPE) — hide those sections.
  const isClickhouse = $derived(engine === 'clickhouse');

  // Editable working copy of the table's columns. Re-seeded from the incoming
  // `columns` whenever the table being designed changes, so reopening the
  // designer on a different table never carries over the previous edits.
  let rows = $state<DesignerRow[]>([]);
  let indexes = $state<DesignerIndex[]>([]);
  let fks = $state<DesignerFk[]>([]);
  let seededFor = $state<string | null>(null);
  $effect(() => {
    if (seededFor !== table) {
      rows = rowsFromColumns(engine, columns);
      indexes = [];
      fks = [];
      seededFor = table;
    }
  });

  function addColumn(): void {
    rows = [...rows, newRow()];
  }
  function addIndex(): void {
    indexes = [...indexes, { name: '', cols: '', unique: false }];
  }
  function addFk(): void {
    fks = [...fks, { name: '', cols: '', refTable: '', refCols: '' }];
  }
  // The prepared statement runs in a query tab against the ACTIVE db, which
  // need not be this table's db — qualify from the selected object's path
  // (`db:{db}/table:{t}` or `schema:{s}/table:{t}`).
  const objSchema = $derived.by(() => {
    const seg = (database.selectedObjectPath ?? '')
      .split('/')
      .find((s) => s.startsWith('db:') || s.startsWith('schema:'));
    return seg ? seg.slice(seg.indexOf(':') + 1) : null;
  });
  const tableRef = $derived(qualifiedName(engine, objSchema, table));

  // Diff the edited rows against the originals → engine-correct ALTER SQL
  // (the generators live in `table-designer-sql.ts`, unit-tested).
  const sql = $derived(designerSql({ engine, tableRef, columns, rows, indexes, fks }));

  function apply(): void {
    if (sql) void database.openInNewTab(sql);
    onclose();
  }
;
async () => {


 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Design ${table}`,"width":860,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("span", { "class":`dim small td-note`,});                }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn small primary`,"disabled":!sql,"onclick":apply,});   }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`td-modal`,});
     { svelteHTML.createElement("div", { "class":`td-cols`,});
       { svelteHTML.createElement("div", { "class":`td-row td-hdr`,});
         { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("span", { "class":`ctr`,});  } { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("span", {}); }
       }
          for(let r of __sveltets_2_ensureArray(rows)){let i = 1;i;
         { svelteHTML.createElement("div", {  "class":`td-row`,});r.drop;
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Column name`,"class":`mono`,"bind:value":r.name,"placeholder":`name`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => r.name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Column type`,"class":`mono`,"bind:value":r.type,"placeholder":`type`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => r.type = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`ctr`,});
             { svelteHTML.createElement("input", {         "type":`checkbox`,"bind:checked":r.notNull,"disabled":isClickhouse,"title":isClickhouse ? 'ClickHouse nullability lives in the type — use Nullable(T)' : undefined,});/*Ωignore_startΩ*/() => r.notNull = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Column default`,"class":`mono`,"bind:value":r.def,"placeholder":`NULL`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => r.def = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"title":r.orig === null ? 'Remove' : r.drop ? 'Keep' : 'Drop column',"aria-label":`Drop column`,"onclick":() => {
              if (r.orig === null) rows = rows.filter((_, j) => j !== i);
              else r.drop = !r.drop;
            },}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
         }
      }
       { svelteHTML.createElement("button", {   "class":`td-add`,"onclick":addColumn,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
     }

    if(isClickhouse){
      
       { svelteHTML.createElement("div", { "class":`td-section`,});
         { svelteHTML.createElement("div", { "class":`td-section-title`,});    }
         { svelteHTML.createElement("div", { "class":`dim small`,});
                      
                
         }
       }
    }else{
    
     { svelteHTML.createElement("div", { "class":`td-section`,});
       { svelteHTML.createElement("div", { "class":`td-section-title`,});  }
          for(let ix of __sveltets_2_ensureArray(indexes)){let i = 1;i;
         { svelteHTML.createElement("div", { "class":`td-ix-row`,});
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Index name`,"class":`mono`,"bind:value":ix.name,"placeholder":`index name (optional)`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => ix.name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Index columns`,"class":`mono`,"bind:value":ix.cols,"placeholder":`columns (comma-separated)`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => ix.cols = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("label", {   "class":`td-chk`,"title":`Unique index`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":ix.unique,});/*Ωignore_startΩ*/() => ix.unique = __sveltets_2_any(null);/*Ωignore_endΩ*/}  }
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Remove index`,"aria-label":`Remove index`,"onclick":() => (indexes = indexes.filter((_, j) => j !== i)),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
         }
      }
       { svelteHTML.createElement("button", {   "class":`td-add`,"onclick":addIndex,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
     }

    
     { svelteHTML.createElement("div", { "class":`td-section`,});
       { svelteHTML.createElement("div", { "class":`td-section-title`,});  }
          for(let fk of __sveltets_2_ensureArray(fks)){let i = 1;i;
         { svelteHTML.createElement("div", { "class":`td-fk-row`,});
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Foreign key columns`,"class":`mono`,"bind:value":fk.cols,"placeholder":`column(s)`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fk.cols = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`td-fk-arrow`,});  }
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Referenced table`,"class":`mono`,"bind:value":fk.refTable,"placeholder":`referenced table`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fk.refTable = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Referenced columns`,"class":`mono`,"bind:value":fk.refCols,"placeholder":`referenced column(s)`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fk.refCols = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Foreign key name`,"class":`mono`,"bind:value":fk.name,"placeholder":`name (optional)`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => fk.name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Remove foreign key`,"aria-label":`Remove foreign key`,"onclick":() => (fks = fks.filter((_, j) => j !== i)),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
         }
      }
       { svelteHTML.createElement("button", {   "class":`td-add`,"onclick":addFk,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
     }
    }

    if(sql){
       { svelteHTML.createElement("pre", { "class":`td-preview mono`,});sql; }
    }else{
       { svelteHTML.createElement("div", { "class":`td-nochange dim`,});   }
    }

   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TableDesigner__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TableDesigner__SvelteComponent_ = ReturnType<typeof TableDesigner__SvelteComponent_>;
/*Ωignore_endΩ*/export default TableDesigner__SvelteComponent_;