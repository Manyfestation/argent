# tree-sitter-argent

Tree-sitter grammar for the Argent language.

Regenerate checked-in parser sources after editing `grammar.js`:

```sh
npm install
npm run generate
```

Run the Rust smoke tests to parse every checked-in Argent example and fixture:

```sh
cargo test
```
