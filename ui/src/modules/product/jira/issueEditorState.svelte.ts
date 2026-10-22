// The Jira issue editor: status transitions, assignee, and the generic
// editmeta-driven field editor (story points, sprint, labels, priority…).
// Lifted out of OverviewTab so the editor is one unit any Jira view can host:
// the host supplies the issue it shows (and how to reload it), this owns the
// pickers, the drafts and every live write — each confirmed via
// `confirmOutward` naming the issue key and old → new.
//
// Writes go through `lib/api/issues` (encoded path segments). The pickers are
// global `ctxMenu` menus (viewport-clamped, height-capped) anchored under the
// clicked button.

import * as issues from '../../../lib/api/issues';
import { confirmOutward } from '../../../lib/confirmOutward';
import { ctxMenu, type MenuItem, type MenuOptions } from '../../../lib/contextmenu.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { refocusEditTrigger } from '../../../lib/inlineEdit';
import { toastError } from '../../../lib/toastError';
import { toasts } from '../../../lib/toast.svelte';
import type { EditableField, IssueFull, JiraTransition, JiraUser } from '../types';

/** The issue a host shows: enough to address it and to word the confirms. */
export interface EditedIssue {
  id: string;
  account_id: string;
  source_key: string;
  title: string;
}

/** What the hosting view provides. Every loader re-checks `isCurrent(s)` after
 *  its await so a response for an issue the view has left never lands. */
export interface IssueEditorHost {
  issue(): EditedIssue | null;
  full(): IssueFull | null;
  setFull(full: IssueFull): void;
  isCurrent(s: EditedIssue): boolean;
  /** Re-fetch the full issue after a write that doesn't answer it. */
  reload(): Promise<void>;
  /** After a status move (the host's list shows the status too). */
  afterTransition?(): Promise<void>;
}

/** Open a ctxMenu directly under `el` (works for mouse and keyboard). */
function menuUnder(el: HTMLElement, items: MenuItem[], opts?: MenuOptions): void {
  const r = el.getBoundingClientRect();
  ctxMenu.show(new MouseEvent('click', { clientX: r.left, clientY: r.bottom + 4 }), items, opts);
}

const SELECT_TYPES = ['option', 'priority', 'version', 'component'];

/** Build the Jira-shaped value for a field from the working draft. */
export function buildFieldValue(ef: EditableField, draft: unknown): unknown {
  switch (ef.schema_type) {
    case 'number': {
      const s = String(draft ?? '').trim();
      return s === '' ? null : Number(s);
    }
    case 'option':
    case 'priority':
    case 'version':
    case 'component': {
      const id = String(draft ?? '').trim();
      return id === '' ? null : { id };
    }
    case 'user': {
      const id = String(draft ?? '').trim();
      return id === '' ? null : { accountId: id };
    }
    case 'array': {
      if (ef.items === 'string') {
        // labels / free-text: split csv
        return String(draft ?? '')
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean);
      }
      const ids = Array.isArray(draft) ? (draft as string[]) : [];
      if (ef.items === 'user') return ids.map((id) => ({ accountId: id }));
      // option / version / component arrays
      return ids.map((id) => ({ id }));
    }
    case 'date':
      // <input type="date"> yields YYYY-MM-DD; Jira date fields accept this.
      return String(draft ?? '').trim() || null;
    case 'datetime': {
      // <input type="datetime-local"> yields "YYYY-MM-DDTHH:mm"; Jira datetime
      // fields require a full ISO-8601 string with offset — emit UTC.
      const raw = String(draft ?? '').trim();
      if (!raw) return null;
      return new Date(raw).toISOString(); // e.g. "2024-01-15T10:00:00.000Z"
    }
    default:
      // unknown → raw string
      return String(draft ?? '');
  }
}

export class JiraIssueEditor {
  // Status transition
  transitions = $state<JiraTransition[]>([]);
  transitionsLoading = $state(false);
  transitionsLoaded = $state(false);
  transitionWorking = $state(false);

  // Assignee
  assignables = $state<JiraUser[]>([]);
  assignablesLoading = $state(false);
  assignablesLoaded = $state(false);
  assigneeWorking = $state(false);

  // editmeta is lazily fetched once per issue; null = not loaded yet.
  editmeta = $state<EditableField[] | null>(null);
  editmetaLoading = $state(false);
  /** Last failed editmeta load (human text) — shown inline with Retry; the
   *  on-open effect gates on it so a failure never retry-loops. */
  editmetaError = $state<string | null>(null);
  editingField = $state<string | null>(null); // field key currently in edit mode
  fieldDraft = $state<unknown>(null); // working value for the field being edited
  fieldSaving = $state(false);

  /** The numeric estimate field (Story Points / Original Estimate), if editable.
   *  Pure read of editmeta — safe inside $derived. Null when none. */
  estimateField = $derived(
    this.editmeta?.find((f) => f.schema_type === 'number' && /story point|estimate/i.test(f.name)) ?? null,
  );

  private readonly host: IssueEditorHost;

  constructor(host: IssueEditorHost) {
    this.host = host;
  }

  /** Drop everything (the host switched issues): no picker, draft or loaded
   *  list of A may be offered — or saved — against B. */
  reset(): void {
    this.transitions = [];
    this.transitionsLoaded = false;
    this.transitionsLoading = false;
    this.assignables = [];
    this.assignablesLoaded = false;
    this.assignablesLoading = false;
    this.editmeta = null;
    this.editmetaLoading = false;
    this.editmetaError = null;
    this.editingField = null;
    this.fieldDraft = null;
    this.fieldSaving = false;
  }

  /** Destination line for a Jira write: "Jira GS-123 “Summary”". */
  where(): string {
    const s = this.host.issue();
    const key = s?.source_key ?? '';
    const summary = this.host.full()?.summary ?? s?.title ?? '';
    return summary ? `Jira ${key} “${summary}”` : `Jira ${key}`;
  }

  // ── Status ────────────────────────────────────────────────────────────────

  async loadTransitions(): Promise<void> {
    const s = this.host.issue();
    if (this.transitionsLoaded || !s) return;
    this.transitionsLoading = true;
    try {
      const rows = await issues.transitions(s.account_id, s.source_key);
      // Transition ids are per issue: A's ids must never be offered (and
      // POSTed) against B after a switch during this await.
      if (!this.host.isCurrent(s)) return;
      this.transitions = rows;
      this.transitionsLoaded = true;
    } catch (e) {
      if (this.host.isCurrent(s)) toastError('Couldn’t load transitions', e);
    } finally {
      if (this.host.isCurrent(s)) this.transitionsLoading = false;
    }
  }

  async openTransitionMenu(e: MouseEvent): Promise<void> {
    const el = e.currentTarget as HTMLElement;
    await this.loadTransitions();
    if (!this.transitionsLoaded) return;
    const items: MenuItem[] =
      this.transitions.length === 0
        ? [{ label: 'No transitions available', disabled: true }]
        : this.transitions.map((t) => ({
            label: t.name === t.to_status ? `${t.name}…` : `${t.name} → ${t.to_status}…`,
            action: () => void this.applyTransition(t),
          }));
    menuUnder(el, items);
  }

  async applyTransition(t: JiraTransition): Promise<void> {
    const s = this.host.issue();
    if (!s) return;
    const key = s.source_key;
    const ok = await confirmOutward({
      verb: `Move to ${t.to_status}`,
      title: `Move ${key} to ${t.to_status}?`,
      where: this.where(),
      what: `Status: ${this.host.full()?.status ?? 'current'} → ${t.to_status}${t.name !== t.to_status ? ` (transition “${t.name}”)` : ''}`,
      who: `Everyone with access to ${key} sees the change; watchers are notified and Jira workflow rules may run.`,
    });
    if (!ok || !this.host.isCurrent(s)) return;
    this.transitionWorking = true;
    try {
      await issues.applyTransition(s.account_id, s.source_key, t.id);
      // Available transitions depend on the status — refetch on next open, or
      // the menu (and its confirm's "from → to") offers the old status's moves.
      this.transitionsLoaded = false;
      toasts.info('Status updated');
      await this.host.reload();
      await this.host.afterTransition?.();
    } catch (e) {
      toastError('Couldn’t move the issue', e);
    } finally {
      this.transitionWorking = false;
    }
  }

  // ── Assignee ──────────────────────────────────────────────────────────────

  async loadAssignables(): Promise<void> {
    const s = this.host.issue();
    if (this.assignablesLoaded || !s) return;
    this.assignablesLoading = true;
    try {
      const rows = await issues.assignableUsers(s.account_id, s.source_key);
      if (!this.host.isCurrent(s)) return;
      this.assignables = rows;
      this.assignablesLoaded = true;
    } catch (e) {
      if (this.host.isCurrent(s)) toastError('Couldn’t load assignable users', e);
    } finally {
      if (this.host.isCurrent(s)) this.assignablesLoading = false;
    }
  }

  async openAssigneeMenu(e: MouseEvent): Promise<void> {
    const el = e.currentTarget as HTMLElement;
    await this.loadAssignables();
    if (!this.assignablesLoaded) return;
    const items: MenuItem[] = [
      { label: 'Unassign…', icon: 'x', pinned: true, disabled: !this.host.full()?.assignee, action: () => void this.assignUser(null) },
      ...(this.assignables.length === 0
        ? [{ label: 'No users found', disabled: true } as MenuItem]
        : this.assignables.map((u) => ({ label: `${u.display_name}…`, action: () => void this.assignUser(u) }) as MenuItem)),
    ];
    menuUnder(el, items, { filter: this.assignables.length > 8, filterPlaceholder: 'Find a person…', maxVisible: 12 });
  }

  /** Assign `u`, or unassign when null — confirmed first. */
  async assignUser(u: JiraUser | null): Promise<void> {
    const s = this.host.issue();
    if (!s) return;
    const key = s.source_key;
    const cur = this.host.full()?.assignee?.display_name ?? 'Unassigned';
    const ok = await confirmOutward({
      verb: u ? `Assign to ${u.display_name}` : 'Unassign',
      title: u ? `Assign ${key}?` : `Unassign ${key}?`,
      where: this.where(),
      what: `Assignee: ${cur} → ${u ? u.display_name : 'Unassigned'}`,
      who: `Jira notifies the old and new assignee and ${key}'s watchers.`,
    });
    if (!ok || !this.host.isCurrent(s)) return;
    this.assigneeWorking = true;
    try {
      await issues.setAssignee(s.account_id, s.source_key, u ? u.account_id : '');
      toasts.info('Assignee updated');
      await this.host.reload();
    } catch (e) {
      toastError('Couldn’t assign', e);
    } finally {
      this.assigneeWorking = false;
    }
  }

  // ── Editable fields ───────────────────────────────────────────────────────

  /** Lazily fetch editmeta once and cache it. A failure leaves `editmeta`
   *  null and records `editmetaError` (rendered inline with Retry — never a
   *  toast that vanishes while every field silently stays read-only). */
  async ensureEditmeta(): Promise<void> {
    const s = this.host.issue();
    if (this.editmeta !== null || this.editmetaLoading || !s) return;
    this.editmetaLoading = true;
    this.editmetaError = null;
    try {
      const meta = await issues.editMeta(s.account_id, s.source_key);
      if (!this.host.isCurrent(s)) return;
      this.editmeta = meta;
    } catch (e) {
      if (!this.host.isCurrent(s)) return;
      this.editmetaError = loadErrorText(e);
    } finally {
      if (this.host.isCurrent(s)) this.editmetaLoading = false;
    }
  }

  /** Retry a failed editmeta load (the inline LoadState's Retry). */
  retryEditmeta(): void {
    this.editmetaError = null;
    void this.ensureEditmeta();
  }

  /** The EditableField metadata for a given key, or undefined if not editable. */
  editableFor(key: string): EditableField | undefined {
    return this.editmeta?.find((f) => f.key === key);
  }

  /** Raw current value for a field, sourced from the full issue
   *  (parse_issue_full strips ".0", so a plain number string seeds fine). */
  rawFieldValue(key: string): string {
    return this.host.full()?.fields.find((ff) => ff.key === key)?.value ?? '';
  }

  /** Enter edit mode for a field, seeding the draft from its current value. */
  async beginEdit(ef: EditableField, currentRaw: string): Promise<void> {
    await this.ensureEditmeta();
    if (!this.editableFor(ef.key)) return; // not actually editable — stay read-only
    if (ef.schema_type === 'array') {
      if (ef.items === 'string') {
        // labels / free-text array: comma text
        this.fieldDraft = currentRaw;
      } else {
        // array of options/users: list of selected ids (best-effort from labels)
        const labels = currentRaw.split(',').map((s) => s.trim()).filter(Boolean);
        this.fieldDraft = ef.allowed_values.filter((o) => labels.includes(o.label)).map((o) => o.id);
      }
    } else if (SELECT_TYPES.includes(ef.schema_type)) {
      // select: pre-select by matching label → id
      this.fieldDraft = ef.allowed_values.find((o) => o.label === currentRaw)?.id ?? '';
    } else if (ef.schema_type === 'user') {
      await this.loadAssignables();
      this.fieldDraft = '';
    } else if (ef.schema_type === 'datetime') {
      // Seed the datetime-local input ("YYYY-MM-DDTHH:mm", local time) from an
      // existing ISO value.
      this.fieldDraft = '';
      if (currentRaw) {
        const d = new Date(currentRaw);
        if (!Number.isNaN(d.getTime())) {
          const local = new Date(d.getTime() - d.getTimezoneOffset() * 60000);
          this.fieldDraft = local.toISOString().slice(0, 16);
        }
      }
    } else {
      // string / number / date / unknown → raw text
      this.fieldDraft = currentRaw;
    }
    this.editingField = ef.key;
  }

  /** Cancel the in-progress field edit. */
  cancelEdit(): void {
    const key = this.editingField;
    this.editingField = null;
    this.fieldDraft = null;
    if (key) void refocusEditTrigger(key);
  }

  /** Toggle membership of an option id in the multi-select array draft. */
  toggleArrayOption(id: string): void {
    const cur = Array.isArray(this.fieldDraft) ? (this.fieldDraft as string[]) : [];
    this.fieldDraft = cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id];
  }

  /** Human-readable form of the working draft, for the outward confirm. */
  draftLabel(ef: EditableField, draft: unknown): string {
    const opt = (id: string) => ef.allowed_values.find((o) => o.id === id)?.label ?? id;
    const user = (id: string) => this.assignables.find((u) => u.account_id === id)?.display_name ?? id;
    if (ef.schema_type === 'array' && ef.items !== 'string') {
      const ids = Array.isArray(draft) ? (draft as string[]) : [];
      return ids.map(ef.items === 'user' ? user : opt).join(', ');
    }
    const raw = String(draft ?? '').trim();
    if (!raw) return '';
    if (ef.schema_type === 'user') return user(raw);
    if (SELECT_TYPES.includes(ef.schema_type)) return opt(raw);
    return raw;
  }

  /** PUT a single field then swap in the refreshed issue the server answers. */
  async saveField(ef: EditableField): Promise<void> {
    const s = this.host.issue();
    if (!s) return;
    const draft = this.fieldDraft;
    // A custom field is a live Jira write like status / title (S18-302): it
    // notifies watchers and can fire Jira automations, so name the issue and
    // show old → new before sending.
    const before = this.rawFieldValue(ef.key).trim() || '(empty)';
    const after = this.draftLabel(ef, draft) || '(empty)';
    const ok = await confirmOutward({
      verb: 'Update field',
      title: `Update ${ef.name} on ${s.source_key}?`,
      where: this.where(),
      what: `${ef.name}: “${before}” → “${after}”`,
      who: `Everyone with access to ${s.source_key} sees the change; watchers are notified and Jira automations may run.`,
    });
    if (!ok || !this.host.isCurrent(s) || this.editingField !== ef.key) return;
    this.fieldSaving = true;
    try {
      const full = await issues.updateFields(s.account_id, s.source_key, { [ef.key]: buildFieldValue(ef, draft) });
      if (!this.host.isCurrent(s)) return;
      this.host.setFull(full);
      this.editingField = null;
      this.fieldDraft = null;
      void refocusEditTrigger(ef.key);
      toasts.info('Field updated');
    } catch (e) {
      toastError('Couldn’t update field', e);
    } finally {
      this.fieldSaving = false;
    }
  }
}
