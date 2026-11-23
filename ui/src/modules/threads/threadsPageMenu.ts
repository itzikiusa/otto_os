// The Threads toolbar's ⋯ menu: the focused thread's actions, then the page's
// view toggles (terminal drawer, project card, layout). The toolbar keeps
// four controls — sidebar, the breadcrumb, right panel, ⋯ — so the
// conversation stays the loudest thing on screen (T3 Code's header).
// Back / Forward stay on ⌘⇧← / ⌘⇧→, the shell's own buttons and ⌘K.
//
// It holds only what has no other home: Pin, Open beside and Move to History
// stay on the sidebar row (its hover buttons and ⋯), New thread is the
// sidebar's pencil and ⌘T while the sidebar shows, and there is ONE terminal
// entry (the thread's own drawer, or the page's while that one is open).
// Layout lives in a View submenu, with the per-device thread prefs
// (threadPrefs.svelte.ts: tool-call visibility, what Enter does while the
// agent works); a phone
// gets no Split / Grid at all, only the prefs.
import type { MenuItem } from '../../lib/contextmenu.svelte';
import type { MidTurn } from './runtime/composerSend';
import type { ToolVisibility } from './runtime/workLog';
import type { ThreadsLayoutMode } from './threadsStore.svelte';

export const RELATED_LABEL = 'Related threads…';
export const TERMINAL_LABEL = 'Terminal drawer';

/** Thread items the sidebar row already offers (runtimeMenu.ts, threadMenu.ts). */
const ROW_ONLY = new Set([
  'Pin thread', 'Unpin thread', 'Open beside', 'Open beside in Split',
  'Move to History', 'Move back to Active', 'Settle to History',
]);
/** Side by side needs room; the hand-off is meaningless on the phone itself. */
const NOT_COMPACT = new Set(['Open with parent in Split', 'Open council side by side']);
const PHONE_HANDOFF = 'Continue on phone…';

const isTerminal = (i: MenuItem): boolean => !!i.label && /terminal drawer/i.test(i.label);

export interface PageMenuState {
  /** The focused thread's own actions (empty on the new-thread stage). */
  threadItems: MenuItem[];
  layout: ThreadsLayoutMode;
  /** The page's terminal drawer (under the stage). */
  terminalOpen: boolean;
  /** Null where the card can't show (phone / tablet: it heads the right panel). */
  cardOn: boolean | null;
  /** The focused thread, for "Related threads…"; null on the stage. */
  threadId: string | null;
  /** Phone / tablet layout: no layouts, no side-by-side. */
  compact?: boolean;
  /** A phone-width screen: no "Continue on phone". */
  phone?: boolean;
  /** The sidebar (with its New thread button) is on screen. */
  sidebarVisible?: boolean;
  /** While the agent works, Enter queues or steers (threadPrefs). */
  midTurn?: MidTurn;
  /** How many tool calls the timeline shows (threadPrefs). */
  tools?: ToolVisibility;
}

export interface PageMenuActions {
  newThread(): void;
  related(id: string): void;
  toggleCard(): void;
  layout(mode: ThreadsLayoutMode): void;
  toggleTerminal(): void;
  setMidTurn?(m: MidTurn): void;
  setTools?(t: ToolVisibility): void;
}

export function threadsPageMenu(s: PageMenuState, act: PageMenuActions): MenuItem[] {
  const compact = s.compact === true;
  // The thread's own terminal is the one entry unless the page's drawer is
  // open (then that one, checked, so it can be closed from here).
  const threadTerminal = s.threadItems.some(isTerminal);
  const pageTerminal = !threadTerminal || s.terminalOpen;
  const thread = trimSeparators(
    s.threadItems.filter(
      (i) =>
        !(i.label && ROW_ONLY.has(i.label)) &&
        !(compact && i.label && NOT_COMPACT.has(i.label)) &&
        !(s.phone && i.label === PHONE_HANDOFF) &&
        !(pageTerminal && isTerminal(i)),
    ),
  );
  const page: MenuItem[] = [
    ...(compact || s.sidebarVisible === false ? [{ label: 'New thread', icon: 'edit', hint: '⌘T', action: () => act.newThread() }] : []),
    // A terminal thread's own menu already carries it (threadMenu.ts).
    ...(s.threadId && !s.threadItems.some((i) => i.label === RELATED_LABEL)
      ? [{ label: RELATED_LABEL, icon: 'people', action: () => act.related(s.threadId!) }] : []),
    ...(pageTerminal
      ? [{ label: TERMINAL_LABEL, icon: 'terminal', checked: s.terminalOpen, hint: '/terminal', action: () => act.toggleTerminal() }] : []),
    ...viewMenu(s, act),
  ];
  return thread.length ? [...thread, { separator: true }, ...page] : page;
}

/** Layout (and the project card where it can float), then the thread
 *  prefs: one row, a flyout. A phone / tablet keeps only the prefs. */
function viewMenu(s: PageMenuState, act: PageMenuActions): MenuItem[] {
  const layout: MenuItem[] = s.compact === true ? [] : [
    { label: 'Single thread', hint: '1-up', checked: s.layout === 'single', action: () => act.layout('single') },
    { label: 'Split', hint: '2-up', checked: s.layout === 'split', action: () => act.layout('split') },
    { label: 'Grid', hint: '4-up', checked: s.layout === 'grid', action: () => act.layout('grid') },
    ...(s.cardOn !== null
      ? [{ separator: true }, { label: 'Project card', icon: 'note', checked: s.cardOn, action: () => act.toggleCard() }] : []),
  ];
  const prefs: MenuItem[] = s.midTurn && act.setMidTurn ? [
    { label: 'While working, messages queue', hint: s.compact ? undefined : '⌘↵ steers', title: 'A message sent while the agent works waits for the turn to end', checked: s.midTurn === 'queue', action: () => act.setMidTurn!('queue') },
    { label: 'While working, messages steer', hint: s.compact ? undefined : '⌘↵ queues', title: 'A message sent while the agent works goes into the running turn; the agent reads it at its next step', checked: s.midTurn === 'steer', action: () => act.setMidTurn!('steer') },
  ] : [];
  const TOOLS: [ToolVisibility, string, string][] = [
    ['all', 'Tool calls: show all', 'Every command, edit and search the agent ran'],
    ['hide-ok', 'Tool calls: hide successful', 'Only failed or gated calls; replies, questions and approvals always show'],
    ['none', 'Tool calls: hide all', 'No tool calls except failures and approvals; replies and questions always show'],
  ];
  const tools: MenuItem[] = s.tools && act.setTools
    ? TOOLS.map(([v, label, title]) => ({ label, title, checked: s.tools === v, action: () => act.setTools!(v) })) : [];
  const children = trimSeparators([...layout, { separator: true }, ...tools, { separator: true }, ...prefs]);
  return children.length ? [{ label: 'View', icon: 'layout', children }] : [];
}

/** No separator first, last or twice in a row once items are filtered out. */
function trimSeparators(items: MenuItem[]): MenuItem[] {
  const out: MenuItem[] = [];
  for (const i of items) {
    if (i.separator && (out.length === 0 || out[out.length - 1].separator)) continue;
    out.push(i);
  }
  while (out.length && out[out.length - 1].separator) out.pop();
  return out;
}
