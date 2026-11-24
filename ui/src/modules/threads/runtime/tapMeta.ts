// Touch screens: a message's time and actions (Edit, Copy) appear on a tap of
// the message and go with the next one (RuntimeTimelineRows: `.meta-on`), so a
// short answer is one line, not four. A tap on a link, a button, a form field,
// the actions themselves or one that selects text is the person's, not this
// toggle; with a pointer that hovers this does nothing (hover shows them).
export function tapMeta(node: HTMLElement): { destroy: () => void } {
  const onClick = (e: MouseEvent): void => {
    if (typeof matchMedia === 'function' && !matchMedia('(hover: none)').matches) return;
    const t = e.target instanceof Element ? e.target : null;
    if (t?.closest('a, button, input, textarea, select, summary, [role="button"], .rt-msg-foot')) return;
    if (String(getSelection?.() ?? '').length > 0) return;
    node.classList.toggle('meta-on');
  };
  node.addEventListener('click', onClick);
  return { destroy: () => node.removeEventListener('click', onClick) };
}
