<script lang="ts">
  // HTML / SVG document preview in a fully sandboxed frame: an EMPTY `sandbox`
  // attribute means no scripts, no forms, no same-origin access, no top-level
  // navigation — the page is rendered, never executed. A strict CSP meta is
  // prepended as a second fence (no network fetches beyond inline/data images).
  // The document (CSP fence + page) is shared with the IDE/Threads preview.
  import { buildHtmlPreviewDoc } from '../../../lib/render/previewText';

  interface Props {
    content: string;
    name: string;
    /** `svg` wraps the markup in a minimal centred page. */
    svg?: boolean;
  }
  let { content, name, svg = false }: Props = $props();

  const doc = $derived(buildHtmlPreviewDoc(content, { svg }));
</script>

<iframe
  class="frame"
  sandbox=""
  referrerpolicy="no-referrer"
  title={`Preview of ${name}`}
  srcdoc={doc}
></iframe>

<style>
  .frame {
    display: block;
    inline-size: 100%;
    block-size: 100%;
    min-block-size: 240px;
    border: 0;
    /* Documents assume a white page; keep them readable in dark mode too. */
    background: var(--surface);
    color-scheme: light;
  }
</style>
