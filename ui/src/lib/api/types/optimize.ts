// Optimize (addendum-chat-optimize.md §2.6; crates/otto-workflows/src/optimize.rs):
// the rule-based draft of a faster workflow version and the apply body.
import type { WorkflowGraph } from './workflows';

export type OptimizeRule =
  | 'parallel'
  | 'git_checkout'
  | 'go_gates_script'
  | 'step_timeout'
  | 'single_repo_skip'
  | 'post_review_comments'
  | 'review_declared_lenses'
  | 'agent';

export type OptimizeEffect = 'same_contract' | 'may_change_output' | 'changes_findings';

/** One change `POST /workflows/{id}/optimize` offers (addendum §2.6). */
export interface OptimizeChange {
  /** `<rule>:<node id>`. */
  id: string;
  rule: OptimizeRule;
  nodes: string[];
  title: string;
  why: string;
  effect: OptimizeEffect;
  preselected: boolean;
  /** Absent under a second. */
  saving_ms?: number;
}

export interface OptimizeDraft {
  base_version: number;
  changes: OptimizeChange[];
  not_offered: { rule: OptimizeRule; nodes: string[]; reason: string }[];
  draft_graph: WorkflowGraph;
  /** The four times are absent (`note` says why) under 3 runs or when the
   *  draft is not faster beyond the noise between runs. */
  estimate: {
    runs: number;
    since?: string;
    typical_before_ms?: number;
    typical_after_ms?: number;
    slow_before_ms?: number;
    slow_after_ms?: number;
    note?: string;
  };
  checks: {
    numbering: { step_no: number; node_id: string; ok: boolean }[];
    renumbered: { node_id: string; from: string; to: string }[];
    files: {
      node_id: string;
      reads: string[];
      writes: string[];
      reads_added: string[];
      reads_lost: string[];
      listing_removed: string[];
    }[];
    warnings: string[];
  };
}

/** `POST /workflows/{id}/optimize/apply` body. */
export interface OptimizeApplyReq {
  base_version: number;
  change_ids: string[];
}
