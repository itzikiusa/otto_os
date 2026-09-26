import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
test.use({ serviceWorkers: 'block' });
test.beforeEach(async ({ page }) => {
 page.on('pageerror', e => console.log('R4_SETTINGS_PAGEERROR', e.stack ?? e.message));
});
const stamp = '2026-09-25T12:00:00Z';
const finding = { severity: 'Medium', code: 'MISSING_EXAMPLE', title: 'Show the recovery workflow', evidence: 'SKILL.md:42 has no retry example.', why: 'Callers need a complete example after a transient error.', fix: 'Add a recovery example with the expected result.' };
function review(id: string, ws: string) { return { id, workspace_id: ws, skill_name: `${id} synthetic review`, skill_source: 'library', status: 'done', agent_mode: 'static', instructions: '', agents: [], static_report: { verdict: 'Ready with fixes', average_score: 4.2, scorecard: [{ area: 'spec_compliance', score: 4, notes: 'Clear trigger and recovery instructions.' }], findings: [finding] }, summary: null, error: null, created_at: stamp, updated_at: stamp }; }
function evaluation(ws: string) { return { id: 'synthetic-evaluation', workspace_id: ws, source_skill: 'repository-recovery-and-validation-guide', task: 'Verify retries preserve the current document and explain actionable failures.', impl_cli: 'codex', target_iterations: 1, status: 'done', summary: 'The final iteration passed validation. Review the captured evidence before promoting.', best_iteration: 1, best_score: 92, created_at: stamp, iterations: [{ id: 'iteration-one', eval_id: 'synthetic-evaluation', iter: 1, skill_name: 'repository-recovery-and-validation-guide', skill_before: '# Recovery\nRetry safely.', skill_after: '# Recovery\nRetry safely and retain edits.', impl_provider: 'codex', impl_summary: 'Retains edits while validation completes.', status: 'done', note: '', score: 92, agents: [{ name: 'Correctness validation', provider: 'codex', model: 'synthetic-model', validation: 'correctness', status: 'done', passed: true, score: 92, findings: [{ severity: 'warn', location: 'src/recovery.ts:42', issue: 'Retry feedback should identify the failed operation.', suggestion: 'Include the operation name.' }], note: '' }], improvement_summary: 'Added an explicit retry example.', skill_diff: '+ Retain unsaved changes.', human_rating: 4, created_at: stamp }] }; }
async function fixtures(page: Page) {
 const { ctx, base } = await apiCtx(); const ws = await seedWorkspace(ctx, base); await ctx.dispose();
 await page.route('**/api/v1/library/provider-skills', r => r.fulfill({ json: [] }));
 const run = evaluation(ws); const reviews = [review('alpha', ws), review('beta', ws)];
 await page.route('**/api/v1/workspaces/*/skill-evaluations', r => r.fulfill({ json: [run] }));
 await page.route('**/api/v1/skill-evaluations/synthetic-evaluation', r => r.fulfill({ json: run }));
 await page.route('**/api/v1/workspaces/*/skill-reviews', r => r.fulfill({ json: reviews }));
 await page.route('**/api/v1/skill-reviews/*', r => r.fulfill({ json: reviews.find(x => r.request().url().endsWith(x.id)) }));
 return { ws, run, reviews };
}
test('review selection ignores a late response after selecting another review or New', async ({ page }) => {
 const { reviews } = await fixtures(page); await page.goto('/#/skills-eval/review'); await expect(page.locator('.lr-detail h3')).toHaveText(reviews[0].skill_name);
 let release!: () => void; const held = new Promise<void>(resolve => release = resolve); let started = false;
 await page.route('**/api/v1/skill-reviews/beta', async r => { started = true; await held; await r.fulfill({ json: reviews[1] }); });
 await page.locator('.lr-item').filter({ hasText: 'beta synthetic review' }).click(); await expect.poll(() => started).toBe(true);
 await page.locator('.lr-item').filter({ hasText: 'alpha synthetic review' }).click();
 const response = page.waitForResponse(r => r.url().endsWith('/skill-reviews/beta')); release(); await (await response).finished();
 await expect(page.locator('.lr-detail h3')).toHaveText(reviews[0].skill_name);
 let releaseNew!: () => void; const heldNew = new Promise<void>(resolve => releaseNew = resolve); let newStarted = false;
 await page.route('**/api/v1/skill-reviews/beta', async r => { newStarted = true; await heldNew; await r.fulfill({ json: reviews[1] }); });
 await page.locator('.lr-item').filter({ hasText: 'beta synthetic review' }).click(); await expect.poll(() => newStarted).toBe(true); await page.getByTestId('new-skill-review').click(); await page.getByTestId('skill-review-instructions').fill('Keep new review instructions');
 const newResponse = page.waitForResponse(r => r.url().endsWith('/skill-reviews/beta')); releaseNew(); await (await newResponse).finished(); await expect(page.getByTestId('skill-review-instructions')).toHaveValue('Keep new review instructions');
});
test('running review polling cannot replace a newer explicit selection', async ({ page }) => {
 const { reviews } = await fixtures(page);
 reviews[0].status = 'running';
 await page.goto('/#/skills-eval/review');
 await expect(page.locator('.lr-detail h3')).toHaveText(reviews[0].skill_name);
 let release!: () => void;
 const held = new Promise<void>(resolve => { release = resolve; });
 let betaStarted = false;
 await page.route('**/api/v1/skill-reviews/beta', async route => {
  betaStarted = true; await held; await route.fulfill({ json: reviews[1] });
 });
 await page.locator('.lr-item').filter({ hasText: 'beta synthetic review' }).click();
 await expect.poll(() => betaStarted).toBe(true);
 // Keep the request pending beyond the running review's real fallback interval.
 await page.waitForTimeout(3000);
 const response = page.waitForResponse(r => r.url().endsWith('/skill-reviews/beta'));
 release(); await (await response).finished();
 await expect(page.locator('.lr-detail h3')).toHaveText(reviews[1].skill_name);
});
test('loaded evaluation actions fit five themes and tablet reports can use full detail width', async ({ page }) => {
 await fixtures(page);
 const variants = [
  { key: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900, direction: 'ltr' },
  { key: 'native-dark-phone', theme: 'native', scheme: 'dark', width: 375, height: 812, direction: 'ltr' },
  { key: 'warm-light-tablet-rtl', theme: 'warm', scheme: 'light', width: 834, height: 1112, direction: 'rtl' },
  { key: 'warm-dark', theme: 'warm', scheme: 'dark', width: 1440, height: 900, direction: 'ltr' },
  { key: 'pro-dark-phone', theme: 'pro-dark', scheme: 'dark', width: 375, height: 812, direction: 'ltr' },
 ];
 await page.goto('/#/skills-eval/evaluator');
 for (const v of variants) {
  await page.setViewportSize(v); await page.evaluate(v => { localStorage.setItem('otto_theme', v.theme); localStorage.setItem('otto_scheme', v.scheme); localStorage.setItem('otto_direction', v.direction); }, v); await page.reload();
  await expect(page.locator('.rd-title')).toContainText('repository-recovery');
  await page.screenshot({ path: `/tmp/otto-ux-r4-settings-eval-${v.key}-before.png` });
  if(v.width === 834) { await page.getByRole('button', { name: 'Hide evaluations list', exact: true }).click(); expect((await page.locator('.se-main').boundingBox())!.width).toBeGreaterThan(480); await page.getByRole('button', { name: 'Show evaluations list', exact: true }).press('Enter'); await expect(page.locator('.se-item.active')).toContainText('repository-recovery'); await page.getByRole('button', { name: 'Hide evaluations list', exact: true }).press('Enter'); }
  await page.getByRole('button', { name: 'Promote improved', exact: true }).scrollIntoViewIfNeeded();
  for(const row of await page.locator('.skill-actions, .rate-row, .improve-top, .iter-head').all()) expect(await row.evaluate(e => e.scrollWidth <= e.clientWidth + 1)).toBeTruthy();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: `/tmp/otto-ux-r4-settings-eval-${v.key}.png` });
 }
});
test('evaluation promotion checks proof, preserves retry input and posts the chosen version', async ({ page }) => {
 await fixtures(page); let fail = true; let body: unknown;
 await page.route('**/api/v1/skill-evaluations/*/promote-gate?*', r => r.fulfill({ json: { allowed: true, score: 92, threshold: 85, proof_status: 'passed', require_proof: true, score_ok: true, proof_ok: true, reasons: [] } }));
 await page.route('**/api/v1/skill-evaluations/*/promote', r => { body = r.request().postDataJSON(); return fail ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic library unavailable' } }) : r.fulfill({ json: { name: 'recovery-guide' } }); });
 await page.goto('/#/skills-eval/evaluator'); await page.getByRole('button', { name: 'Promote improved', exact: true }).click();
 await expect(page.getByTestId('gate-ok')).toBeVisible(); await page.getByLabel('Library skill name').fill('invalid name'); await expect(page.getByTestId('promote-confirm')).toBeDisabled();
 await page.getByLabel('Library skill name').fill('recovery-guide'); await page.getByTestId('promote-confirm').click(); await expect(page.getByText('Synthetic library unavailable', { exact: true })).toBeVisible(); await expect(page.getByLabel('Library skill name')).toHaveValue('recovery-guide');
 fail = false; await page.getByTestId('promote-confirm').click(); await expect(page.getByRole('dialog')).toHaveCount(0); expect(body).toEqual({ iteration_id: 'iteration-one', source: 'improved', name: 'recovery-guide', force: false });
});
test('golden task editing and matrix cells open the corresponding evaluated report', async ({ page }) => {
 const { ws, run } = await fixtures(page);
 let golden = { id: 'golden-one', workspace_id: ws, repo_key: ws, name: 'Recovery regression', prompt: 'Keep the current draft after retry.', skill: run.source_skill, test_cmd: 'npm test', lint_cmd: '', build_cmd: '', rubric: 'Draft survives', tags: [], origin: 'regression', enabled: true, created_by: 'root', created_at: stamp, updated_at: stamp };
 await page.route('**/api/v1/workspaces/*/golden-tasks', r => r.fulfill({ json: [golden] }));
 await page.route('**/api/v1/golden-tasks/golden-one', r => { golden = { ...golden, ...r.request().postDataJSON() }; return r.fulfill({ json: golden }); });
 await page.route('**/api/v1/golden-tasks/golden-one/run', r => r.fulfill({ json: run }));
 const matrix = { id: 'matrix-one', workspace_id: ws, name: 'Recovery comparison', status: 'done', repo_key: ws, mode: 'score_only', providers: ['codex'], skills: [run.source_skill], prompts: [{ label: 'Retry failure', task: 'Retain draft' }], cells: [{ eval_id: run.id, provider: 'codex', skill: run.source_skill, prompt: 'Retry failure', status: 'done', composite_score: 92, proof_status: 'passed', best_iteration: 1 }], created_at: stamp };
 await page.route('**/api/v1/workspaces/*/eval-matrices', r => r.fulfill({ json: [matrix] }));
 await page.route('**/api/v1/eval-matrices/matrix-one', r => r.fulfill({ json: matrix }));
 await page.goto('/#/skills-eval/evaluator/golden'); await page.getByTestId('golden-card').getByRole('button', { name: 'Edit', exact: true }).click();
 await page.getByTestId('golden-name').fill('Recovery regression updated'); await page.getByTestId('golden-save').click(); await expect(page.getByTestId('golden-card')).toContainText('Recovery regression updated');
 await page.screenshot({ path: '/tmp/otto-ux-r4-settings-golden.png' }); await page.getByTestId('golden-run').click(); await expect(page.locator('.rd-title')).toHaveText(run.source_skill);
 await page.getByRole('tab', { name: 'Matrix', exact: true }).click(); await expect(page.getByTestId('matrix-grid')).toContainText('92'); await page.screenshot({ path: '/tmp/otto-ux-r4-settings-matrix.png' });
 await page.getByTestId('matrix-cell').getByRole('button').click(); await expect(page.locator('.rd-title')).toHaveText(run.source_skill);
});
test('mocked Jira account setup retains fields on failure and tests its saved connection', async ({ page }) => {
 await page.setViewportSize({ width: 375, height: 812 }); let fail = true; let body: Record<string, unknown> = {};
 await page.route('**/api/v1/issue/accounts', r => { if(r.request().method() === 'GET') return r.fulfill({ json: [] }); body = r.request().postDataJSON(); return fail ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic account store unavailable' } }) : r.fulfill({ json: { ...body, id: 'synthetic-jira', token: undefined, token_expires_at: null } }); });
 let connectFail = true; await page.route('**/api/v1/issue/projects?account_id=*', r => connectFail ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic connection unavailable' } }) : r.fulfill({ json: [{ id: 'one' }] }));
 await page.goto('/#/settings/jira'); await page.getByRole('button', { name: 'Add account', exact: true }).first().click();
 await page.getByLabel('Label', { exact: true }).fill('Synthetic Jira'); await page.getByLabel('Base URL').fill('https://synthetic.example'); await page.getByLabel('Email', { exact: true }).fill('person@synthetic.example'); await page.getByLabel('API token', { exact: true }).fill('synthetic-token');
 await page.getByRole('dialog').getByRole('button', { name: 'Add account', exact: true }).click(); await expect(page.getByText('Synthetic account store unavailable', { exact: true })).toBeVisible(); await expect(page.getByLabel('Label', { exact: true })).toHaveValue('Synthetic Jira');
 fail = false; await page.getByRole('dialog').getByRole('button', { name: 'Add account', exact: true }).click(); await expect(page.getByRole('dialog')).toHaveCount(0); expect(body.token).toBe('synthetic-token');
 await page.getByRole('button', { name: 'Test', exact: true }).click(); await expect(page.locator('.test-result')).toContainText('Synthetic connection unavailable'); connectFail = false;
 await page.getByRole('button', { name: 'Test', exact: true }).click(); await expect(page.locator('.test-result')).toContainText('Connected · 1 project visible'); await expectNoHorizontalOverflow(page); await page.screenshot({ path: '/tmp/otto-ux-r4-settings-account-phone.png' }); expect((await page.locator('.acct .grow').boundingBox())!.width).toBeGreaterThan(200);
});
test('MCP tester validates JSON, policy creation validates keys, and approvals retain failed notes', async ({ page }) => {
 await page.setViewportSize({ width: 375, height: 812 });
 const { ws } = await fixtures(page); const server = { id: 'synthetic-server', workspace_id: ws, name: 'Synthetic reports', transport: 'stdio', command: 'fixture-command', args: [], env: {}, url: null, description: 'Fixture only', headers: {}, secret_env_keys: [], secret_header_keys: [], has_secret: false, injection_risk: 'low', managed: false, default_tool_access: 'deny', enabled: true, health_status: 'healthy', health_checked_at: null, health_latency_ms: 7, health_error: null, tools_count: 1, tools_discovered_at: stamp, created_by: 'root', created_at: stamp, updated_at: stamp };
 const tool = { id: 'tool-one', server_id: server.id, name: 'reports.preview', title: 'Preview synthetic report', description: 'Previews a local fixture report.', risk_label: 'read', injection_risk: 'low', enabled: true, require_approval: true, risk_overridden: false, input_schema: { type: 'object' } };
 await page.route('**/api/v1/workspaces/*/mcp/servers', r => r.fulfill({ json: [server] }));
 await page.route('**/api/v1/access/mcp_server/synthetic-server/capabilities*', r => r.fulfill({ json: { kind: 'mcp_server', resource_id: server.id, user_id: 'root', child: null, mode: 'legacy', operations: {} } }));
 await page.route('**/api/v1/mcp/servers/synthetic-server/tools', r => r.fulfill({ json: [tool] }));
 let invokes = 0; await page.route('**/api/v1/mcp/servers/*/tools/*/invoke', r => { invokes++; return r.fulfill({ json: { decision: 'pending_approval', reason: 'Review required by policy', dry_run: true, approval_id: 'approval-one' } }); });
 const approval = { id: 'approval-one', kind: 'tool_call', title: 'Preview synthetic report', status: 'pending', server_id: server.id, server_name: server.name, tool: tool.name, requested_by: 'synthetic-agent', requested_by_kind: 'agent', args_redacted_json: '{"report":"fixture"}', created_at: stamp, risk_label: 'read' };
 let decided = false; let fail = true; let note: unknown;
 await page.route('**/api/v1/mcp/approvals?*', r => r.fulfill({ json: decided ? [] : [approval] }));
 await page.route('**/api/v1/mcp/approvals/approval-one/decide', r => { note = r.request().postDataJSON(); if(fail) return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic decision failed' } }); decided = true; return r.fulfill({ json: { ...approval, status: 'approved' } }); });
 await page.route('**/api/v1/mcp/audit?*', r => r.fulfill({ json: [] }));
 await page.route('**/api/v1/workspaces/*/mcp/allowlist', r => r.fulfill({ json: [] }));
 let policyBody: unknown; await page.route('**/api/v1/mcp/policies*', r => { if(r.request().method() === 'POST') { policyBody = r.request().postDataJSON(); return r.fulfill({ json: { id: 'policy-one', ...r.request().postDataJSON() } }); } return r.fulfill({ json: [] }); });
 let accessPolicy = { kind: 'mcp_server', resource_id: server.id, mode: 'legacy', revision: 1, rules: [] as unknown[] };
 await page.route('**/api/v1/access/mcp_server/synthetic-server', r => { if(r.request().method() === 'PUT') accessPolicy = { ...r.request().postDataJSON().policy, revision: 2 }; return r.fulfill({ json: accessPolicy }); });
 await page.route('**/api/v1/access/mcp_server/synthetic-server/subjects', r => r.fulfill({ json: { users: [{ id: 'fixture-user', username: 'reader', display_name: 'Fixture Reader' }], groups: [{ id: 'fixture-group', name: 'Report readers' }], roles: [{ id: 'fixture-role', name: 'Read reports', kind: 'mcp_server', operations: ['view', 'invoke'], grantable_operations: [] }] } }));
 await page.route('**/api/v1/access/mcp_server/synthetic-server/preview', r => r.fulfill({ json: { token: 'fixture-preview', revision: 1, issues: [], changes: [{ user_id: 'fixture-user', display_name: 'Fixture Reader', before: { invoke: { allowed: false } }, after: { invoke: { allowed: true } }, children: [] }] } }));
 await page.goto('/#/mcp/servers');
 await page.getByRole('button', { name: 'More actions for Synthetic reports' }).click(); await page.getByRole('menuitem', { name: 'Manage access…', exact: true }).click();
 const access = page.getByRole('region', { name: 'Resource access' }); await access.getByRole('button', { name: 'Add rule', exact: true }).click(); await access.getByLabel('Copy role preset', { exact: true }).selectOption('fixture-role');
 await expect(access.getByRole('button', { name: 'Save access', exact: true })).toBeDisabled(); await access.getByRole('button', { name: 'Review changes', exact: true }).click(); await expect(access.getByRole('region', { name: 'Access comparison' })).toContainText('Denied → Allowed');
 await access.getByRole('button', { name: 'Save access', exact: true }).click(); await expect(page.getByText('Access saved', { exact: true })).toBeVisible(); expect(accessPolicy.rules).toHaveLength(1); await expect(access).toHaveCount(0);
 await page.getByTitle('View tools', { exact: true }).click(); await page.getByRole('button', { name: 'Test a tool', exact: true }).click();
 await page.getByLabel('Arguments (JSON)').fill('{'); await page.getByRole('button', { name: 'Run', exact: true }).click(); await expect(page.getByText(/Arguments must be valid JSON/)).toBeVisible(); expect(invokes).toBe(0);
 await page.getByLabel('Arguments (JSON)').fill('{"report":"fixture"}'); await page.getByRole('button', { name: 'Run', exact: true }).click(); await expect(page.locator('.pending')).toContainText('approval-one');
 await page.screenshot({ path: '/tmp/otto-ux-r4-settings-mcp-tester.png' }); await expectNoHorizontalOverflow(page); await page.screenshot({ path: '/tmp/otto-ux-r4-settings-mcp-tester-phone.png' });
 await page.getByTestId('mcp-rules-btn').click(); await page.getByRole('button', { name: 'Policies', exact: true }).click(); await page.getByRole('button', { name: 'New policy', exact: true }).click();
 await page.getByRole('dialog', { name: 'New policy' }).getByLabel('Name', { exact: true }).fill('Review reports'); await page.getByLabel('Match (JSON)', { exact: false }).fill('{"unknown":true}'); await page.getByRole('button', { name: 'Create', exact: true }).click(); await expect(page.getByRole('alert')).toContainText('Unknown match key');
 await page.getByLabel('Match (JSON)', { exact: false }).fill('{"tool_glob":"reports.*"}'); await page.getByRole('button', { name: 'Create', exact: true }).click(); await expect(page.getByRole('dialog', { name: 'New policy' })).toHaveCount(0); expect(policyBody).toMatchObject({ name: 'Review reports', effect: 'require_approval', match: { tool_glob: 'reports.*' } });
 await page.getByRole('button', { name: 'Close rules', exact: true }).click(); await page.getByRole('tab', { name: /^Activity/ }).click(); await page.getByPlaceholder('Note (optional)').fill('Reviewed synthetic fixture'); await page.getByRole('button', { name: 'Approve', exact: true }).click(); await expect(page.getByText('Synthetic decision failed', { exact: true })).toBeVisible(); await expect(page.getByPlaceholder('Note (optional)')).toHaveValue('Reviewed synthetic fixture');
 fail = false; await page.getByRole('button', { name: 'Approve', exact: true }).click(); await expect(page.getByRole('button', { name: 'Approve', exact: true })).toHaveCount(0); expect(note).toEqual({ approved: true, note: 'Reviewed synthetic fixture' });
});
test('saving an access group locks selection until the saved group returns', async ({ page }) => {
 const groups = [{ id: 'alpha-group', name: 'Alpha readers', description: 'Read-only team' }, { id: 'beta-group', name: 'Beta operators', description: 'Operations team' }];
 await page.route('**/api/v1/access/groups', r => r.fulfill({ json: groups })); await page.route('**/api/v1/access/roles', r => r.fulfill({ json: [] })); await page.route('**/api/v1/access/groups/*/members', r => r.fulfill({ json: [] }));
 let release!: () => void; const held = new Promise<void>(resolve => release = resolve); let started = false;
 await page.route('**/api/v1/access/groups/alpha-group', async r => { started = true; await held; groups[0] = { ...groups[0], ...r.request().postDataJSON() }; await r.fulfill({ json: groups[0] }); });
 await page.goto('/#/settings/access-groups'); await page.getByLabel('Group name', { exact: true }).fill('Alpha revised'); await page.getByRole('button', { name: 'Save group', exact: true }).click(); await expect.poll(() => started).toBe(true);
 await expect(page.getByRole('button', { name: 'Beta operators', exact: true })).toBeDisabled(); await expect(page.getByLabel('Group name', { exact: true })).toBeDisabled();
 const response = page.waitForResponse(r => r.url().endsWith('/access/groups/alpha-group')); release(); await (await response).finished(); await expect(page.getByText('Group saved', { exact: true })).toBeVisible(); await expect(page.getByLabel('Group name', { exact: true })).toHaveValue('Alpha revised'); await page.getByRole('button', { name: 'Beta operators', exact: true }).click(); await expect(page.getByLabel('Group name', { exact: true })).toHaveValue('Beta operators');
});

async function contrastOf(page: Page, sel: string): Promise<number> {
  return page.evaluate((selector) => {
    type C = [number, number, number, number];
    const parse = (s: string): C | null => {
      let m = /rgba?\(([^)]+)\)/.exec(s);
      if (m) {
        const p = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
        return [p[0] / 255, p[1] / 255, p[2] / 255, p[3] ?? 1];
      }
      m = /color\(srgb ([^)]+)\)/.exec(s);
      if (m) {
        const p = m[1].split(/[\s/]+/).filter(Boolean).map(Number);
        return [p[0], p[1], p[2], p[3] ?? 1];
      }
      return null;
    };
    const el = document.querySelector(selector) as HTMLElement;
    const chain: HTMLElement[] = [];
    for (let n: HTMLElement | null = el; n; n = n.parentElement) chain.unshift(n);
    let bg = [1, 1, 1];
    for (const n of chain) {
      const c = parse(getComputedStyle(n).backgroundColor);
      if (c && c[3] > 0) bg = bg.map((v, i) => c[i] * c[3] + v * (1 - c[3]));
    }
    const fg = parse(getComputedStyle(el).color)!;
    const f = [0, 1, 2].map((i) => fg[i] * fg[3] + bg[i] * (1 - fg[3]));
    const lum = (c: number[]) => {
      const l = c.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
      return 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2];
    };
    const a = lum(f);
    const b = lum(bg);
    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  }, sel);
}

test('loaded Review report stays readable across themes and tablet history can collapse', async ({ page }) => {
 await fixtures(page); await page.goto('/#/skills-eval/review');
 for (const v of [
  { key: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900, direction: 'ltr' },
  { key: 'native-dark-phone', theme: 'native', scheme: 'dark', width: 375, height: 812, direction: 'ltr' },
  { key: 'warm-light-tablet-rtl', theme: 'warm', scheme: 'light', width: 834, height: 1112, direction: 'rtl' },
  { key: 'warm-dark', theme: 'warm', scheme: 'dark', width: 1440, height: 900, direction: 'ltr' },
  { key: 'pro-dark-phone', theme: 'pro-dark', scheme: 'dark', width: 375, height: 812, direction: 'ltr' },
 ]) {
  await page.setViewportSize(v); await page.evaluate(v => { localStorage.setItem('otto_theme', v.theme); localStorage.setItem('otto_scheme', v.scheme); localStorage.setItem('otto_direction', v.direction); }, v); await page.reload(); await expect(page.getByTestId('static-report')).toBeVisible();
  await page.screenshot({ path: `/tmp/otto-ux-r4-settings-review-${v.key}-before.png` });
  if(v.width === 834) { await page.getByRole('button', { name: 'Hide reviews list', exact: true }).click(); expect((await page.locator('.lr-main').boundingBox())!.width).toBeGreaterThan(480); await page.getByRole('button', { name: 'Show reviews list', exact: true }).press('Enter'); await expect(page.locator('.lr-item.active')).toContainText('alpha synthetic review'); await page.getByRole('button', { name: 'Hide reviews list', exact: true }).press('Enter'); }
  for(const row of await page.locator('.lr-static, .lr-score, .lr-verdict, .lr-fix').all()) expect(await row.evaluate(e => e.scrollWidth <= e.clientWidth + 1)).toBeTruthy();
  for(const sel of ['.lr-detail h3', '.lr-notes', '.lr-verdict-badge', '.severity-chip']) { expect(await contrastOf(page, sel)).toBeGreaterThanOrEqual(4.5); expect(await page.locator(sel).first().evaluate(e => parseFloat(getComputedStyle(e).fontSize))).toBeGreaterThanOrEqual(11); }
  await expectNoHorizontalOverflow(page); await page.screenshot({ path: `/tmp/otto-ux-r4-settings-review-${v.key}.png` });
 }
});
test('applying review fixes retains failure context and starts only a mocked fixer', async ({ page }) => {
 const { reviews } = await fixtures(page); let fail = true; let payload: unknown;
 await page.route('**/api/v1/skill-reviews/alpha/apply', r => { payload = r.request().postDataJSON(); return fail ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic fixer unavailable' } }) : r.fulfill({ json: { ...reviews[0], fix_agent: { name: 'Fixture fixer', provider: 'codex', model: 'fixture', status: 'done', note: 'Updated the synthetic recovery example.', findings: [] } } }); });
 await page.goto('/#/skills-eval/review'); await page.getByLabel('Extra instructions for the fixer', { exact: true }).fill('Retain existing examples'); await page.getByRole('button', { name: 'Apply fixes with agent', exact: true }).click(); await expect(page.getByText('Synthetic fixer unavailable', { exact: true })).toBeVisible(); await expect(page.getByLabel('Extra instructions for the fixer', { exact: true })).toHaveValue('Retain existing examples');
 await page.locator('.toast.error').getByRole('button').click(); fail = false; await page.getByRole('button', { name: 'Apply fixes with agent', exact: true }).click(); await expect(page.getByText('Updated the synthetic recovery example.', { exact: true })).toBeVisible(); expect(payload).toMatchObject({ instructions: 'Retain existing examples' });
});
test('tablet access administration gives group fields usable width beside both navigation rails', async ({ page }) => {
 await page.setViewportSize({ width: 834, height: 1112 });
 await page.route('**/api/v1/workspaces', r => r.fulfill({ json: [] }));
 const groups = Array.from({ length: 8 }, (_, i) => ({ id: `fixture-group-${i}`, name: `Platform report readers ${i + 1}`, description: 'Read-only access for the synthetic reporting team.' }));
 const users = Array.from({ length: 6 }, (_, i) => ({ id: `fixture-user-${i}`, username: `report-reader-${i + 1}`, display_name: `Platform Reader ${i + 1}`, is_root: false, disabled: false, created_at: stamp }));
 await page.route('**/api/v1/access/groups', r => r.fulfill({ json: groups })); await page.route('**/api/v1/access/roles', r => r.fulfill({ json: [] })); await page.route('**/api/v1/access/groups/*/members', r => r.fulfill({ json: ['fixture-user-0'] }));
 await page.route('**/api/v1/users', r => r.fulfill({ json: users })); await page.route('**/api/v1/users/*/grants', r => r.fulfill({ json: { grants: [] } })); await page.route('**/api/v1/workspaces/*/members', r => r.fulfill({ json: [] }));
 await page.goto('/#/settings/users'); await page.getByRole('button', { name: 'By user', exact: true }).click(); await expect(page.getByRole('group', { name: 'Set the role in every workspace' })).toBeVisible(); await page.screenshot({ path: '/tmp/otto-ux-r4-settings-users-tablet.png' });
 await page.goto('/#/settings/access-groups'); await expect(page.getByLabel('Group name', { exact: true })).toHaveValue(groups[0].name); await page.screenshot({ path: '/tmp/otto-ux-r4-settings-groups-tablet-before.png' });
 expect((await page.locator('fieldset.detail').boundingBox())!.width).toBeGreaterThan(280); await expectNoHorizontalOverflow(page); await page.screenshot({ path: '/tmp/otto-ux-r4-settings-groups-tablet.png' });
});
