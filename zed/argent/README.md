# Argent for Zed

Zed language support for `.ag` source files.

Features:

- Argent keywords, primitive types, builtins, comments, literals, types, and
  calls are syntax highlighted by Argent's dedicated Tree-sitter grammar in
  [`tree-sitter`](../../tree-sitter).
- Code completion covers Argent keywords, primitive types, builtins, and
  declarations from the current file and recursively imported relative files.
- Callable completions insert parameter placeholders.
- Structural indentation, comment toggling, bracket matching, and automatic
  delimiter pairing are configured for Argent source.

## Install for development

The extension uses the language server built into `argentc`. From the Argent
repository root, install the current compiler so Zed can find it on `PATH`:

```sh
cargo install --path . --locked
```

In Zed, run `zed: install dev extension` from the command palette and choose
the repository's `zed/argent` directory. Open any `.ag` file; completions are
shown automatically or with `ctrl-space`.

To run a compiler build without installing it, point Zed at the binary in your
settings. The arguments default to `lsp` when omitted.

```json
{
  "lsp": {
    "argent-language-server": {
      "binary": { "path": "/path/to/argent/target/debug/argentc" }
    }
  }
}
```

## Grammar development

Zed does not compile the grammar from this directory's neighbours. It clones
the `repository` named in `extension.toml`, checks out `rev`, compiles
`tree-sitter/src/parser.c` from that checkout, and stores the result as
`grammars/argent.wasm` inside the extension directory (ignored by git). Two
consequences:

- `rev` must be a commit that exists on the remote before the manifest is
  usable from another machine.
- Local grammar edits are not picked up until they are committed and the
  manifest points at that commit.

For local iteration, commit the grammar change, then temporarily set
`repository` to an absolute `file://` URL of your checkout and `rev` to the new
commit, and run `zed: install dev extension` again:

```toml
[grammars.argent]
repository = "file:///absolute/path/to/argent"
rev = "<local commit>"
path = "tree-sitter"
```

To try uncommitted grammar changes, build the parser yourself and replace the
compiled grammar, then run `zed: reload extensions`:

```sh
cd tree-sitter && npm run build
cp tree-sitter-argent.wasm ../zed/argent/grammars/argent.wasm
```

Restore the GitHub URL and a pushed commit before committing `extension.toml`.

The queries under `languages/argent` are copies of `tree-sitter/queries`. Edit
the grammar copies and run `npm run generate` in `tree-sitter` to regenerate
the parser and sync them; the tests below fail when the copies drift.

## Verify

Run the compiler/LSP tests and validate the grammar, the Zed adapter, and the
queries:

```sh
cargo test lsp
cargo test --manifest-path tree-sitter/Cargo.toml --locked
cargo test --manifest-path zed/argent/Cargo.toml --locked
cargo build --manifest-path zed/argent/Cargo.toml --target wasm32-wasip2 --locked
```
