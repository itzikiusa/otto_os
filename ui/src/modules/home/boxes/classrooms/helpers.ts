// Otto School — helpers for the Classrooms box: placing a projected HTML
// label over the 3D stage (clamped inside it), the "needs you" queue's
// heading, the stage's one-line control hint, the stage as a keyboard
// surface and a kid's context-menu items.

import type { MenuItem } from '../../../../lib/contextmenu.svelte';
import type { Kid } from '../../school/model';
import type { SchoolView } from '../../school/scene';

export function place(el: HTMLElement | null | undefined, p: { x: number; y: number; visible: boolean } | null, origin: DOMRect, below = false): void {
  if (!el) return;
  if (!p || !p.visible) {
    el.style.visibility = 'hidden';
    return;
  }
  el.style.visibility = 'visible';
  // Centred over the point, then CLAMPED inside the stage (a tall peek
  // over a back-row kid must never leave the top edge).
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  const x = Math.min(Math.max(4, p.x - origin.left - w / 2), Math.max(4, origin.width - w - 4));
  const y = Math.min(Math.max(4, p.y - origin.top - (below ? 0 : h)), Math.max(4, origin.height - h - 4));
  el.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
}

export function queueLabelFor(queue: readonly Kid[]): string {
  const need = queue.filter((k) => k.pose === 'needs-you').length;
  const failed = queue.length - need;
  return [need ? `${need} ${need === 1 ? 'needs' : 'need'} you` : '', failed ? `${failed} failed` : ''].filter(Boolean).join(' · ');
}

export function hintFor(view: SchoolView): string {
  return view.kind === 'corridor'
    ? 'Click a door to walk in · drag to look · W A S D to walk'
    : view.kind === 'room'
      ? 'Hover a kid to peek · click for its card · double-click opens it · Esc for the corridor'
      : 'Enter opens the session · Esc goes back';
}

/** Make the stage a keyboard surface (focusable; keys move the camera).
 *  Keys still held when it loses focus are let go through `release`. */
export function keySurface(node: HTMLElement, onKey: (e: KeyboardEvent, isDown: boolean) => void, release: (key: string) => void) {
  node.tabIndex = 0;
  const pressed = new Set<string>();
  const kd = (e: KeyboardEvent) => {
    if (e.target === node && !e.defaultPrevented) pressed.add(e.key);
    onKey(e, true);
  };
  const ku = (e: KeyboardEvent) => {
    pressed.delete(e.key);
    onKey(e, false);
  };
  const bl = () => {
    for (const key of pressed) release(key);
    pressed.clear();
  };
  node.addEventListener('keydown', kd);
  node.addEventListener('keyup', ku);
  node.addEventListener('blur', bl);
  return {
    destroy() {
      node.removeEventListener('keydown', kd);
      node.removeEventListener('keyup', ku);
      node.removeEventListener('blur', bl);
    },
  };
}

export interface KidActions {
  open: (k: Kid) => Promise<void>;
  free: (k: Kid) => Promise<void>;
  lookAtScreen: (id: string) => void;
  checkOn: (k: Kid) => void;
  detention: (k: Kid) => Promise<void>;
  kick: (k: Kid) => Promise<void>;
}

/** A kid's context menu; the look / check-on items only for a kid seated in
 *  the open room. */
export function kidMenuItems(k: Kid, inRoom: boolean, a: KidActions): MenuItem[] {
  if (k.detention) {
    return [{ label: 'Release from detention', icon: 'undo', disabled: !k.canManage, action: () => void a.free(k) }];
  }
  return [
    { label: 'Open session', icon: 'external', action: () => void a.open(k) },
    ...(inRoom ? [{ label: 'Look at the screen', icon: 'eye' as const, action: () => a.lookAtScreen(k.id) }] : []),
    ...(inRoom ? [{ label: 'Headmaster: check on them', icon: 'search' as const, action: () => a.checkOn(k) }] : []),
    ...(k.canManage
      ? [
          { separator: true as const },
          { label: 'Send to detention (archive)', icon: 'archive' as const, action: () => void a.detention(k) },
          { label: 'Kick out…', icon: 'trash' as const, danger: true, action: () => void a.kick(k) },
        ]
      : []),
  ];
}
