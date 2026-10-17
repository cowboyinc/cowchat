import test from 'node:test';
import assert from 'node:assert/strict';
import { preflightView, limits } from './preflight.mjs';
import { readFile } from 'node:fs/promises';
const library = {components:Object.fromEntries(['Stack','Card','Text','Metric','Table','BarChart','Series'].map(name => [name,{}]))};
const repeatedAliases = await readFile(new URL('./fixtures/repeated-aliases.openui', import.meta.url),'utf8');

test('raw AST preflight rejects compact repeated aliases without expanding or invoking Renderer', () => {
  assert.ok(repeatedAliases.length < 400);
  assert.throws(() => preflightView(repeatedAliases,{},library), /limits/);
  assert.ok(preflightView('item = Text("Hi")\nroot = Stack([item,item])',{},library).includes('root = Stack([Text("Hi"),Text("Hi")])'));
});

test('cycles, unknown references, runtime calls and recursive syntax are rejected before lowering', () => {
  for (const source of ['root = Stack([missing])', 'item = Stack([item])\nroot = Stack([item])',
    'root = Stack([Query("fetch",{})])','root = Stack([@Each($rows,row,Text(row))])',
    'root = Stack([@Set($value,"new")])', '$x = 1\nroot = Stack([])',
    `root = ${'Stack(['.repeat(limits.depth + 1)}Text("Hi")${'])'.repeat(limits.depth + 1)}`]) {
    assert.throws(() => preflightView(source,{},library), /limits/);
  }
});

test('bound data participates in expansion cost; runtime control objects cannot enter as JSON', () => {
  const source = 'root = Stack([Table([{key:"name",label:"Name"}],$rows)])';
  assert.ok(preflightView(source,{rows:{one:{name:'Alice'}}},library).includes('$rows'));
  assert.throws(() => preflightView('root = Stack([$payload])',{payload:{k:'Comp',name:'Text',args:[]}},library), /limits/);
  const rows = Object.fromEntries(Array.from({length:60},(_,index) => [index,{name:'Value'}]));
  const columns = Array.from({length:20},(_,index) => `{key:"${index}",label:"Column"}`).join(',');
  const table = `Table([${columns}],$rows)`;
  assert.throws(() => preflightView(`root = Stack([${Array(20).fill(table).join(',')}])`,{rows},library), /limits/);
});

test('canonical source contains only the bounded root and inlines named references', () => {
  const source = 'unused = Text("Unused")\nitem = Text("Hi")\nroot = Stack([item,item])';
  assert.equal(preflightView(source,{},library),'root = Stack([Text("Hi"),Text("Hi")])');
});
