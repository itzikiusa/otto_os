import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';

// Exercise the editor's actual parse/encode boundary: edits to another form
// field must not erase an unavailable upload's marker and make it sendable.
const source = readFileSync(new URL('../src/modules/api/RequestBuilder.svelte',import.meta.url),'utf8');
const start = source.indexOf('  type FieldType =');
const end = source.indexOf('  let formRows =',start);
assert.ok(start >= 0 && end > start);
const context = {exports:{} as Record<string, any>};
runInNewContext(ts.transpileModule(source.slice(start,end)+'\nexports.parseForm=parseForm; exports.encodeForm=encodeForm;',
  {compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,context);
const {parseForm,encodeForm} = context.exports;

for (const marker of [{missing_content:true},{size:5}]) {
  test(`editing another multipart row preserves unavailable file marker ${JSON.stringify(marker)}`, () => {
    const rows = parseForm(JSON.stringify([{key:'file',type:'file',filename:'hello.txt',value:'',...marker},{key:'text',type:'text',value:'old'}]),'multipart');
    rows[1].value = 'new';
    const encoded = JSON.parse(encodeForm(rows,'multipart'));
    assert.equal(encoded[0].missing_content,true);
    assert.equal(encoded[1].value,'new');
    rows[0] = {...rows[0],value:'aGVsbG8=',missing_content:false,size:undefined};
    assert.equal(JSON.parse(encodeForm(rows,'multipart'))[0].missing_content,undefined);
  });
}

test('selected empty files stay distinct from historical unavailable files', () => {
  const rows = parseForm('[{"key":"file","type":"file","filename":"empty.txt","value":""}]','multipart');
  assert.equal(JSON.parse(encodeForm(rows,'multipart'))[0].missing_content,undefined);
});
