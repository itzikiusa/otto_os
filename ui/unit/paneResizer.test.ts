import { test } from 'node:test';
import assert from 'node:assert/strict';
import { paneResizer } from '../src/lib/paneResizer.ts';

// A minimal separator node: attributes, a keydown listener and a direction.
function node(direction = 'ltr') {
  const attrs = new Map<string, string>();
  const handlers = new Map<string, (e: unknown) => void>();
  const el = {
    tabIndex: -1,
    setAttribute: (k: string, v: string) => attrs.set(k, v),
    removeAttribute: (k: string) => attrs.delete(k),
    addEventListener: (k: string, h: (e: unknown) => void) => handlers.set(k, h),
    removeEventListener: (k: string) => handlers.delete(k),
  };
  (globalThis as any).getComputedStyle = () => ({ direction });
  const press = (key: string) =>
    handlers.get('keydown')!({ key, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, preventDefault() {} });
  return { el: el as unknown as HTMLElement, attrs, press };
}

for (const invert of [false, true]) {
  test(`Home/End set the value's min/max${invert ? ' on an inverted (leading-edge) separator' : ''}`, () => {
    const { el, attrs, press } = node();
    const seen: number[] = [];
    paneResizer(el, { value: 400, min: 320, max: 655, invert, onChange: (v) => seen.push(v) });
    assert.equal(attrs.get('aria-valuemin'), '320');
    press('Home');
    press('End');
    assert.deepEqual(seen, [320, 655]);
  });
}

test('invert flips the arrows (and RTL mirrors them)', () => {
  const seen: number[] = [];
  const ltr = node('ltr');
  paneResizer(ltr.el, { value: 400, min: 320, max: 655, step: 10, invert: true, onChange: (v) => seen.push(v) });
  ltr.press('ArrowRight');
  const rtl = node('rtl');
  paneResizer(rtl.el, { value: 400, min: 320, max: 655, step: 10, invert: true, onChange: (v) => seen.push(v) });
  rtl.press('ArrowRight');
  assert.deepEqual(seen, [390, 410]);
});
