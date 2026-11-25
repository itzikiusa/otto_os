<script lang="ts">
  import { flushSync } from 'svelte';
  import CodeEditor from '../../src/lib/components/CodeEditor.svelte';
  import type { IdeBreakpoint } from '../../src/lib/api/types';
  // A new IDE editor (the {#key} remount a return to the IDE does) whose breakpoints change in
  // the same tick as its mount.
  let path = $state('/w/a.go');
  let breakpoints = $state<IdeBreakpoint[]>([]);
  (window as unknown as { remountWithBreakpoint: () => boolean }).remountWithBreakpoint = () => {
    path = '/w/b.go';
    flushSync();
    const built = !!document.querySelector('[data-path="/w/b.go"] .cm-editor'); // the new editor exists before the change
    breakpoints = [{ path: '/w/b.go', line: 2, enabled: true }];
    flushSync();
    return built;
  };
</script>
{#key path}
  <div style="block-size: 300px" data-testid="editor-for" data-path={path}>
    <CodeEditor {path} content={'package main\n\nfunc main() {}\n'} root="/w" ideMode readOnly={false} {breakpoints} onToggleBreakpoint={() => {}} />
  </div>
{/key}
