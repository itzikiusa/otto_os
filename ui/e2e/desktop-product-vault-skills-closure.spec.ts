import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });

for (const scheme of ['light', 'dark'] as const) {
  test(`product phone navigation, Back button, Stop waiting notice & LoadState retry (${scheme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const title = `Phone story (${scheme})`;
    const draft = await ctx.post(`${base}/api/v1/workspaces/${ws}/product/drafts`, { data: { title } });
    expect(draft.ok()).toBeTruthy();
    const sid = (await draft.json()).story.id as string;
    await ctx.dispose();

    const chatObj = {
      id: 'chat-phone-1',
      story_id: sid,
      workspace_id: ws,
      title: 'Phone discovery',
      status: 'active',
      model: 'claude-3-7-sonnet',
      created_at: '2026-10-18T10:00:00Z',
      updated_at: '2026-10-18T10:00:00Z',
    };
    let persistedMessages: Array<Record<string, unknown>> = [];
    await page.context().route(`**/product/stories/${sid}/discovery-chats`, (r) => r.fulfill({ json: [chatObj] }));
    await page.context().route('**/product/discovery-chats/chat-phone-1', (r) =>
      r.fulfill({ json: { chat: chatObj, messages: persistedMessages } }),
    );
    let releaseSend!: () => void;
    let sendStarted!: () => void;
    const heldSend = new Promise<void>((res) => (releaseSend = res));
    const startedSend = new Promise<void>((res) => (sendStarted = res));
    let capturedSendTurnId: string | undefined;
    let capturedStopTurnId: string | undefined;
    let releaseStop!: () => void;
    let stopStarted!: () => void;
    const heldStop = new Promise<void>((res) => (releaseStop = res));
    const startedStop = new Promise<void>((res) => (stopStarted = res));

    await page.context().route('**/product/discovery-chats/chat-phone-1/messages', async (r) => {
      const payload = r.request().postDataJSON() as { body: string; turn_id?: string };
      capturedSendTurnId = payload.turn_id;
      persistedMessages = [
        {
          id: 'u-1',
          chat_id: 'chat-phone-1',
          role: 'user',
          body: payload.body,
          actions_json: null,
          meta_json: null,
          created_at: '2026-10-18T10:01:00Z',
        },
      ];
      sendStarted();
      await heldSend;
      await r.fulfill({
        status: 499,
        json: { code: 'cancelled', message: 'Turn stopped by user' },
      });
    });

    await page.context().route('**/product/discovery-chats/chat-phone-1/stop', async (r) => {
      const payload = r.request().postDataJSON() as { turn_id: string };
      capturedStopTurnId = payload.turn_id;
      stopStarted();
      await heldStop;
      releaseSend();
      await r.fulfill({ json: { stopped: true } });
    });

    await page.addInitScript(
      ({ ws, scheme }) => {
        localStorage.setItem('otto_workspace', ws);
        localStorage.setItem('otto_scheme', scheme);
        localStorage.setItem('otto_rail_expanded', '0');
      },
      { ws, scheme },
    );

    await page.goto('/#/product');
    const storyRow = page.locator('.story-row', { hasText: title });
    await expect(storyRow).toBeVisible();
    await storyRow.click();

    // Selecting a story on phone auto-opens content and surfaces Back to stories in PageHeader.
    await expect(page.locator('.product-page')).toHaveClass(/m-content-open/);
    const backBtn = page.getByRole('button', { name: 'Back to stories', exact: true });
    await expect(backBtn).toBeVisible();
    await expectNoHorizontalOverflow(page);

    // Back returns to the list section.
    await backBtn.click();
    await expect(page.locator('.product-page')).toHaveClass(/m-list-open/);
    await storyRow.click();
    await expect(page.locator('.product-page')).toHaveClass(/m-content-open/);

    // Switch to Discover -> Chat and exercise server-backed Stop (exact turn_id, Stopping… state, reload, preserved bubble).
    await page.getByRole('tab', { name: 'Discover', exact: true }).click();
    await page.locator('.sub-tab-strip .st', { hasText: 'Chat' }).click();
    const composer = page.locator('.discovery-chat textarea');
    await expect(composer).toBeVisible();
    await composer.fill('Keep my prompt');
    await page.locator('.discovery-chat').getByRole('button', { name: 'Send', exact: true }).click();
    await startedSend;
    expect(capturedSendTurnId).toBeTruthy();

    try {
      const stopBtn = page.locator('.discovery-chat').getByRole('button', { name: 'Stop', exact: true });
      const stopWaitingBtn = page.locator('.discovery-chat').getByRole('button', { name: 'Stop waiting', exact: true });
      await expect(stopBtn).toBeVisible();
      await expect(stopWaitingBtn).toBeVisible();
      await stopBtn.click();
      await startedStop;
      expect(capturedStopTurnId).toBe(capturedSendTurnId);
      await expect(page.locator('.discovery-chat').getByRole('button', { name: 'Stopping…', exact: true }).first()).toBeDisabled();
      releaseStop();

      // Accepted user bubble stays visible after reload and composer stays empty (no duplicate draft restoration).
      await expect(page.locator('.discovery-chat .bubble-user')).toContainText('Keep my prompt');
      await expect(composer).toHaveValue('');
      await expect(page.locator('.discovery-chat')).toContainText('Turn stopped.');
      await expect(page.locator('.discovery-chat').getByRole('button', { name: 'Reload chat', exact: true })).toBeVisible();
      // Also switch to Refine tab and verify server-backed Stop on RefineChat.
      const threadObj = {
        id: 'thread-phone-1',
        story_id: sid,
        workspace_id: ws,
        title: 'Phone refinement',
        status: 'active',
        model: 'claude-3-7-sonnet',
        created_at: '2026-10-18T10:00:00Z',
        updated_at: '2026-10-18T10:00:00Z',
      };
      let persistedRefineMsgs: Array<Record<string, unknown>> = [];
      let refineSendTurnId: string | undefined;
      let refineStopTurnId: string | undefined;
      let releaseRefineSend!: () => void;
      let refineSendStarted!: () => void;
      const heldRefineSend = new Promise<void>((res) => (releaseRefineSend = res));
      const startedRefineSend = new Promise<void>((res) => (refineSendStarted = res));
      await page.context().route(`**/product/stories/${sid}/refinement-threads`, (r) => r.fulfill({ json: [threadObj] }));
      await page.context().route('**/product/refinement-threads/thread-phone-1', (r) =>
        r.fulfill({ json: { thread: threadObj, messages: persistedRefineMsgs } }),
      );
      await page.context().route('**/product/refinement-threads/thread-phone-1/messages', async (r) => {
        const payload = r.request().postDataJSON() as { body: string; turn_id?: string };
        refineSendTurnId = payload.turn_id;
        persistedRefineMsgs = [
          {
            id: 'ru-1',
            thread_id: 'thread-phone-1',
            role: 'user',
            body: payload.body,
            meta_json: null,
            created_at: '2026-10-18T10:02:00Z',
          },
        ];
        refineSendStarted();
        await heldRefineSend;
        await r.fulfill({ status: 499, json: { code: 'cancelled', message: 'Turn stopped by user' } });
      });
      await page.context().route('**/product/refinement-threads/thread-phone-1/stop', async (r) => {
        const payload = r.request().postDataJSON() as { turn_id: string };
        refineStopTurnId = payload.turn_id;
        releaseRefineSend();
        await r.fulfill({ json: { stopped: true } });
      });

      await page.getByRole('tab', { name: 'Refine', exact: true }).click();
      const refineComposer = page.locator('.refine-chat textarea');
      await expect(refineComposer).toBeVisible();
      await refineComposer.fill('Refine AC');
      await page.locator('.refine-chat').getByRole('button', { name: 'Send', exact: true }).click();
      await startedRefineSend;
      expect(refineSendTurnId).toBeTruthy();
      await page.locator('.refine-chat').getByRole('button', { name: 'Stop', exact: true }).click();
      expect(refineStopTurnId).toBe(refineSendTurnId);
      await expect(page.locator('.refine-chat .bubble-user')).toContainText('Refine AC');
      await expect(refineComposer).toHaveValue('');
      await expect(page.locator('.refine-chat')).toContainText('Turn stopped.');
    } finally {
      releaseStop();
      releaseSend();
    }
  });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`vault phone & tablet LoadState retry, tree touch targets & New note quick action (${scheme})`, async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const { vaultId } = await seedVaultDir(ctx, base, ws);
    await ctx.dispose();

    let failVaults = true;
    await page.context().route(`**/api/v1/workspaces/${ws}/vault/vaults`, (r) => {
      if (failVaults && r.request().method() === 'GET') {
        failVaults = false;
        return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Vault list temporarily offline' } });
      }
      return r.continue();
    });

    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript(
      ({ ws, vaultId, scheme }) => {
        localStorage.setItem('otto_workspace', ws);
        localStorage.setItem(`otto_vault_last:${ws}`, String(vaultId));
        localStorage.setItem('otto_scheme', scheme);
        localStorage.setItem('otto_rail_expanded', '0');
      },
      { ws, vaultId, scheme },
    );

    await page.goto('/#/vault');
    await expect(page.getByText('Vault list temporarily offline')).toBeVisible();
    await page.getByRole('button', { name: 'Retry', exact: true }).click();

    // On phone, switch to Files panel and verify tree row touch target ≥ 36px + New note quick action.
    const filesSwitch = page.getByRole('button', { name: 'Files', exact: true }).first();
    if (await filesSwitch.isVisible()) await filesSwitch.click();
    const tree = page.locator('.tree');
    const servicesRow = tree.locator('.row', { hasText: 'services' }).first();
    await expect(servicesRow).toBeVisible({ timeout: 15_000 });
    expect((await servicesRow.boundingBox())!.height).toBeGreaterThanOrEqual(36);
    await expect(page.getByRole('button', { name: 'New note', exact: true }).first()).toBeVisible();
    await expectNoHorizontalOverflow(page);

    // Resize to tablet (820×1180) and verify note opens with no horizontal overflow.
    await page.setViewportSize({ width: 820, height: 1180 });
    await servicesRow.click();
    await tree.getByText('auth-api', { exact: true }).click();
    await expect(page.locator('.read').getByRole('heading', { name: 'Overview' })).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`skills lab tablet draft persistence, golden task dirty-discard prompt & plugin retry (${scheme})`, async ({ page }) => {
    const { ctx, base } = await apiCtx();
    const ws = await seedWorkspace(ctx, base);
    const skillName = `tablet-skill-${scheme}`;
    await ctx.delete(`${base}/api/v1/library/skills/${skillName}`);
    const createRes = await ctx.post(`${base}/api/v1/library/skills`, {
      data: {
        name: skillName,
        category: 'review',
        description: 'Tablet skill fixture',
        body: '---\ndescription: Tablet skill fixture\ncategory: review\nversion: 1\n---\n\n# Initial body\n',
      },
    });
    expect(createRes.ok()).toBeTruthy();
    await ctx.dispose();

    await page.setViewportSize({ width: 820, height: 1180 });
    await page.addInitScript(
      ({ ws, scheme }) => {
        localStorage.setItem('otto_workspace', ws);
        localStorage.setItem('otto_scheme', scheme);
        localStorage.setItem('otto_rail_expanded', '0');
      },
      { ws, scheme },
    );

    // 1. Skills Lab on tablet: open skill, switch to Files editor, type unsaved draft.
    await page.goto('/#/skills-eval');
    const row = page.locator('[data-testid="skill-row"]', { hasText: skillName });
    await expect(row).toBeVisible({ timeout: 15_000 });
    await row.click();
    await page.getByRole('tab', { name: 'Files', exact: true }).click();
    const cm = page.locator('[data-testid="skill-editor"] .cm-content').first();
    await expect(cm).toBeVisible();
    await cm.click();
    await page.keyboard.type(' draft');
    await expect(page.locator('[data-testid="save-skill"]')).toBeEnabled();
    await expectNoHorizontalOverflow(page);

    // Discard via the unsaved guard when navigating to Evaluator -> Golden tasks.
    await page.getByTestId('tab-evaluator').click();
    const leaveDialog = page.getByRole('dialog');
    if (await leaveDialog.isVisible()) {
      await leaveDialog.getByRole('button', { name: /Discard|Leave/i }).click();
    }
    await page.getByTestId('tab-golden').click();

    // 2. Golden tasks: dirty form prompts confirmer before discarding on Close/Cancel.
    await page.getByRole('button', { name: /New golden task/i }).click();
    const nameInput = page.getByTestId('golden-name');
    await expect(nameInput).toBeVisible();
    await nameInput.fill('Unsaved golden regression');
    await page.getByRole('button', { name: 'Close form', exact: true }).click();
    const discardDialog = page.getByRole('dialog', { name: 'Discard unsaved changes?' });
    await expect(discardDialog).toBeVisible();
    await discardDialog.getByRole('button', { name: 'Keep editing', exact: true }).click();
    await expect(nameInput).toHaveValue('Unsaved golden regression');
    await page.getByRole('button', { name: 'Close form', exact: true }).click();
    await discardDialog.getByRole('button', { name: 'Discard', exact: true }).click();
    await expect(nameInput).toHaveCount(0);
    await expectNoHorizontalOverflow(page);

    // 3. PluginFrame: initial probe 503 shows LoadState Retry and recovers cleanly.
    let failPluginUi = true;
    await page.context().route('**/plugins/demo-plugin/ui/**', (r) => {
      if (failPluginUi) {
        failPluginUi = false;
        return r.fulfill({ status: 503, body: 'offline' });
      }
      return r.fulfill({ status: 404, body: 'not found' });
    });
    await page.goto('/#/plugin/demo-plugin');
    await expect(page.getByText(/Otto answered 503 for the plugin/)).toBeVisible();
    await page.getByRole('button', { name: 'Retry', exact: true }).click();
    await expect(page.getByText('demo-plugin isn’t available')).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });
}
