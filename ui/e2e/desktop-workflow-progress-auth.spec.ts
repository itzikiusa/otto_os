import {test, expect, request, type APIRequestContext} from '@playwright/test';
import {randomUUID} from 'node:crypto';
import {mkdirSync, readFileSync, realpathSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {basename, dirname, join} from 'node:path';
import {apiCtx} from './seed';

// Real HTTP/router coverage; no page.route mocks, SQL injection, provider steps
// or outbound requests. Global setup/teardown own the throwaway daemon and data.
test('workflow summaries and encoded details recheck workspace access, including unchanged polls', async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser project only');
  test.setTimeout(90_000);
  const {ctx: root, base} = await apiCtx();
  const clients: APIRequestContext[] = [];
  try {
    const slot = process.env.OTTO_E2E_SLOT ?? '0';
    const meta = JSON.parse(readFileSync(join(process.cwd(), 'e2e', `.auth-${slot}`, 'daemon.json'), 'utf8')) as {dataDir:string;port:string};
    const dataDir = realpathSync(meta.dataDir);
    expect(dirname(dataDir)).toBe(realpathSync(tmpdir()));
    expect(basename(dataDir)).toMatch(/^otto-e2e-/);
    expect(base).toBe(`http://127.0.0.1:${meta.port}`);
    const fixtureId = randomUUID();
    const workspaceRoot = join(dataDir, `workflow-auth-${fixtureId}`);
    mkdirSync(workspaceRoot);
    const workspaceResponse = await root.post(`${base}/api/v1/workspaces`, {data:{name:'Workflow progress auth fixture',root_path:workspaceRoot}});
    expect(workspaceResponse.ok(), await workspaceResponse.text()).toBeTruthy();
    const workspace = (await workspaceResponse.json()).id as string;

    async function actor(label:string):Promise<{id:string;ctx:APIRequestContext}> {
      const username = `workflow-${label}-${fixtureId}`;
      const password = randomUUID();
      const created = await root.post(`${base}/api/v1/users`, {data:{username,password,display_name:label}});
      expect(created.ok(), `create ${label}: ${created.status()}`).toBeTruthy();
      const id = (await created.json()).id as string;
      // Give BOTH users the feature grant, so the denied matrix specifically
      // exercises run workspace authorization rather than the feature guard.
      const grants = await root.put(`${base}/api/v1/users/${id}/grants`, {data:{grants:[{feature:'workflows',capability:'view'}]}});
      expect(grants.ok(), await grants.text()).toBeTruthy();
      const login = await root.post(`${base}/api/v1/auth/login`, {data:{username,password}});
      expect(login.ok(), `login ${label}: ${login.status()}`).toBeTruthy();
      const token = (await login.json()).token as string;
      const ctx = await request.newContext({extraHTTPHeaders:{Authorization:`Bearer ${token}`}});
      clients.push(ctx);
      return {id,ctx};
    }
    const viewer = await actor('viewer');
    const outsider = await actor('nonmember');
    const membership = await root.put(`${base}/api/v1/workspaces/${workspace}/members`, {data:{members:[{user_id:viewer.id,role:'viewer'}]}});
    expect(membership.ok(), await membership.text()).toBeTruthy();

    const nodeId = 'loop#route';
    const checkpointId = `${nodeId}#1.0`;
    const marker = `exact-checkpoint-${fixtureId}`;
    const workflowResponse = await root.post(`${base}/api/v1/workspaces/${workspace}/workflows`, {data:{
      name:'Local transform only', graph:{nodes:[{id:nodeId,kind:'loop',name:'Literal # node',params:{
        max_iterations:1,until:'last.done == true',steps:[{kind:'transform',name:'Saved exact body',params:{json:{done:true,payload:marker}}}],
      }}],edges:[]},
    }});
    expect(workflowResponse.ok(), await workflowResponse.text()).toBeTruthy();
    const workflow = (await workflowResponse.json()).id as string;
    const started = await root.post(`${base}/api/v1/workflows/${workflow}/run`, {data:{}});
    expect(started.ok(), await started.text()).toBeTruthy();
    const runId = (await started.json()).id as string;
    const runUrl = `${base}/api/v1/workflow-runs/${runId}`;
    await expect.poll(async () => {
      const response = await root.get(runUrl);
      expect(response.ok()).toBeTruthy();
      return (await response.json()).status;
    }, {timeout:30_000}).toBe('success');
    const full = await (await root.get(runUrl)).json();
    const checkpoint = full.checkpoints.find((row:{node_id:string}) => row.node_id === checkpointId);
    const node = full.nodes.find((row:{node_id:string}) => row.node_id === nodeId);
    expect(checkpoint.output).toMatchObject({done:true,payload:marker});

    const progressResponse = await viewer.ctx.get(`${runUrl}/progress`);
    expect(progressResponse.status(), await progressResponse.text()).toBe(200);
    let progress = await progressResponse.json();
    expect(progress.changed).toBe(true);
    expect(progress.run).toMatchObject({id:runId,summary:true,checkpoint_count:2});
    expect(JSON.stringify(progress)).not.toContain(marker);
    // Final proof metadata can advance rev just after the terminal status.
    // Follow that legitimate update, then assert the actual unchanged branch.
    await expect.poll(async () => {
      const unchanged = await viewer.ctx.get(`${runUrl}/progress?after_rev=${progress.rev}`);
      expect(unchanged.status()).toBe(200);
      const result = await unchanged.json();
      if (result.changed) progress = result;
      else expect(result).toEqual({changed:false,rev:progress.rev});
      return result.changed;
    }).toBe(false);

    const pageResponse = await viewer.ctx.get(`${runUrl}/checkpoints?limit=1`);
    expect(pageResponse.status(), await pageResponse.text()).toBe(200);
    const firstPage = await pageResponse.json();
    expect(firstPage.items).toHaveLength(1);
    expect(firstPage.next_cursor).toEqual(expect.any(String));
    const secondPageResponse = await viewer.ctx.get(`${runUrl}/checkpoints?limit=1&cursor=${encodeURIComponent(firstPage.next_cursor)}`);
    expect(secondPageResponse.status()).toBe(200);
    const secondPage = await secondPageResponse.json();
    expect(secondPage.items).toHaveLength(1);
    expect(secondPage.next_cursor).toBeNull();
    const summaries = [...firstPage.items, ...secondPage.items];
    expect(new Set(summaries.map(row => row.node_id)).size).toBe(2);
    expect(JSON.stringify(summaries)).not.toContain(marker);

    const checkpointPath = `/checkpoints/${encodeURIComponent(checkpointId)}`;
    const nodePath = `/nodes/${encodeURIComponent(nodeId)}`;
    expect(checkpointPath).toContain('%23');
    expect(nodePath).toContain('%23');
    const checkpointResponse = await viewer.ctx.get(`${runUrl}${checkpointPath}`);
    expect(checkpointResponse.status(), await checkpointResponse.text()).toBe(200);
    const detail = await checkpointResponse.json();
    expect(detail.body).toEqual(checkpoint);
    expect(detail.detail_version).toBe(summaries.find(row => row.node_id === checkpointId).detail_version);
    expect(detail.rev).toBeGreaterThanOrEqual(progress.rev);
    const nodeResponse = await viewer.ctx.get(`${runUrl}${nodePath}`);
    expect(nodeResponse.status(), await nodeResponse.text()).toBe(200);
    const nodeDetail = await nodeResponse.json();
    expect(nodeDetail.body).toEqual(node);
    expect(nodeDetail.detail_version).toBe(progress.run.nodes.find((row:{node_id:string}) => row.node_id === nodeId).detail_version);
    expect(nodeDetail.rev).toBeGreaterThanOrEqual(progress.rev);
    expect((await viewer.ctx.get(`${runUrl}/checkpoints/${encodeURIComponent('missing#checkpoint')}`)).status()).toBe(404);
    expect((await viewer.ctx.get(`${runUrl}/nodes/${encodeURIComponent('missing#node')}`)).status()).toBe(404);

    const paths = ['/progress', `/progress?after_rev=${progress.rev}`, '/checkpoints?limit=1', `/checkpoints?cursor=${encodeURIComponent(firstPage.next_cursor)}`, checkpointPath, nodePath];
    async function denied(ctx:APIRequestContext,label:string):Promise<void> {
      for (const path of paths) {
        const response = await ctx.get(`${runUrl}${path}`);
        expect(response.status(), `${label} ${path}`).toBe(403);
        const body = await response.text();
        expect(body).not.toContain(marker);
        expect(body).not.toContain('"changed":false');
        expect(body).not.toContain('detail_version');
      }
    }
    await denied(outsider.ctx,'nonmember with workflow feature grant');
    const revoked = await root.put(`${base}/api/v1/workspaces/${workspace}/members`, {data:{members:[]}});
    expect(revoked.ok(), await revoked.text()).toBeTruthy();
    // Reuse the already-successful login context and revision/cursor: current
    // membership must be checked again, even when cached data is unchanged.
    await denied(viewer.ctx,'revoked Viewer with same token and cursor');
    expect((await root.get(`${runUrl}/progress?after_rev=${progress.rev}`)).status()).toBe(200);
  } finally {
    await Promise.all(clients.map(ctx => ctx.dispose()));
    await root.dispose();
  }
});
