// Personal-agent templates offered by the "New agent" sheet. A template
// pre-fills name / avatar / persona and, on create, adds its first schedule.
// Templates ship no avatar: an empty one renders as the name's monogram.
// The persona carries a marker comment so other modules (the Kubernetes
// Monitor's Insights tab) can find agents created from it.
//
// The ONE source is `docs/contracts/personal-agent-templates.json`: the
// daemon include_str!s the same file (`GET /personal-agents/templates`,
// `otto.create_personal_agent {template}`), so an agent and this sheet always
// offer the same templates.

import raw from '../../../../docs/contracts/personal-agent-templates.json';

export const K8S_WATCHDOG_MARK = '<!-- otto-template:k8s-watchdog -->';
export const SLACK_DIGEST_MARK = '<!-- otto-template:slack-daily-digest -->';

export interface AgentTemplate {
  id: string;
  title: string;
  description: string;
  avatar: string;
  name: string;
  soul_md: string;
  /** `null` = no routine (the main agent is set up by chatting). */
  schedule: { cadence: 'interval'; every_min: number } | { cadence: 'daily'; at: string } | null;
  directive: string;
  /** Creating from it makes the agent the workspace's main agent. */
  primary?: boolean;
}

const TEMPLATES = raw.templates as AgentTemplate[];

export function agentTemplates(): AgentTemplate[] {
  return TEMPLATES;
}

export function templateById(id: string): AgentTemplate | undefined {
  return TEMPLATES.find((t) => t.id === id);
}
