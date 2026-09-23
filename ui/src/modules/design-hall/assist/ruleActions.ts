// Human-only decisions on learned team rules — approve / reject a proposal,
// roll back an approved rule — through otto-improve's edit flow. Every one
// asks first (what changes, where it lives, that it can be undone) and reports
// the outcome; the learning page and the lobby rail share them.

import { ApiError } from '../../../lib/api/client';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import type { DesignLearnedEdit, DesignLearnedRule } from '../../../lib/api/types';
import { ruleEdits } from './api';
import { editHeadline } from './model';

function why(e: unknown): string {
  if (e instanceof ApiError && e.status === 403) return 'Deciding on rules needs Self-improvement edit access in this workspace.';
  if (e instanceof ApiError && e.status === 409) return 'The rules file changed since this was proposed. Otto re-proposes it against the current file on the next pass.';
  return e instanceof Error ? e.message : String(e);
}

const quote = (s: string) => (s.length > 160 ? `${s.slice(0, 159)}…` : s);

export async function approveRule(edit: DesignLearnedEdit, skill = 'design-team-style'): Promise<boolean> {
  const ok = await confirmer.ask(
    `Add “${quote(editHeadline(edit))}” to your team rules (skill ${skill})? Otto follows it in every design turn in this workspace. You can roll it back any time.`,
    { title: 'Approve team rule', confirmLabel: 'Approve rule', danger: false },
  );
  if (!ok) return false;
  try {
    await ruleEdits.approve(edit.edit_id);
    toasts.success(`Added to ${skill}`, 'Rollback is available under Rules.');
    return true;
  } catch (e) {
    toasts.error('Couldn’t approve the rule', why(e));
    return false;
  }
}

export async function rejectRule(edit: DesignLearnedEdit): Promise<boolean> {
  const ok = await confirmer.ask(
    `Reject “${quote(editHeadline(edit))}”? Otto won’t propose it again. The signals it came from stay in the log.`,
    { title: 'Reject proposal', confirmLabel: 'Reject', danger: false },
  );
  if (!ok) return false;
  try {
    await ruleEdits.reject(edit.edit_id);
    toasts.info('Proposal rejected', 'Otto won’t propose this rule again.');
    return true;
  } catch (e) {
    toasts.error('Couldn’t reject the proposal', why(e));
    return false;
  }
}

export async function rollbackRule(rule: DesignLearnedRule): Promise<boolean> {
  if (!rule.edit_id) {
    toasts.warn('This rule can’t be rolled back here', 'It wasn’t added through an approved proposal — edit the skill file instead.');
    return false;
  }
  const ok = await confirmer.ask(
    `Roll back “${quote(rule.rule)}”? Otto stops following it from the next design turn. Its history is kept.`,
    { title: 'Roll back team rule', confirmLabel: 'Roll back', danger: true },
  );
  if (!ok) return false;
  try {
    await ruleEdits.rollback(rule.edit_id);
    toasts.success('Rule rolled back');
    return true;
  } catch (e) {
    toasts.error('Couldn’t roll back the rule', why(e));
    return false;
  }
}
