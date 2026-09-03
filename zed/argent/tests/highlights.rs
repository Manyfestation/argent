use std::collections::HashSet;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

const HIGHLIGHTS: &str = include_str!("../languages/argent/highlights.scm");
const INDENTS: &str = include_str!("../languages/argent/indents.scm");
const BRACKETS: &str = include_str!("../languages/argent/brackets.scm");

type Highlighted = HashSet<(String, String)>;

fn language() -> Language {
    tree_sitter_argent::LANGUAGE.into()
}

/// Parses `source` and returns every (capture name, highlighted text) pair.
fn highlight(language: &Language, source: &[u8]) -> (tree_sitter::Tree, Highlighted) {
    let mut parser = Parser::new();
    parser.set_language(language).expect("load Argent grammar");
    let tree = parser.parse(source, None).expect("parser returned a tree");
    let query = Query::new(language, HIGHLIGHTS).expect("compile highlight query");
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(&query, tree.root_node(), source);
    let mut highlighted = HashSet::new();
    while let Some((matched, capture_index)) = captures.next() {
        let capture = matched.captures[*capture_index];
        let text = std::str::from_utf8(&source[capture.node.byte_range()]).expect("sample is UTF-8");
        highlighted.insert((capture_names[capture.index as usize].to_string(), text.to_string()));
    }
    (tree, highlighted)
}

fn has(highlighted: &Highlighted, capture: &str, text: &str) -> bool {
    highlighted.contains(&(capture.to_string(), text.to_string()))
}

#[test]
fn zed_queries_match_the_canonical_grammar_queries() {
    assert_eq!(
        HIGHLIGHTS,
        include_str!("../../../tree-sitter/queries/highlights.scm"),
        "run `npm run sync:queries` in tree-sitter: Zed and grammar highlighting queries drifted"
    );
    assert_eq!(
        INDENTS,
        include_str!("../../../tree-sitter/queries/indents.scm"),
        "run `npm run sync:queries` in tree-sitter: Zed and grammar indentation queries drifted"
    );
}

#[test]
fn zed_queries_compile_against_the_grammar() {
    let language = language();
    Query::new(&language, HIGHLIGHTS).expect("compile highlight query");
    Query::new(&language, BRACKETS).expect("compile bracket query");
    Query::new(&language, INDENTS).expect("compile indentation query");
}

#[test]
fn query_highlights_argent_words_and_core_syntax() {
    let source = br#"
        state WalletState {
            byte[32] owner;
            int balance;
        }

        actor Wallet owns WalletState {
            entry send(int amount) emits next: Wallet {
                require(checkSig(signature, public_key));
                require(this.activeInputIndex == 0);
                become next <- Wallet({ balance: self.balance - amount });
            }
        }
    "#;
    let (tree, highlighted) = highlight(&language(), source);
    assert!(!tree.root_node().has_error(), "{}", tree.root_node().to_sexp());

    assert!(has(&highlighted, "keyword", "actor"));
    assert!(has(&highlighted, "keyword", "entry"));
    assert!(has(&highlighted, "keyword", "become"));
    assert!(has(&highlighted, "type.builtin", "int"));
    assert!(has(&highlighted, "function.builtin", "checkSig"));
    assert!(has(&highlighted, "variable.special", "self"));
    assert!(has(&highlighted, "variable.special", "this"));
    assert!(has(&highlighted, "property", "activeInputIndex"));
}

#[test]
fn query_keeps_highlighting_an_incomplete_entry() {
    let source = b"actor Wallet owns WalletState { entry send(int amount) emits";
    let (tree, highlighted) = highlight(&language(), source);
    assert!(tree.root_node().has_error(), "sample should exercise error recovery");

    assert!(has(&highlighted, "keyword", "actor"));
    assert!(has(&highlighted, "keyword", "entry"));
    assert!(has(&highlighted, "type.builtin", "int"));
}
