use std::path::{Path, PathBuf};

use super::format_source;

#[test]
fn reindents_bracket_nesting_to_four_spaces_per_level() {
    let input = "actor Ticket owns TicketState {\n entry redeem(sig s) {\n\t\trequire(redeemed == 0);\n   }\n}\n";
    let expected = "actor Ticket owns TicketState {\n    entry redeem(sig s) {\n        require(redeemed == 0);\n    }\n}\n";
    assert_eq!(format_source(input), expected);
}

#[test]
fn indents_continuation_operator_lines_one_extra_level() {
    let input = "fn f() -> byte[36] {\nbyte[36] outpoint = byte[36](\nOpOutpointTxId(this.activeInputIndex)\n\
                 + (OpOutpointIndex(this.activeInputIndex) as byte[4])\n);\nreturn outpoint;\n}\n";
    let expected = "fn f() -> byte[36] {\n    byte[36] outpoint = byte[36](\n        OpOutpointTxId(this.activeInputIndex)\n            \
                    + (OpOutpointIndex(this.activeInputIndex) as byte[4])\n    );\n    return outpoint;\n}\n";
    assert_eq!(format_source(input), expected);
}

#[test]
fn dedents_lines_that_start_with_closing_brackets() {
    let input = "entry issue(\nsig admin_sig\n) emits {\nticket: Ticket,\n} {\nrequire(true);\n}\n";
    let expected = "entry issue(\n    sig admin_sig\n) emits {\n    ticket: Ticket,\n} {\n    require(true);\n}\n";
    assert_eq!(format_source(input), expected);
}

#[test]
fn normalizes_comma_and_semicolon_spacing() {
    assert_eq!(format_source("f(a ,b,c ,  d) ;\n"), "f(a, b, c, d);\n");
    assert_eq!(format_source("int x = { a: 1,b: 2, };\n"), "int x = { a: 1, b: 2, };\n");
}

#[test]
fn ensures_a_single_space_before_an_attached_brace() {
    assert_eq!(format_source("actor A owns S{\n}\n"), "actor A owns S {\n}\n");
    assert_eq!(format_source("if (x){\n}\n"), "if (x) {\n}\n");
    assert_eq!(format_source("if (x)  {\n}\n"), "if (x) {\n}\n");
    assert_eq!(format_source("f({ a: 1 });\n"), "f({ a: 1 });\n");
}

#[test]
fn keeps_author_spacing_for_braces_that_follow_a_bracket() {
    assert_eq!(format_source("State[2] x = State[2]{ value, local };\n"), "State[2] x = State[2]{ value, local };\n");
    assert_eq!(format_source("fn f() -> byte[32] {\n}\n"), "fn f() -> byte[32] {\n}\n");
}

#[test]
fn collapses_blank_line_runs_and_trims_the_file_edges() {
    assert_eq!(format_source("\n\nconst int A = 1;\n\n\n\nconst int B = 2;\n\n\n"), "const int A = 1;\n\nconst int B = 2;\n");
}

#[test]
fn strips_trailing_whitespace() {
    assert_eq!(format_source("const int A = 1;   \n// note\t\n"), "const int A = 1;\n// note\n");
}

#[test]
fn leaves_strings_untouched() {
    let input = "byte[] d = byte[](\"a ,b  {  // }\");\n";
    assert_eq!(format_source(input), input);
    assert_eq!(format_source("f(\"\\\" ,x\" ,y);\n"), "f(\"\\\" ,x\", y);\n");
}

#[test]
fn leaves_comment_interiors_untouched() {
    let input = "// keep  ,this {  spacing\nconst int A = 1; // and , this {\n";
    assert_eq!(format_source(input), input);
}

#[test]
fn keeps_multi_line_block_comment_interiors_verbatim() {
    let input = "fn f() {\n    /*\n       hand-drawn {  ,  art\n         more */\n    require(true);\n}\n";
    assert_eq!(format_source(input), input);
}

#[test]
fn handles_nested_block_comments_like_the_compiler_lexer() {
    let input = "/* outer /* inner */ still a comment { */\nconst int A = 1;\n";
    assert_eq!(format_source(input), input);
}

#[test]
fn brackets_inside_comments_and_strings_do_not_affect_depth() {
    let input = "fn f() {\n    // if (x) {\n    require(g(\"{\"));\n}\n";
    assert_eq!(format_source(input), input);
}

#[test]
fn does_not_disturb_hex_style_literals() {
    let input = "const byte OWNER_P2PK_SCHNORR = 0x00;\n";
    assert_eq!(format_source(input), input);
}

#[test]
fn indents_line_comments_with_the_surrounding_code() {
    let input = "actor A owns S {\n// leading note\nentry f() {\n}\n}\n";
    let expected = "actor A owns S {\n    // leading note\n    entry f() {\n    }\n}\n";
    assert_eq!(format_source(input), expected);
}

#[test]
fn preserves_crlf_line_endings() {
    assert_eq!(
        format_source("actor A owns S {\r\nentry f() {\r\n}\r\n}\r\n"),
        "actor A owns S {\r\n    entry f() {\r\n    }\r\n}\r\n"
    );
}

#[test]
fn ends_the_file_with_exactly_one_newline() {
    assert_eq!(format_source("const int A = 1;"), "const int A = 1;\n");
    assert_eq!(format_source(""), "\n");
}

#[test]
fn is_tolerant_of_unbalanced_input() {
    assert_eq!(format_source("}\n)\nconst int A = 1;\n"), "}\n)\nconst int A = 1;\n");
    assert_eq!(format_source("fn f() {\nrequire(true);\n"), "fn f() {\n    require(true);\n");
}

fn repository_sources() -> Vec<PathBuf> {
    fn walk(directory: &Path, sources: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).expect("repository directory is readable") {
            let entry = entry.expect("repository entry is readable");
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "target" || name == "node_modules" || name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                walk(&path, sources);
            } else if name.ends_with(".ag") {
                sources.push(path);
            }
        }
    }

    let mut sources = Vec::new();
    walk(Path::new(env!("CARGO_MANIFEST_DIR")), &mut sources);
    sources.sort();
    sources
}

#[test]
fn repository_sources_are_canonical() {
    let sources = repository_sources();
    assert!(!sources.is_empty(), "expected repository .ag sources");
    for source in sources {
        let original = std::fs::read_to_string(&source).expect("repository source is readable");
        let formatted = format_source(&original);
        assert_eq!(formatted, original, "{} is not canonically formatted; run `argentc fmt`", source.display());
    }
}
