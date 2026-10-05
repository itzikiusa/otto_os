import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash, webcrypto } from 'node:crypto';
import { componentFunctions } from './componentFunctions.ts';

const component = new URL('../src/modules/product/PublishDialog.svelte', import.meta.url);
const body = 'Reviewed café.\r\n  Exact trailing spaces.  \n';
const hash = (text: string) => createHash('sha256').update(text, 'utf8').digest('hex');
class ApiError extends Error { status: number; constructor(status: number) { super('Preview changed'); this.status = status; } }

function fixture(mode: 'story' | 'rfc', content: string | null = body) {
  const story = { id: 'A', title: 'Reviewed title', source_kind: 'confluence', url: 'https://example.test/rfc/123' };
  const published: any[] = [];
  const state = componentFunctions(component, ['loadPreview', 'submit', 'setError'], {
    previewSequence: 0, previewStoryId: null, previewError: '', previewBody: null, reviewedContent: null,
    submitting: false, formError: '', formErrorDetail: '', mode, accountId: 'account', projectKey: 'PRJ',
    issueType: 'Story', spaceKey: 'RFC', parentId: '42', rfcTitle: 'My reviewed RFC title',
    crypto: webcrypto, TextEncoder, ApiError,
    product: {
      selectedId: 'A', detail: { story: { ...story } },
      getVersion: async () => ({ id: 'version', kind: 'draft', body_md: content }),
      publishAsStory: async (req: unknown) => { published.push(req); return { story }; },
      publishAsRfc: async (req: unknown) => { published.push(req); return { story }; },
    },
    api: { get: async (path: string) => path.endsWith('/versions')
      ? (content === null ? [] : [{ id: 'version', kind: 'draft', body_md: '' }])
      : { story: { ...story } } },
    toasts: { success() {} }, onclose() {},
  });
  return { state, story, published };
}

for (const mode of ['story', 'rfc'] as const) {
  test(`${mode} submit binds the exact reviewed body and metadata despite later detail updates`, async () => {
    const { state, story, published } = fixture(mode);
    await state.loadPreview('A');
    state.product.detail.story = { ...story, title: 'Unreviewed title', url: 'https://example.test/new-ref' };
    await state.submit();
    assert.equal(published.length, 1);
    assert.deepEqual(JSON.parse(JSON.stringify(published[0].reviewed_content ?? null)), {
      version_id: 'version', body_sha256: hash(body), title: story.title, source_kind: story.source_kind, url: story.url,
    });
    if (mode === 'rfc') {
      assert.equal(published[0].title, 'My reviewed RFC title');
      assert.equal(published[0].parent_id, '42');
    }
  });
}

test('a preview without a content version binds null identity and the exact empty-body digest', async () => {
  const { state, published } = fixture('story', null);
  await state.loadPreview('A'); await state.submit();
  assert.equal(published[0].reviewed_content?.version_id, null);
  assert.equal(published[0].reviewed_content?.body_sha256, hash(''));
});

test('publication conflict disables the stale preview until explicit review reload', async () => {
  const { state } = fixture('story');
  await state.loadPreview('A');
  state.product.publishAsStory = async () => { throw new ApiError(409); };
  await state.submit();
  assert.match(state.previewError, /review|preview/i);
  const invalidatedReview = state.reviewedContent;
  assert.equal(invalidatedReview, null);
  await state.loadPreview('A');
  assert.equal(state.previewError, '');
  assert.equal(state.reviewedContent?.body_sha256, hash(body));
});
