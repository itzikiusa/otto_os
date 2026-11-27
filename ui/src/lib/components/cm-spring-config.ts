// CodeMirror 6 autocompletion for Spring Boot `application*.{properties,yml,yaml}`
// and `bootstrap*.{properties,yml,yaml}` configuration files.
// Shares the single curated property catalog `docs/contracts/spring-boot-properties.json`
// with the Rust backend (`crates/otto-ide/src/spring_boot/props.rs`).
import { autocompletion, type Completion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import type { Extension } from '@codemirror/state';
import rawCatalog from '../../../../docs/contracts/spring-boot-properties.json' with { type: 'json' };

export interface SpringPropSpec {
  key: string;
  type: 'string' | 'integer' | 'boolean' | 'enum';
  defaultValue?: string;
  info: string;
  allowed?: readonly string[];
  deprecated?: string;
}

interface RawSpringProp {
  key: string;
  value_type: 'string' | 'integer' | 'boolean' | 'enum';
  default_value?: string;
  description: string;
  deprecated_replacement?: string;
  allowed_values?: string[];
}

export const SPRING_BOOT_CONFIG_PROPS: readonly SpringPropSpec[] = (rawCatalog as RawSpringProp[])
  .filter((p) => !p.deprecated_replacement)
  .map((p) => ({
    key: p.key,
    type: p.value_type,
    ...(p.default_value !== undefined ? { defaultValue: p.default_value } : {}),
    info: p.description,
    ...(p.allowed_values && p.allowed_values.length > 0 ? { allowed: p.allowed_values } : {}),
  }));

export function isSpringConfigPath(filePath: string): boolean {
  const base = (filePath.split(/[\\/]/).pop() ?? '').toLowerCase();
  return (
    (base.startsWith('application') || base.startsWith('bootstrap'))
    && (base.endsWith('.properties') || base.endsWith('.yml') || base.endsWith('.yaml'))
  );
}

function lookupSpec(key: string): SpringPropSpec | undefined {
  const exact = SPRING_BOOT_CONFIG_PROPS.find((p) => p.key === key);
  if (exact) return exact;
  if (key.startsWith('logging.level.')) {
    return SPRING_BOOT_CONFIG_PROPS.find((p) => p.key === 'logging.level.root');
  }
  return undefined;
}

function inferYamlParentPrefix(lines: string[], targetIdx: number): string[] {
  const targetLine = lines[targetIdx] ?? '';
  const targetIndent = targetLine.length - targetLine.trimStart().length;
  const stack: Array<{ indent: number; key: string }> = [];
  for (let i = 0; i < targetIdx; i++) {
    const raw = lines[i];
    const trimmed = raw.trim();
    if (!trimmed || trimmed.startsWith('#') || trimmed.startsWith('---') || trimmed.startsWith('- ')) continue;
    const colon = trimmed.indexOf(':');
    if (colon <= 0) continue;
    const indent = raw.length - raw.trimStart().length;
    const seg = trimmed.slice(0, colon).trim().replace(/^['"]|['"]$/g, '');
    const rest = trimmed.slice(colon + 1).trim();
    while (stack.length > 0 && stack[stack.length - 1].indent >= indent) stack.pop();
    if (!rest || rest.startsWith('#')) {
      stack.push({ indent, key: seg });
    }
  }
  while (stack.length > 0 && stack[stack.length - 1].indent >= targetIndent) stack.pop();
  return stack.map((s) => s.key);
}

function detectYamlIndentUnit(lines: string[]): string {
  for (const raw of lines) {
    const m = /^( +)\S/.exec(raw);
    if (m && m[1].length >= 2 && m[1].length <= 4) return m[1];
  }
  return '  ';
}

export interface SpringCompletionMatch {
  from: number;
  options: Completion[];
}

/** Pure completion engine for Spring Boot `.properties` and `.yml`/`.yaml` buffers. */
export function completeSpringConfigAt(
  filePath: string,
  docText: string,
  pos: number,
  explicit: boolean,
  indentUnit?: string,
): SpringCompletionMatch | null {
  if (!isSpringConfigPath(filePath)) return null;
  const lineStart = docText.lastIndexOf('\n', Math.max(0, pos - 1)) + 1;
  const before = docText.slice(lineStart, pos);
  const trimmedBefore = before.trimStart();
  if (trimmedBefore.startsWith('#') || trimmedBefore.startsWith('!')) return null;

  const isProps = filePath.toLowerCase().endsWith('.properties');
  if (isProps) {
    const eqIdx = before.search(/[=:]/);
    if (eqIdx >= 0) {
      const key = before.slice(0, eqIdx).trim();
      const spec = lookupSpec(key);
      if (!spec?.allowed?.length) return null;
      const valPrefix = before.slice(eqIdx + 1).trimStart();
      const from = pos - valPrefix.length;
      const options: Completion[] = spec.allowed
        .filter((v) => !valPrefix || v.toLowerCase().startsWith(valPrefix.toLowerCase()))
        .map((v) => ({ label: v, type: 'enum', detail: spec.type, info: spec.info }));
      return options.length ? { from, options } : null;
    }
    if (!explicit && trimmedBefore.length < 2) return null;
    const from = pos - trimmedBefore.length;
    const q = trimmedBefore.toLowerCase();
    const options: Completion[] = SPRING_BOOT_CONFIG_PROPS
      .filter((p) => !q || p.key.toLowerCase().includes(q))
      .map((p) => ({
        label: p.key,
        type: 'property',
        detail: p.defaultValue ? `${p.type} = ${p.defaultValue}` : p.type,
        info: p.info,
        apply: `${p.key}=`,
      }));
    return options.length ? { from, options } : null;
  }

  // YAML mode (`.yml` / `.yaml`)
  const lines = docText.split('\n');
  const lineIdx = docText.slice(0, lineStart).split('\n').length - 1;
  const parentPrefix = inferYamlParentPrefix(lines, lineIdx);
  const colonIdx = before.indexOf(':');

  if (colonIdx >= 0) {
    const localKey = before.slice(0, colonIdx).trim();
    const fullKey = [...parentPrefix, localKey].filter(Boolean).join('.');
    const spec = lookupSpec(fullKey);
    if (!spec?.allowed?.length) return null;
    const valPrefix = before.slice(colonIdx + 1).trimStart();
    const from = pos - valPrefix.length;
    const options: Completion[] = spec.allowed
      .filter((v) => !valPrefix || v.toLowerCase().startsWith(valPrefix.toLowerCase()))
      .map((v) => ({ label: v, type: 'enum', detail: spec.type, info: spec.info }));
    return options.length ? { from, options } : null;
  }

  if (!explicit && trimmedBefore.length < 1 && parentPrefix.length === 0) return null;
  const from = pos - trimmedBefore.length;
  const lineIndent = before.slice(0, before.length - trimmedBefore.length);
  const unit = indentUnit ?? detectYamlIndentUnit(lines);
  const dottedPrefix = parentPrefix.length > 0 ? `${parentPrefix.join('.')}.` : '';
  const q = `${dottedPrefix}${trimmedBefore}`.toLowerCase();

  const seen = new Set<string>();
  const options: Completion[] = [];
  for (const p of SPRING_BOOT_CONFIG_PROPS) {
    if (dottedPrefix && !p.key.startsWith(dottedPrefix)) continue;
    const remainder = dottedPrefix ? p.key.slice(dottedPrefix.length) : p.key;
    const nextSeg = remainder.split('.')[0];
    if (!nextSeg) continue;
    const fullCand = `${dottedPrefix}${nextSeg}`.toLowerCase();
    if (q && !fullCand.startsWith(q) && !p.key.toLowerCase().startsWith(q)) continue;
    if (seen.has(nextSeg)) continue;
    seen.add(nextSeg);
    const isLeaf = !remainder.includes('.');
    options.push({
      label: nextSeg,
      type: 'property',
      detail: isLeaf ? (p.defaultValue ? `${p.type} = ${p.defaultValue}` : p.type) : 'group',
      info: isLeaf ? p.info : `Spring Boot config group (${dottedPrefix}${nextSeg}.*)`,
      apply: isLeaf ? `${nextSeg}: ` : `${nextSeg}:\n${lineIndent}${unit}`,
    });
  }
  return options.length ? { from, options } : null;
}

export function springConfigCompletion(filePath: string): Extension {
  if (!isSpringConfigPath(filePath)) return [];
  return autocompletion({
    override: [
      (ctx: CompletionContext): CompletionResult | null => {
        const res = completeSpringConfigAt(filePath, ctx.state.doc.toString(), ctx.pos, ctx.explicit);
        if (!res) return null;
        return { from: res.from, options: res.options, validFor: /^[\w.-]*$/ };
      },
    ],
  });
}
