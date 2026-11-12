// The page-level half of the Threads toolbar's ⋯ menu: the view toggles that
// used to sit in the toolbar as their own icon buttons (project card, layout,
// terminal drawer) plus New thread and Related threads. The toolbar keeps
// four controls — sidebar, the breadcrumb, right panel, ⋯ — so the
// conversation stays the loudest thing on screen (T3 Code's header).
// Back / Forward stay on ⌘⇧← / ⌘⇧→, the shell's own buttons and ⌘K.
import type { MenuItem } from '../../lib/contextmenu.svelte';
import type { ThreadsLayoutMode } from './threadsStore.svelte';

export const RELATED_LABEL = 'Related threads…';

export interface PageMenuState {
  /** The focused thread's own actions (empty on the new-thread stage). */
  threadItems: MenuItem[];
  layout: ThreadsLayoutMode;
  terminalOpen: boolean;
  /** Null where the card can't show (phone / tablet: it heads the right panel). */
  cardOn: boolean | null;
  /** The focused thread, for "Related threads…"; null on the stage. */
  threadId: string | null;
}

export interface PageMenuActions {
  newThread(): void;
  related(id: string): void;
  toggleCard(): void;
  layout(mode: ThreadsLayoutMode): void;
  toggleTerminal(): void;
}

export function threadsPageMenu(s: PageMenuState, act: PageMenuActions): MenuItem[] {
  const view: MenuItem[] = [
    { label: 'New thread', icon: 'edit', action: () => act.newThread() },
    // A terminal thread's own menu already carries it (threadMenu.ts).
    ...(s.threadId && !s.threadItems.some((i) => i.label === RELATED_LABEL)
      ? [{ label: RELATED_LABEL, icon: 'people', action: () => act.related(s.threadId!) }] : []),
    { separator: true },
    ...(s.cardOn !== null ? [{ label: 'Project card', icon: 'note', checked: s.cardOn, action: () => act.toggleCard() }] : []),
    { label: 'Terminal drawer', icon: 'terminal', checked: s.terminalOpen, hint: '/terminal', action: () => act.toggleTerminal() },
    { separator: true },
    { label: 'Single thread', hint: '1-up', checked: s.layout === 'single', action: () => act.layout('single') },
    { label: 'Split', hint: '2-up', checked: s.layout === 'split', action: () => act.layout('split') },
    { label: 'Grid', hint: '4-up', checked: s.layout === 'grid', action: () => act.layout('grid') },
  ];
  return s.threadItems.length ? [...s.threadItems, { separator: true }, ...view] : view;
}
