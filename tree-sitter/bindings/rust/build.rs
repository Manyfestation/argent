fn main() {
    let src_dir = std::path::Path::new("src");
    let parser_path = src_dir.join("parser.c");

    cc::Build::new().std("c11").include(src_dir).file(&parser_path).compile("tree-sitter-argent");

    println!("cargo:rerun-if-changed={}", parser_path.display());
}
