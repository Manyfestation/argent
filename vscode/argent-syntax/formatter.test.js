'use strict';

const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');
const { formatSource } = require('./formatter');

test('reindents bracket nesting to four spaces per level', () => {
  const input = ['actor Ticket owns TicketState {', ' entry redeem(sig s) {', '\t\trequire(redeemed == 0);', '   }', '}', ''].join(
    '\n',
  );
  const expected = [
    'actor Ticket owns TicketState {',
    '    entry redeem(sig s) {',
    '        require(redeemed == 0);',
    '    }',
    '}',
    '',
  ].join('\n');
  assert.strictEqual(formatSource(input), expected);
});

test('indents continuation-operator lines one extra level', () => {
  const input = [
    'fn f() -> byte[36] {',
    'byte[36] outpoint = byte[36](',
    'OpOutpointTxId(this.activeInputIndex)',
    '+ (OpOutpointIndex(this.activeInputIndex) as byte[4])',
    ');',
    'return outpoint;',
    '}',
    '',
  ].join('\n');
  const expected = [
    'fn f() -> byte[36] {',
    '    byte[36] outpoint = byte[36](',
    '        OpOutpointTxId(this.activeInputIndex)',
    '            + (OpOutpointIndex(this.activeInputIndex) as byte[4])',
    '    );',
    '    return outpoint;',
    '}',
    '',
  ].join('\n');
  assert.strictEqual(formatSource(input), expected);
});

test('dedents lines that start with closing brackets', () => {
  const input = ['entry issue(', 'sig admin_sig', ') emits {', 'ticket: Ticket,', '} {', 'require(true);', '}', ''].join('\n');
  const expected = [
    'entry issue(',
    '    sig admin_sig',
    ') emits {',
    '    ticket: Ticket,',
    '} {',
    '    require(true);',
    '}',
    '',
  ].join('\n');
  assert.strictEqual(formatSource(input), expected);
});

test('normalizes comma and semicolon spacing', () => {
  assert.strictEqual(formatSource('f(a ,b,c ,  d) ;\n'), 'f(a, b, c, d);\n');
  assert.strictEqual(formatSource('int x = { a: 1,b: 2, };\n'), 'int x = { a: 1, b: 2, };\n');
});

test('ensures a single space before an attached brace', () => {
  assert.strictEqual(formatSource('actor A owns S{\n}\n'), 'actor A owns S {\n}\n');
  assert.strictEqual(formatSource('if (x){\n}\n'), 'if (x) {\n}\n');
  assert.strictEqual(formatSource('if (x)  {\n}\n'), 'if (x) {\n}\n');
  assert.strictEqual(formatSource('f({ a: 1 });\n'), 'f({ a: 1 });\n');
});

test('keeps author spacing for braces that follow a bracket', () => {
  assert.strictEqual(formatSource('State[2] x = State[2]{ value, local };\n'), 'State[2] x = State[2]{ value, local };\n');
  assert.strictEqual(formatSource('fn f() -> byte[32] {\n}\n'), 'fn f() -> byte[32] {\n}\n');
});

test('collapses blank line runs and trims the file edges', () => {
  const input = '\n\nconst int A = 1;\n\n\n\nconst int B = 2;\n\n\n';
  assert.strictEqual(formatSource(input), 'const int A = 1;\n\nconst int B = 2;\n');
});

test('strips trailing whitespace', () => {
  assert.strictEqual(formatSource('const int A = 1;   \n// note\t\n'), 'const int A = 1;\n// note\n');
});

test('leaves strings untouched', () => {
  const input = 'byte[] d = byte[]("a ,b  {  // }");\n';
  assert.strictEqual(formatSource(input), input);
  assert.strictEqual(formatSource('f("\\" ,x" ,y);\n'), 'f("\\" ,x", y);\n');
});

test('leaves comment interiors untouched', () => {
  const input = '// keep  ,this {  spacing\nconst int A = 1; // and , this {\n';
  assert.strictEqual(formatSource(input), input);
});

test('keeps multi-line block comment interiors verbatim', () => {
  const input = ['fn f() {', '    /*', '       hand-drawn {  ,  art', '         more */', '    require(true);', '}', ''].join('\n');
  assert.strictEqual(formatSource(input), input);
});

test('handles nested block comments like the compiler lexer', () => {
  const input = ['/* outer /* inner */ still a comment { */', 'const int A = 1;', ''].join('\n');
  assert.strictEqual(formatSource(input), input);
});

test('brackets inside comments and strings do not affect depth', () => {
  const input = ['fn f() {', '    // if (x) {', '    require(g("{"));', '}', ''].join('\n');
  assert.strictEqual(formatSource(input), input);
});

test('does not disturb hex-style literals', () => {
  const input = 'const byte OWNER_P2PK_SCHNORR = 0x00;\n';
  assert.strictEqual(formatSource(input), input);
});

test('indents line comments with the surrounding code', () => {
  const input = ['actor A owns S {', '// leading note', 'entry f() {', '}', '}', ''].join('\n');
  const expected = ['actor A owns S {', '    // leading note', '    entry f() {', '    }', '}', ''].join('\n');
  assert.strictEqual(formatSource(input), expected);
});

test('preserves CRLF line endings', () => {
  assert.strictEqual(formatSource('actor A owns S {\r\nentry f() {\r\n}\r\n}\r\n'), 'actor A owns S {\r\n    entry f() {\r\n    }\r\n}\r\n');
});

test('ends the file with exactly one newline', () => {
  assert.strictEqual(formatSource('const int A = 1;'), 'const int A = 1;\n');
  assert.strictEqual(formatSource(''), '\n');
});

test('is tolerant of unbalanced input', () => {
  assert.strictEqual(formatSource('}\n)\nconst int A = 1;\n'), '}\n)\nconst int A = 1;\n');
  assert.strictEqual(formatSource('fn f() {\nrequire(true);\n'), 'fn f() {\n    require(true);\n');
});

test('is idempotent across the repository sources', () => {
  const repoRoot = path.resolve(__dirname, '..', '..');
  const sources = [];
  const walk = (directory) => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      if (entry.name === 'target' || entry.name === 'node_modules' || entry.name.startsWith('.')) {
        continue;
      }
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.name.endsWith('.ag')) {
        sources.push(full);
      }
    }
  };
  walk(repoRoot);
  assert.ok(sources.length > 0, 'expected repository .ag sources');
  for (const source of sources) {
    const original = fs.readFileSync(source, 'utf8');
    const once = formatSource(original);
    assert.strictEqual(formatSource(once), once, `formatting is not idempotent for ${source}`);
  }
});
