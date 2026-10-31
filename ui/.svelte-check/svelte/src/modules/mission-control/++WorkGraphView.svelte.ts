///<reference types="svelte" />
;
import type { GraphView, GraphNode } from '../../lib/api/types';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { sentenceCase } from '../../lib/status';
import { KIND_LABEL, STATUS_LABEL, WORK_KINDS, statusColor, statusIcon } from './lib';
;

  interface Props {
    graph: GraphView;
    selectedId: string | null;
    onOpen: (id: string) => void;
    /** A refetch is in flight over a graph that is already on screen. */
    busy?: boolean;
  };;

  type Placed = { x: number; y: number; r: number; node: GraphNode };;function $$render() {

  
  
  
  
  

  let { graph, selectedId, onOpen, busy = false }: Props = $props();

  const COL_W = 200;
  const ROW_H = 96;
  const PAD = 48;


  const layout = $derived.by(() => {
    const byKind = new Map<string, GraphNode[]>();
    for (const n of graph.nodes) {
      const arr = byKind.get(n.kind) ?? [];
      arr.push(n);
      byKind.set(n.kind, arr);
    }
    const cols = WORK_KINDS.filter((k) => byKind.has(k));
    const pos = new Map<string, Placed>();
    let maxRows = 0;
    cols.forEach((k, ci) => {
      const arr = byKind.get(k) ?? [];
      maxRows = Math.max(maxRows, arr.length);
      arr.forEach((n, ri) => {
        const cost = n.cost_so_far > 0 ? n.cost_so_far : 0;
        const r = 13 + Math.min(9, Math.sqrt(cost) * 3.2);
        pos.set(n.id, { x: PAD + ci * COL_W + 66, y: PAD + 34 + ri * ROW_H, r, node: n });
      });
    });
    const width = Math.max(440, cols.length * COL_W + PAD * 2);
    const height = Math.max(260, maxRows * ROW_H + PAD * 2);
    const edges = graph.edges.filter((e) => pos.has(e.from_item_id) && pos.has(e.to_item_id));
    return { cols, pos: Array.from(pos.values()), posMap: pos, width, height, edges };
  });

  function trunc(s: string, n = 20): string {
    return s.length > n ? s.slice(0, n - 1) + '…' : s;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`graph-wrap`,});
  if(graph.nodes.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`share`,"title":`Nothing to graph`,"body":`No work items match the current filters.`,}});}
  }else{
    
     { svelteHTML.createElement("svg", {                "class":`graph-svg`,"width":layout.width,"height":layout.height,"viewBox":`0 0 ${layout.width} ${layout.height}`,"role":`group`,"aria-label":`Work graph`,"aria-busy":busy,});busy;
      
          for(let k of __sveltets_2_ensureArray(layout.cols)){let ci = 1;k;
         { svelteHTML.createElement("text", {       "class":`col-head`,"x":PAD + ci * COL_W + 66,"y":26,"text-anchor":`middle`,});KIND_LABEL[k]; }
      }

      
         for(let e of __sveltets_2_ensureArray(layout.edges)){e.from_item_id + e.to_item_id + e.relation;
        const a = layout.posMap.get(e.from_item_id);
        const b = layout.posMap.get(e.to_item_id);
        if(a && b){
           { svelteHTML.createElement("line", {          "class":`edge`,"x1":a.x,"y1":a.y,"x2":b.x,"y2":b.y,});}
           { svelteHTML.createElement("text", {       "class":`edge-label`,"x":(a.x + b.x) / 2,"y":(a.y + b.y) / 2 - 3,"text-anchor":`middle`,});
            sentenceCase(e.relation);
           }
        }
      }

      
         for(let p of __sveltets_2_ensureArray(layout.pos)){p.node.id;
         { svelteHTML.createElement("g", {                "class":`node`,"role":`button`,"tabindex":0,"aria-label":`${p.node.title} — ${KIND_LABEL[p.node.kind]}, ${STATUS_LABEL[p.node.status] ?? p.node.status}${p.node.needs_approval ? ', needs approval' : ''}`,"aria-pressed":p.node.id === selectedId,"onclick":() => onOpen(p.node.id),"onkeydown":(ev) => {
            if (ev.key === 'Enter' || ev.key === ' ') {
              ev.preventDefault();
              onOpen(p.node.id);
            }
          },});p.node.id === selectedId;
           { svelteHTML.createElement("title", {});p.node.title;  STATUS_LABEL[p.node.status] ?? p.node.status; }
           { svelteHTML.createElement("circle", {        "cx":p.x,"cy":p.y,"r":p.r,"fill":statusColor(p.node.status),});}
          if(p.node.needs_approval){
             { svelteHTML.createElement("circle", {        "class":`approve-ring`,"cx":p.x,"cy":p.y,"r":p.r + 4,});}
          }
           { svelteHTML.createElement("g", {     "class":`node-icon`,"transform":`translate(${p.x - 6} ${p.y - 6})`,"aria-hidden":`true`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":statusIcon(p.node.status),"size":12,}});}
           }
           { svelteHTML.createElement("text", {       "class":`node-label`,"x":p.x,"y":p.y + p.r + 14,"text-anchor":`middle`,});trunc(p.node.title); }
           { svelteHTML.createElement("text", {       "class":`node-status`,"x":p.x,"y":p.y + p.r + 28,"text-anchor":`middle`,});STATUS_LABEL[p.node.status]; }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const WorkGraphView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WorkGraphView__SvelteComponent_ = ReturnType<typeof WorkGraphView__SvelteComponent_>;
/*Ωignore_endΩ*/export default WorkGraphView__SvelteComponent_;