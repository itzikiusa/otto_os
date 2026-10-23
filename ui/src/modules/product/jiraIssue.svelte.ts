// The Jira issue an Overview tab shows (architecture-5-04): the full issue
// (`/issues/.../full`), its development info (linked branches / commits / PRs
// via Jira dev-status) and the `JiraIssueEditor` that writes to it. Lifted out
// of OverviewTab so the tab keeps the story body and its own inline editors,
// and `JiraIssueSection.svelte` renders the Details sidebar from this.
//
// Every loader captures its story and re-checks `host.isCurrent(s)` after the
// await, so a response for a story the tab has left never lands.

import * as issues from '../../lib/api/issues';
import { loadErrorText } from '../../lib/loadError';
import { JiraIssueEditor, type EditedIssue } from './jira/issueEditorState.svelte';
import type { DevStatus, IssueFull } from './types';

export interface JiraIssueHost {
  /** The story the tab shows (null while none is loaded). */
  story(): EditedIssue | null;
  isCurrent(s: EditedIssue): boolean;
  /** After a status move (the story list shows the status too). */
  afterTransition(): Promise<void>;
}

export class JiraIssue {
  full = $state<IssueFull | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);

  // Development info: lazily fetched once the section is expanded.
  dev = $state<DevStatus | null>(null);
  devLoading = $state(false);
  devLoaded = $state(false); // guards the lazy-load (distinct from "has data")
  devError = $state<string | null>(null);

  /** Status, assignee and the editmeta field editor (jira/issueEditorState).
   *  Every write is a confirmed live Jira write against THIS tab's story. */
  readonly editor: JiraIssueEditor;
  private readonly host: JiraIssueHost;

  constructor(host: JiraIssueHost) {
    this.host = host;
    this.editor = new JiraIssueEditor({
      issue: () => host.story(),
      full: () => this.full,
      setFull: (full) => { this.full = full; },
      isCurrent: (s) => host.isCurrent(s),
      reload: () => this.load(),
      afterTransition: () => host.afterTransition(),
    });
  }

  /** The tab switched stories. Loading flags too: a stale `loading` blocked
   *  B's own /full load and let A's response win (S18-02). */
  reset(): void {
    this.full = null;
    this.error = null;
    this.loading = false;
    this.editor.reset();
    this.dev = null;
    this.devLoaded = false;
    this.devLoading = false;
    this.devError = null;
  }

  async load(): Promise<void> {
    const s = this.host.story();
    if (!s) return;
    this.loading = true;
    this.error = null;
    try {
      const full = await issues.issueFull(s.account_id, s.source_key);
      if (!this.host.isCurrent(s)) return;
      this.full = full;
    } catch (e) {
      if (this.host.isCurrent(s)) this.error = loadErrorText(e);
    } finally {
      if (this.host.isCurrent(s)) this.loading = false;
    }
  }

  // Best-effort dev-status fetch. Early-returns once loaded so the expand-trigger
  // and the on-open effect can both call it safely.
  async loadDev(): Promise<void> {
    const s = this.host.story();
    if (this.devLoaded || !s) return;
    this.devLoading = true;
    this.devError = null;
    try {
      const dev = await issues.devStatus(s.account_id, s.source_key, this.full?.id);
      if (!this.host.isCurrent(s)) return;
      this.dev = dev;
    } catch (e) {
      if (this.host.isCurrent(s)) this.devError = loadErrorText(e);
    } finally {
      // Mark loaded even on error: the on-open $effect gates on `devLoaded`, so
      // leaving it false after a failed fetch would re-trigger this every time
      // the request settles → an infinite retry loop. (Same guard rationale as
      // ensureEditmeta's `editmetaError` gate.) `reloadDev` resets it to force a
      // fresh fetch.
      if (this.host.isCurrent(s)) {
        this.devLoaded = true;
        this.devLoading = false;
      }
    }
  }

  /** Force the dev-status section to repopulate with fresh data (Refresh). */
  async reloadDev(): Promise<void> {
    this.devLoaded = false;
    this.dev = null;
    await this.loadDev();
  }
}
