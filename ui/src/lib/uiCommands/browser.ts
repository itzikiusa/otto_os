// Agent UI control — Browser handlers (`otto.ui_browser_*`). They drive the
// Browser module's store (stores/browser.svelte.ts) in the document showing
// it: tabs open and navigate in the strip, the reader page renders, a mark
// highlights like the user's own, and a summary lands in the same panel as a
// click on Summarize — the user sees every step.
//
// Risk (docs/contracts/ui-commands.json): opening / navigating / switching
// mode is `navigate` (the daemon's netguard still checks every URL); reading
// and summarizing are `read`; a mark is persisted — `local_write`, an
// attributed confirm the user may remember for the session.

import { registerUiCommands, registerUiState, UiCommandError, whenMounted, type UiCommandCtx } from '../uiCommands';
import { browser } from '../stores/browser.svelte';
import { ws } from '../stores/workspace.svelte';
import * as browserApi from '../api/browser';
import type { BrowserTab } from '../api/types';
import { buildSelector } from '../../modules/browser/selector';
import { asUiError, capList, highlightWhenReady, waitFor } from './pagePort';

const PAGE_MAX_CHARS = 20_000;
const READER_ROOT = '.reader article.page';
/** Blocks a `text_contains` search considers, in document order. */
const TEXT_BLOCKS = 'p, li, h1, h2, h3, h4, h5, h6, pre, blockquote, td, th, dt, dd, figcaption';

/** `example.com/x` → `https://example.com/x` (the address bar's rule). */
export function normalizeUrl(raw: string): string {
  const t = (raw ?? '').trim();
  if (!t) throw new UiCommandError('invalid_args', '`url` is empty.');
  return /^[a-z][a-z0-9+.-]*:\/\//i.test(t) ? t : `https://${t}`;
}

/** Workspace open and this document's tab strip loaded. */
async function ready(signal: AbortSignal): Promise<string> {
  const wid = ws.currentId;
  if (!wid) throw new UiCommandError('failed', 'No workspace is open in this Otto window.');
  await waitFor(() => !browser.loadingTabs, signal, 15_000, 'the Browser tabs');
  if (browser.tabs.length === 0) await browser.loadTabs(wid);
  return wid;
}

function tabFor(id: unknown): BrowserTab {
  if (id === undefined || id === null || id === '') {
    const t = browser.activeTab;
    if (!t) throw new UiCommandError('not_found', 'No Browser tab is open — open one with otto.ui_browser_open.');
    return t;
  }
  const t = browser.tabs.find((x) => x.id === id);
  if (!t) throw new UiCommandError('not_found', `No Browser tab “${String(id)}” — otto.ui_browser_state lists them.`);
  return t;
}

/** Wait for the active tab's reader load to settle (≤ 40 s). */
async function settled(signal: AbortSignal): Promise<void> {
  await waitFor(() => !browser.loadingPage, signal, 40_000, 'the page to load');
}

function pageSummary() {
  const tab = browser.activeTab;
  return {
    tab_id: tab?.id ?? null,
    url: tab?.url ?? null,
    title: browser.page?.title || tab?.title || null,
    mode: tab?.mode ?? null,
    engine: browser.page?.engine ?? null,
    degraded: browser.page?.degraded ?? null,
    page_error: browser.pageError || null,
  };
}

registerUiState('browser', () => ({
  active_tab: browser.activeTab ? { id: browser.activeTab.id, url: browser.activeTab.url, mode: browser.activeTab.mode } : null,
  tabs: browser.tabs.length,
  marks: browser.annotations.length,
  summary_shown: !!browser.summary,
}));

registerUiCommands('browser', {
  async browser_state(_args: Record<string, never>, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    return {
      tabs: browser.tabs.map((t) => ({ id: t.id, url: t.url, title: t.title, mode: t.mode, active: t.id === browser.activeId })),
      ...pageSummary(),
      loading: browser.loadingPage,
      marks: capList(
        browser.annotations.map((a) => ({ id: a.id, selector: a.selector, text: a.text.slice(0, 200), comment: a.comment })),
        50,
      ),
      summary: browser.summary || null,
    };
  },

  async browser_open(args: { url: string; mode?: 'reader' | 'live' }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    const url = normalizeUrl(args.url);
    if (args.mode !== undefined && args.mode !== 'reader' && args.mode !== 'live') {
      throw new UiCommandError('invalid_args', `Unknown mode “${String(args.mode)}”.`);
    }
    try {
      const tab = args.mode === 'live' ? await browser.openLiveTab(url) : await browser.openTab(url);
      await settled(ctx.signal);
      await highlightWhenReady(ctx, '.strip .tab.active');
      return { ...pageSummary(), tab_id: tab.id };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async browser_navigate(args: { tab_id?: string; url: string }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    const url = normalizeUrl(args.url);
    if (args.tab_id) {
      const t = tabFor(args.tab_id);
      if (t.id !== browser.activeId) browser.select(t.id);
    }
    try {
      await browser.navigate(url);
      browser.summary = '';
      await settled(ctx.signal);
      return pageSummary();
    } catch (e) {
      throw asUiError(e);
    }
  },

  async browser_select_tab(args: { tab_id: string }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    const t = tabFor(args.tab_id);
    browser.select(t.id);
    await settled(ctx.signal);
    return pageSummary();
  },

  async browser_set_mode(args: { tab_id?: string; mode: 'reader' | 'live' }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    if (args.mode !== 'reader' && args.mode !== 'live') throw new UiCommandError('invalid_args', `Unknown mode “${String(args.mode)}”.`);
    const t = tabFor(args.tab_id);
    try {
      await browser.setMode(t.id, args.mode);
      await settled(ctx.signal);
      return pageSummary();
    } catch (e) {
      throw asUiError(e);
    }
  },

  async browser_get_page(args: { max_chars?: number }, ctx: UiCommandCtx) {
    const wid = await ready(ctx.signal);
    const tab = tabFor(undefined);
    await settled(ctx.signal);
    let page = browser.page && browser.activeTab?.id === tab.id ? browser.page : null;
    if (!page) {
      // A live tab has no reader page on screen: fetch its text without
      // changing what the user sees.
      try {
        page = await browserApi.getPage(wid, tab.url);
      } catch (e) {
        throw asUiError(e);
      }
    }
    const max = Math.min(Math.max(1, Math.trunc(args.max_chars ?? PAGE_MAX_CHARS)), 200_000);
    const md = page.markdown ?? '';
    return {
      tab_id: tab.id,
      url: page.url,
      title: page.title,
      engine: page.engine,
      degraded: page.degraded,
      markdown: md.length > max ? md.slice(0, max) : md,
      truncated: md.length > max,
    };
  },

  async browser_annotate(args: { selector?: string; text_contains?: string; comment?: string }, ctx: UiCommandCtx) {
    const wid = await ready(ctx.signal);
    const tab = tabFor(undefined);
    if (!args.selector && !args.text_contains) {
      throw new UiCommandError('invalid_args', 'Pass `selector` or `text_contains` to say which passage to mark.');
    }
    if (tab.mode === 'live') {
      throw new UiCommandError('invalid_args', 'Marks are made on reader pages — switch the tab to reader first (otto.ui_browser_set_mode).');
    }
    await settled(ctx.signal);
    const page = browser.page;
    if (!page) throw new UiCommandError('not_found', browser.pageError || 'The page is not loaded.');
    const root = await whenMounted(READER_ROOT, ctx.signal);
    let el: Element | null = null;
    if (args.selector) {
      try {
        el = root.querySelector(args.selector);
      } catch {
        throw new UiCommandError('invalid_args', `\`${args.selector}\` is not a valid CSS selector.`);
      }
    } else {
      const needle = args.text_contains!.trim().toLowerCase();
      el = [...root.querySelectorAll(TEXT_BLOCKS)].find((n) => (n.textContent ?? '').toLowerCase().includes(needle)) ?? null;
    }
    if (!el || el === root) throw new UiCommandError('not_found', 'No passage on the page matches — read it with otto.ui_browser_get_page.');
    const selector = buildSelector(el, root);
    const text = (el.textContent || '').trim().slice(0, 500);
    ctx.highlight(el);
    const comment = args.comment?.trim() ?? '';
    const ok = await ctx.confirmWrite({
      what: `Mark “${text.length > 200 ? `${text.slice(0, 199)}…` : text}”${comment ? `\n\nNote: ${comment}` : ''}`,
      where: page.title || page.url,
      verb: 'Mark',
      connId: `browser:${wid}`,
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the mark.');
    try {
      const ann = await browser.createAnnotation({
        url: page.url,
        selector,
        excerpt: el.outerHTML.slice(0, 2000),
        text,
        comment,
      });
      return { annotation_id: ann.id, selector, text };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async browser_summarize(_args: Record<string, never>, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    const tab = tabFor(undefined);
    if (browser.summarizing) throw new UiCommandError('failed', 'A summary is already being drafted — try again when it finishes.');
    ctx.progress(`Summarizing ${tab.title || tab.url}`);
    try {
      const summary = await browser.runSummarize(tab.url);
      await highlightWhenReady(ctx, '.summary');
      return { tab_id: tab.id, url: tab.url, summary };
    } catch (e) {
      throw asUiError(e);
    }
  },
});
