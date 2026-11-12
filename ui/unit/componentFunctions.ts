import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';

/** Run actual component handlers with explicit boundary adapters, without a DOM.
 * Keep each test's state visible; this does not emulate Svelte reactivity. */
export function componentFunctions(path: URL, names: string[], state: Record<string, any>) {
  const text = readFileSync(path, 'utf8');
  // A plain `.ts` module (helpers moved out of a component) is read whole.
  const script = text.includes('<script lang="ts">') ? text.split('<script lang="ts">')[1].split('</script>')[0] : text;
  const file = ts.createSourceFile('component.ts', script, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const declarations = file.statements.filter(n => ts.isFunctionDeclaration(n) && n.name && names.includes(n.name.text));
  const source = declarations.map(n => n.getText(file).replace(/^export\s+/, '')).join('\n');
  const context = { Error, setTimeout, clearTimeout, ...state };
  runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return context as Record<string, any>;
}
