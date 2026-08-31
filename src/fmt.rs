//! Canonical whitespace formatter for Argent source.
//!
//! The formatter only touches whitespace; it never wraps, joins, or reorders
//! lines, and everything inside strings, line comments, and (nested) block
//! comments is preserved byte for byte. The rules:
//!
//! 1. Indentation is four spaces per bracket depth. A line whose first tokens
//!    are closing brackets dedents one level per leading closer. A line that
//!    starts with a binary operator indents one extra continuation level.
//!    Lines that begin inside a multi-line block comment or string are kept
//!    verbatim.
//! 2. Trailing whitespace is removed, except inside multi-line strings.
//! 3. Runs of blank lines collapse to one, leading blank lines are removed,
//!    and the file ends with exactly one newline.
//! 4. No whitespace before `,` or `;`, and exactly one space after them when
//!    more content follows on the same line.
//! 5. Exactly one space before `{` when it follows other content on the same
//!    line, unless the preceding character is `(`, `[`, or `{`. When `{`
//!    follows `]` the author's spacing is kept: constructor literals such as
//!    `State[2]{ ... }` attach, while block braces after array return types
//!    (`-> byte[32] {`) do not, and telling them apart needs a parser.
//!
//! The VS Code extension (vscode/argent-syntax/formatter.js) implements the
//! same rules; keep the two in sync.

#[cfg(test)]
mod tests;

const INDENT_UNIT: &str = "    ";

const CONTINUATION_OPERATORS: &[&str] =
    &["->", "<-", "==", "!=", "<=", ">=", "&&", "||", "+", "-", "*", "/", "%", "<", ">", "=", ".", "|", "&"];

/// Formats Argent source into its canonical whitespace form.
pub fn format_source(source: &str) -> String {
    let eol = if source.contains("\r\n") { "\r\n" } else { "\n" };
    let mut formatted: Vec<String> = Vec::new();
    let mut scan_state = ScanState::default();
    let mut depth = 0usize;
    let mut pending_blank = false;

    for line in split_lines(source) {
        let starts_protected = scan_state.block_depth > 0 || scan_state.string_quote.is_some();
        let (protected, next_state) = scan_line(line, scan_state);
        scan_state = next_state;
        let ends_in_string = scan_state.string_quote.is_some();

        if starts_protected {
            let kept = if ends_in_string { line } else { line.trim_end_matches([' ', '\t']) };
            formatted.push(kept.to_string());
            depth = update_bracket_depth(depth, line, &protected);
            continue;
        }

        let (body_start, body_end) = line_body(line, &protected, ends_in_string);
        if body_start == body_end {
            pending_blank = !formatted.is_empty();
            continue;
        }
        if pending_blank {
            formatted.push(String::new());
            pending_blank = false;
        }
        let body = &line[body_start..body_end];
        let level = indent_level(depth, body);
        formatted.push(format!("{}{}", INDENT_UNIT.repeat(level), normalize_body(body, &protected[body_start..body_end])));
        depth = update_bracket_depth(depth, line, &protected);
    }

    while formatted.last().is_some_and(String::is_empty) {
        formatted.pop();
    }
    let mut output = formatted.join(eol);
    output.push_str(eol);
    output
}

#[derive(Debug, Clone, Copy, Default)]
struct ScanState {
    block_depth: u32,
    string_quote: Option<u8>,
}

fn is_line_whitespace(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

fn is_open_bracket(byte: u8) -> bool {
    matches!(byte, b'(' | b'[' | b'{')
}

fn is_close_bracket(byte: u8) -> bool {
    matches!(byte, b')' | b']' | b'}')
}

/// Splits on `\r\n`, `\r`, or `\n`, keeping the (possibly empty) final line.
fn split_lines(source: &str) -> Vec<&str> {
    let bytes = source.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut pos = 0;
    while pos < bytes.len() {
        match bytes[pos] {
            b'\n' => {
                lines.push(&source[start..pos]);
                pos += 1;
                start = pos;
            }
            b'\r' => {
                lines.push(&source[start..pos]);
                pos += if bytes.get(pos + 1) == Some(&b'\n') { 2 } else { 1 };
                start = pos;
            }
            _ => pos += 1,
        }
    }
    lines.push(&source[start..]);
    lines
}

/// Marks which bytes of one line are protected (inside a string, line
/// comment, or block comment), given the scanner state at the start of the
/// line. Block comments nest, matching the compiler lexer.
fn scan_line(line: &str, mut state: ScanState) -> (Vec<bool>, ScanState) {
    let bytes = line.as_bytes();
    let mut protected = vec![false; bytes.len()];
    let mut pos = 0;

    while pos < bytes.len() {
        let byte = bytes[pos];

        if state.block_depth > 0 {
            protected[pos] = true;
            if byte == b'*' && bytes.get(pos + 1) == Some(&b'/') {
                protected[pos + 1] = true;
                state.block_depth -= 1;
                pos += 2;
            } else if byte == b'/' && bytes.get(pos + 1) == Some(&b'*') {
                protected[pos + 1] = true;
                state.block_depth += 1;
                pos += 2;
            } else {
                pos += 1;
            }
            continue;
        }

        if let Some(quote) = state.string_quote {
            protected[pos] = true;
            if byte == b'\\' && pos + 1 < bytes.len() {
                protected[pos + 1] = true;
                pos += 2;
            } else {
                if byte == quote {
                    state.string_quote = None;
                }
                pos += 1;
            }
            continue;
        }

        if byte == b'/' && bytes.get(pos + 1) == Some(&b'/') {
            protected[pos..].fill(true);
            break;
        }
        if byte == b'/' && bytes.get(pos + 1) == Some(&b'*') {
            protected[pos] = true;
            protected[pos + 1] = true;
            state.block_depth = 1;
            pos += 2;
            continue;
        }
        if byte == b'"' || byte == b'\'' {
            protected[pos] = true;
            state.string_quote = Some(byte);
            pos += 1;
            continue;
        }
        pos += 1;
    }

    (protected, state)
}

fn update_bracket_depth(depth: usize, line: &str, protected: &[bool]) -> usize {
    let mut depth = depth;
    for (pos, &byte) in line.as_bytes().iter().enumerate() {
        if protected[pos] {
            continue;
        }
        if is_open_bracket(byte) {
            depth += 1;
        } else if is_close_bracket(byte) {
            depth = depth.saturating_sub(1);
        }
    }
    depth
}

/// The byte range between the leading indentation and any trailing
/// whitespace. Trailing whitespace is kept when the line ends inside a string.
fn line_body(line: &str, protected: &[bool], ends_in_string: bool) -> (usize, usize) {
    let bytes = line.as_bytes();
    let mut start = 0;
    while start < bytes.len() && is_line_whitespace(bytes[start]) {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && is_line_whitespace(bytes[end - 1]) && !(ends_in_string && protected[end - 1]) {
        end -= 1;
    }
    (start, end)
}

fn is_continuation_line(body: &str) -> bool {
    if body.starts_with("//") || body.starts_with("/*") {
        return false;
    }
    CONTINUATION_OPERATORS.iter().any(|operator| body.starts_with(operator))
}

fn indent_level(depth: usize, body: &str) -> usize {
    let bytes = body.as_bytes();
    let mut closers = 0usize;
    let mut pos = 0;
    while pos < bytes.len() && (is_line_whitespace(bytes[pos]) || is_close_bracket(bytes[pos])) {
        if is_close_bracket(bytes[pos]) {
            closers += 1;
        }
        pos += 1;
    }
    if closers > 0 {
        return depth.saturating_sub(closers);
    }
    if is_continuation_line(body) { depth + 1 } else { depth }
}

fn trim_unprotected_trailing_space(out: &mut Vec<u8>, out_protected: &mut Vec<bool>) {
    while let (Some(&byte), Some(&protected)) = (out.last(), out_protected.last()) {
        if protected || !is_line_whitespace(byte) {
            break;
        }
        out.pop();
        out_protected.pop();
    }
}

/// Applies the punctuation spacing rules to one line body.
fn normalize_body(body: &str, protected: &[bool]) -> String {
    let bytes = body.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() + 8);
    let mut out_protected: Vec<bool> = Vec::with_capacity(bytes.len() + 8);
    let mut pos = 0;

    while pos < bytes.len() {
        let byte = bytes[pos];
        if protected[pos] {
            out.push(byte);
            out_protected.push(true);
            pos += 1;
            continue;
        }
        if byte == b',' || byte == b';' {
            trim_unprotected_trailing_space(&mut out, &mut out_protected);
            out.push(byte);
            out_protected.push(false);
            let mut next = pos + 1;
            while next < bytes.len() && !protected[next] && is_line_whitespace(bytes[next]) {
                next += 1;
            }
            if next < bytes.len() {
                out.push(b' ');
                out_protected.push(false);
            }
            pos = next;
            continue;
        }
        if byte == b'{' {
            let mut last_content = out.len();
            while last_content > 0 && is_line_whitespace(out[last_content - 1]) {
                last_content -= 1;
            }
            let follows_bracket = last_content > 0 && out[last_content - 1] == b']';
            if !follows_bracket {
                trim_unprotected_trailing_space(&mut out, &mut out_protected);
                if out.last().is_some_and(|&last| !is_open_bracket(last)) {
                    out.push(b' ');
                    out_protected.push(false);
                }
            }
            out.push(b'{');
            out_protected.push(false);
            pos += 1;
            continue;
        }
        out.push(byte);
        out_protected.push(false);
        pos += 1;
    }

    String::from_utf8(out).expect("formatting edits whitespace at ASCII boundaries only")
}
