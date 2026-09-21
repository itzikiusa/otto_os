import { test, expect } from '@playwright/test';

test('human evidence, questions and role sessions survive reload', async ({ page }) => {
  const criterion = { id: 'h', text: 'Inspect the report', verify: 'Open the report', verify_kind: 'human', verify_cmd: null };
  const loop = { id:'goal', name:'Goal verification', workspace_id:'ws', status:'blocked', phase:'done', current_iteration:1, progress_pct:0,
    elapsed_secs:10, run_started_at:null, worktree_path:'/isolated/retained-work',
    definition:{ title:'Goal', summary:'Review the generated report', acceptance_criteria:[criterion] },
    config:{ executors:[{ name:'Executor', provider:'codex', model:'' }], mode:'build', allow_commits:false },
    limits:{ max_iterations:1, max_runtime_secs:300 }, ledger:{
      verifications:[] as Array<{criterion_id:string;criterion_revision:string;verified_by:string;evidence:string;verified_at:string}>,
      questions:[{id:'q',question:'Which report format?',answer:null as string | null,answered_by:null as string | null}],
      next_action:'Verify the report', review_summary:'', review_passed:false,
    } };
  const iterations = [{id:'it',idx:1,status:'done',plan:'Write report',agents:[
    {name:'Executor',provider:'codex',status:'done',note:'Written',session_id:'executor'},
    {name:'Planner',provider:'agy',status:'done',note:'Planned',session_id:'planner'},
  ]}];
  await page.addInitScript(() => {
    localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token','fixture');
  });
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request(); const path = new URL(request.url()).pathname;
    if (path.endsWith('/criteria/h/verify')) {
      loop.ledger.verifications = [{criterion_id:'h',criterion_revision:JSON.stringify(criterion),verified_by:'human-user',evidence:request.postDataJSON().evidence,verified_at:new Date().toISOString()}];
      return route.fulfill({json:loop});
    }
    if (path.endsWith('/questions/q/answer')) {
      loop.ledger.questions[0].answer = request.postDataJSON().answer;
      loop.ledger.questions[0].answered_by = 'human-user';
      return route.fulfill({json:loop});
    }
    if (path === '/api/v1/goal-loops/goal') return route.fulfill({json:{loop,iterations}});
    return route.fulfill({json:[]});
  });
  await page.goto('/e2e/fixtures/goal-loop-verification.html');
  await expect(page.getByRole('button',{name:'Resume',exact:true})).toBeDisabled();
  await expect(page.getByText('Planner',{exact:true})).toBeVisible();
  await expect(page.getByText('agy',{exact:true})).toBeVisible();
  await page.getByLabel('Evidence for h').fill('Read the complete report');
  await page.getByRole('button',{name:'Record verification'}).click();
  await expect(page.getByText('Verified by human-user: Read the complete report')).toBeVisible();
  await page.getByLabel('Answer question').fill('Use Markdown');
  await page.getByRole('button',{name:'Record answer'}).click();
  await expect(page.getByRole('button',{name:'Resume',exact:true})).toBeEnabled();
  await page.reload();
  await expect(page.getByText('Verified by human-user: Read the complete report')).toBeVisible();
  await expect(page.getByText('Use Markdown',{exact:false})).toBeVisible();
  await expect(page.getByText('/isolated/retained-work')).toBeVisible();
});
