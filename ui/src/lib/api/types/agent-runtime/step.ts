// Mirror of crates/otto-core/src/agent_runtime/step.rs: what an unattended
// agent step reports while it runs and leaves behind when it settles. Shared
// by the step runners and the workflow run record (`NodeRunState`).

import type { Id } from '../core';
import type { TurnTokenUsage } from './events';

/** Why a step has not started yet: the machine's admission gate (`slots` ·
 *  `memory` · `load`) or the engine (`run_slots` · `run_order`). `text` is
 *  shown as is. */
export interface WaitReason {
  code: string;
  text: string;
  since: string;
  detail?: WaitDetail;
}

export interface WaitDetail {
  live: number;
  max?: number;
  floor?: number;
  ahead?: number;
  /** Labels of the agents holding the places. */
  holders?: string[];
}

/** `unknown`: a kind a newer daemon added. */
export type AttentionKind = 'approval' | 'question' | 'usage_limit' | 'verify' | 'unknown';
export type AttentionVia = 'provider' | 'mcp';

/** A live wait on a person. Answered through
 *  `POST /runtime-sessions/{thread_id}/requests/{request_id}`. */
export interface Attention {
  kind: AttentionKind;
  via?: AttentionVia;
  /** ≤ 300 chars. */
  text: string;
  since: string;
  thread_id?: Id;
  request_id?: Id;
  /** `usage_limit`: when the provider's limit resets. */
  resets_at?: string;
}

export type OutwardOutcome = 'ok' | 'failed' | 'pending_approval';

/** An Otto MCP call a step's thread made that reaches outside Otto. */
export interface OutwardAction {
  tool: string;
  target?: string;
  outcome: OutwardOutcome;
  message?: string;
}

/** The compact record of a step's thread (≤ 8 KiB), kept after the thread is deleted. */
export interface TranscriptDigest {
  turns: number;
  tool_calls: number;
  files_changed: string[];
  last_answer_excerpt: string;
  errors: string[];
  actions: OutwardAction[];
  started_at?: string;
  ended_at?: string;
}

export interface DigestItem {
  role: string;
  text: string;
  tool?: string;
}

export interface SubThreadUse {
  id: Id;
  provider: string;
  model?: string;
  title?: string;
  status: string;
  usage: TurnTokenUsage;
}
