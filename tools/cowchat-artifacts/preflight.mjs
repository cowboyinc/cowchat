import { tokenize, split, parseExpression } from '@openuidev/lang-core';

export const limits = Object.freeze({ bytes:65536, tokens:8192, statements:128, depth:32,
  expandedNodes:20000, components:256, renderCost:20000, dataNodes:10000, dataDepth:16, dataBytes:524288 });
const astKinds = new Set(['Comp','Str','Num','Bool','Null','Arr','Obj','Ref','Ph','StateRef','RuntimeRef','BinOp','UnaryOp','Ternary','Member','Index','Assign']);
const unsafeKey = key => ['__proto__','prototype','constructor'].includes(key);
const fail = () => { throw new Error('This view exceeds the supported declarative layout limits.'); };
const quote = value => JSON.stringify(value).replaceAll('<','\\u003c');
const textCost = value => 1 + Math.ceil(value.length / 64);
const controlObject = value => value && typeof value === 'object' && !Array.isArray(value)
  && (astKinds.has(value.k) || value.type === 'element' || value.__reactive || Array.isArray(value.steps));

// No createParser/Renderer call is safe until named-reference expansion is bounded.
// These public APIs parse syntax only; they never resolve references or run builtins.
export function preflightView(source, data, library) {
  if (typeof source !== 'string' || !source.trim() || new TextEncoder().encode(source).length > limits.bytes) fail();
  if (!data || typeof data !== 'object' || Array.isArray(data)) fail();
  const dataWeights = new WeakMap();
  let dataNodes = 0;
  function weigh(value, depth = 0) {
    if (++dataNodes > limits.dataNodes || depth > limits.dataDepth || controlObject(value)) fail();
    if (!value || typeof value !== 'object') return typeof value === 'string' ? textCost(value) : 1;
    if (dataWeights.has(value)) return dataWeights.get(value);
    let weight = 1;
    for (const [key, item] of Object.entries(value)) { if (unsafeKey(key)) fail(); weight += weigh(item, depth + 1); }
    dataWeights.set(value, weight);
    return weight;
  }
  weigh(data);
  if (new TextEncoder().encode(JSON.stringify(data)).length > limits.dataBytes) fail();
  const tokens = tokenize(source);
  if (tokens.length > limits.tokens) fail();
  const brackets = [];
  const closing = new Map([[2,1],[4,3],[6,5]]);
  for (let index = 0; index < tokens.length; index++) {
    const token = tokens[index];
    if ([1,3,5].includes(token.t)) { brackets.push(token.t); if (brackets.length > limits.depth) fail(); }
    else if (closing.has(token.t) && brackets.pop() !== closing.get(token.t)) fail();
    // Reject operators/runtime calls before the recursive expression parser.
    else if (token.t > 19 && !(token.t === 21 && tokens[index + 1]?.t === 15)) fail();
  }
  if (brackets.length) fail();
  const statements = split(tokens);
  if (!statements.length || statements.length > limits.statements) fail();
  const definitions = new Map();
  for (const statement of statements) {
    if (statement.idTokenType === 18 || definitions.has(statement.id) || unsafeKey(statement.id)) fail();
    if (statement.tokens.some(token => token.t === 9)) fail();
    definitions.set(statement.id, parseExpression(statement.tokens));
  }
  if (!definitions.has('root')) fail();
  function children(node) {
    switch (node.k) {
      case 'Comp': if (node.fn || !Object.hasOwn(library.components, node.name)) fail(); return node.args;
      case 'Arr': return node.els;
      case 'Obj': if (node.entries.some(([key]) => unsafeKey(key))) fail(); return node.entries.map(([, value]) => value);
      case 'Member': if (unsafeKey(node.field)) fail(); return [node.obj];
      case 'Index': if (!['Str','Num'].includes(node.index.k) || unsafeKey(String(node.index.v))) fail(); return [node.obj,node.index];
      case 'UnaryOp': if (node.op !== '-' || node.operand.k !== 'Num') fail(); return [node.operand];
      case 'Str': case 'Num': case 'Bool': case 'Null': case 'Ref': case 'StateRef': return [];
      default: fail();
    }
  }
  // Check raw expression depth iteratively before any recursive AST walk.
  const pending = [...definitions.values()].map(node => [node,1]);
  while (pending.length) {
    const [node, depth] = pending.pop();
    if (depth > limits.depth) fail();
    for (const child of children(node)) pending.push([child,depth + 1]);
  }
  const own = (value, key) => value != null && Object.hasOwn(Object(value), key) ? value[key] : null;
  const values = new WeakMap();
  const visiting = new Set();
  function value(node) {
    if (values.has(node)) return values.get(node);
    let result;
    switch (node.k) {
      case 'Ref': result = value(definitions.get(node.n)); break;
      case 'StateRef': result = own(data,node.n.slice(1)); break;
      case 'Member': result = own(value(node.obj),node.field); break;
      case 'Index': result = own(value(node.obj),node.index.v); break;
      case 'Arr': result = node.els.map(value); break;
      case 'Obj': result = Object.fromEntries(node.entries.map(([key,item]) => [key,value(item)])); break;
      case 'UnaryOp': result = -node.operand.v; break;
      case 'Null': result = null; break;
      case 'Comp': result = {component:node.name,args:node.args.map(value)}; break;
      default: result = node.v;
    }
    if (controlObject(result)) fail();
    values.set(node,result);
    return result;
  }
  const memo = new WeakMap();
  function budget(node) {
    if (memo.has(node)) return memo.get(node);
    if (visiting.has(node)) fail();
    visiting.add(node);
    let parts = children(node);
    if (node.k === 'Ref') { if (!definitions.has(node.n)) fail(); parts = [definitions.get(node.n)]; }
    const cost = {nodes:node.k === 'Str' ? textCost(node.v) : 1, components:node.k === 'Comp' ? 1 : 0, depth:1, render:0};
    for (const child of parts) {
      const next = budget(child);
      cost.nodes += next.nodes; cost.components += next.components; cost.render += next.render;
      cost.depth = Math.max(cost.depth,1 + next.depth);
      check(cost);
    }
    if (node.k === 'Member') cost.nodes += textCost(node.field);
    if (node.k === 'StateRef') {
      const datum = own(data,node.n.slice(1));
      cost.nodes += datum && typeof datum === 'object' ? dataWeights.get(datum) : typeof datum === 'string' ? textCost(datum) : 1;
    }
    if (node.k === 'Comp') {
      // Count multiplicative work inside adapters, as well as the expanded tree.
      if (node.name === 'Table') {
        const columns = value(node.args[0]); const rows = node.args[1] ? value(node.args[1]) : null;
        cost.render += (Array.isArray(columns) ? columns.length : 0) * (rows && typeof rows === 'object' ? Object.keys(rows).length : 0);
      } else if (node.name.endsWith('Chart')) {
        const args = node.args.map(value);
        const first = Array.isArray(args[0]) ? args[0] : [];
        const second = Array.isArray(args[1]) ? args[1] : [];
        cost.render += first.length * Math.max(1,second.length);
      }
    }
    check(cost);
    visiting.delete(node); memo.set(node,cost); return cost;
  }
  function check(cost) {
    if (cost.nodes > limits.expandedNodes || cost.components > limits.components || cost.depth > limits.depth || cost.render > limits.renderCost) fail();
  }
  // Validate all definitions, including orphans, so no hidden runtime declarations survive.
  for (const node of definitions.values()) budget(node);
  const root = value(definitions.get('root'));
  if (root?.component !== 'Stack') fail();
  function serialize(node) {
    switch (node.k) {
      case 'Comp': return `${node.name}(${node.args.map(serialize).join(',')})`;
      case 'Ref': return serialize(definitions.get(node.n));
      case 'StateRef': return node.n;
      case 'Member': return `${serialize(node.obj)}[${quote(node.field)}]`;
      case 'Index': return `${serialize(node.obj)}[${serialize(node.index)}]`;
      case 'Arr': return `[${node.els.map(serialize).join(',')}]`;
      case 'Obj': return `{${node.entries.map(([key,item]) => `${quote(key)}:${serialize(item)}`).join(',')}}`;
      case 'UnaryOp': return String(-node.operand.v);
      case 'Null': return 'null';
      default: return quote(node.v);
    }
  }
  const canonical = `root = ${serialize(definitions.get('root'))}`;
  if (new TextEncoder().encode(canonical).length > limits.bytes) fail();
  return canonical;
}
