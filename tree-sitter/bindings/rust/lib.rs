//! Tree-sitter grammar for the Argent language.

use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_argent() -> *const ();
}

/// The Tree-sitter language function for Argent.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_argent) };

/// The generated static node descriptions.
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

/// The canonical syntax-highlighting query.
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");

#[cfg(test)]
mod tests {
    #[test]
    fn grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).expect("load Argent grammar");
    }
}
