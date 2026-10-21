// The Insights report frame's link bridge, out of ReportDetail.svelte so its
// security rules are tested by behaviour (ui/unit/frameMessages.test.ts), not
// by matching the component's source.
//
// The report iframe is sandboxed WITHOUT `allow-popups`, so its links can't
// open themselves; the report's script posts `{type:'otto-insights-open',
// url}` instead and the bridge opens it in the system browser — but only when:
//   - the message comes from THIS frame's window (never another frame/tab);
//   - the URL is http(s) (no `javascript:`, `file:`, `otto:` …);
//   - the frame is under a real user gesture (a click inside the frame
//     activates its ancestors too), so a script can't open tabs on its own;
//   - at most one open per OPEN_GAP_MS, so a script can't spray tabs.

/** Minimum gap between two opens from the report frame. */
export const OPEN_GAP_MS = 1_000;

/** The parts of a `MessageEvent` the bridge reads. */
export interface FrameMessage {
  source: unknown;
  data: unknown;
}

export interface ReportLinkBridgeDeps {
  /** The report iframe's `contentWindow` right now (null before mount). */
  frameWindow: () => unknown;
  /** Open the URL outside the app. */
  open: (url: string) => void;
  /** Clock (ms). Defaults to `Date.now`. */
  now?: () => number;
  /** Is a user gesture active? `undefined` = the browser has no
   *  `navigator.userActivation`, which never blocks. */
  userActive?: () => boolean | undefined;
}

function browserUserActive(): boolean | undefined {
  const nav = (globalThis as { navigator?: { userActivation?: { isActive: boolean } } }).navigator;
  return nav?.userActivation?.isActive;
}

/** A `message` handler for the report frame; returns true when it opened. */
export function reportLinkBridge(deps: ReportLinkBridgeDeps): (ev: FrameMessage) => boolean {
  const now = deps.now ?? Date.now;
  const userActive = deps.userActive ?? browserUserActive;
  let lastOpenAt = Number.NEGATIVE_INFINITY;
  return (ev) => {
    const win = deps.frameWindow();
    if (win == null || ev.source == null || ev.source !== win) return false;
    const m = ev.data as { type?: unknown; url?: unknown } | null;
    if (!m || m.type !== 'otto-insights-open' || typeof m.url !== 'string') return false;
    if (!/^https?:\/\//i.test(m.url)) return false;
    if (userActive() === false) return false;
    const t = now();
    if (t - lastOpenAt < OPEN_GAP_MS) return false;
    lastOpenAt = t;
    deps.open(m.url);
    return true;
  };
}
