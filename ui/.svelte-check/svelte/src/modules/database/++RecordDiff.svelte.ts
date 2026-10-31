///<reference types="svelte" />
;
  // Side-by-side diff of two records (Vertical / JSON views' "Compare"): both
  // are flattened to dotted leaf paths (`flattenPaths` — arrays by index, BSON
  // sentinels as scalars) and lined up path by path, so a changed `items.2.qty`
  // is one row rather than two walls of JSON. "Only differences" hides the
  // unchanged rows; "Copy as JSON patch" copies the ops that turn LEFT into
  // RIGHT (`set` / `unset` per leaf path) for a hand-written update. The two
  // pure helpers live in the module script so they are importable (and
  // testable) without mounting the modal.
  import { flattenPaths } from '../../lib/jsonTree/expansion-plan';
  import { cellStr } from './results-format';

  export type DiffKind = 'same' | 'changed' | 'only-left' | 'only-right';
  export interface DiffRow {
    path: string;
    kind: DiffKind;
    l: unknown;
    r: unknown;
  }
  export type PatchOp = { op: 'set'; path: string; value: unknown } | { op: 'unset'; path: string };

  /** Leaf equality: same rendered text AND same JS type, so `1` vs `"1"` and
   *  `null` vs `"null"` count as changes. */
  function sameLeaf(a: unknown, b: unknown): boolean {
    return typeof a === typeof b && cellStr(a) === cellStr(b);
  }

  /** One row per leaf path of either record — left's order first, then the
   *  paths only the right one has. */
  export function diffRecords(left: unknown, right: unknown): DiffRow[] {
    const L = flattenPaths(left);
    const R = flattenPaths(right);
    const out: DiffRow[] = [];
    for (const [path, l] of L) {
      if (!R.has(path)) out.push({ path, kind: 'only-left', l, r: undefined });
      else out.push({ path, kind: sameLeaf(l, R.get(path)) ? 'same' : 'changed', l, r: R.get(path) });
    }
    for (const [path, r] of R) if (!L.has(path)) out.push({ path, kind: 'only-right', l: undefined, r });
    return out;
  }

  /** `[{op:'set',path,value} | {op:'unset',path}]` turning LEFT into RIGHT. */
  export function recordPatch(rows: DiffRow[]): PatchOp[] {
    return rows
      .filter((r) => r.kind !== 'same')
      .map((r) => (r.kind === 'only-left' ? { op: 'unset', path: r.path } : { op: 'set', path: r.path, value: r.r }));
  }
;;

import { plural, pluralNoun } from '../../lib/plural';
import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import { copyWithToast } from './results-format';
;

  interface Props {
    left: Record<string, unknown>;
    right: Record<string, unknown>;
    leftLabel: string;
    rightLabel: string;
    onclose: () => void;
  };function $$render() {

  
  
  
  

  let { left, right, leftLabel, rightLabel, onclose }: Props = $props();

  let onlyDiff = $state(true);

  const rows = $derived(diffRecords(left, right));
  const diffCount = $derived(rows.filter((r) => r.kind !== 'same').length);
  const shown = $derived(onlyDiff ? rows.filter((r) => r.kind !== 'same') : rows);

  function show(v: unknown, present: boolean): string {
    if (!present) return '—';
    return v === null || v === undefined ? '∅' : cellStr(v);
  }

  function copyPatch(): void {
    const ops = recordPatch(rows);
    void copyWithToast(JSON.stringify(ops, null, 2), ['Copied', `JSON patch with ${plural(ops.length, 'op')}`]);
  }
;
async () => {



 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Compare records`,"width":900,onclose,}});
   { svelteHTML.createElement("div", {   "class":`rd`,...__sveltets_2_empty({"data-testid":`record-diff`}),});
     { svelteHTML.createElement("div", { "class":`rd-bar`,});
       { svelteHTML.createElement("span", { "class":`rd-count`,});
        if(diffCount === 0){      }else{ { svelteHTML.createElement("strong", {});diffCount; } pluralNoun(diffCount, 'difference');  plural(rows.length, 'field');}
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("label", { "class":`rd-only`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":onlyDiff,});/*Ωignore_startΩ*/() => onlyDiff = __sveltets_2_any(null);/*Ωignore_endΩ*/}   }
       { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":diffCount === 0,"onclick":copyPatch,"title":`Copy the set/unset operations that turn ${leftLabel} into ${rightLabel}`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}    }
     }
     { svelteHTML.createElement("div", { "class":`rd-scroll`,});
       { svelteHTML.createElement("table", { "class":`rd-table mono`,});
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "class":`rd-path`,});  } { svelteHTML.createElement("th", {});leftLabel; } { svelteHTML.createElement("th", {});rightLabel; } }
         }
         { svelteHTML.createElement("tbody", {});
             for(let row of __sveltets_2_ensureArray(shown)){row.path;
             { svelteHTML.createElement("tr", { "class":row.kind,});
               { svelteHTML.createElement("td", {   "class":`rd-path`,"title":row.path,});row.path; }
               { svelteHTML.createElement("td", { "class":`rd-val`,});show(row.l, row.kind !== 'only-right'); }
               { svelteHTML.createElement("td", { "class":`rd-val`,});show(row.r, row.kind !== 'only-left'); }
             }
          }
             { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", {   "class":`rd-empty`,"colspan":3,});   } }
          
         }
       }
     }
   }
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RecordDiff__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RecordDiff__SvelteComponent_ = ReturnType<typeof RecordDiff__SvelteComponent_>;
/*Ωignore_endΩ*/export default RecordDiff__SvelteComponent_;