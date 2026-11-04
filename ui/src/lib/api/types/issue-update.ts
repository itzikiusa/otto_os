// Issue edit — mirrors otto-issues `issue_update.rs`.

/**
 * PATCH /issue/{account_id}/issues/{key} (Jira only). Every field optional,
 * at least one required; an omitted field is left unchanged.
 */
export interface UpdateIssueReq {
  /** Single line, at most 255 characters. */
  summary?: string;
  /** Markdown — converted to ADF as on create. 409 if the current description can't round-trip. */
  description?: string;
  /** Replaces the label set (`[]` clears). Not combinable with add/remove. */
  labels?: string[];
  add_labels?: string[];
  remove_labels?: string[];
  /** A priority name or id. */
  priority?: string;
  /** A Jira accountId; `null` or `"none"` unassigns. Omit to leave it unchanged. */
  assignee?: string | null;
  /** The parent issue key. */
  parent?: string;
  /** An Epic's key (becomes the parent). */
  epic?: string;
}

/** Response of PATCH /issue/{account_id}/issues/{key}. */
export interface UpdateIssueResp {
  key: string;
  url: string;
  /** The fields written, e.g. `["summary", "labels"]`. */
  updated: string[];
}
