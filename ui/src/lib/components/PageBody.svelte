<script lang="ts">
  // The scrolling body under a PageHeader. One content-width rule app-wide:
  //
  // • width="full"     — tool / dashboard / list pages use the whole pane.
  // • width="readable" — text-heavy forms & settings-like pages cap their
  //   column at --page-readable (1200px) and stay LEFT-aligned with the header
  //   title (no centred island with dead margins on both sides).
  //
  // `padded={false}` for pages that manage their own inner layout (split panes).
  // `fill` pins the content to the pane height (no page scroll) for bodies
  // whose children scroll internally (split views, boards).
  import type { Snippet } from 'svelte';

  interface Props {
    width?: 'full' | 'readable';
    padded?: boolean;
    fill?: boolean;
    class?: string;
    children: Snippet;
  }
  let { width = 'full', padded = true, fill = false, class: klass = '', children }: Props = $props();
</script>

<div class="page-body {klass}" class:padded class:fill>
  <div class="page-body-inner" class:readable={width === 'readable'}>
    {@render children()}
  </div>
</div>

<style>
  .page-body {
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
  .page-body.padded {
    padding: 18px 20px 40px;
  }
  .page-body.fill {
    overflow: hidden;
  }
  .page-body.fill.padded {
    padding-bottom: 16px;
  }
  .page-body-inner {
    flex: 1 0 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .fill > .page-body-inner {
    flex: 1 1 0;
    min-height: 0;
  }
  .page-body-inner.readable {
    width: 100%;
    max-width: var(--page-readable, 1200px);
  }
  @media (max-width: 640px) {
    .page-body.padded {
      padding: 12px 14px 32px;
    }
  }
</style>
