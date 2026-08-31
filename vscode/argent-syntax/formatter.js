'use strict';

/**
 * Canonical whitespace formatter for Argent source.
 *
 * The formatter only touches whitespace; it never wraps, joins, or reorders
 * lines, and everything inside strings, line comments, and (nested) block
 * comments is preserved byte for byte. The rules:
 *
 * 1. Indentation is four spaces per bracket depth. A line whose first tokens
 *    are closing brackets dedents one level per leading closer. A line that
 *    starts with a binary operator indents one extra continuation level.
 *    Lines that begin inside a multi-line block comment or string are kept
 *    verbatim.
 * 2. Trailing whitespace is removed, except inside multi-line strings.
 * 3. Runs of blank lines collapse to one, leading blank lines are removed,
 *    and the file ends with exactly one newline.
 * 4. No whitespace before `,` or `;`, and exactly one space after them when
 *    more content follows on the same line.
 * 5. Exactly one space before `{` when it follows other content on the same
 *    line, unless the preceding character is `(`, `[`, or `{`. When `{`
 *    follows `]` the author's spacing is kept: constructor literals such as
 *    `State[2]{ ... }` attach, while block braces after array return types
 *    (`-> byte[32] {`) do not, and telling them apart needs a parser.
 *
 * `argentc fmt` (src/fmt.rs in the compiler repo) implements the same rules;
 * keep the two in sync.
 */

const INDENT_UNIT = '    ';

const CONTINUATION_OPERATORS = ['->', '<-', '==', '!=', '<=', '>=', '&&', '||', '+', '-', '*', '/', '%', '<', '>', '=', '.', '|', '&'];

const OPEN_BRACKETS = '([{';
const CLOSE_BRACKETS = ')]}';

function isLineWhitespace(char) {
  return char === ' ' || char === '\t';
}

/**
 * Marks which characters of one line are protected (inside a string, line
 * comment, or block comment), given the scanner state at the start of the
 * line. Block comments nest, matching the compiler lexer.
 */
function scanLine(line, state) {
  const protectedFlags = new Array(line.length).fill(false);
  let { blockDepth, stringQuote } = state;
  let pos = 0;

  while (pos < line.length) {
    const char = line[pos];

    if (blockDepth > 0) {
      protectedFlags[pos] = true;
      if (char === '*' && line[pos + 1] === '/') {
        protectedFlags[pos + 1] = true;
        blockDepth -= 1;
        pos += 2;
      } else if (char === '/' && line[pos + 1] === '*') {
        protectedFlags[pos + 1] = true;
        blockDepth += 1;
        pos += 2;
      } else {
        pos += 1;
      }
      continue;
    }

    if (stringQuote) {
      protectedFlags[pos] = true;
      if (char === '\\' && pos + 1 < line.length) {
        protectedFlags[pos + 1] = true;
        pos += 2;
      } else {
        if (char === stringQuote) {
          stringQuote = undefined;
        }
        pos += 1;
      }
      continue;
    }

    if (char === '/' && line[pos + 1] === '/') {
      protectedFlags.fill(true, pos);
      break;
    }
    if (char === '/' && line[pos + 1] === '*') {
      protectedFlags[pos] = true;
      protectedFlags[pos + 1] = true;
      blockDepth = 1;
      pos += 2;
      continue;
    }
    if (char === '"' || char === "'") {
      protectedFlags[pos] = true;
      stringQuote = char;
      pos += 1;
      continue;
    }
    pos += 1;
  }

  return { protectedFlags, state: { blockDepth, stringQuote } };
}

function updateBracketDepth(depth, line, protectedFlags) {
  for (let pos = 0; pos < line.length; pos += 1) {
    if (protectedFlags[pos]) {
      continue;
    }
    if (OPEN_BRACKETS.includes(line[pos])) {
      depth += 1;
    } else if (CLOSE_BRACKETS.includes(line[pos])) {
      depth = Math.max(0, depth - 1);
    }
  }
  return depth;
}

/**
 * The line content between the leading indentation and any trailing
 * whitespace. Trailing whitespace is kept when the line ends inside a string.
 */
function lineBody(line, protectedFlags, endsInString) {
  let start = 0;
  while (start < line.length && isLineWhitespace(line[start])) {
    start += 1;
  }
  let end = line.length;
  while (end > start && isLineWhitespace(line[end - 1]) && !(endsInString && protectedFlags[end - 1])) {
    end -= 1;
  }
  return { text: line.slice(start, end), protectedFlags: protectedFlags.slice(start, end) };
}

function isContinuationLine(text) {
  if (text.startsWith('//') || text.startsWith('/*')) {
    return false;
  }
  return CONTINUATION_OPERATORS.some((operator) => text.startsWith(operator));
}

function indentLevel(depth, bodyText) {
  let closers = 0;
  let pos = 0;
  while (pos < bodyText.length && (isLineWhitespace(bodyText[pos]) || CLOSE_BRACKETS.includes(bodyText[pos]))) {
    if (CLOSE_BRACKETS.includes(bodyText[pos])) {
      closers += 1;
    }
    pos += 1;
  }
  if (closers > 0) {
    return Math.max(0, depth - closers);
  }
  return isContinuationLine(bodyText) ? depth + 1 : depth;
}

/** Applies the punctuation spacing rules to one line body. */
function normalizeBody(text, protectedFlags) {
  let out = '';
  const outProtected = [];
  const push = (char, isProtected) => {
    out += char;
    outProtected.push(isProtected);
  };
  const trimUnprotectedTrailingSpace = () => {
    while (out.length > 0 && !outProtected.at(-1) && isLineWhitespace(out.at(-1))) {
      out = out.slice(0, -1);
      outProtected.pop();
    }
  };

  let pos = 0;
  while (pos < text.length) {
    const char = text[pos];
    if (protectedFlags[pos]) {
      push(char, true);
      pos += 1;
      continue;
    }
    if (char === ',' || char === ';') {
      trimUnprotectedTrailingSpace();
      push(char, false);
      let next = pos + 1;
      while (next < text.length && !protectedFlags[next] && isLineWhitespace(text[next])) {
        next += 1;
      }
      if (next < text.length) {
        push(' ', false);
      }
      pos = next;
      continue;
    }
    if (char === '{') {
      let lastContent = out.length - 1;
      while (lastContent >= 0 && isLineWhitespace(out[lastContent])) {
        lastContent -= 1;
      }
      if (lastContent < 0 || out[lastContent] !== ']') {
        trimUnprotectedTrailingSpace();
        if (out.length > 0 && !OPEN_BRACKETS.includes(out.at(-1))) {
          push(' ', false);
        }
      }
      push('{', false);
      pos += 1;
      continue;
    }
    push(char, false);
    pos += 1;
  }
  return out;
}

function formatSource(source) {
  const eol = source.includes('\r\n') ? '\r\n' : '\n';
  const lines = source.split(/\r\n|\r|\n/);
  const formatted = [];
  let scanState = { blockDepth: 0, stringQuote: undefined };
  let depth = 0;
  let pendingBlank = false;

  for (const line of lines) {
    const startsProtected = scanState.blockDepth > 0 || scanState.stringQuote !== undefined;
    const { protectedFlags, state } = scanLine(line, scanState);
    scanState = state;
    const endsInString = scanState.stringQuote !== undefined;

    if (startsProtected) {
      formatted.push(endsInString ? line : line.replace(/[ \t]+$/, ''));
      depth = updateBracketDepth(depth, line, protectedFlags);
      continue;
    }

    const body = lineBody(line, protectedFlags, endsInString);
    if (body.text.length === 0) {
      pendingBlank = formatted.length > 0;
      continue;
    }
    if (pendingBlank) {
      formatted.push('');
      pendingBlank = false;
    }
    const level = indentLevel(depth, body.text);
    formatted.push(INDENT_UNIT.repeat(level) + normalizeBody(body.text, body.protectedFlags));
    depth = updateBracketDepth(depth, line, protectedFlags);
  }

  while (formatted.length > 0 && formatted.at(-1) === '') {
    formatted.pop();
  }
  return formatted.join(eol) + eol;
}

module.exports = { formatSource };
