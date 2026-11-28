export interface FirstTurnAnchor {
  node: HTMLElement | null;
  docY: number;
}

export function captureFirstTurnAnchor(el: HTMLDivElement | null): FirstTurnAnchor {
  if (!el) return { node: null, docY: 0 };
  const node = el.querySelector<HTMLElement>('.turn[data-turn-id]');
  if (!node) return { node: null, docY: 0 };
  return { node, docY: node.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop };
}

export function restoreFirstTurnAnchor(
  el: HTMLDivElement | null,
  anchor: FirstTurnAnchor,
  beforeH: number,
  beforeTop: number,
): void {
  if (!el || !el.isConnected) return;
  if (anchor.node && anchor.node.isConnected) {
    const afterDocY = anchor.node.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop;
    const delta = afterDocY - anchor.docY;
    if (Math.abs(delta) >= 0.5) el.scrollTop += delta;
    return;
  }
  el.scrollTop = el.scrollHeight - beforeH + beforeTop;
}

export function findAdjacentUserTurn(list: HTMLDivElement | null, dir: -1 | 1): HTMLElement | null {
  if (!list) return null;
  const mine = Array.from(list.querySelectorAll<HTMLElement>('.turn[data-role="user"]'));
  if (!mine.length) return null;
  const top = list.getBoundingClientRect().top + 8;
  const target =
    dir < 0
      ? [...mine].reverse().find((el) => el.getBoundingClientRect().top < top - 4)
      : mine.find((el) => el.getBoundingClientRect().top > top + 4);
  return target ?? null;
}

export function flashUserTurnBubble(target: HTMLElement): void {
  target.scrollIntoView({ block: 'start' });
  (target.querySelector('.bubble') as HTMLElement | null)?.animate?.(
    [{ boxShadow: '0 0 0 3px var(--accent-soft)' }, { boxShadow: '0 0 0 0 transparent' }],
    { duration: matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 900 },
  );
}
