use std::fs;
use std::path::{Path, PathBuf};

use tree_sitter::{Parser, Query};

fn collect_argent_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_argent_files(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("ag") {
            files.push(path);
        }
    }
}

#[test]
fn parses_all_checked_in_argent_sources_without_errors() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repository root");
    let mut files = Vec::new();
    collect_argent_files(&repository.join("examples"), &mut files);
    collect_argent_files(&repository.join("tests/fixtures"), &mut files);
    collect_argent_files(&repository.join("std"), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no Argent sources found");

    let language = tree_sitter_argent::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language).expect("load Argent grammar");
    let mut failures = Vec::new();

    for file in files {
        let source = fs::read_to_string(&file).unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
        let tree = parser.parse(&source, None).expect("parser returned a tree");
        if tree.root_node().has_error() {
            failures.push(format!("{}\n{}", file.display(), tree.root_node().to_sexp()));
        }
    }

    assert!(failures.is_empty(), "Argent grammar failures:\n{}", failures.join("\n\n"));
}

#[test]
fn highlighting_query_compiles() {
    let language = tree_sitter_argent::LANGUAGE.into();
    Query::new(&language, tree_sitter_argent::HIGHLIGHTS_QUERY).expect("compile Argent highlighting query");
}
