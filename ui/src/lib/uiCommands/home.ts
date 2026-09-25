// Agent UI control — Home handlers (`otto.ui_home_*`). Home's layout (up to
// four views of live boxes) is a per-device preference in the home store, so
// the handlers drive that store directly and the user sees the grid change.
// Adding / removing boxes changes the user's own layout → `local_write` with
// the attributed, rememberable confirm; switching views / zooming a box is
// `navigate`.

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { auth } from '../stores/auth.svelte';
import { home, MAX_BOXES, type HomeView } from '../../modules/home/home.svelte';
import { HOME_KINDS, isHomeKind, kindDef } from '../../modules/home/kinds';
import { highlightWhenReady, resolveByIdOrName, waitFor } from './pagePort';

const can = (f: Parameters<typeof auth.can>[0]): boolean => auth.can(f, 'view');

async function showHome(ctx: UiCommandCtx): Promise<void> {
  if (router.parts[0] !== 'home') router.go('home');
  await waitFor(() => router.parts[0] === 'home', ctx.signal, 5000, 'Home');
  if (auth.me) home.ensureDefault(can);
}

function viewFor(key: string | number | undefined): HomeView {
  if (key === undefined || key === null || key === '') {
    const v = home.active;
    if (!v) throw new UiCommandError('not_found', 'Home has no views.');
    return v;
  }
  if (typeof key === 'number' || /^\d+$/.test(String(key))) {
    const i = Number(key) - 1; // 1-based, like the 01–04 space chips
    const v = home.views[i];
    if (v) return v;
  }
  return resolveByIdOrName(home.views, String(key), (v) => v.id, (v) => v.name, 'Home view');
}

function viewSummary(v: HomeView, i: number) {
  return {
    index: i + 1,
    id: v.id,
    name: v.name,
    active: i === home.activeIndex,
    boxes: v.boxes.map((b) => ({ id: b.id, kind: b.kind, w: b.w, h: b.h, config: b.config })),
  };
}

function boxLocation(boxId: string): { view: HomeView; index: number } {
  for (let i = 0; i < home.views.length; i++) {
    if (home.views[i].boxes.some((b) => b.id === boxId)) return { view: home.views[i], index: i };
  }
  throw new UiCommandError('not_found', `No Home box “${boxId}” (otto.ui_home_state lists them).`);
}

registerUiCommands('home', {
  async home_state(_args, ctx) {
    await showHome(ctx);
    return {
      views: home.views.map(viewSummary),
      zoomed_box: home.zoomedId,
      auto_rotate: home.autoRotate,
      available_kinds: HOME_KINDS.filter((k) => can(k.feature)).map((k) => ({ kind: k.kind, label: k.label, blurb: k.blurb })),
    };
  },

  async home_go_to_view(args: { view: string | number }, ctx) {
    await showHome(ctx);
    const v = viewFor(args.view);
    home.goTo(home.views.indexOf(v));
    return { active: viewSummary(v, home.views.indexOf(v)) };
  },

  async home_zoom_box(args: { box_id: string; zoom?: boolean }, ctx) {
    await showHome(ctx);
    const { index } = boxLocation(args.box_id);
    home.goTo(index);
    const want = args.zoom ?? true;
    if ((home.zoomedId === args.box_id) !== want) home.toggleZoom(args.box_id);
    void highlightWhenReady(ctx, `[data-box-id="${CSS.escape(args.box_id)}"]`);
    return { zoomed_box: home.zoomedId };
  },

  async home_add_box(args: { kind: string; view?: string | number; config?: Record<string, unknown> }, ctx) {
    await showHome(ctx);
    if (!isHomeKind(args.kind)) {
      throw new UiCommandError('invalid_args', `Unknown box kind “${args.kind}” (one of ${HOME_KINDS.map((k) => k.kind).join(', ')}).`);
    }
    const def = kindDef(args.kind);
    if (!can(def.feature)) throw new UiCommandError('forbidden', `The user can't view ${def.label}.`);
    const v = viewFor(args.view);
    if (v.boxes.length >= MAX_BOXES) throw new UiCommandError('failed', `“${v.name}” already has ${MAX_BOXES} boxes.`);
    home.goTo(home.views.indexOf(v));
    const ok = await ctx.confirmWrite({ what: `Add a “${def.label}” box`, where: `Home view “${v.name}”`, connId: 'home', verb: 'Add' });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined adding the box.');
    const box = home.addBox(v.id, args.kind, args.config ?? {});
    if (!box) throw new UiCommandError('failed', 'The box could not be added.');
    void highlightWhenReady(ctx, `[data-box-id="${CSS.escape(box.id)}"]`);
    return { view: v.name, box: { id: box.id, kind: box.kind, w: box.w, h: box.h } };
  },

  async home_remove_box(args: { box_id: string }, ctx) {
    await showHome(ctx);
    const { view, index } = boxLocation(args.box_id);
    const box = view.boxes.find((b) => b.id === args.box_id)!;
    home.goTo(index);
    void highlightWhenReady(ctx, `[data-box-id="${CSS.escape(box.id)}"]`);
    const ok = await ctx.confirmWrite({ what: `Remove the “${kindDef(box.kind).label}” box`, where: `Home view “${view.name}”`, connId: 'home', verb: 'Remove' });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined removing the box.');
    home.removeBox(view.id, box.id);
    return { view: view.name, removed: box.id };
  },
});

// `otto.ui_state` view: the active Home view and its boxes.
registerUiState('home', () => ({
  view: home.active ? { index: home.activeIndex + 1, name: home.active.name } : null,
  views: home.views.length,
  boxes: home.active?.boxes.map((b) => ({ id: b.id, kind: b.kind })) ?? [],
  zoomed_box: home.zoomedId,
}));
