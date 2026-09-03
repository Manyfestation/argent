# tree-sitter-argent

Tree-sitter grammar for the Argent language. `queries/` holds the canonical
highlighting and indentation queries; the Zed extension in `zed/argent`
carries copies that must stay byte-identical.

After editing `grammar.js` or anything in `queries/`, regenerate the checked-in
parser sources and sync the queries into the Zed extension:

```sh
npm install
npm run generate
```

Build a WebAssembly parser (also regenerates and syncs first):

```sh
npm run build
```

Run the Rust smoke tests, which parse every checked-in Argent example, fixture,
and standard-library file and compile the highlighting query:

```sh
cargo test
```
