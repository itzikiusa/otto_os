import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import {
  applyHtmlBlockDirections,
  detectBlockDirection,
  stripTechnicalSpans,
} from '../src/lib/textDirection.ts';

describe('detectBlockDirection', () => {
  it('detects pure Hebrew prose as rtl', () => {
    assert.equal(detectBlockDirection('שלום עולם, מה נשמע היום?'), 'rtl');
  });

  it('detects pure Arabic prose as rtl', () => {
    assert.equal(detectBlockDirection('مرحبا بالعالم، يرجى التحقق من الملف الآن'), 'rtl');
  });

  it('detects pure English prose as ltr', () => {
    assert.equal(detectBlockDirection('Check the session store and verify the event stream.'), 'ltr');
  });

  it('keeps Hebrew sentences RTL when inline code, paths, @mentions, or URLs are embedded', () => {
    assert.equal(detectBlockDirection('בדוק את הקובץ `src/main.rs` בבקשה'), 'rtl');
    assert.equal(
      detectBlockDirection('פתח את /Users/dev/project/ui/src/App.svelte או @ui/src/lib/router.svelte.ts'),
      'rtl',
    );
    assert.equal(
      detectBlockDirection('קרא את התיעוד בכתובת https://docs.example.com/api/v1/sessions/stream לפני השינוי'),
      'rtl',
    );
    assert.equal(
      detectBlockDirection('1. `Composer.svelte`: הוסף תמיכה ב-RTL מלא עבור הודעות'),
      'rtl',
    );
  });

  it('keeps Hebrew RTL even when leading English identifiers appear in a mixed technical sentence', () => {
    assert.equal(
      detectBlockDirection('ConversationView ו-TurnItem צריכים לתמוך ביישור אוטומטי לכל פסקה'),
      'rtl',
    );
    assert.equal(
      detectBlockDirection('עדכן את DatabaseStore ב-TypeScript עבור WebSocket events'),
      'rtl',
    );
  });

  it('keeps English prose LTR when quoting a single Hebrew word', () => {
    assert.equal(
      detectBlockDirection('The greeting שלום is commonly used in Israeli UI localization tests.'),
      'ltr',
    );
  });

  it('ignores fenced code blocks when scoring surrounding Hebrew prose', () => {
    const md = [
      'הנה דוגמה לקוד שצריך לתקן:',
      '```ts',
      'export function renderChatMarkdown(md: string): string {',
      '  return marked.parse(md);',
      '}',
      '```',
    ].join('\n');
    assert.equal(detectBlockDirection(md), 'rtl');
  });

  it('returns auto for neutral text (numbers, punctuation, code-only)', () => {
    assert.equal(detectBlockDirection('12345 — 67890!'), 'auto');
    assert.equal(detectBlockDirection('`cargo test --workspace`'), 'auto');
    assert.equal(detectBlockDirection('   '), 'auto');
  });

  it('strips HTML entities so escaped quotes/ampersands do not count as English words', () => {
    const cleaned = stripTechnicalSpans('<p>שלום &quot;עולם&quot; &amp; חברים</p>');
    assert.ok(!cleaned.includes('quot'));
    assert.equal(detectBlockDirection('<p>שלום &quot;עולם&quot; &amp; חברים</p>'), 'rtl');
  });
});

describe('applyHtmlBlockDirections', () => {
  it('stamps per-paragraph dir attributes and isolates inline + fenced code as ltr', () => {
    const input = [
      '<p>בדוק את הקובץ <code>src/main.rs</code> בבקשה</p>',
      '<pre><code class="language-rust">fn main() { println!("שלום"); }</code></pre>',
      '<p>Now run the test suite and verify the output.</p>',
    ].join('\n');

    const output = applyHtmlBlockDirections(input);
    assert.match(output, /<p dir="rtl">בדוק את הקובץ <code dir="ltr" style="unicode-bidi: isolate">src\/main\.rs<\/code> בבקשה<\/p>/);
    assert.match(
      output,
      /<pre dir="ltr" style="unicode-bidi: isolate"><code class="language-rust" dir="ltr" style="unicode-bidi: isolate">/,
    );
    assert.match(output, /<p dir="ltr">Now run the test suite and verify the output\.<\/p>/);
  });

  it('handles mixed RTL/LTR list items, headings, blockquotes, and tables independently', () => {
    const input = [
      '<h2>סיכום שינויים</h2>',
      '<ul>',
      '<li>הוספת זיהוי כיוון אוטומטי לכל בלוק טקסט<ul><li>Implemented in <code>textDirection.ts</code> with unit tests</li></ul></li>',
      '<li>Updated Appearance settings and documentation</li>',
      '</ul>',
      '<blockquote><p>ציטוט בעברית בתוך בלוק</p></blockquote>',
      '<table><thead><tr><th>שדה</th><th>Type</th></tr></thead><tbody><tr><td>שם משתמש</td><td><code>string</code></td></tr></tbody></table>',
    ].join('');

    const output = applyHtmlBlockDirections(input);
    assert.match(output, /<h2 dir="rtl">סיכום שינויים<\/h2>/);
    assert.match(output, /<li dir="rtl">הוספת זיהוי כיוון אוטומטי לכל בלוק טקסט<ul><li dir="ltr">Implemented in <code dir="ltr" style="unicode-bidi: isolate">textDirection\.ts<\/code> with unit tests<\/li><\/ul><\/li>/);
    assert.match(output, /<li dir="ltr">Updated Appearance settings and documentation<\/li>/);
    assert.match(output, /<blockquote dir="rtl"><p dir="rtl">ציטוט בעברית בתוך בלוק<\/p><\/blockquote>/);
    assert.match(output, /<th dir="rtl">שדה<\/th><th dir="ltr">Type<\/th>/);
    assert.match(output, /<td dir="rtl">שם משתמש<\/td><td dir="ltr"><code dir="ltr" style="unicode-bidi: isolate">string<\/code><\/td>/);
  });

  it('preserves existing attributes and merges style on code/pre tags without duplicating unicode-bidi', () => {
    const input = '<code class="inline" style="color: red">const x = 1</code>';
    const output = applyHtmlBlockDirections(input);
    assert.equal(output, '<code class="inline" style="color: red; unicode-bidi: isolate" dir="ltr">const x = 1</code>');
    // Idempotent when called a second time.
    assert.equal(applyHtmlBlockDirections(output), output);
  });
});
