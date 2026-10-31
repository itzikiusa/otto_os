///<reference types="svelte" />
;
import { radioKey } from '../../../lib/radioKey';
// The viewport's STATES bar (Spline-style): Idle · Hover · Flipped… Picking
// one tweens the scene to it; while a non-default state shows, gizmo and
// inspector transform edits are recorded as that state's overrides. ⋯ holds
// rename / timing / start state / duplicate / delete, all through `ops.ts`.
import Icon from '../../../lib/components/Icon.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import {
    EASINGS,
    addState,
    clearStateOverride,
    initialState,
    removeState,
    renameState,
    setDefaultState,
    setStateTiming,
    type Scene3dDoc,
  } from '../../../lib/design/scene3d';
;

  interface Props {
    doc: Scene3dDoc;
    stateId: string | null;
    selectedId?: string | null;
    readonly?: boolean;
    onpick: (id: string | null) => void;
    onchange: (doc: Scene3dDoc) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { doc, stateId, selectedId = null, readonly = false, onpick, onchange }: Props = $props();

  const states = $derived(doc.states ?? []);
  const start = $derived(initialState(doc));
  const current = $derived(states.find((s) => s.id === stateId) ?? states.find((s) => s.id === start) ?? null);

  async function add(): Promise<void> {
    const name = await confirmer.promptText('Name the new state. It starts as a copy of the one showing; move things to record how it differs.', {
      title: 'New state',
      confirmLabel: 'Add state',
      placeholder: states.length ? 'Flipped' : 'Idle',
    });
    if (!name) return;
    const r = addState(doc, name, current?.id ?? null);
    onchange(r.doc);
    onpick(r.id);
  }

  function menu(e: MouseEvent): void {
    const st = current;
    if (!st) return;
    const selOverride = selectedId ? st.overrides?.[selectedId] : undefined;
    const items: MenuItem[] = [
      {
        label: 'Rename…',
        icon: 'edit',
        disabled: readonly,
        action: async () => {
          const n = await confirmer.promptText('State name', { title: 'Rename state', confirmLabel: 'Rename', initial: st.name ?? st.id });
          if (n) onchange(renameState(doc, st.id, n));
        },
      },
      {
        label: `Transition: ${st.duration_ms ?? 400} ms…`,
        icon: 'clock',
        disabled: readonly,
        action: async () => {
          const v = await confirmer.promptText('How long the tween INTO this state takes, in milliseconds (0–10 000).', {
            title: 'Transition duration',
            confirmLabel: 'Set',
            initial: String(st.duration_ms ?? 400),
          });
          const n = v === null ? NaN : Number(v);
          if (Number.isFinite(n)) onchange(setStateTiming(doc, st.id, { duration_ms: n }));
        },
      },
      { separator: true },
      ...EASINGS.map<MenuItem>((ez) => ({
        label: `Easing: ${ez}`,
        checked: (st.easing ?? 'ease-in-out') === ez,
        disabled: readonly,
        action: () => onchange(setStateTiming(doc, st.id, { easing: ez })),
      })),
      { separator: true },
      { label: 'Start in this state', icon: 'check', disabled: readonly || st.id === start, action: () => onchange(setDefaultState(doc, st.id)) },
      {
        label: 'Duplicate',
        icon: 'copy',
        disabled: readonly,
        action: () => {
          const r = addState(doc, `${st.name ?? st.id} copy`, st.id);
          onchange(r.doc);
          onpick(r.id);
        },
      },
    ];
    if (selectedId && selOverride) {
      items.push({ label: 'Reset selection in this state', icon: 'refresh', disabled: readonly, action: () => onchange(clearStateOverride(doc, st.id, selectedId!)) });
    }
    items.push(
      { separator: true },
      {
        label: 'Delete state…',
        icon: 'trash',
        danger: true,
        disabled: readonly,
        action: async () => {
          const ok = await confirmer.ask(`Delete the state “${st.name ?? st.id}”? Its recorded differences go with it.`, {
            title: 'Delete state',
            confirmLabel: 'Delete',
            danger: true,
          });
          if (!ok) return;
          onchange(removeState(doc, st.id));
          onpick(null);
        },
      },
    );
    ctxMenu.show(e, items);
  }
;
async () => {

 { svelteHTML.createElement("div", {       "class":`states`,"role":`group`,"aria-label":`States`,...__sveltets_2_empty({"data-testid":`s3d-states`}),});
   { svelteHTML.createElement("span", { "class":`k`,});  }
  if(states.length){
     { svelteHTML.createElement("div", {     "class":`seg`,"role":`radiogroup`,"aria-label":`Scene state`,});
         for(let s of __sveltets_2_ensureArray(states)){s.id;
         { svelteHTML.createElement("button", {                "role":`radio`,"aria-checked":current?.id === s.id,"tabindex":current?.id === s.id ? 0 : -1,"onkeydown":radioKey,"title":s.id === start ? `${s.name ?? s.id} (start state)` : `${s.name ?? s.id} · ${s.duration_ms ?? 400} ms`,"onclick":() => onpick(s.id),...__sveltets_2_empty({"data-testid":`s3d-state-${s.id}`}),});current?.id === s.id;
          s.name ?? s.id;
         }
      }
     }
  }else{
     { svelteHTML.createElement("span", { "class":`none`,});  }
  }
  if(!readonly){
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void add(),"aria-label":`Add state`,"title":`Add state`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}
     }
    if(current){
       { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":menu,"aria-label":`State options`,"title":`State options`,"aria-haspopup":`menu`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});}
       }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StatesBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StatesBar__SvelteComponent_ = ReturnType<typeof StatesBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default StatesBar__SvelteComponent_;