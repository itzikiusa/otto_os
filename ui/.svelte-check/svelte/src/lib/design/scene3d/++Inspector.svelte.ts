///<reference types="svelte" />
;
import { radioKey } from '../../radioKey';
import { plural } from '../../plural';
// Scene inspector — the game-studio right panel. With nothing selected it shows
// the SCENE (background, grid) and CAMERA panels; an object gets Transform +
// Material (+ Text / Model) + Notes; a light gets its light panel; a group gets
// its members. Every field patches the JSON doc through `ops.ts` and emits
// `onchange(newDoc)` live (no debounce here — the arena owns the autosave).
//
// v2: a physical-material preset grid, the brand kit's colour swatches (a
// click stores `token:color.<name>`, so the scene follows the kit), the
// physical sliders (clearcoat / transmission / sheen), a box corner radius,
// and — when a non-default STATE is showing — transform edits go to that
// state's overrides (`editState`), exactly like the viewport gizmo.
import NumberDrag from './NumberDrag.svelte';
import Icon from '../../components/Icon.svelte';
import { LIGHT_TYPES, type Scene3dDoc, type Scene3dMaterial, type Vec3 } from './types';
import {
    findNode,
    nodeLabel,
    parentGroup,
    patchLight,
    rename,
    setCamera,
    setMaterial,
    setMaterialPreset,
    setNotes,
    setRadius,
    setScene,
    setStateOverride,
    setText,
    setTransform,
    setVisible,
    summarize,
  } from './ops';
import { MATERIAL_PRESETS, resolveMaterial } from './presets';
import { colorLabel, isTokenRef, type BrandSwatch } from './tokens';
import { plainColors, type ColorResolver } from './build';
import { findState } from './states';
;

  interface Props {
    doc: Scene3dDoc;
    selectedId?: string | null;
    onchange: (doc: Scene3dDoc) => void;
    readonly?: boolean;
    /** v2: the brand kit's colours (Design Hall passes the scene's kit). */
    swatches?: BrandSwatch[];
    /** v2: the kit's display name for the Material panel ("Acme Brand Kit v4"). */
    brandName?: string | null;
    /** v2: resolves `token:` colours for the hex readout (the host's brand kit). */
    colors?: ColorResolver;
    /** v2: transform edits of the selected object go to this state's overrides. */
    editState?: string | null;
  };function $$render() {
 const panelHead/*Ωignore_positionΩ*/ = (key: string, title: string, meta?: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("button", {     "class":`s3d-ph`,"onclick":() => toggle(key),"aria-expanded":open[key] !== false,});
     { svelteHTML.createElement("span", {    "class":`s3d-ph-chev`,"aria-hidden":`true`,});open[key] === false; { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
     { svelteHTML.createElement("span", { "class":`s3d-ph-title`,});title; }
    if(meta){ { svelteHTML.createElement("span", { "class":`s3d-ph-meta`,});meta; }}
   }
};return __sveltets_2_any(0)}; const vec3Row/*Ωignore_positionΩ*/ = (label: string, v: Vec3, step: number, digits: number, on: (nv: Vec3) => void, unit?: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", { "class":`s3d-field`,});
     { svelteHTML.createElement("span", { "class":`s3d-flabel`,});label; }
     { svelteHTML.createElement("div", { "class":`s3d-vec`,});
         for(let i of __sveltets_2_ensureArray(([0, 1, 2]))){i;
         { const $$_garDrebmuN2C = __sveltets_2_ensureComponent(NumberDrag); new $$_garDrebmuN2C({ target: __sveltets_2_any(), props: {           "label":axis[i],"value":v[i],step,digits,unit,"disabled":readonly,"onchange":(n) => on(vec(v, i, n)),}});}
      }
     }
   }
};return __sveltets_2_any(0)}; const rangeRow/*Ωignore_positionΩ*/ = (label: string, value: number, min: number, max: number, step: number, on: (n: number | undefined) => void, resettable = true)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", { "class":`s3d-field`,});
     { svelteHTML.createElement("span", { "class":`s3d-flabel`,});label; }
     { svelteHTML.createElement("input", {              "class":`s3d-range`,"type":`range`,min,max,step,value,"disabled":readonly,"aria-label":label,"oninput":(e) => on(Number(inputVal(e))),});}
     { const $$_garDrebmuN1C = __sveltets_2_ensureComponent(NumberDrag); new $$_garDrebmuN1C({ target: __sveltets_2_any(), props: {            "label":"","value":value,"step":step,min,max,"digits":2,"disabled":readonly,"onchange":(n) => on(n),}});}
    if(resettable){
        { svelteHTML.createElement("button", {         "class":`s3d-reset`,"title":`Reset to default`,"aria-label":`Reset ${label}`,"disabled":readonly,"onclick":() => on(undefined),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
    }
   }
};return __sveltets_2_any(0)}; const colorRow/*Ωignore_positionΩ*/ = (label: string, value: string | undefined, fallback: string, on: (c: string | undefined) => void)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", { "class":`s3d-field`,});
     { svelteHTML.createElement("span", { "class":`s3d-flabel`,});label; }
     { svelteHTML.createElement("input", {            "class":`s3d-color`,"type":`color`,"value":value ?? fallback,"disabled":readonly,"aria-label":label,"oninput":(e) => on(inputVal(e)),});}
     { svelteHTML.createElement("input", {                    "dir":`auto`,"class":`s3d-hex`,"type":`text`,"value":value ?? '',"placeholder":fallback,"maxlength":7,"spellcheck":`false`,"disabled":readonly,"aria-label":`${label} hex`,"onchange":(e) => {
      const t = inputVal(e).trim().toLowerCase();
      if (!t) on(undefined);
      else if (/^#[0-9a-f]{6}$/.test(t)) on(t);
      else (e.currentTarget as HTMLInputElement).value = value ?? '';
    },});}
      { svelteHTML.createElement("button", {         "class":`s3d-reset`,"title":`Reset to default`,"aria-label":`Reset ${label}`,"disabled":readonly,"onclick":() => on(undefined),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    doc,
    selectedId = $bindable<string | null>(null),
    onchange,
    readonly = false,
    swatches = [],
    brandName = null,
    colors = plainColors,
    editState = null,
  }: Props = $props()/*Ωignore_startΩ*/;selectedId;/*Ωignore_endΩ*/;

  const sel = $derived(findNode(doc, selectedId));
  const obj = $derived(sel?.kind === 'object' ? sel.node : null);
  const light = $derived(sel?.kind === 'light' ? sel.node : null);
  const group = $derived(sel?.kind === 'group' ? sel.node : null);
  const parent = $derived(selectedId ? parentGroup(doc, selectedId) : null);
  const mat = $derived<Scene3dMaterial>(obj?.material ?? {});
  /** Preset defaults folded in — what the sliders show when a field isn't set. */
  const resolved = $derived(resolveMaterial(obj?.material, colors));
  const stateEdit = $derived(obj && editState ? findState(doc, editState) : null);
  /** The transform the rows show: the state's override over the base. */
  const shownTransform = $derived.by(() => {
    if (!obj) return null;
    const ov = stateEdit?.overrides?.[obj.id];
    return { position: ov?.position ?? obj.position, rotation: ov?.rotation ?? obj.rotation, scale: ov?.scale ?? obj.scale };
  });
  function emitTransform(patch: { position?: Vec3; rotation?: Vec3; scale?: Vec3 }): void {
    if (!obj) return;
    emit(stateEdit ? setStateOverride(doc, stateEdit.id, obj.id, patch) : setTransform(doc, obj.id, patch));
  }

  // Collapsible panels remember their state for the session.
  let open = $state<Record<string, boolean>>({ transform: true, material: true, light: true, scene: true, camera: true, notes: false, model: true, text: true, group: true });
  function toggle(k: string): void {
    open = { ...open, [k]: !open[k] };
  }

  function emit(next: Scene3dDoc): void {
    if (readonly || next === doc) return;
    onchange(next);
  }
  function vec(v: Vec3, i: number, n: number): Vec3 {
    const out: Vec3 = [v[0], v[1], v[2]];
    out[i] = n;
    return out;
  }
  const axis = ['X', 'Y', 'Z'];

  function onMat<K extends keyof Scene3dMaterial>(k: K, v: Scene3dMaterial[K] | undefined): void {
    if (!obj) return;
    emit(setMaterial(doc, obj.id, { [k]: v } as Partial<Scene3dMaterial>));
  }
  function inputVal(e: Event): string {
    return (e.currentTarget as HTMLInputElement).value;
  }
  function inputChecked(e: Event): boolean {
    return (e.currentTarget as HTMLInputElement).checked;
  }
;
async () => {









 { svelteHTML.createElement("div", { "class":`s3d-inspector`,});
  if(!sel){
     { svelteHTML.createElement("div", { "class":`s3d-head`,});
       { svelteHTML.createElement("div", { "class":`s3d-head-name`,});  }
       { svelteHTML.createElement("div", { "class":`s3d-head-sub`,});summarize(doc);       }
     }

     { svelteHTML.createElement("section", { "class":`s3d-panel`,});
      ;__sveltets_2_ensureSnippet(panelHead('scene', 'Scene'));
      if(open.scene !== false){
         { svelteHTML.createElement("div", { "class":`s3d-panel-body`,});
          ;__sveltets_2_ensureSnippet(colorRow('Background', doc.background, '#0f172a', (c) => emit(setScene(doc, { background: c ?? null }))));
           { svelteHTML.createElement("label", { "class":`s3d-field s3d-check`,});
             { svelteHTML.createElement("span", { "class":`s3d-flabel`,});  }
             { svelteHTML.createElement("input", {        "type":`checkbox`,"checked":doc.grid ?? true,"disabled":readonly,"onchange":(e) => emit(setScene(doc, { grid: inputChecked(e) })),});}
             { svelteHTML.createElement("span", { "class":`s3d-hint`,});      }
           }
         }
      }
     }

     { svelteHTML.createElement("section", { "class":`s3d-panel`,});
      ;__sveltets_2_ensureSnippet(panelHead('camera', 'Camera', 'used by Play, the agent and Blender'));
      if(open.camera !== false){
         { svelteHTML.createElement("div", { "class":`s3d-panel-body`,});
          ;__sveltets_2_ensureSnippet(vec3Row('Position', doc.camera.position, 0.05, 2, (v) => emit(setCamera(doc, { position: v })), 'm'));
          ;__sveltets_2_ensureSnippet(vec3Row('Target', doc.camera.target, 0.05, 2, (v) => emit(setCamera(doc, { target: v })), 'm'));
          ;__sveltets_2_ensureSnippet(rangeRow('FOV', doc.camera.fov, 10, 120, 1, (n) => emit(setCamera(doc, { fov: n ?? 50 })), false));
           { svelteHTML.createElement("div", { "class":`s3d-field`,});
             { svelteHTML.createElement("span", { "class":`s3d-flabel`,});  }
             { svelteHTML.createElement("div", { "class":`s3d-vec`,});
               { const $$_garDrebmuN5C = __sveltets_2_ensureComponent(NumberDrag); new $$_garDrebmuN5C({ target: __sveltets_2_any(), props: {              "label":`near`,"value":doc.camera.near ?? 0.1,"step":0.01,"min":0.001,"digits":3,"disabled":readonly,"onchange":(n) => emit(setCamera(doc, { near: n })),}});}
               { const $$_garDrebmuN5C = __sveltets_2_ensureComponent(NumberDrag); new $$_garDrebmuN5C({ target: __sveltets_2_any(), props: {              "label":`far`,"value":doc.camera.far ?? 500,"step":1,"min":1,"digits":0,"disabled":readonly,"onchange":(n) => emit(setCamera(doc, { far: n })),}});}
             }
           }
           { svelteHTML.createElement("div", { "class":`s3d-hint`,});         { svelteHTML.createElement("em", {});   }     }
         }
      }
     }
  }else{
     { svelteHTML.createElement("div", { "class":`s3d-head`,});
       { svelteHTML.createElement("input", {            "dir":`auto`,"class":`s3d-head-name-input`,"value":nodeLabel(sel),"disabled":readonly,"aria-label":`Name`,"onchange":(e) => emit(rename(doc, sel.node.id, inputVal(e))),});}
       { svelteHTML.createElement("div", { "class":`s3d-head-sub`,});
         { svelteHTML.createElement("span", { "class":`s3d-kind`,});sel.kind === 'object' ? sel.node.type : sel.kind === 'light' ? `${sel.node.type} light` : 'group'; }
         { svelteHTML.createElement("code", {   "class":`s3d-id`,"title":`id (what the agent references)`,});sel.node.id; }
        if(parent){ { svelteHTML.createElement("span", {});   { svelteHTML.createElement("strong", {});parent.name; } }}
       }
       { svelteHTML.createElement("label", { "class":`s3d-check s3d-vis`,});
         { svelteHTML.createElement("input", {        "type":`checkbox`,"checked":sel.node.visible !== false,"disabled":readonly,"onchange":(e) => emit(setVisible(doc, sel.node.id, inputChecked(e))),});}
         { svelteHTML.createElement("span", {});  }
       }
     }

    if(obj){
       { svelteHTML.createElement("section", { "class":`s3d-panel`,});
        ;__sveltets_2_ensureSnippet(panelHead('transform', 'Transform'));
        if(open.transform !== false){
           { svelteHTML.createElement("div", { "class":`s3d-panel-body`,});
            if(stateEdit){
               { svelteHTML.createElement("div", {   "class":`s3d-hint s3d-state-hint`,...__sveltets_2_empty({"data-testid":`s3d-state-edit`}),});   { svelteHTML.createElement("strong", {});stateEdit.name ?? stateEdit.id; }          }
            }
            ;__sveltets_2_ensureSnippet(vec3Row('Position', shownTransform!.position, 0.01, 3, (v) => emitTransform({ position: v }), 'm'));
            ;__sveltets_2_ensureSnippet(vec3Row('Rotation', shownTransform!.rotation, 0.5, 1, (v) => emitTransform({ rotation: v }), '°'));
            ;__sveltets_2_ensureSnippet(vec3Row('Scale', shownTransform!.scale, 0.01, 3, (v) => emitTransform({ scale: v })));
            if(obj.type === 'box'){
              ;__sveltets_2_ensureSnippet(rangeRow('Corners', obj.radius ?? 0, 0, 0.5, 0.005, (n) => emit(setRadius(doc, obj.id, n ?? 0))));
            }
             { svelteHTML.createElement("div", { "class":`s3d-row-btns`,});
               { svelteHTML.createElement("button", {     "class":`s3d-mini`,"disabled":readonly,"onclick":() => emit(setTransform(doc, obj.id, { position: [0, obj.type === 'plane' ? 0 : 0.5, 0] })),});  }
               { svelteHTML.createElement("button", {     "class":`s3d-mini`,"disabled":readonly,"onclick":() => emit(setTransform(doc, obj.id, { rotation: obj.type === 'plane' ? [-90, 0, 0] : [0, 0, 0] })),});  }
               { svelteHTML.createElement("button", {     "class":`s3d-mini`,"disabled":readonly,"onclick":() => emit(setTransform(doc, obj.id, { scale: [1, 1, 1] })),});  }
               { svelteHTML.createElement("button", {       "class":`s3d-mini`,"disabled":readonly,"title":`Rest the unit on the floor (y = half height × scale)`,"onclick":() => emit(setTransform(doc, obj.id, { position: [obj.position[0], obj.type === 'plane' ? 0 : 0.5 * obj.scale[1], obj.position[2]] })),});   }
             }
           }
        }
       }

      if(obj.type === 'gltf'){
         { svelteHTML.createElement("section", { "class":`s3d-panel`,});
          ;__sveltets_2_ensureSnippet(panelHead('model', 'Model'));
          if(open.model !== false){
             { svelteHTML.createElement("div", { "class":`s3d-panel-body`,});
               { svelteHTML.createElement("div", { "class":`s3d-field`,});
                 { svelteHTML.createElement("span", { "class":`s3d-flabel`,});  }
                 { svelteHTML.createElement("code", {   "class":`s3d-id s3d-grow`,"title":`attachment_id — loaded through the authed attachment route`,});obj.attachment_id; }
               }
               { svelteHTML.createElement("div", { "class":`s3d-hint`,});           }
             }
          }
         }
      }else{
        if(obj.type === 'text'){
           { svelteHTML.createElement("section", { "class":`s3d-panel`,});
            ;__sveltets_2_ensureSnippet(panelHead('text', 'Text'));
            if(open.text !== false){
               { svelteHTML.createElement("div", { "class":`s3d-panel-body`,});
                 { svelteHTML.createElement("input", {              "dir":`auto`,"class":`s3d-text`,"value":obj.text ?? obj.name,"maxlength":500,"disabled":readonly,"aria-label":`Text`,"oninput":(e) => emit(setText(doc, obj.id, inputVal(e))),});}
                 { svelteHTML.createElement("div", { "class":`s3d-hint`,});                  }
               }
            }
           }
        }
         { svelteHTML.createElement("section", { "class":`s3d-panel`,});
          ;__sveltets_2_ensureSnippet(panelHead('material', 'Material', brandName ?? undefined));
          if(open.material !== false){
             { svelteHTML.createElement("div", { "class":`s3d-panel-body`,});
               { svelteHTML.createElement("div", {     "class":`s3d-presets`,"role":`radiogroup`,"aria-label":`Material preset`,});
                   for(let p of __sveltets_2_ensureArray(MATERIAL_PRESETS)){p.id;
                  const tint = p.id === 'brushed-metal' || p.id === 'frosted-glass' ? p.swatch : resolved.color;
                   { svelteHTML.createElement("button", {                    "role":`radio`,"aria-checked":mat.preset === p.id,"tabindex":mat.preset === p.id ? 0 : -1,"onkeydown":radioKey,"class":`s3d-preset ${p.id}`,"title":p.hint,"disabled":readonly,...__sveltets_2_empty({"data-testid":`s3d-preset-${p.id}`}),"onclick":() => emit(setMaterialPreset(doc, obj.id, mat.preset === p.id ? null : p.id)),});mat.preset === p.id;
                     { svelteHTML.createElement("span", {  "class":`s3d-ball`,});__sveltets_2_ensureType(String, Number, tint); }
                     { svelteHTML.createElement("span", { "class":`s3d-plabel`,});p.label; }
                   }
                }
               }
              if(swatches.length){
                 { svelteHTML.createElement("div", { "class":`s3d-field s3d-swatch-row`,});
                   { svelteHTML.createElement("span", { "class":`s3d-flabel`,});  }
                   { svelteHTML.createElement("div", {     "class":`s3d-swatches`,"role":`radiogroup`,"aria-label":`Brand colors`,});
                       for(let sw of __sveltets_2_ensureArray((swatches.slice(0, 12)))){sw.ref;
                       { svelteHTML.createElement("button", {                     "role":`radio`,"aria-checked":mat.color === sw.ref,"tabindex":mat.color === sw.ref ? 0 : -1,"onkeydown":radioKey,"class":`s3d-swatch`,"title":`${colorLabel(sw.ref)} · ${sw.value}`,"aria-label":`${colorLabel(sw.ref)} ${sw.value}`,"disabled":readonly,"onclick":() => onMat('color', sw.ref),});mat.color === sw.ref;__sveltets_2_ensureType(String, Number, sw.value); }
                    }
                   }
                 }
              }
              if(isTokenRef(mat.color)){
                 { svelteHTML.createElement("div", { "class":`s3d-field`,});
                   { svelteHTML.createElement("span", { "class":`s3d-flabel`,});  }
                   { svelteHTML.createElement("span", {   "class":`s3d-token`,"title":`Follows the brand kit: changing the kit recolors this object`,});
                     { svelteHTML.createElement("span", {  "class":`s3d-dot`,});__sveltets_2_ensureType(String, Number, resolved.color); }colorLabel(mat.color);  { svelteHTML.createElement("code", {});resolved.color; }
                   }
                    { svelteHTML.createElement("button", {         "class":`s3d-reset`,"title":`Detach from the brand kit (keep ${resolved.color})`,"aria-label":`Detach color from the brand kit`,"disabled":readonly,"onclick":() => onMat('color', resolved.color),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
                 }
              }else{
                ;__sveltets_2_ensureSnippet(colorRow('Color', mat.color, resolved.color, (c) => onMat('color', c)));
              }
              ;__sveltets_2_ensureSnippet(rangeRow('Roughness', mat.roughness ?? resolved.roughness, 0, 1, 0.01, (n) => onMat('roughness', n)));
              ;__sveltets_2_ensureSnippet(rangeRow('Metalness', mat.metalness ?? resolved.metalness, 0, 1, 0.01, (n) => onMat('metalness', n)));
              ;__sveltets_2_ensureSnippet(rangeRow('Clearcoat', mat.clearcoat ?? resolved.clearcoat, 0, 1, 0.01, (n) => onMat('clearcoat', n)));
              ;__sveltets_2_ensureSnippet(rangeRow('Glass', mat.transmission ?? resolved.transmission, 0, 1, 0.01, (n) => onMat('transmission', n)));
              ;__sveltets_2_ensureSnippet(rangeRow('Sheen', mat.sheen ?? resolved.sheen, 0, 1, 0.01, (n) => onMat('sheen', n)));
              ;__sveltets_2_ensureSnippet(rangeRow('Opacity', mat.opacity ?? 1, 0, 1, 0.01, (n) => onMat('opacity', n)));
              ;__sveltets_2_ensureSnippet(colorRow('Emissive', mat.emissive, '#000000', (c) => onMat('emissive', c)));
               { svelteHTML.createElement("label", { "class":`s3d-field s3d-check`,});
                 { svelteHTML.createElement("span", { "class":`s3d-flabel`,});  }
                 { svelteHTML.createElement("input", {        "type":`checkbox`,"checked":mat.wireframe ?? false,"disabled":readonly,"onchange":(e) => onMat('wireframe', inputChecked(e) || undefined),});}
               }
             }
          }
         }
      }
    }

    if(light){
       { svelteHTML.createElement("section", { "class":`s3d-panel`,});
        ;__sveltets_2_ensureSnippet(panelHead('light', 'Light'));
        if(open.light !== false){
           { svelteHTML.createElement("div", { "class":`s3d-panel-body`,});
             { svelteHTML.createElement("div", { "class":`s3d-field`,});
               { svelteHTML.createElement("span", { "class":`s3d-flabel`,});  }
               { svelteHTML.createElement("select", {       "class":`s3d-select`,"value":light.type,"disabled":true,"aria-label":`Light type`,"title":`Delete and add a new light to change its type`,});
                   for(let t of __sveltets_2_ensureArray(LIGHT_TYPES)){t; { svelteHTML.createElement("option", { "value":t,});t; }}
               }
             }
            ;__sveltets_2_ensureSnippet(rangeRow('Intensity', light.intensity ?? 1, 0, light.type === 'ambient' || light.type === 'hemisphere' ? 3 : 10, 0.05, (n) => emit(patchLight(doc, light.id, { intensity: n }))));
            ;__sveltets_2_ensureSnippet(colorRow('Color', light.color, '#ffffff', (c) => emit(patchLight(doc, light.id, { color: c }))));
            if(light.type === 'hemisphere'){
              ;__sveltets_2_ensureSnippet(colorRow('Ground', light.ground_color, '#334155', (c) => emit(patchLight(doc, light.id, { ground_color: c }))));
            }
            if(light.type !== 'ambient' && light.type !== 'hemisphere'){
              ;__sveltets_2_ensureSnippet(vec3Row('Position', light.position ?? [5, 10, 5], 0.05, 2, (v) => emit(setTransform(doc, light.id, { position: v })), 'm'));
            }
            if(light.type === 'directional' || light.type === 'spot'){
              ;__sveltets_2_ensureSnippet(vec3Row('Target', light.target ?? [0, 0, 0], 0.05, 2, (v) => emit(patchLight(doc, light.id, { target: v })), 'm'));
            }
            if(light.type === 'spot'){
              ;__sveltets_2_ensureSnippet(rangeRow('Angle', light.angle ?? 30, 1, 90, 1, (n) => emit(patchLight(doc, light.id, { angle: n }))));
            }
            if(light.type === 'spot' || light.type === 'point'){
              ;__sveltets_2_ensureSnippet(rangeRow('Distance', light.distance ?? 0, 0, 100, 0.5, (n) => emit(patchLight(doc, light.id, { distance: n }))));
            }
            if(light.type !== 'ambient' && light.type !== 'hemisphere'){
               { svelteHTML.createElement("label", { "class":`s3d-field s3d-check`,});
                 { svelteHTML.createElement("span", { "class":`s3d-flabel`,});  }
                 { svelteHTML.createElement("input", {        "type":`checkbox`,"checked":light.shadow ?? false,"disabled":readonly,"onchange":(e) => emit(patchLight(doc, light.id, { shadow: inputChecked(e) || undefined })),});}
               }
            }
           }
        }
       }
    }

    if(group){
       { svelteHTML.createElement("section", { "class":`s3d-panel`,});
        ;__sveltets_2_ensureSnippet(panelHead('group', 'Group', `${plural(group.children.length, 'member')}`));
        if(open.group !== false){
           { svelteHTML.createElement("div", { "class":`s3d-panel-body`,});
            if(group.children.length){
               { svelteHTML.createElement("ul", { "class":`s3d-members`,});
                   for(let cid of __sveltets_2_ensureArray(group.children)){cid;
                  const c = findNode(doc, cid);
                  if(c){
                     { svelteHTML.createElement("li", {}); { svelteHTML.createElement("button", {   "class":`s3d-link`,"onclick":() => (selectedId = cid),});nodeLabel(c); }  { svelteHTML.createElement("span", { "class":`s3d-hint`,});c.kind === 'object' ? c.node.type : c.kind; } }
                  }
                }
               }
            }else{
               { svelteHTML.createElement("div", { "class":`s3d-hint`,});          { svelteHTML.createElement("em", {});  group.name; }  }
            }
             { svelteHTML.createElement("div", { "class":`s3d-hint`,});           }
           }
        }
       }
    }

     { svelteHTML.createElement("section", { "class":`s3d-panel`,});
      ;__sveltets_2_ensureSnippet(panelHead('notes', 'Notes', sel.node.notes ? `${sel.node.notes.length}` : undefined));
      if(open.notes !== false){
         { svelteHTML.createElement("div", { "class":`s3d-panel-body`,});
           { svelteHTML.createElement("textarea", {                  "dir":`auto`,"class":`s3d-notes`,"rows":4,"maxlength":4000,"placeholder":`Design intent, review comments, TODOs for the agent…`,"value":sel.node.notes ?? '',"disabled":readonly,"aria-label":`Notes`,"oninput":(e) => emit(setNotes(doc, sel.node.id, (e.currentTarget as HTMLTextAreaElement).value)),}); }
           { svelteHTML.createElement("div", { "class":`s3d-hint`,});           }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('selectedId'), slots: {}, events: {} }}
const Inspector__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Inspector__SvelteComponent_ = ReturnType<typeof Inspector__SvelteComponent_>;
/*Ωignore_endΩ*/export default Inspector__SvelteComponent_;