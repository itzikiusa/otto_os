import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parse } from 'svelte/compiler';

// accessibility-responsive-6w2-01 (git): a status line inserted together with
// its text inside an `{#if}` is usually not spoken. The git account test
// verdicts and the conflict resolver's "is resolved" note now announce
// themselves with `announceShown` (a failed test interrupts) and carry no
// role. (structure: walks the parsed component, no source regexes.)

const git = (name: string) => new URL(`../src/modules/git/${name}`, import.meta.url);

type Node = { type: string; name?: string; attributes?: any[]; fragment?: { nodes: Node[] }; [k: string]: any };

/** Every element whose static `class` contains `cls`, with whether it sits
 *  inside an `{#if}` branch of the same component. */
function elementsWithClass(file: string, cls: string): { el: Node; inIf: boolean }[] {
  const ast = parse(readFileSync(git(file), 'utf8'), { modern: true });
  const out: { el: Node; inIf: boolean }[] = [];
  const walk = (n: any, inIf: boolean): void => {
    if (!n || typeof n !== 'object') return;
    if (Array.isArray(n)) return n.forEach((c) => walk(c, inIf));
    if (n.type === 'RegularElement') {
      const c = n.attributes.find((a: any) => a.name === 'class');
      if (c && Array.isArray(c.value) && c.value.some((v: any) => v.type === 'Text' && v.data.split(/\s+/).includes(cls))) out.push({ el: n, inIf });
    }
    const nested = inIf || n.type === 'IfBlock';
    for (const key of ['fragment', 'consequent', 'alternate', 'nodes', 'body']) walk(n[key], nested);
  };
  walk(ast.fragment, false);
  return out;
}

const roleOf = (el: Node) => el.attributes!.find((a: any) => a.name === 'role' || a.name === 'aria-live');
const announce = (el: Node) => el.attributes!.find((a: any) => a.type === 'UseDirective' && a.name === 'announceShown');

test('git account test verdicts announce themselves and a failure interrupts (structure)', () => {
  const hits = elementsWithClass('GitAccounts.svelte', 'test-result');
  assert.equal(hits.length, 2, 'the row verdict and the form verdict');
  for (const { el, inIf } of hits) {
    assert.ok(inIf, 'inserted with its state');
    assert.equal(roleOf(el), undefined, 'no role mounted together with its words');
    const use = announce(el);
    assert.ok(use, 'spoken by announceShown');
    const props = use.expression.properties.map((p: any) => p.key.name);
    assert.deepEqual(props, ['text', 'assertive']);
  }
});

test('the conflict resolver’s "is resolved" note is spoken politely, without a role (structure)', () => {
  // The empty-pane placeholders: none carries a role; the resolved note speaks.
  const hits = elementsWithClass('ConflictResolverView.svelte', 'detail-empty');
  assert.ok(hits.length >= 1);
  for (const { el } of hits) assert.equal(roleOf(el), undefined);
  const spoken = hits.filter(({ el }) => announce(el));
  assert.equal(spoken.length, 1);
  assert.ok(spoken[0].inIf);
  assert.equal(announce(spoken[0].el).expression.type, 'TemplateLiteral');
});
