// The WIP list's keyboard model (split out of WipPanel.svelte): ↑/↓
// (Home/End) move between rows across sections, ←/→ fold a folder; with a
// diff open the moved-to file opens in its place (Tower / VS Code). The
// checkbox and discard stay one Tab away.
import type { Attachment } from 'svelte/attachments';
import type { WipSection as Section } from './wipTree';

export function wipRowNav(diffOpen: () => boolean, open: (section: Section, path: string) => void): Attachment<HTMLElement> {
  return (el) => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      const cur = (e.target as HTMLElement | null)?.closest?.<HTMLElement>('[data-wp-nav]');
      if (!cur) return;
      const all = Array.from(el.querySelectorAll<HTMLElement>('[data-wp-nav]'));
      const i = all.indexOf(cur);
      const rtl = getComputedStyle(el).direction === 'rtl';
      let next: HTMLElement | undefined;
      if (e.key === 'ArrowDown') next = all[i + 1];
      else if (e.key === 'ArrowUp') next = all[i - 1];
      else if (e.key === 'Home') next = all[0];
      else if (e.key === 'End') next = all.at(-1);
      else if ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && cur.classList.contains('wp-fold-name')) {
        e.preventDefault();
        const expand = (e.key === 'ArrowRight') !== rtl;
        if (expand !== (cur.getAttribute('aria-expanded') === 'true')) cur.click();
        return;
      } else return;
      e.preventDefault();
      if (!next) return;
      next.focus();
      next.scrollIntoView({ block: 'nearest' });
      const key = next.dataset.wpNav ?? '';
      const section = next.closest<HTMLElement>('[data-wp-section]')?.dataset.wpSection as Section | undefined;
      if (diffOpen() && section && key.startsWith('f:')) open(section, key.slice(2));
    };
    el.addEventListener('keydown', onKey);
    return () => el.removeEventListener('keydown', onKey);
  };
}
