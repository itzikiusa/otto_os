// The film's agent-debug + DB Watch stills (appended by scripts/capture-agent-debug.mjs
// to the helpers of ui/e2e/desktop-ide-agent-debug-real.spec.ts, so it drives
// the same real path: a stand-in `shell` agent session calls otto.ui_ide_debug_*
// through `ottod mcp-tools`, the IDE runs real delve on the go-orders fixture
// against a real MariaDB + Redis). One still per beat of the chapter.
const FILM = process.env.OTTO_TOUR_FILM_DIR!;
const snap = async (page: Page, name: string) => {
  await page.mouse.move(-20, -20).catch(() => {});
  await sleep(500);
  await page.screenshot({ path: join(FILM, name + '.jpg'), type: 'jpeg', quality: 90 });
  console.log('[film] ' + name);
};
/** Like confirmed(), but films the confirm before answering it. */
async function filmed(page: Page, call: () => Promise<Out>, button: string, name: string) {
  const dlg = ideIn(page).getByRole('dialog').filter({ hasText: /wants to|Allow/ }).last();
  await expect(dlg).toHaveCount(0); // the previous confirm has closed
  let pending = call();
  for (let i = 0; i < 3; i++) {
    const first = await Promise.race([pending.then((o) => o), dlg.waitFor({ timeout: 60_000 }).then(() => null)]);
    if (!first) break;
    if (!/did not report ready in time/.test(first.text)) return first;
    pending = call();
  }
  await sleep(900);
  await snap(page, name);
  const btn = dlg.getByRole('button', { name: button, exact: true });
  const b = await btn.boundingBox();
  if (b) { await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 20 }); await sleep(300); }
  await btn.click();
  await expect(dlg).toHaveCount(0);
  const out = await pending;
  expect(out.isError, out.text).toBeFalsy();
  return out;
}

test('film: an agent debugs the orders service while the person watches', async ({ page }) => {
  test.setTimeout(600_000);
  await page.setViewportSize({ width: 1600, height: 900 });
  const bp1 = lineOf('main.go', 'id, err := insertOrder(');
  const bp2 = lineOf('main.go', 'if err := redisSetEx(');
  {
    const { ctx, base } = await apiCtx();
    await ctx.post(base + '/api/v1/sessions/' + sessionId + '/input', { data: { text: 'clear; stty -echo; cat > /dev/null', submit: true } });
    await ctx.dispose();
  }
  await page.goto('/#/agents/' + sessionId);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const mcp = await startMcp(sessionId, sessionToken);
  try {
    // The grant prompt beside the session.
    const prompt = page.getByTestId('ui-control-request').filter({ hasText: 'orders-agent' });
    let call = mcp.call('request_control', {});
    await prompt.waitFor({ timeout: 30_000 }).catch(() => {});
    if (await prompt.isVisible()) {
      await sleep(800);
      await snap(page, 'dbg-grant');
      await prompt.getByRole('button', { name: 'Allow for this session', exact: true }).click();
      await call;
    }
    const started = await filmed(page, () => mcp.call('ide_debug_start', {
      target: { kind: 'main', file: 'main.go' },
      env: { ORDERS_DSN: dsn, ORDERS_REDIS: redisAddr, ORDERS_ADDR: '127.0.0.1:' + APP_PORT },
      breakpoints: [{ file: 'main.go', line: bp1 }, { file: 'main.go', line: bp2 }],
      ready_pattern: 'orders listening on',
      db_watch: true,
      say: 'Starting the orders service to follow one POST /orders',
    }), 'Debug', 'dbg-start-confirm');
    expect(started.r.state).toBe('running');
    await expect(ideIn(page).getByTestId('agent-driving-say')).toContainText('Starting the orders service');
    await sleep(1500);
    await snap(page, 'dbg-running');
    const req = await filmed(page, () => mcp.call('ide_debug_request', { method: 'POST', url: 'http://127.0.0.1:' + APP_PORT + '/orders', headers: { 'Content-Type': 'application/json' }, body: '{"customer":"ava","amount":42}' }), 'Send', 'dbg-request-confirm');
    expect(req.r.stop.location.line).toBe(bp1);
    await mcp.call('ide_debug_control', { action: 'status', say: 'Stopped in the handler: the order is valid, ava for 42. Next: the insert.' });
    await sleep(1500);
    await snap(page, 'dbg-stop');
    let added = false, changed = false;
    for (let i = 0; i < 8 && !(added && changed); i++) {
      const out = await mcp.call('ide_debug_control', { action: 'step_over', wait_ms: 20_000 });
      const orders = (out.r.db?.objects ?? []).find((o: { object: string }) => o.object === 'orders');
      if (orders?.added === 1 && !added) {
        added = true;
        await mcp.call('ide_debug_control', { action: 'status', say: 'The insert wrote one row to orders: status new.' });
        await sleep(1500);
        await snap(page, 'dbg-step-added');
      }
      if (orders?.changed === 1) changed = true;
      if (out.r.location?.line === bp2) break;
    }
    await mcp.call('ide_debug_control', { action: 'status', say: 'Paused before the Redis write: the order row is paid.' });
    await sleep(1200);
    await snap(page, 'dbg-say');
    await showDebugDock(page);
    const strip = ideIn(page).getByTestId('ide-dbw-strip');
    await expect(strip).toBeVisible();
    await sleep(800);
    await snap(page, 'dbg-strip');
    await strip.getByTestId('ide-dbw-strip-open').click();
    const panel = ideIn(page).getByTestId('ide-dbwatch-panel');
    await expect(panel).toBeVisible();
    await sleep(1200);
    await snap(page, 'dbw-last');
    await panel.getByTestId('ide-dbw-since-start').click();
    await expect(panel.getByTestId('ide-dbw-diff-row').first()).toContainText('ava');
    await sleep(1200);
    await snap(page, 'dbw-start');
    const bar = ideIn(page).getByTestId('agent-driving-bar');
    const proceed = bar.getByTestId('agent-driving-proceed');
    const pb = await proceed.boundingBox();
    if (pb) { await page.mouse.move(pb.x + pb.width / 2, pb.y + pb.height / 2, { steps: 24 }); await sleep(500); }
    await page.screenshot({ path: join(FILM, 'dbg-proceed.jpg'), type: 'jpeg', quality: 90 });
    await proceed.click();
    let r = (await mcp.call('ide_debug_control', { action: 'continue', wait_ms: 5_000 })).r;
    if (r.state === 'paused' && r.location?.line === bp2) r = (await mcp.call('ide_debug_control', { action: 'continue', wait_ms: 5_000 })).r;
    await mcp.call('ide_debug_control', { action: 'status', say: 'The request answered 201 and the order is paid. Done.' }).catch(() => null);
    await sleep(1500);
    await snap(page, 'dbg-done');
    console.log('[film] response ' + JSON.stringify(r.request ?? {}).slice(0, 300));
    await mcp.call('ide_debug_control', { action: 'stop' });
  } finally {
    mcp.kill();
  }
});
